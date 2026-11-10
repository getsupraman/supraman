//! `--selftest` step 7: the Work tab against a [`FakeWorkman`] on 127.0.0.1 (never the real server): its stream, the Inbox's
//! cards clicked in the real sidebar and an Attach to the gate's claude session; the board's logic is work-view's tests.

use std::{
    path::Path,
    sync::OnceLock,
    time::{Duration, Instant},
};

use anyhow::{Context as _, anyhow, bail};
use supraman_claude::Session;
use supraman_core::{Folder, Registry, Workspace};
use supraman_workman::fake::{Answer, FakeWorkman};
use gpui::{AsyncWindowContext, Entity, KeyDownEvent, Keystroke, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, PlatformInput};
use serde_json::{Value, json};

use crate::{
    selftest::{click, close_attach, frames, open_attach, scrolled_into_view, sleep},
    view::{FleetView, TabKind},
    work_view::{HOUSEKEEPING_KEY, WorkView},
};

pub const WORKSPACE: &str = "selftest-work";
const PROJECT: &str = "5e1f0000-0000-4000-8000-000000000001";
const LIVE_RUN: &str = "5e1f0000-0000-4000-8000-0000000000a2";
const DONE_RUN: &str = "5e1f0000-0000-4000-8000-0000000000a1";
const TASK_1: &str = "5e1f0000-0000-4000-8000-0000000002a1";
const TASK_2: &str = "5e1f0000-0000-4000-8000-0000000002a2";
const SHIPPED: &str = "5e1f0000-0000-4000-8000-0000000001a1";
/// The workspace folder's `origin`; the fake's project is bound to it (Workman's `project_repo`).
const REMOTE: &str = "https://example.invalid/supraman/selftest-work.git";
const REMOTE_KEY: &str = "example.invalid/supraman/selftest-work";
const LOADED_WITHIN: Duration = Duration::from_secs(5);
const LIVE_WITHIN: Duration = Duration::from_secs(2);

static FAKE: OnceLock<FakeWorkman> = OnceLock::new();
static OAUTH: OnceLock<std::sync::Arc<supraman_workman::fake::FakeAuth>> = OnceLock::new();

/// The blocked row in Needs you, and Housekeeping's header (sw1/1 is under it), open or closed.
pub fn inbox_keys() -> (String, [String; 2]) {
    let housekeeping = ["closed", "open"].map(|state| format!("{HOUSEKEEPING_KEY}={state}"));
    (format!("inbox-ws-blocked:{TASK_2}"), housekeeping)
}

pub fn fake() -> Option<(&'static FakeWorkman, &'static supraman_workman::fake::FakeAuth)> {
    Some((FAKE.get()?, OAUTH.get()?.as_ref()))
}

fn item(id: &str, qref: &str, kind: &str, title: &str, status: &str) -> Value {
    let ref_ = qref.rsplit('/').next().unwrap_or(qref);
    json!({ "id": id, "ref": ref_, "qualifiedRef": qref, "kind": kind, "title": title, "status": status })
}

fn rollup(total: u32, done: u32, in_progress: u32, open: u32, cancelled: u32) -> Value {
    json!({ "total": total, "done": done, "inProgress": in_progress, "review": 0, "blocked": 0, "open": open, "cancelled": cancelled, "superseded": 0 })
}

fn project() -> Value {
    item(PROJECT, WORKSPACE, "project", "Supraman selftest project", "in_progress")
}

fn live_run() -> Value {
    item(LIVE_RUN, "sw2", "run", "The live run", "in_progress")
}

fn done_run() -> Value {
    item(DONE_RUN, "sw1", "run", "The finished run", "done")
}

fn row(item: Value, parent: &str, depth: u32, seq: &str) -> Value {
    json!({ "item": item, "parentId": parent, "depth": depth, "seq": seq, "hasChildren": false, "criteria": { "checked": 0, "total": 0 }, "blockedBy": [], "flags": [] })
}

fn overview(seq: i64) -> String {
    json!({
        "asOf": "2026-10-26T12:00:00+00:00", "seq": seq,
        "projects": [{
            "item": project(), "rollup": rollup(6, 2, 1, 2, 1), "repos": [{ "kind": "remote", "key": REMOTE_KEY }],
            "runs": [
                { "item": live_run(), "rollup": rollup(3, 0, 1, 2, 0), "branch": "sw2/x", "activeClaims": 1, "inboxCount": 1,
                  "lastEventAt": "2026-10-26T11:59:00+00:00", "lastVerification": null },
                { "item": done_run(), "rollup": rollup(3, 2, 0, 0, 1), "activeClaims": 0, "inboxCount": 0,
                  "lastEventAt": "2026-10-25T18:00:00+00:00", "lastVerification": { "passed": true, "at": "2026-10-25T17:55:00+00:00" } }
            ]
        }],
        "workingNow": [], "inboxCount": 1, "honesty": {}
    })
    .to_string()
}

