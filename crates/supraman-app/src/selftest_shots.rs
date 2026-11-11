//! `--selftest-shots`: the README's screenshots, from fixture sessions only (no `claude agents` listing).

use std::{
    path::{Path, PathBuf},
    sync::{OnceLock, atomic::Ordering},
    time::Duration,
};

use anyhow::{Context as _, anyhow, bail};
use supraman_core::{Folder, FolderKind, Registry, Workspace};
use supraman_workman::fake::{Answer, FakeWorkman};
use gpui::{AsyncWindowContext, Entity, MouseButton};
use serde_json::{Value, json};

use crate::{
    selftest::{home, remove_dir_retrying, sleep},
    selftest_manage::capture,
    view::{FleetView, MenuTarget, TabKind},
};

pub const WORKSPACE: &str = "demo";
const ROOT: &str = r"C:\src\demo";
const FOLDERS: [(&str, &str); 4] = [("api", "main"), ("web", "feature/login"), ("docs", "main"), ("infra", "main")];

const PROJECT: &str = "de000000-0000-4000-8000-000000000001";
const RUN: &str = "de000000-0000-4000-8000-0000000000a1";
/// The fixture folders' `origin`; the fake's project is bound to it.
const REMOTE: &str = "https://github.com/demo/shop.git";
const AS_OF: &str = "2026-11-02T10:00:00+00:00";
const SELECTED: &str = "de000000-0000-4000-8000-000000000202";

static FAKE: OnceLock<FakeWorkman> = OnceLock::new();
static OAUTH: OnceLock<std::sync::Arc<supraman_workman::fake::FakeAuth>> = OnceLock::new();

/// `SUPRAMAN_SHOTS`: where a run that takes screenshots writes them.
pub(crate) fn dir() -> Option<PathBuf> {
    std::env::var_os("SUPRAMAN_SHOTS").map(PathBuf::from)
}

fn out_dir() -> anyhow::Result<PathBuf> {
    dir().ok_or_else(|| anyhow!("--selftest-shots writes to SUPRAMAN_SHOTS, which is unset"))
}

pub fn prepare() -> anyhow::Result<()> {
    std::fs::create_dir_all(out_dir()?)?;
    crate::selftest_seams::FIXTURES_ONLY.store(true, Ordering::SeqCst);
    crate::selftest::install_guards(crate::Mode::SelftestShots);
    let home = home();
    remove_dir_retrying(&home).with_context(|| format!("cannot clear {}", home.display()))?;
    std::fs::create_dir_all(&home)?;
    unsafe { std::env::set_var(supraman_core::HOME_VAR, &home) };
    let folders = FOLDERS
        .iter()
        .map(|(name, branch)| Folder {
            name: (*name).to_string(),
            path: Path::new(ROOT).join(name),
            kind: FolderKind::Git,
            has_claude_md: true,
            has_agents_md: false,
            branch: Some((*branch).to_string()),
            enabled: true,
            added: true,
        })
        .collect();
    Registry::from_env()?.save(&Workspace::new(WORKSPACE, ROOT, folders), true)?;
    let fake = workman();
    OAUTH.set(fake.enable_oauth()).map_err(|_| anyhow!("the fake OAuth server was started twice"))?;
    crate::selftest::set_workman(fake.base())?;
    crate::signin::sign_in(&home, fake.base()).map_err(|e| anyhow!("signing in to the fake Workman: {e}"))?;
    println!(
        "shots: SUPRAMAN_HOME={} ; workspace {WORKSPACE} at {ROOT} (fixture folders, not on disk); fake Workman at {}",
        home.display(),
        fake.base()
    );
    FAKE.set(fake).map_err(|_| anyhow!("the fake Workman was started twice"))
}

fn item(id: &str, qref: &str, kind: &str, title: &str, status: &str) -> Value {
    let ref_ = qref.rsplit('/').next().unwrap_or(qref);
    json!({ "id": id, "ref": ref_, "qualifiedRef": qref, "kind": kind, "title": title, "status": status })
}

/// A fixture folder's repo, not on disk: every one is the demo shop's checkout.
fn fixture_repo(dir: &Path) -> Option<supraman_supervisor::MintRepo> {
    dir.starts_with(ROOT).then(|| supraman_supervisor::MintRepo { root: ROOT.to_string(), remote: Some(REMOTE.to_string()) })
}

/// The fixture's "now": its overview's `asOf`.
fn fixture_now() -> i64 {
    supraman_core::time::rfc3339_ms(AS_OF).unwrap_or_default()
}

