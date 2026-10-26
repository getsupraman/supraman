//! `fm work overview|board` against a fake WorkState, with a fake `.env` named by a scratch
//! `FOREMAN_HOME`'s settings.toml. Never the real server or a real token (C21).

use std::path::Path;
use std::process::{Command, Output};

use foreman_workstate::fake::{Answer, FakeWorkState};

const TOKEN: &str = "fake-reader-token";
const RUN: &str = "22222222-0000-4000-8000-000000000004";
const OTHER_RUN: &str = "22222222-0000-4000-8000-00000000dead";
const BACKLOG_RUN: &str = "22222222-0000-4000-8000-0000000000bb";

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!("{}/../foreman-workstate/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

fn fm(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fm"))
        .args(args)
        .env("FOREMAN_HOME", home)
        .env_remove("APPDATA")
        .output()
        .unwrap()
}

fn text(out: &[u8]) -> String {
    String::from_utf8_lossy(out).into_owned()
}

fn fake() -> FakeWorkState {
    let json = |t: &str, f: &str| (t.to_string(), Answer::json(fixture(f)));
    FakeWorkState::start(vec![
        json("/api/v1/resolve?ref=autowork", "resolve-autowork.json"),
        json("/api/v1/resolve?ref=m4", "resolve-m4.json"),
        json("/api/v1/resolve?ref=backlog", "resolve-backlog.json"),
        json("/api/v1/overview", "overview.json"),
        json(&format!("/api/v1/runs/{RUN}/board"), "board.json"),
        json(&format!("/api/v1/runs/{OTHER_RUN}/board"), "board-other.json"),
        // another project's backlog run, for "no such run here"
        json(&format!("/api/v1/runs/{BACKLOG_RUN}/board"), "board-other.json"),
        ("/api/v1/resolve?ref=locked".into(), Answer::status(401, r#"{"error":"token fake-reader-token is not a reader"}"#)),
    ])
}

/// A scratch home whose settings name a fake `.env` and the fake server.
fn home(env: &str, url: &str) -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    let env_path = home.path().join("workstate.env");
    std::fs::write(&env_path, env).unwrap();
    let settings = format!("workstate_env = '{}'\nworkstate_url = '{url}'\n", env_path.display());
    std::fs::write(home.path().join("settings.toml"), settings).unwrap();
    home
}

#[test]
fn work_overview_resolves_the_workspace_and_prints_its_runs() {
    let fake = fake();
    let home = home(&format!("WORKSTATE_CONTROLLER_TOKEN=fake-controller\nWORKSTATE_READER_TOKEN={TOKEN}\n"), &fake.mcp_url());
    let out = fm(home.path(), &["work", "overview", "autowork"]);
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
    assert_eq!(seen.iter().map(|s| s.target.as_str()).collect::<Vec<_>>(), ["/api/v1/resolve?ref=autowork", "/api/v1/overview"]);
    assert!(seen.iter().all(|s| s.method == "GET" && s.authorization.as_deref() == Some("Bearer fake-reader-token")), "{seen:?}");
}

#[test]
fn work_board_picks_the_workspaces_run_among_same_named_ones() {
    let fake = fake();
    // No reader token: the controller token reads.
    let home = home("WORKSTATE_CONTROLLER_TOKEN=fake-controller\n", &fake.mcp_url());
    let out = fm(home.path(), &["work", "board", "autowork", "m4"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let stdout = text(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "m4  The work browser  (autowork)  1/3 done, 1 in progress, 1 blocked  (seq 1234)");
    assert_eq!(lines[1], "branch m4/1-workstate-client");
    assert_eq!(
        lines[2],
        "  1        in_progress 1/4   aw-m4-1          -     foreman-workstate: read API v1 client  (1/1 done)  [unconsumed_notes]"
    );
    assert_eq!(lines[3], "    1.1    done        2/2   -                pass  SSE parser");
    assert_eq!(lines[4], "  2        blocked     0/3   -                FAIL  The board view  blocked by m4/1  [done_unverified a_future_flag]");
    assert_eq!(lines.len(), 5, "{stdout}");
    let targets: Vec<String> = fake.seen().into_iter().map(|s| s.target).collect();
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
    assert!(fake.seen().iter().all(|s| s.authorization.as_deref() == Some("Bearer fake-controller")));
}

#[test]
fn work_errors_are_one_line_without_the_token() {
    let fake = fake();
    let home = home(&format!("WORKSTATE_READER_TOKEN={TOKEN}\n"), &fake.mcp_url());
    let one_line = |args: &[&str], want: &str| {
        let out = fm(home.path(), args);
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        let stderr = text(&out.stderr);
        assert_eq!(stderr.trim_end(), want);
        assert_eq!(stderr.lines().count(), 1, "{stderr}");
        assert!(!stderr.contains(TOKEN) && !text(&out.stdout).contains(TOKEN));
    };
    one_line(&["work", "overview", "nope"], "error: WorkState answered HTTP 404: no route /api/v1/resolve?ref=nope");
    one_line(&["work", "overview", "locked"], "error: WorkState answered HTTP 401: token <redacted> is not a reader");
    one_line(&["work", "board", "autowork", "backlog"], "error: WorkState project 'autowork' has no run 'backlog'");

    let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
    let dead = self::home(&format!("WORKSTATE_READER_TOKEN={TOKEN}\n"), &format!("http://{free}/mcp"));
    let out = fm(dead.path(), &["work", "overview", "autowork"]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let stderr = text(&out.stderr);
    assert!(stderr.starts_with(&format!("error: WorkState at http://{free} is unreachable")), "{stderr}");
    assert_eq!(stderr.lines().count(), 1, "{stderr}");
    assert!(!stderr.contains(TOKEN));

    let none = tempfile::tempdir().unwrap();
    let out = fm(none.path(), &["work", "overview", "autowork"]);
    assert_eq!(text(&out.stderr).trim_end(), "error: no WorkState .env in settings.toml (workstate_env)");
}

#[test]
fn work_is_in_the_help() {
    let home = tempfile::tempdir().unwrap();
    let help = text(&fm(home.path(), &["--help"]).stdout);
    assert!(help.contains("fm work overview <workspace>"), "{help}");
    assert!(help.contains("fm work board <workspace> <run-ref>"), "{help}");
}