fn live_board(seq: i64, finished: bool) -> String {
    let task1 = item(TASK_1, "sw2/1", "task", "Wire the Work tab", if finished { "done" } else { "in_progress" });
    let mut r1 = row(task1.clone(), LIVE_RUN, 1, "1");
    r1["criteria"] = json!({ "checked": if finished { 3 } else { 1 }, "total": 3 });
    if finished {
        r1["verification"] = json!({ "passed": true, "runBy": "controller", "at": "2026-10-26T12:01:00+00:00" });
        r1["commit"] = json!("c0ffee1234567890");
    } else {
        r1["claim"] = json!({ "item": task1, "run": live_run(), "session": { "name": "aw-selftest" }, "agent": "aw-selftest",
                              "role": "worker", "tree": "P:/w/sw2-1", "expired": false });
        r1["flags"] = json!(["stale_claim"]);
    }
    let mut r2 = row(item(TASK_2, "sw2/2", "task", "Render the board", "open"), LIVE_RUN, 1, "2");
    r2["hasChildren"] = json!(true);
    if !finished {
        r2["blockedBy"] = json!([item(TASK_1, "sw2/1", "task", "Wire the Work tab", "in_progress")]);
    }
    let r21 = row(item("5e1f0000-0000-4000-8000-0000000002b1", "sw2/2.1", "subtask", "Indent by depth", "open"), TASK_2, 2, "2.1");
    // The server lists the run itself first, with a numeric ref.
    let mut run_row = row(item(LIVE_RUN, "13", "run", "The live run", "in_progress"), PROJECT, 0, "13");
    run_row["parentId"] = json!(PROJECT);
    json!({
        "asOf": "2026-10-26T12:00:00+00:00", "seq": seq, "run": live_run(), "project": project(), "intent": "selftest",
        "branch": "sw2/x", "rollup": rollup(3, u32::from(finished), u32::from(!finished), 2, 0),
        "rows": [run_row, r1, r2, r21]
    })
    .to_string()
}

fn done_board() -> String {
    let mut r1 = row(item(SHIPPED, "sw1/1", "task", "Ship it", "done"), DONE_RUN, 1, "1");
    r1["criteria"] = json!({ "checked": 2, "total": 2 });
    r1["verification"] = json!({ "passed": true, "runBy": "worker", "at": "2026-10-25T17:55:00+00:00" });
    r1["commit"] = json!("abc1234def567890");
    r1["flags"] = json!(["done_unverified"]);
    let mut r2 = row(item("5e1f0000-0000-4000-8000-0000000001a2", "sw1/2", "task", "Dropped idea", "cancelled"), DONE_RUN, 1, "2");
    r2["flags"] = json!(["done_unverified"]);
    json!({
        "asOf": "2026-10-26T12:00:00+00:00", "seq": 100, "run": done_run(), "project": project(), "intent": "selftest",
        "rollup": rollup(3, 2, 0, 0, 1), "rows": [r1, r2]
    })
    .to_string()
}

fn item_target(id: &str) -> String {
    format!("/api/v1/items/{id}")
}