fn workman() -> FakeWorkman {
    let project = item(PROJECT, WORKSPACE, "project", "Demo shop", "in_progress");
    let run = item(RUN, "checkout", "run", "Checkout flow", "in_progress");
    let task = |n: &str, title: &str, status: &str| item(&format!("de000000-0000-4000-8000-00000000020{n}"), &format!("checkout/{n}"), "task", title, status);
    let row = |n: &str, title: &str, status: &str, criteria: (u32, u32)| {
        json!({ "item": task(n, title, status), "depth": 1, "hasChildren": false, "criteria": { "checked": criteria.0, "total": criteria.1 }, "blockedBy": [], "flags": [] })
    };
    let mut verified = row("1", "Payment provider adapter", "done", (3, 3));
    verified["verification"] = json!({ "passed": true, "runBy": "orchestrator", "at": "2026-11-02T08:00:00+00:00" });
    let board = json!({
        "asOf": "2026-11-02T10:00:00+00:00", "seq": 10, "run": run, "project": project,
        "rows": [verified, row("2", "Login form validation", "blocked", (1, 4)), row("3", "Order summary page", "in_progress", (0, 2)),
                 row("4", "Receipt email", "open", (0, 0)), row("5", "Refunds from the admin view", "open", (0, 0))]
    });
    let claim = json!({ "item": task("3", "Order summary page", "in_progress"), "run": run, "role": "worker", "agent": "fm-demo-web", "tree": r"C:\src\demo\web",
                        "session": { "id": "s1", "name": "fm-demo-web", "role": "worker", "firstSeenAt": "x", "lastSeenAt": "x" },
                        "acquiredAt": "2026-11-02T09:30:00+00:00", "heartbeatAt": "2026-11-02T09:55:00+00:00", "expiresAt": "2026-11-02T10:26:00+00:00", "expired": false });
    let overview = json!({
        "asOf": "2026-11-02T10:00:00+00:00", "seq": 10, "workingNow": [claim],
        "honesty": { "doneUnverified": 0, "forced": 1, "ungated": 0, "followupsOpen": 3 },
        "projects": [{ "item": project, "rollup": { "total": 5, "done": 1, "inProgress": 1, "blocked": 1, "open": 2 }, "repos": [{ "kind": "remote", "key": "github.com/demo/shop" }],
                       "runs": [{ "item": run, "rollup": { "total": 5, "done": 1, "inProgress": 1, "blocked": 1, "open": 2 } }] }]
    });
    let selected = json!({
        "asOf": "2026-11-02T10:00:00+00:00", "seq": 10, "item": task("2", "Login form validation", "blocked"), "path": [project, run],
        "intent": "The login form says what is wrong with an entry before it is sent.", "verifyCmd": "npm test -- login",
        "openedAt": "2026-11-01T09:00:00+00:00",
        "criteria": [{ "id": "c1", "text": "an empty field is named", "checked": true, "evidence": "login.test.ts: 4 cases" },
                     { "id": "c2", "text": "a wrong password keeps the name typed", "checked": false },
                     { "id": "c3", "text": "the session API answers the check", "checked": false },
                     { "id": "c4", "text": "screen reader announces the error", "checked": false }],
        "notes": [{ "id": "n1", "kind": "question", "body": "Waits for the session API.", "createdAt": "2026-11-02T09:10:00+00:00" }]
    });
    let prompts = |scope: Value, list: Value| json!({ "asOf": "2026-11-02T10:00:00+00:00", "seq": 10, "scope": scope, "prompts": list }).to_string();
    let rows = vec![
        json!({ "key": "question:t1", "kind": "question", "item": task("1", "Payment provider adapter", "in_progress"), "run": run,
                "what": "Card or invoice first?", "since": "2026-11-02T09:40:00+00:00", "call": "" }),
        json!({ "key": "blocked:t2", "kind": "blocked", "item": task("2", "Login form validation", "blocked"), "run": run,
                "what": "Blocked: waits for the session API", "since": "2026-11-02T09:10:00+00:00", "call": "" }),
    ];
    let inbox = json!({ "asOf": "2026-11-02T10:00:00+00:00", "seq": 10, "rows": rows }).to_string();
    let event = |seq: i64, at: &str, n: &str, title: &str, status: &str, kind: &str, summary: &str| {
        json!({ "seq": seq, "at": at, "item": task(n, title, status), "run": run, "actor": "fm-demo-web", "role": "worker", "kind": kind, "summary": summary })
    };
    let journal = json!({ "asOf": "2026-11-02T10:00:00+00:00", "seq": 10, "cursor": 10, "hasMore": false, "events": [
        event(5, "2026-11-01T16:20:00+00:00", "1", "Payment provider adapter", "done", "verification", "checkout/1 green by orchestrator: 12 passed"),
        event(6, "2026-11-01T16:21:00+00:00", "1", "Payment provider adapter", "done", "transition", "checkout/1 review → done"),
        event(7, "2026-11-02T09:10:00+00:00", "2", "Login form validation", "blocked", "transition", "checkout/2 in_progress → blocked"),
        event(8, "2026-11-02T09:30:00+00:00", "3", "Order summary page", "in_progress", "claim", "checkout/3 claimed by fm-demo-web for 30m"),
        event(9, "2026-11-02T09:40:00+00:00", "1", "Payment provider adapter", "in_progress", "note", "question: card or invoice first?")
    ] }).to_string();
    FakeWorkman::start(vec![
        ("/health".into(), Answer::text("Healthy")),
        (format!("/api/v1/inbox?project={PROJECT}"), Answer::json(inbox)),
        (format!("/api/v1/journal?scope={PROJECT}&limit=150"), Answer::json(journal)),
        (format!("/api/v1/prompts?scope={PROJECT}"), Answer::json(prompts(project.clone(), json!([{ "key": "continue", "label": "Continue", "count": null, "text": "Continue project `demo`." },
                                                                                                   { "key": "cleanup", "label": "Clean up", "count": 2, "text": "Clean up project `demo`." }])))),
        ("/api/v1/overview".into(), Answer::json(overview.to_string())),
        (format!("/api/v1/runs/{RUN}/board"), Answer::json(board.to_string())),
        (format!("/api/v1/items/{SELECTED}"), Answer::json(selected.to_string())),
        (
            format!("/api/v1/prompts?scope={RUN}"),
            Answer::json(prompts(run.clone(), json!([{ "key": "resume", "label": "Resume", "count": null, "text": "Resume run `checkout`." },
                                                      { "key": "finalize", "label": "Finalize run", "count": 1, "text": "Finalize run `checkout`." }]))),
        ),
        (format!("/api/v1/prompts?scope={SELECTED}"), Answer::json(prompts(task("2", "Login form validation", "blocked"), json!([{ "key": "unblock", "label": "Unblock", "count": 1, "text": "Unblock `checkout/2`." }])))),
    ])
}

