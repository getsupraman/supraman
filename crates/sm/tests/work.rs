use std::path::Path;
use std::process::Output;
use std::sync::Arc;

use supraman_workman::auth::{form_decode, http_browser};
use supraman_workman::fake::{Answer, FakeAuth, FakeWorkman, closed_base};
use supraman_workman::secret::default_store;
use supraman_workman::Auth;
const RUN: &str = "22222222-0000-4000-8000-000000000004";
const OTHER_RUN: &str = "22222222-0000-4000-8000-00000000dead";
const BACKLOG_RUN: &str = "22222222-0000-4000-8000-0000000000bb";

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!("{}/../supraman-workman/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

/// A scratch home with settings naming `url`; signed in when `oauth` is given.
fn home(url: &str, oauth: Option<&Arc<FakeAuth>>) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("settings.toml"), format!("[workman]
url = '{url}'
")).unwrap();
    if oauth.is_some() {
        Auth::open(dir.path()).sign_in(url, &http_browser()).unwrap();
    }
    dir
}

fn sm(home: &Path, args: &[&str]) -> Output {
    supraman_core::process::command(env!("CARGO_BIN_EXE_sm"))
        .args(args)
        .env("SUPRAMAN_HOME", home)
        .env_remove("APPDATA")
        .output()
        .unwrap()
}

/// Every read carried an access token of the fake's sign-in.
fn oauth_bearer(fake: &FakeWorkman, oauth: &FakeAuth) {
    let issued = oauth.issued();
    for s in fake.seen().iter().filter(|s| s.target.starts_with("/api/")) {
        assert_eq!(s.method, "GET");
        let token = s.authorization.as_deref().and_then(|a| a.strip_prefix("Bearer ")).unwrap_or_default();
        assert!(token.starts_with("eyJ") && issued.iter().any(|t| t == token), "{s:?}");
    }
}

fn refreshes_sent(fake: &FakeWorkman) -> Vec<String> {
    fake.seen()
        .into_iter()
        .filter(|s| s.target == "/connect/token")
        .map(|s| form_decode(&s.body))
        .filter(|f| f.get("grant_type").map(String::as_str) == Some("refresh_token"))
        .map(|f| f.get("refresh_token").cloned().unwrap_or_default())
        .collect()
}

/// Evidence without a token: each refresh token as `rt<n>`, its place among those the fake issued; which one the
/// store held before and after one `sm`; what sm sent; why the fake refused.
fn rt_report(fake: &FakeWorkman, oauth: &FakeAuth, before: Option<&str>, sent_before: usize, after: Option<&str>) -> String {
    let issued: Vec<String> = oauth.issued().into_iter().filter(|t| t.starts_with("fake-rt-")).collect();
    let name = |t: Option<&str>| match t {
        None => "none".to_string(),
        Some(t) => issued.iter().position(|i| i == t).map_or("not issued".into(), |n| format!("rt{n}")),
    };
    let sent = refreshes_sent(fake);
    let all: Vec<String> = sent.iter().map(|t| name(Some(t))).collect();
    format!(
        "F-05: issued rt0..rt{}; store before {}, after {}; this sm sent {:?}; all sent {:?}; refusals {:?}",
        issued.len().saturating_sub(1),
        name(before),
        name(after),
        &all[sent_before.min(all.len())..],
        all,
        oauth.refusals()
    )
}

fn text(out: &[u8]) -> String {
    String::from_utf8_lossy(out).into_owned()
}

fn fake() -> FakeWorkman {
    let json = |t: &str, f: &str| (t.to_string(), Answer::json(fixture(f)));
    FakeWorkman::start(vec![
        json("/api/v1/resolve?ref=autowork", "resolve-autowork.json"),
        json("/api/v1/resolve?ref=m4", "resolve-m4.json"),
        json("/api/v1/resolve?ref=backlog", "resolve-backlog.json"),
        json("/api/v1/overview", "overview.json"),
        json(&format!("/api/v1/runs/{RUN}/board"), "board.json"),
        json(&format!("/api/v1/runs/{OTHER_RUN}/board"), "board-other.json"),
        // another project's backlog run, for "no such run here"
        json(&format!("/api/v1/runs/{BACKLOG_RUN}/board"), "board-other.json"),
        ("/api/v1/resolve?ref=locked".into(), Answer::status(403, r#"{"error":"insufficient_scope"}"#)),
    ])
}

#[test]
fn work_overview_resolves_the_workspace_and_prints_its_runs() {
    let fake = fake();
    let oauth = fake.enable_oauth();
    let home = home(fake.base(), Some(&oauth));
    let out = sm(home.path(), &["work", "overview", "autowork"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let stdout = text(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "autowork  Foreman  10/13 done, 1 in progress, 1 blocked, 1 open  (seq 1234, as of 2026-10-26T10:05:00+00:00)");
    assert_eq!(
        lines[1],
        "  m4  The work browser  1/3 done, 1 in progress, 1 blocked  [in_progress]  branch m4/1-workstate-client  claims 1  inbox 2  last verify pass"
    );
    assert_eq!(lines[2], "  m3  The supervisor  9/10 done, 1 open  [done]");
    assert_eq!(lines[3], "working now");
    assert_eq!(lines[4], "  m4/1  aw-m4-1 (worker)  until 2026-10-26T11:00:00+00:00");
    assert_eq!(lines[5], "inbox 3  done-unverified 0  forced 1  ungated 0  follow-ups open 4");
    assert_eq!(lines.len(), 6, "{stdout}");
    let seen = fake.seen();
    let reads: Vec<&str> = seen.iter().map(|s| s.target.as_str()).filter(|t| t.starts_with("/api/")).collect();
    assert_eq!(reads, ["/api/v1/resolve?ref=autowork", "/api/v1/overview"]);
    oauth_bearer(&fake, &oauth);
    let refreshes = seen.iter().filter(|s| s.target == "/connect/token" && s.body.contains("grant_type=refresh_token")).count();
    assert_eq!(refreshes, 1, "sm refreshed the stored grant once");
    let stored = default_store(home.path()).get(&oauth.issuer()).unwrap().unwrap();
    assert_eq!(oauth.issued().last(), Some(&stored), "and stored the rotated refresh token");
}

#[test]
fn work_board_picks_the_workspaces_run_among_same_named_ones() {
    let fake = fake();
    let oauth = fake.enable_oauth();
    let home = home(fake.base(), Some(&oauth));
    let out = sm(home.path(), &["work", "board", "autowork", "m4"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let stdout = text(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "m4  The work browser  (autowork)  1/3 done, 1 in progress, 1 blocked  (seq 1234)");
    assert_eq!(lines[1], "branch m4/1-workstate-client");
    assert_eq!(
        lines[2],
        "  1        in_progress 1/4   aw-m4-1          -     supraman-workstate: read API v1 client  (1/1 done)  [unconsumed_notes]"
    );
    assert_eq!(lines[3], "    1.1    done        2/2   -                pass  SSE parser");
    assert_eq!(lines[4], "  2        blocked     0/3   -                FAIL  The board view  blocked by m4/1  [done_unverified a_future_flag]");
    assert_eq!(lines.len(), 5, "{stdout}");
    let targets: Vec<String> = fake.seen().into_iter().map(|s| s.target).filter(|t| t.starts_with("/api/")).collect();
    assert_eq!(
        targets,
        [
            "/api/v1/resolve?ref=autowork".to_string(),
            "/api/v1/resolve?ref=m4".into(),
            format!("/api/v1/runs/{OTHER_RUN}/board"),
            format!("/api/v1/runs/{RUN}/board"),
        ],
        "ids in the URLs, another project's m4 skipped"
    );
    oauth_bearer(&fake, &oauth);
}

#[test]
fn work_errors_are_one_line_without_the_token() {
    let fake = fake();
    let oauth = fake.enable_oauth();
    let home = home(fake.base(), Some(&oauth));
    let store = default_store(home.path());
    let one_line = |args: &[&str], want: &str| {
        let before = store.get(&oauth.issuer()).unwrap();
        let sent_before = refreshes_sent(&fake).len();
        let out = sm(home.path(), args);
        let stored = store.get(&oauth.issuer()).unwrap();
        let refreshes = rt_report(&fake, &oauth, before.as_deref(), sent_before, stored.as_deref());
        assert_eq!(out.status.code(), Some(1), "{out:?}\n{refreshes}");
        let stderr = text(&out.stderr);
        assert_eq!(stderr.trim_end(), want, "{refreshes}");
        assert_eq!(stored.as_ref(), oauth.issued().last(), "sm stored the rotated refresh token\n{refreshes}");
        assert_eq!(stderr.lines().count(), 1, "{stderr}");
        for t in oauth.issued() {
            assert!(!stderr.contains(&t) && !text(&out.stdout).contains(&t));
        }
    };
    one_line(&["work", "overview", "nope"], "error: Workman answered HTTP 404: no route /api/v1/resolve?ref=nope");
    one_line(&["work", "overview", "locked"], "error: Workman answered HTTP 403: insufficient_scope");
    one_line(&["work", "board", "autowork", "backlog"], "error: Workman project 'autowork' has no run 'backlog'");

    let free = closed_base();
    let out = sm(self::home(&free, None).path(), &["work", "overview", "autowork"]);
    assert_eq!(text(&out.stderr).trim_end(), format!("error: not signed in to Workman at {free}"));

    let none = tempfile::tempdir().unwrap();
    let out = supraman_core::process::command(env!("CARGO_BIN_EXE_sm"))
        .args(["work", "overview", "autowork"])
        .env("SUPRAMAN_HOME", none.path())
        .env_remove("APPDATA")
        .output()
        .unwrap();
    assert_eq!(text(&out.stderr).trim_end(), format!("error: not signed in to Workman at {}", supraman_core::DEFAULT_WORKMAN_URL));
}