fn task1_page(seq: i64, claimed_by: &Session, all_checked: bool) -> String {
    let task1 = item(TASK_1, "sw2/1", "task", "Wire the Work tab", "done");
    let session = json!({ "id": "ws-session-1", "name": claimed_by.name, "role": "worker", "claudeSessionId": claimed_by.session_id });
    json!({
        "asOf": "2026-10-26T12:02:00+00:00", "seq": seq, "item": task1, "path": [project(), live_run()],
        "intent": "The Work tab lists a project's runs, each expandable into its board.",
        "verifyCmd": "target/release/supraman.exe --selftest", "riskTags": [], "priority": 0,
        "openedAt": "2026-10-26T10:00:00+00:00", "startedAt": "2026-10-26T10:05:00+00:00", "closedAt": "2026-10-26T12:01:00+00:00",
        "criteria": [
            { "id": "c1", "text": "runs render in overview order", "checked": true, "evidence": "step 7 painted sw2, sw1", "checkedAt": "2026-10-26T11:00:00+00:00" },
            { "id": "c2", "text": "a row click selects it", "checked": true, "evidence": null, "checkedAt": "2026-10-26T11:10:00+00:00" },
            { "id": "c3", "text": "stream events re-read the board", "checked": all_checked, "evidence": if all_checked { json!("1 re-read for 3 events") } else { Value::Null } }
        ],
        "claim": { "item": task1, "run": live_run(), "session": session, "agent": claimed_by.name, "role": "worker",
                   "tree": "P:/w/sw2-1", "acquiredAt": "2026-10-26T10:05:00+00:00", "heartbeatAt": "2026-10-26T12:00:00+00:00",
                   "expiresAt": "2026-10-26T13:00:00+00:00", "expired": false },
        "reports": [{ "attempt": 1, "session": session, "agent": claimed_by.name, "status": "done",
                      "files": ["crates/supraman-app/src/work_view.rs"], "verify": "supraman.exe --selftest: PASS", "redCheck": "board_to_reload forced to None: step 7 FAIL",
                      "deviations": null, "risks": "frame budget", "notDone": "nothing", "at": "2026-10-26T11:58:00+00:00" }],
        "verifications": [{ "command": "target/release/supraman.exe --selftest", "resultLine": "PASS (8 steps)", "passed": true,
                            "counts": null, "runBy": "controller", "session": null, "redCheck": null, "at": "2026-10-26T12:01:00+00:00" }],
        "artifacts": [{ "kind": "commit", "value": "c0ffee1234567890", "at": "2026-10-26T12:00:30+00:00" }],
        "children": [], "findings": [],
        "notes": [{ "id": "n-1", "kind": "decision", "body": "Closed runs start collapsed.", "from": live_run(), "to": task1,
                    "consumedAt": null, "createdAt": "2026-10-26T10:30:00+00:00", "session": null }],
        "links": { "out": [{ "type": "blocks", "item": item(TASK_2, "sw2/2", "task", "Render the board", "open") }],
                   "in": [{ "type": "relates", "item": item(SHIPPED, "sw1/1", "task", "Ship it", "done") }] },
        "assumptions": [{ "ref": "A1", "text": "The server lists the run row first.", "status": "open", "revisitWhen": null }],
        "events": [
            { "seq": 90, "at": "2026-10-26T10:05:00+00:00", "actor": claimed_by.name, "role": "worker", "kind": "claim", "summary": "claimed", "payload": {} },
            { "seq": 95, "at": "2026-10-26T11:58:00+00:00", "actor": claimed_by.name, "role": "worker", "kind": "report", "summary": "reported done", "payload": {} },
            { "seq": 101, "at": "2026-10-26T12:01:00+00:00", "actor": "controller", "role": "controller", "kind": "verification", "summary": "verified: PASS", "payload": {} }
        ],
        "eventsHasMore": false
    })
    .to_string()
}

/// Shaped like the real server's done task: every field it may null is null, the intent first of all.
fn shipped_page() -> String {
    let session = json!({ "id": "ws-session-0", "name": "aw-old", "role": "worker", "externalRole": null, "origin": null,
                          "launchedBy": null, "claudeSessionId": null, "cwd": null, "tree": null, "client": null,
                          "firstSeenAt": "2026-10-25T09:00:00+00:00", "lastSeenAt": "2026-10-25T17:55:00+00:00" });
    let reports = json!([{ "attempt": 1, "session": session, "agent": null, "startedAt": "2026-10-25T17:00:00+00:00", "endedAt": null,
                           "status": "done", "files": [], "verify": null, "redCheck": null, "deviations": null, "risks": null, "notDone": null,
                           "at": "2026-10-25T17:50:00+00:00" }]);
    let notes = json!([{ "id": "n-0", "kind": "decision", "body": "Ship as is.", "from": null, "to": null, "consumedAt": null,
                         "createdAt": "2026-10-25T17:56:00+00:00", "session": null }]);
    let events = json!([
        { "seq": 38, "at": "2026-10-25T17:50:00+00:00", "item": null, "run": null, "session": session, "actor": "aw-old",
          "role": "worker", "kind": "report", "summary": null, "payload": null },
        { "seq": 39, "at": "2026-10-25T17:55:00+00:00", "item": null, "run": null, "session": session, "actor": "aw-old",
          "role": "worker", "kind": "transition", "summary": "done",
          "payload": { "from": "in_progress", "to": null, "forced": null, "reason": null } }
    ]);
    json!({
        "asOf": "2026-10-26T12:02:00+00:00", "seq": 100, "item": item(SHIPPED, "sw1/1", "task", "Ship it", "done"),
        "path": [project(), done_run()], "intent": null, "verifyCmd": null, "riskTags": [], "priority": 0,
        "severity": null, "source": null, "resolution": "accepted", "closedBy": null, "openedAt": "2026-10-25T09:00:00+00:00",
        "startedAt": null, "closedAt": "2026-10-25T17:55:00+00:00",
        "criteria": [{ "id": "c1", "text": "shipped", "checked": true, "evidence": null, "checkedAt": null },
                     { "id": "c2", "text": "announced", "checked": true, "evidence": null, "checkedAt": null }],
        "claim": null,
        "reports": reports,
        "verifications": [{ "command": "cargo test", "resultLine": "ok", "passed": true, "counts": null, "runBy": "worker",
                            "session": null, "redCheck": null, "at": "2026-10-25T17:55:00+00:00" }],
        "artifacts": [{ "kind": "commit", "value": "abc1234def567890", "at": "2026-10-25T17:54:00+00:00" }],
        "children": [], "findings": [],
        "notes": notes, "links": { "out": [], "in": [] }, "assumptions": [], "events": events,
        "eventsHasMore": false, "imported": null
    })
    .to_string()
}