fn rows() -> anyhow::Result<Vec<supraman_claude::Session>> {
    let now = supraman_claude::now_ms();
    let pid = std::process::id();
    let row = |id: &str, name: &str, folder: Option<&str>, kind: &str, state: &str, status: &str, waiting: Option<&str>, age_min: i64| {
        let cwd = folder.map_or_else(|| PathBuf::from(ROOT), |f| Path::new(ROOT).join(f));
        let mut v = serde_json::json!({
            "id": id,
            "sessionId": format!("{id}-0000-4000-8000-00000000demo"),
            "name": name,
            "cwd": cwd,
            "kind": kind,
            "startedAt": now - age_min * 60_000,
            "state": state,
            "status": status,
            "pid": pid,
        });
        if let Some(w) = waiting {
            v["waitingFor"] = w.into();
        }
        v
    };
    let json = serde_json::Value::Array(vec![
        row("d0000001", "fm-demo", None, "background", "working", "busy", None, 95),
        row("d0000002", "fm-demo-api", Some("api"), "background", "working", "busy", None, 42),
        row("d0000003", "fm-demo-web", Some("web"), "background", "blocked", "waiting", Some("permission prompt"), 17),
        row("d0000004", "fm-demo-docs", Some("docs"), "background", "working", "idle", None, 63),
        row("d0000005", "refactor-auth", Some("api"), "interactive", "working", "busy", None, 8),
        row("d0000006", "fm-demo-infra", Some("infra"), "background", "stopped", "idle", None, 240),
    ]);
    Ok(supraman_claude::parse(&json.to_string())?)
}