fn inbox(seq: i64, restarted: bool) -> String {
    let rows = vec![
        json!({ "key": format!("blocked:{TASK_2}"), "kind": "blocked", "item": item(TASK_2, "sw2/2", "task", "Render the board", "blocked"),
                "run": live_run(), "what": "Blocked: waits for the board API", "since": "2026-10-26T11:30:00+00:00",
                "call": "work_status(ref:\"selftest-work/sw2/2\", to:\"in_progress\")",
                "audience": "human", "detail": "waits on: the board API", "actor": "aw-selftest", "tree": null, "target": null, "count": 1,
                "url": "/r/selftest-work/sw2/2" }),
        json!({ "key": format!("done_unverified:{SHIPPED}"), "kind": "done_unverified", "item": item(SHIPPED, "sw1/1", "task", "Ship it", "done"),
                "run": done_run(), "what": "Done, but only the worker's verification says so",
                "since": if restarted { "2026-10-26T12:30:00+00:00" } else { "2026-10-25T17:55:00+00:00" },
                "call": "work_verify(ref:\"selftest-work/sw1/1\", …)",
                "audience": "controller", "detail": null, "actor": "aw-selftest", "tree": null, "target": null, "count": 1, "url": null }),
    ];
    json!({ "asOf": "2026-10-26T12:00:00+00:00", "seq": seq, "rows": rows }).to_string()
}

fn inbox_target() -> String {
    format!("/api/v1/inbox?project={PROJECT}")
}

fn task2_page() -> String {
    json!({
        "asOf": "2026-10-26T12:02:00+00:00", "seq": 120, "item": item(TASK_2, "sw2/2", "task", "Render the board", "blocked"),
        "path": [project(), live_run()], "intent": "Render the board.", "openedAt": "2026-10-26T10:00:00+00:00"
    })
    .to_string()
}

/// A run's own page: what the detail shows while no row is selected.
fn run_page(run: Value, children: Vec<Value>) -> String {
    let children: Vec<Value> = children.into_iter().map(|item| json!({ "item": item, "criteria": { "checked": 0, "total": 0 } })).collect();
    json!({
        "asOf": "2026-10-26T12:02:00+00:00", "seq": 100, "item": run, "path": [project()], "intent": "selftest",
        "openedAt": "2026-10-26T09:00:00+00:00", "children": children
    })
    .to_string()
}

fn prompts_target(scope: &str) -> String {
    format!("/api/v1/prompts?scope={scope}")
}

fn prompts(scope: Value, list: &[(&str, &str, Option<u32>)]) -> String {
    let prompts: Vec<Value> = list.iter().map(|(key, label, count)| json!({ "key": key, "label": label, "count": count, "text": format!("{label} `selftest-work`.") })).collect();
    json!({ "asOf": "2026-10-26T12:00:00+00:00", "seq": 100, "scope": scope, "prompts": prompts }).to_string()
}

fn board_target(run: &str) -> String {
    format!("/api/v1/runs/{run}/board")
}