pub async fn run(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let out = out_dir()?;
    let fixtures = rows()?;
    let count = fixtures.len();
    view.update(cx, |v, cx| v.work_inbox.update(cx, |inbox, _| inbox.use_repo_of(fixture_repo)));
    view.read_with(cx, |v, _| v.polls.injected.lock().unwrap_or_else(|e| e.into_inner()).extend(fixtures));
    crate::selftest::until(view, cx, Duration::from_secs(20), "the fixture sessions in the tree", |v| {
        let tree = v.tree();
        let node = tree.workspaces.iter().find(|n| n.name == WORKSPACE)?;
        (node.rows().count() >= count).then_some(())
    })
    .await?;
    sleep(cx, Duration::from_millis(1500)).await;
    let main = capture(out.join("supraman-main.png"), None, cx).await?;
    println!("shot: {}", main.display());

    let target = MenuTarget::Session {
        workspace: WORKSPACE.to_string(),
        repo: "api".to_string(),
        session_id: "d0000002-0000-4000-8000-00000000demo".to_string(),
    };
    let tab = view.update(cx, |v, cx| v.open_details(target, cx));
    if tab.is_none() {
        bail!("no Details tab for fm-demo-api");
    }
    sleep(cx, Duration::from_millis(1500)).await;
    let details = capture(out.join("supraman-details.png"), None, cx).await?;
    println!("shot: {}", details.display());

    let work = view.update(cx, |v, cx| {
        let tab = v.open_work(WORKSPACE, cx);
        match v.tab(tab).map(|t| &t.kind) {
            Some(TabKind::Work(work)) => Some(work.clone()),
            _ => None,
        }
    });
    let work = work.ok_or_else(|| anyhow!("no Work tab for {WORKSPACE}"))?;
    work.update(cx, |w, _| w.now = fixture_now);
    crate::selftest::until(view, cx, Duration::from_secs(10), "the Work tab's rows", |v| v.hits.borrow().contains_key("work-row-checkout/2").then_some(())).await?;
    work.update(cx, |w, cx| w.select(SELECTED.to_string(), cx));
    crate::selftest::until(view, cx, Duration::from_secs(10), "the selected item in the detail", |v| v.hits.borrow().contains_key("work-pill-checkout/2=stop").then_some(())).await?;
    sleep(cx, Duration::from_millis(800)).await;
    let shot = capture(out.join("supraman-work.png"), None, cx).await?;
    println!("shot: {}", shot.display());
    work_pages(view, &work, &out, cx).await?;
    git_palette(view, &out, cx).await
}

/// The Work tab's Overview, Activity and the open Ctrl+P palette.
async fn work_pages(view: &Entity<FleetView>, work: &Entity<crate::work_view::WorkView>, out: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    use crate::work_view::Page;
    work.update(cx, |w, cx| w.show_page(Page::Overview, cx));
    crate::selftest::until(view, cx, Duration::from_secs(10), "the Overview's projects", |v| v.hits.borrow().contains_key("work-prompt-project-demo-cleanup").then_some(())).await?;
    sleep(cx, Duration::from_millis(800)).await;
    let shot = capture(out.join("supraman-work-overview.png"), None, cx).await?;
    println!("shot: {}", shot.display());
    work.update(cx, |w, cx| w.show_page(Page::Activity, cx));
    crate::selftest::until(view, cx, Duration::from_secs(10), "the Activity's events", |v| v.hits.borrow().contains_key("work-activity-9").then_some(())).await?;
    sleep(cx, Duration::from_millis(800)).await;
    let shot = capture(out.join("supraman-work-activity.png"), None, cx).await?;
    println!("shot: {}", shot.display());
    cx.update(|window, cx| work.update(cx, |w, cx| w.open_palette(window, cx)))?;
    crate::selftest::until(view, cx, Duration::from_secs(10), "the palette's rows", |v| v.hits.borrow().contains_key("work-find-selected").then_some(())).await?;
    sleep(cx, Duration::from_millis(800)).await;
    let shot = capture(out.join("supraman-work-palette.png"), None, cx).await?;
    println!("shot: {}", shot.display());
    cx.update(|window, cx| work.update(cx, |w, cx| w.close_palette(window, cx)))?;
    Ok(())
}

/// A Git tab of a fixture repo with its Ctrl+P palette open.
async fn git_palette(view: &Entity<FleetView>, out: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let parent = home().join("git-fixture");
    std::fs::create_dir_all(&parent)?;
    let fixture = cx.background_executor().spawn(async move { supraman_gitman::fixture::Fixture::new_in(&parent) }).await;
    let dir = fixture.dir.clone();
    view.update(cx, |v, cx| v.open_git(dir.clone(), "Git · shop".into(), cx));
    let git = crate::selftest::until_app(view, cx, Duration::from_secs(20), "the Git tab", |v, cx| crate::selftest_git::git_tabs(v, cx, &dir).first().map(|(_, g)| g.clone())).await?;
    crate::selftest::until(view, cx, Duration::from_secs(20), "the graph's rows", |v| v.hits.borrow().contains_key("git-row-0").then_some(())).await?;
    cx.update(|window, cx| git.update(cx, |g, cx| g.open_find(None, window, cx)))?;
    crate::selftest::until(view, cx, Duration::from_secs(10), "the Git palette's rows", |v| v.hits.borrow().contains_key("git-find-selected").then_some(())).await?;
    sleep(cx, Duration::from_millis(800)).await;
    let shot = capture(out.join("supraman-git-palette.png"), None, cx).await?;
    println!("shot: {}", shot.display());
    cx.update(|window, cx| git.update(cx, |g, cx| g.close_find(window, cx)))?;
    git_wip(view, &git, out, cx).await?;
    git_agent(view, &dir, out, cx).await?;
    drop(fixture);
    Ok(())
}

/// A session working in the same tab's worktree: its agent row, its page as the fleet gives it, and the sidebar's menu on a right-click.
/// The page and the menu come from `FleetView`, which no unit test builds.
async fn git_agent(view: &Entity<FleetView>, dir: &Path, out: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let folder = dir.to_path_buf();
    cx.background_executor()
        .spawn(async move { Registry::from_env()?.edit(WORKSPACE, supraman_core::Edit::AddFolder { path: folder, enable: false }) })
        .await
        .context("adding the Git fixture to the demo workspace")?;
    let session = json!([{
        "id": "d0000007",
        "sessionId": "d0000007-0000-4000-8000-00000000demo",
        "name": "fm-demo-shop",
        "cwd": dir,
        "kind": "background",
        "startedAt": supraman_claude::now_ms() - 5 * 60_000,
        "state": "working",
        "status": "busy",
        "pid": std::process::id(),
    }]);
    let sessions = supraman_claude::parse(&session.to_string())?;
    view.read_with(cx, |v, _| v.polls.injected.lock().unwrap_or_else(|e| e.into_inner()).extend(sessions));
    crate::selftest::until(view, cx, Duration::from_secs(20), "the session's row in the tree", |v| v.tree().workspaces.iter().find(|n| n.name == WORKSPACE)?.rows().any(|r| r.session.name == "fm-demo-shop").then_some(())).await?;
    crate::selftest::until(view, cx, Duration::from_secs(20), "the session's agent row", |v| v.hits.borrow().contains_key("git-agent-0").then_some(())).await?;
    crate::selftest::click(view, "git-agent-0", cx).await?;
    crate::selftest::until(view, cx, Duration::from_secs(10), "the agent row's page", |v| v.hits.borrow().contains_key("git-agent-section-session").then_some(())).await?;
    sleep(cx, Duration::from_millis(800)).await;
    let shot = capture(out.join("supraman-git-agent.png"), None, cx).await?;
    println!("shot: {}", shot.display());
    let window = cx.update(|window, _| window.viewport_size())?;
    crate::selftest_git::resize(view, gpui::size(window.width * 0.75, window.height), cx).await?;
    crate::selftest::until(view, cx, Duration::from_secs(10), "the agent row's name cut in the narrower window", |v| {
        (v.hits.borrow().contains_key("git-agent-0") && supraman_ui::clipped::cut_at("git-agent-0-name").is_some()).then_some(())
    })
    .await?;
    sleep(cx, Duration::from_millis(800)).await;
    let shot = capture(out.join("supraman-git-agent-narrow.png"), None, cx).await?;
    println!("shot: {}", shot.display());
    crate::selftest_git::resize(view, window, cx).await?;
    crate::selftest::press(view, "git-agent-0", MouseButton::Right, cx).await?;
    crate::selftest::until(view, cx, Duration::from_secs(10), "the session's menu over the Git tab", |v| v.dialogs.menu.as_ref().filter(|m| m.title == "fm-demo-shop").map(|_| ())).await?;
    view.update(cx, |v, cx| {
        v.dialogs.menu = None;
        cx.notify();
    });
    Ok(())
}

/// The same tab's WIP panel: staged and unstaged files and a typed summary in the commit area.
async fn git_wip(view: &Entity<FleetView>, git: &Entity<crate::git_view::GitView>, out: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    git.update(cx, |g, cx| g.draft_commit("Stage, discard and commit from the WIP panel", cx));
    crate::selftest::until(view, cx, Duration::from_secs(10), "the commit area", |v| v.hits.borrow().contains_key("wip-commit").then_some(())).await?;
    sleep(cx, Duration::from_millis(800)).await;
    let shot = capture(out.join("supraman-git-wip.png"), None, cx).await?;
    println!("shot: {}", shot.display());
    Ok(())
}