pub fn prepare(home: &Path, root: &Path) -> anyhow::Result<()> {
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo)?;
    for args in [&["init", "-q"][..], &["config", "remote.origin.url", REMOTE]] {
        let out = supraman_core::process::git(&repo, args).map_err(|e| anyhow!("git {args:?} in {}: {e}", repo.display()))?;
        if !out.success() {
            bail!("git {args:?} in {} failed: {}", repo.display(), out.stderr.trim());
        }
    }
    Registry::from_env()?.save(&Workspace::new(WORKSPACE, root, vec![Folder::new("repo", &repo)]), true)?;
    let fake = FakeWorkman::start(vec![
        ("/health".into(), Answer::text("Healthy")),
        ("/api/v1/overview".into(), Answer::json(overview(100))),
        (board_target(LIVE_RUN), Answer::json(live_board(100, false))),
        (board_target(DONE_RUN), Answer::json(done_board())),
        (item_target(SHIPPED), Answer::json(shipped_page())),
        (item_target(TASK_2), Answer::json(task2_page())),
        (item_target(LIVE_RUN), Answer::json(run_page(live_run(), vec![item(TASK_1, "sw2/1", "task", "Wire the Work tab", "in_progress")]))),
        (item_target(DONE_RUN), Answer::json(run_page(done_run(), vec![item(SHIPPED, "sw1/1", "task", "Ship it", "done")]))),
        (prompts_target(LIVE_RUN), Answer::json(prompts(live_run(), &[("resume", "Resume", None), ("finalize", "Finalize run", Some(2))]))),
        (prompts_target(TASK_2), Answer::json(prompts(item(TASK_2, "sw2/2", "task", "Render the board", "blocked"), &[("unblock", "Unblock", None)]))),
        (inbox_target(), Answer::json(inbox(100, false))),
    ]);
    // signed in to the fake's own OAuth server, so the Workman notice has nothing to say during the gate; step 7d signs out and in again through the UI
    let oauth = fake.enable_oauth();
    crate::selftest::set_workman(fake.base())?;
    crate::signin::sign_in(home, fake.base()).map_err(|e| anyhow!("signing in to the fake Workman: {e}"))?;
    println!(
        "step 1w: workspace {WORKSPACE} = {} (folder repo, origin {REMOTE}); fake Workman at {} (project {WORKSPACE} bound to that remote, runs sw2 live, sw1 done), signed in through its fake OAuth server",
        root.display(),
        fake.base()
    );
    OAUTH.set(oauth).map_err(|_| anyhow!("the fake OAuth server was started twice"))?;
    FAKE.set(fake).map_err(|_| anyhow!("the fake Workman was started twice"))
}

async fn painted(view: &Entity<FleetView>, keys: &[&str], within: Duration, what: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<Duration> {
    let started = Instant::now();
    loop {
        let missing: Vec<&str> = view.read_with(cx, |v, _| {
            let hits = v.hits.borrow();
            keys.iter().copied().filter(|k| !hits.contains_key(*k)).collect()
        });
        if missing.is_empty() {
            return Ok(started.elapsed());
        }
        if started.elapsed() > within {
            bail!("{what}: not painted within {} ms: {missing:?}", within.as_millis());
        }
        sleep(cx, Duration::from_millis(20)).await;
    }
}

fn is_painted(view: &Entity<FleetView>, key: &str, cx: &mut AsyncWindowContext) -> bool {
    let shown = view.read_with(cx, |v, _| v.shown(key));
    crate::selftest::frame_if_asked(cx);
    shown
}

fn escape(cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let event = KeyDownEvent {
        keystroke: Keystroke { modifiers: Modifiers::default(), key: "escape".into(), key_char: None },
        is_held: false,
        prefer_character_input: false,
    };
    cx.update(|window, cx| {
        window.dispatch_event(PlatformInput::KeyDown(event), cx);
    })?;
    Ok(())
}

/// The live run's page the Work tab paints from the fake: the navigator's run, its rows, the run in the detail.
const BOARD: [&str; 5] = ["work-nav-run-sw2", "work-row-sw2/1", "work-row-sw2/2", "work-row-sw2/2.1", "work-pill-sw2=doing"];

/// `--selftest-perf`: the frame budget while the Work tab loads the fake's board.
pub async fn board_frames(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let before = frames::baseline(view, cx).await;
    let recording = frames::start(view, cx);
    click(view, &format!("work-{WORKSPACE}"), cx).await?;
    let loaded = painted(view, &BOARD, LOADED_WITHIN, "the live run's page", cx).await;
    sleep(cx, Duration::from_millis(200)).await;
    let during = recording.stop(view, cx);
    let loaded = loaded?;
    let stats = frames::judge("Work tab board load", &before, &during, view, cx).await?;
    Ok(format!("the board painted {} ms after the click; {stats}", loaded.as_millis()))
}

pub async fn run(view: &Entity<FleetView>, launched: &Session, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let fake = FAKE.get().ok_or_else(|| anyhow!("no fake Workman (prepare did not run)"))?;
    let mut lines = Vec::new();

    let blocked = format!("inbox-ws-blocked:{TASK_2}");
    let closed = format!("{HOUSEKEEPING_KEY}=closed");
    let shown = painted(view, &[&blocked, &closed], LOADED_WITHIN, "the Workman cards in the Inbox", cx).await?;
    crate::selftest_manage::shot("m5-14-inbox", cx).await;
    lines.push(format!("Inbox painted blocked sw2/2 under Needs you and Housekeeping closed (done_unverified sw1/1) {} ms into step 7, before any Work tab", shown.as_millis()));

    let (tab, work, opened) = open_board(view, cx).await?;
    lines.push(opened);
    click(view, "work-filter-all", cx).await?;

    lines.push(stream_events(view, &work, fake, cx).await?);
    lines.push(item_page(view, &work, fake, launched, cx).await.context("the detail")?);
    lines.push(inbox_cards(view, tab, &work, fake, cx).await.context("inbox")?);

    lines.push(reads_only(fake)?);
    lines.push(middle_click_closes(view, tab, cx).await?);
    Ok(lines.join("; "))
}

async fn open_board(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<(u64, Entity<WorkView>, String)> {
    click(view, &format!("work-{WORKSPACE}"), cx).await?;
    let (tab, work) = view
        .read_with(cx, |v, _| {
            v.tabs.iter().find_map(|t| match &t.kind {
                TabKind::Work(w) if v.active == Some(t.id) => Some((t.id, w.clone())),
                _ => None,
            })
        })
        .ok_or_else(|| anyhow!("clicking work-{WORKSPACE} opened no active Work tab"))?;
    let loaded = painted(view, &BOARD, LOADED_WITHIN, "the live run's page", cx).await?;
    crate::selftest_manage::shot("m5-14-work-board", cx).await;
    for absent in ["work-row-13", "work-nav-run-sw1"] {
        if is_painted(view, absent, cx) {
            bail!("{absent} is painted: the run's own row is the page's title, and a closed run waits under N closed");
        }
    }
    let title = view.read_with(cx, |v, _| v.tab(tab).map(|t| t.title.clone()).unwrap_or_default());
    let line = format!(
        "tab \"{title}\": project bound by the folder's origin, navigator | sw2's page (3 rows, depth 1, 1, 2) | sw2 in the detail, {} ms after the click; sw1 under 1 closed",
        loaded.as_millis()
    );
    Ok((tab, work, line))
}

async fn stream_events(view: &Entity<FleetView>, work: &Entity<WorkView>, fake: &FakeWorkman, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let deadline = Instant::now() + LOADED_WITHIN;
    while fake.streams() == 0 && fake.push(": ping") == 0 {
        if Instant::now() > deadline {
            bail!("the app never opened /api/stream on the fake");
        }
        sleep(cx, Duration::from_millis(50)).await;
    }
    let (reads_before, overview_before) = work.read_with(cx, |w, _| (w.board_reads, w.overview_reads));
    fake.set(board_target(LIVE_RUN), Answer::json(live_board(101, true)));
    fake.set("/api/v1/overview", Answer::json(overview(101)));
    let pushed = Instant::now();
    for seq in 101..104 {
        let frame = json!({ "seq": seq, "runId": LIVE_RUN, "itemId": TASK_1, "kind": "transition" }).to_string();
        if fake.push(&frame) == 0 {
            bail!("the app's stream was gone when event {seq} was pushed");
        }
    }
    painted(view, &["work-dot-sw2/1=proven"], LIVE_WITHIN, "sw2/1 verified after the stream events", cx).await?;
    let live_after = pushed.elapsed();
    sleep(cx, Duration::from_millis(500)).await;
    let (reads, overviews) = work.read_with(cx, |w, _| (w.board_reads - reads_before, w.overview_reads - overview_before));
    if reads != 1 || overviews != 1 {
        bail!("3 events within the debounce caused {reads} board and {overviews} overview re-reads, expected 1 and 1");
    }
    if is_painted(view, "work-dot-sw2/1=doing", cx) {
        bail!("sw2/1 is painted both in progress and verified");
    }
    Ok(format!("3 stream events for sw2 → 1 board + 1 overview re-read; sw2/1's dot Verified {} ms after the push", live_after.as_millis()))
}

fn reads_only(fake: &FakeWorkman) -> anyhow::Result<String> {
    let oauth = OAUTH.get().ok_or_else(|| anyhow!("no fake OAuth server"))?;
    let issued = oauth.issued();
    let is_oauth = |t: &str| t.starts_with("/.well-known/") || t.starts_with("/connect/");
    let seen: Vec<_> = fake.seen().into_iter().filter(|s| !is_oauth(&s.target)).collect();
    let token_ok = |s: &supraman_workman::fake::Seen| match s.target.as_str() {
        "/health" => s.authorization.is_none(),
        _ => s.authorization.as_deref().and_then(|a| a.strip_prefix("Bearer ")).is_some_and(|t| issued.iter().any(|i| i == t)),
    };
    if let Some(bad) = seen.iter().find(|s| s.method != "GET" || !token_ok(s)) {
        bail!("a request that is not a GET with an access token of the sign-in (none for /health): {} {}", bad.method, bad.target);
    }
    if let Some(bad) = seen.iter().find(|s| !(s.target.starts_with("/api/v1/") || s.target == "/api/stream" || s.target == "/health")) {
        bail!("a request outside the read API: {}", bad.target);
    }
    Ok(format!("the fake saw {} reads, every one a GET with an access token of the sign-in (none for /health)", seen.len()))
}

async fn middle_click_closes(view: &Entity<FleetView>, tab: u64, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let position = scrolled_into_view(view, &format!("tab-{tab}"), cx).await?.center();
    cx.update(|window, cx| {
        let modifiers = Modifiers::default();
        window.dispatch_event(PlatformInput::MouseDown(MouseDownEvent { button: MouseButton::Middle, position, modifiers, click_count: 1, first_mouse: false }), cx);
        window.dispatch_event(PlatformInput::MouseUp(MouseUpEvent { button: MouseButton::Middle, position, modifiers, click_count: 1 }), cx);
    })?;
    sleep(cx, Duration::from_millis(100)).await;
    if view.read_with(cx, |v, _| v.tab(tab).is_some()) {
        bail!("a middle click on the Work tab left it open");
    }
    Ok("middle click closed the tab".into())
}

async fn item_page(
    view: &Entity<FleetView>,
    work: &Entity<WorkView>,
    fake: &FakeWorkman,
    launched: &Session,
    cx: &mut AsyncWindowContext,
) -> anyhow::Result<String> {
    let mut lines = Vec::new();
    fake.set(item_target(TASK_1), Answer::json(task1_page(110, launched, false)));

    click(view, "work-row-sw2/1", cx).await?;
    let attach_key = format!("work-attach-{}", launched.id);
    let check = |i: usize, done: bool| format!("work-detail-section-criteria-check-{i}={}", if done { "done" } else { "open" });
    let (c1, c2, c3) = (check(0, true), check(1, true), check(2, false));
    let sections = [
        "work-pill-sw2/1=proven",
        "work-detail-copy",
        "work-detail-web",
        "work-detail-section-intent",
        "work-detail-section-criteria",
        "work-detail-section-verify",
        &c1,
        &c2,
        &c3,
        &attach_key,
        "work-tab-evidence",
        "work-row-sw2/2",
    ];
    let opened = painted(view, &sections, LOADED_WITHIN, "sw2/1 in the detail", cx).await?;
    lines.push(format!("clicked sw2/1: the detail beside the list after {} ms (Verified, intent, Done when 2 of 3, Verified by running, Attach)", opened.as_millis()));

    let reads_before = work.read_with(cx, |w, _| w.item_reads);
    fake.set(item_target(TASK_1), Answer::json(task1_page(111, launched, true)));
    let pushed = Instant::now();
    let frame = json!({ "seq": 111, "runId": LIVE_RUN, "itemId": TASK_1, "kind": "criterion" }).to_string();
    if fake.push(&frame) == 0 {
        bail!("the app's stream was gone when the criterion event was pushed");
    }
    painted(view, &[&check(2, true)], LIVE_WITHIN, "c3 checked after the stream event", cx).await?;
    let live_after = pushed.elapsed();
    if is_painted(view, &check(2, false), cx) {
        bail!("c3 is painted both checked and unchecked");
    }
    sleep(cx, Duration::from_millis(400)).await;
    let reads = work.read_with(cx, |w, _| w.item_reads - reads_before);
    if reads != 1 {
        bail!("1 stream event caused {reads} item re-reads, expected 1");
    }
    lines.push(format!("1 stream event → 1 item re-read; c3 painted checked {} ms after the push", live_after.as_millis()));

    crate::selftest_manage::shot("m5-14-work-item", cx).await;
    if let Some(secs) = crate::selftest::hold_secs() {
        println!("step 7: holding the detail {secs} s (SUPRAMAN_SELFTEST_HOLD_SECS)");
        sleep(cx, Duration::from_secs(secs)).await;
    }

    click(view, "work-tab-evidence", cx).await?;
    painted(view, &["work-evidence-report-1", "work-evidence-verification-0=pass"], LIVE_WITHIN, "the Evidence tab", cx).await?;
    click(view, "work-tab-notes", cx).await?;
    painted(view, &["work-detail-section-notes"], LIVE_WITHIN, "the Notes tab", cx).await?;
    click(view, "work-tab-activity", cx).await?;
    painted(view, &["work-event-90", "work-event-95", "work-event-101"], LIVE_WITHIN, "the Activity tab", cx).await?;
    lines.push("Evidence: the report beside the controller's verification; Notes; Activity's 3 events".into());

    escape(cx)?;
    painted(view, &["work-pill-sw2=doing", "work-row-sw2/1"], LOADED_WITHIN, "the run in the detail after Esc", cx).await?;
    if work.read_with(cx, |w, _| w.selected.is_some()) {
        bail!("Escape left a row selected");
    }
    lines.push("Esc cleared the selection: the run in the detail, the list kept".into());

    click(view, "work-row-sw2/1", cx).await?;
    painted(view, &[&attach_key], LOADED_WITHIN, "sw2/1's Attach", cx).await?;
    let attached = open_attach(view, &attach_key, &launched.id, cx).await?;
    let title = attached.title.clone();
    let closed = close_attach(view, &attached, &launched.name, cx).await?;
    lines.push(format!("claim joined {} on claudeSessionId → Attach opened \"{title}\"; {closed}", launched.name));
    Ok(lines.join("; "))
}

async fn inbox_cards(view: &Entity<FleetView>, tab: u64, work: &Entity<WorkView>, fake: &FakeWorkman, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let mut lines: Vec<String> = Vec::new();
    let (blocked, shipped) = (format!("inbox-ws-blocked:{TASK_2}"), format!("inbox-ws-done_unverified:{SHIPPED}"));

    click(view, &blocked, cx).await?;
    painted(view, &["work-pill-sw2/2=stop", "work-prompt-item-unblock"], LOADED_WITHIN, "sw2/2 in the detail from the Inbox", cx).await?;
    let active = view.read_with(cx, |v, _| v.active);
    if active != Some(tab) {
        bail!("the Inbox row activated tab {active:?}, not the workspace's Work tab {tab}");
    }
    lines.push("Inbox row blocked sw2/2 → the Work tab, sw2/2 in the detail (Blocked, its Unblock prompt chip)".into());

    click(view, &format!("{HOUSEKEEPING_KEY}=closed"), cx).await?;
    painted(view, &[&shipped, &format!("{HOUSEKEEPING_KEY}=open")], LIVE_WITHIN, "sw1/1's card under the opened Housekeeping", cx).await?;
    click(view, &format!("inbox-ws-dismiss-done_unverified:{SHIPPED}"), cx).await?;
    crate::selftest::drawn(cx).await?;
    if is_painted(view, &shipped, cx) {
        bail!("× left {shipped} painted");
    }
    if work.read_with(cx, |w, _| w.item.as_ref().map(|o| o.id.clone())).as_deref() != Some(TASK_2) {
        bail!("the × click reached the row under it: the detail left sw2/2");
    }
    lines.push("× hid sw1/1's card, the click not reaching the row under it".into());
    crate::selftest_messages::pointer_off(view, cx).await?;
    fake.set(inbox_target(), Answer::json(inbox(121, true)));
    fake.push(&json!({ "seq": 121, "runId": DONE_RUN, "itemId": SHIPPED, "kind": "transition" }).to_string());
    painted(view, &[&shipped], LIVE_WITHIN, "sw1/1's card back once it started over", cx).await?;

    Ok(lines.join("; "))
}
