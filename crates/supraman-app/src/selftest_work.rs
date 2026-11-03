//! `--selftest` step 7: the Work tab against a [`FakeWorkman`] on 127.0.0.1 (never the real server, C21).

use std::{
    cell::RefCell,
    path::Path,
    rc::Rc,
    sync::OnceLock,
    time::{Duration, Instant},
};

use anyhow::{Context as _, anyhow, bail};
use supraman_claude::Session;
use supraman_core::{Registry, Settings, Workspace};
use supraman_workman::{
    ItemStatus,
    fake::{Answer, FakeWorkman},
};
use gpui::{AsyncWindowContext, Entity, KeyDownEvent, Keystroke, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, PlatformInput};
use serde_json::{Value, json};

use crate::{
    selftest::{click, close_attach, frames, open_attach, scrolled_into_view, sleep},
    view::{FleetView, TabKind},
    work_view::{DISMISSED_FILE, WorkEvent, WorkView, criterion_key, status_key, verification_key},
};

pub const WORKSPACE: &str = "selftest-work";
const PROJECT: &str = "5e1f0000-0000-4000-8000-000000000001";
const LIVE_RUN: &str = "5e1f0000-0000-4000-8000-0000000000a2";
const DONE_RUN: &str = "5e1f0000-0000-4000-8000-0000000000a1";
const TASK_1: &str = "5e1f0000-0000-4000-8000-0000000002a1";
const TASK_2: &str = "5e1f0000-0000-4000-8000-0000000002a2";
const SHIPPED: &str = "5e1f0000-0000-4000-8000-0000000001a1";
const LOADED_WITHIN: Duration = Duration::from_secs(5);
const LIVE_WITHIN: Duration = Duration::from_secs(2);

static FAKE: OnceLock<FakeWorkman> = OnceLock::new();
static OAUTH: OnceLock<std::sync::Arc<supraman_workman::fake::FakeAuth>> = OnceLock::new();

pub fn inbox_keys() -> [String; 2] {
    [format!("inbox-ws-blocked:{TASK_2}"), format!("inbox-ws-done_unverified:{SHIPPED}")]
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
            "item": project(), "rollup": rollup(6, 2, 1, 2, 1),
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

fn inbox(seq: i64, extra: bool, restarted: bool) -> String {
    let mut rows = vec![
        json!({ "key": format!("blocked:{TASK_2}"), "kind": "blocked", "item": item(TASK_2, "sw2/2", "task", "Render the board", "blocked"),
                "run": live_run(), "what": "Blocked: waits for the board API", "since": "2026-10-26T11:30:00+00:00",
                "call": "work_status(ref:\"selftest-work/sw2/2\", to:\"in_progress\")" }),
        json!({ "key": format!("done_unverified:{SHIPPED}"), "kind": "done_unverified", "item": item(SHIPPED, "sw1/1", "task", "Ship it", "done"),
                "run": done_run(), "what": "Done, but only the worker's verification says so",
                "since": if restarted { "2026-10-26T12:30:00+00:00" } else { "2026-10-25T17:55:00+00:00" },
                "call": "work_verify(ref:\"selftest-work/sw1/1\", …)" }),
    ];
    if extra {
        rows.insert(0, json!({ "key": format!("question:{TASK_1}"), "kind": "question", "item": item(TASK_1, "sw2/1", "task", "Wire the Work tab", "done"),
                               "run": live_run(), "what": "Which frame budget?", "since": "2026-10-26T12:10:00+00:00", "call": "" }));
    }
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

fn journal_event(seq: i64, item_: Option<Value>, kind: &str, summary: &str) -> Value {
    json!({ "seq": seq, "at": "2026-10-26T12:00:00+00:00", "item": item_, "run": live_run(), "session": { "name": "aw-selftest" },
            "actor": "aw-selftest", "role": "worker", "kind": kind, "summary": summary, "payload": {} })
}

fn journal(cursor: i64, more: bool, events: Vec<Value>) -> String {
    json!({ "asOf": "2026-10-26T12:00:00+00:00", "seq": 200, "cursor": cursor, "hasMore": more, "events": events }).to_string()
}

fn journal_target(since: Option<i64>, limit: u32) -> String {
    match since {
        Some(since) => format!("/api/v1/journal?scope={PROJECT}&since={since}&limit={limit}"),
        None => format!("/api/v1/journal?scope={PROJECT}&limit={limit}"),
    }
}

fn board_target(run: &str) -> String {
    format!("/api/v1/runs/{run}/board")
}

pub fn prepare(home: &Path, root: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(root)?;
    Registry::from_env()?.save(&Workspace::new(WORKSPACE, root, Vec::new()), true)?;
    let resolved = json!({ "asOf": "2026-10-26T12:00:00+00:00", "seq": 100, "matches": [project()] }).to_string();
    let fake = FakeWorkman::start(vec![
        ("/health".into(), Answer::text("Healthy")),
        (format!("/api/v1/resolve?ref={WORKSPACE}"), Answer::json(resolved)),
        ("/api/v1/overview".into(), Answer::json(overview(100))),
        (board_target(LIVE_RUN), Answer::json(live_board(100, false))),
        (board_target(DONE_RUN), Answer::json(done_board())),
        (item_target(SHIPPED), Answer::json(shipped_page())),
        (item_target(TASK_2), Answer::json(task2_page())),
        (inbox_target(), Answer::json(inbox(100, false, false))),
        (
            format!("/api/v1/resolve?ref={}", crate::selftest::WORKSPACE),
            Answer::json(json!({ "asOf": "2026-10-26T12:00:00+00:00", "seq": 100, "matches": [] }).to_string()),
        ),
        (
            journal_target(None, 50),
            Answer::json(journal(
                152,
                true,
                vec![
                    journal_event(150, Some(item(TASK_1, "sw2/1", "task", "Wire the Work tab", "in_progress")), "claim", "claimed"),
                    journal_event(151, None, "note", "a note on the run"),
                    journal_event(152, Some(item(TASK_2, "sw2/2", "task", "Render the board", "blocked")), "transition", "open → blocked"),
                ],
            )),
        ),
    ]);
    // signed in to the fake's own OAuth server, so the Workman notice has nothing to say during the gate; step 7d signs out and in again through the UI
    let oauth = fake.enable_oauth();
    let mut settings = Settings::load(home)?;
    settings.workman.url = Some(fake.base().to_string());
    settings.save(home)?;
    crate::signin::sign_in(home, fake.base()).map_err(|e| anyhow!("signing in to the fake Workman: {e}"))?;
    println!(
        "step 1w: workspace {WORKSPACE} = {}; fake Workman at {} (project {WORKSPACE}, runs sw2 live, sw1 done), signed in through its fake OAuth server",
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

fn painted_any(view: &Entity<FleetView>, prefix: &str, cx: &mut AsyncWindowContext) -> bool {
    view.read_with(cx, |v, _| v.hits.borrow().keys().any(|k| k.starts_with(prefix)))
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

/// The runs and the live run's board the Work tab paints from the fake.
const BOARD: [&str; 5] = ["work-run-sw2", "work-run-sw1", "work-row-sw2/1", "work-row-sw2/2", "work-row-sw2/2.1"];

/// `--selftest-perf`: the frame budget while the Work tab loads the fake's board.
pub async fn board_frames(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let before = frames::baseline(view, cx).await;
    let recording = frames::start(view, cx);
    click(view, &format!("work-{WORKSPACE}"), cx).await?;
    let loaded = painted(view, &BOARD, LOADED_WITHIN, "runs and the live run's board", cx).await;
    sleep(cx, Duration::from_millis(200)).await;
    let during = recording.stop(view, cx);
    let loaded = loaded?;
    let stats = frames::judge("Work tab board load", &before, &during, view, cx).await?;
    Ok(format!("the board painted {} ms after the click; {stats}", loaded.as_millis()))
}

pub async fn run(view: &Entity<FleetView>, launched: &Session, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let fake = FAKE.get().ok_or_else(|| anyhow!("no fake Workman (prepare did not run)"))?;
    let mut lines = Vec::new();

    let (blocked, shipped) = (format!("inbox-ws-blocked:{TASK_2}"), format!("inbox-ws-done_unverified:{SHIPPED}"));
    let shown = painted(view, &[&blocked, &shipped], LOADED_WITHIN, "the Workman rows in the Inbox", cx).await?;
    crate::selftest_manage::shot("m5-14-inbox", cx).await;
    lines.push(format!("Inbox painted the fake's 2 Workman rows (blocked sw2/2, done_unverified sw1/1) {} ms into step 7, before any Work tab", shown.as_millis()));

    click(view, &format!("work-{WORKSPACE}"), cx).await?;
    let (tab, work) = view
        .read_with(cx, |v, _| {
            v.tabs.iter().find_map(|t| match &t.kind {
                TabKind::Work(w) if v.active == Some(t.id) => Some((t.id, w.clone())),
                _ => None,
            })
        })
        .ok_or_else(|| anyhow!("clicking work-{WORKSPACE} opened no active Work tab"))?;
    let loaded = painted(view, &BOARD, LOADED_WITHIN, "runs and the live run's board", cx).await?;
    crate::selftest_manage::shot("m5-14-work-board", cx).await;
    for absent in ["work-row-13", "work-row-sw1/1"] {
        if is_painted(view, absent, cx) {
            bail!("{absent} is painted: the run's own row must be the header, and the done run must start collapsed");
        }
    }
    let title = view.read_with(cx, |v, _| v.tab(tab).map(|t| t.title.clone()).unwrap_or_default());
    lines.push(format!("tab \"{title}\" painted 2 runs and sw2's 3 rows (depth 1, 1, 2) {} ms after the click; run row as header, sw1 collapsed", loaded.as_millis()));

    click(view, "work-run-sw1", cx).await?;
    let expanded = painted(view, &["work-row-sw1/1", "work-row-sw1/2", &status_key("sw1/1", ItemStatus::Done)], LOADED_WITHIN, "sw1 expanded", cx).await?;
    lines.push(format!("clicked sw1: its 2 rows painted after {} ms", expanded.as_millis()));

    let selected: Rc<RefCell<Vec<String>>> = Rc::default();
    let sink = selected.clone();
    cx.update(|_, cx| {
        cx.subscribe(&work, move |_: Entity<WorkView>, event: &WorkEvent, _| match event {
            WorkEvent::ItemSelected(id) => sink.borrow_mut().push(id.clone()),
        })
        .detach();
    })?;
    click(view, "work-row-sw2/2", cx).await?;
    let now = work.read_with(cx, |w, _| w.selected.clone());
    if now.as_deref() != Some(TASK_2) || selected.borrow().as_slice() != [TASK_2.to_string()] {
        bail!("clicking work-row-sw2/2 selected {now:?}, emitted {:?}; expected {TASK_2}", selected.borrow());
    }
    lines.push("clicked sw2/2: selected and ItemSelected emitted".into());

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
    let done_key = status_key("sw2/1", ItemStatus::Done);
    painted(view, &[&done_key], LIVE_WITHIN, "sw2/1 done after the stream events", cx).await?;
    let live_after = pushed.elapsed();
    sleep(cx, Duration::from_millis(500)).await;
    let (reads, overviews) = work.read_with(cx, |w, _| (w.board_reads - reads_before, w.overview_reads - overview_before));
    if reads != 1 || overviews != 1 {
        bail!("3 events within the debounce caused {reads} board and {overviews} overview re-reads, expected 1 and 1");
    }
    if is_painted(view, &status_key("sw2/1", ItemStatus::InProgress), cx) {
        bail!("sw2/1 is painted both in_progress and done");
    }
    lines.push(format!("3 stream events for sw2 → 1 board + 1 overview re-read; sw2/1 painted done {} ms after the push", live_after.as_millis()));

    lines.push(item_page(view, &work, fake, launched, cx).await.context("item page")?);
    lines.push(inbox_and_journal(view, tab, &work, fake, cx).await.context("inbox and journal")?);

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
    lines.push(format!("the fake saw {} reads, every one a GET with an access token of the sign-in (none for /health)", seen.len()));

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
    lines.push("middle click closed the tab".into());
    Ok(lines.join("; "))
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
    let attach_key = format!("item-attach-{}", launched.id);
    let (c1, c2, c3, v0) = (criterion_key("c1", true), criterion_key("c2", true), criterion_key("c3", false), verification_key(0, true));
    let sections = [
        "item-pane",
        "item-header-sw2/1",
        "item-path-selftest-work",
        "item-path-sw2",
        "item-intent",
        "item-verify-cmd",
        "item-claim",
        &c1,
        &c2,
        &c3,
        "item-report-1",
        &v0,
        &attach_key,
        "item-artifact-c0ffee1234567890",
        "item-note-n-1",
        "item-link-out-blocks-sw2/2",
        "item-link-in-relates-sw1/1",
        "item-assumption-A1",
        "item-events",
    ];
    let opened = painted(view, &sections, LOADED_WITHIN, "sw2/1's item page", cx).await?;
    for absent in ["item-done-unverified", "item-event-", "item-child-", "item-finding-"] {
        if painted_any(view, absent, cx) {
            bail!("{absent}* is painted on sw2/1's page: verified, events collapsed, no children or findings");
        }
    }
    lines.push(format!(
        "clicked sw2/1: item page beside the board after {} ms (criteria 2/3, 1 report \"claimed\", 1 verification \"proven\", claim, artifact, note, 2 links, assumption, events collapsed; no warning)",
        opened.as_millis()
    ));

    let reads_before = work.read_with(cx, |w, _| w.item_reads);
    fake.set(item_target(TASK_1), Answer::json(task1_page(111, launched, true)));
    let pushed = Instant::now();
    let frame = json!({ "seq": 111, "runId": LIVE_RUN, "itemId": TASK_1, "kind": "criterion" }).to_string();
    if fake.push(&frame) == 0 {
        bail!("the app's stream was gone when the criterion event was pushed");
    }
    painted(view, &[&criterion_key("c3", true)], LIVE_WITHIN, "c3 checked after the stream event", cx).await?;
    let live_after = pushed.elapsed();
    if is_painted(view, &criterion_key("c3", false), cx) {
        bail!("c3 is painted both checked and unchecked");
    }
    sleep(cx, Duration::from_millis(400)).await;
    let reads = work.read_with(cx, |w, _| w.item_reads - reads_before);
    if reads != 1 {
        bail!("1 stream event caused {reads} item re-reads, expected 1");
    }
    lines.push(format!("1 stream event → 1 item re-read; c3 painted checked {} ms after the push", live_after.as_millis()));

    crate::selftest_manage::shot("m5-14-work-item", cx).await;
    if let Some(secs) = std::env::var("SUPRAMAN_SELFTEST_HOLD_SECS").ok().and_then(|v| v.parse::<u64>().ok()) {
        println!("step 7: holding the item page {secs} s (SUPRAMAN_SELFTEST_HOLD_SECS)");
        sleep(cx, Duration::from_secs(secs)).await;
    }

    click(view, "item-events", cx).await?;
    painted(view, &["item-event-90", "item-event-95", "item-event-101"], LIVE_WITHIN, "events expanded", cx).await?;

    click(view, "item-link-in-relates-sw1/1", cx).await?;
    painted(view, &["item-header-sw1/1", "item-done-unverified", &verification_key(0, true), "item-report-1"], LOADED_WITHIN, "sw1/1 via the link", cx).await?;
    if is_painted(view, "item-intent", cx) {
        bail!("sw1/1's intent is null, yet an Intent section is painted");
    }
    lines.push("link → sw1/1 (the real server's nulls, intent null: no Intent section): done with only a worker verification, warning painted".into());

    click(view, "item-back", cx).await?;
    painted(view, &["item-header-sw2/1"], LOADED_WITHIN, "Back to sw2/1", cx).await?;
    if is_painted(view, "item-done-unverified", cx) {
        bail!("Back to sw2/1 still paints sw1/1's warning");
    }
    click(view, "item-back", cx).await?;
    sleep(cx, Duration::from_millis(100)).await;
    if painted_any(view, "item-", cx) || work.read_with(cx, |w, _| w.item.is_some()) {
        bail!("the second Back left the item page open");
    }
    painted(view, &["work-row-sw2/1", "work-row-sw2/2"], LIVE_WITHIN, "the board after Back", cx).await?;
    lines.push("Back → sw2/1, Back → the board".into());

    click(view, "work-row-sw2/1", cx).await?;
    painted(view, &["item-header-sw2/1"], LOADED_WITHIN, "sw2/1 again", cx).await?;
    escape(cx)?;
    sleep(cx, Duration::from_millis(100)).await;
    if painted_any(view, "item-", cx) {
        bail!("Escape left the item page open");
    }
    lines.push("Escape closed it".into());

    click(view, "work-row-sw2/1", cx).await?;
    painted(view, &[&attach_key], LOADED_WITHIN, "sw2/1's Attach", cx).await?;
    let attached = open_attach(view, &attach_key, &launched.id, cx).await?;
    let title = attached.title.clone();
    let closed = close_attach(view, &attached, &launched.name, cx).await?;
    lines.push(format!("claim joined {} on claudeSessionId → Attach opened \"{title}\"; {closed}", launched.name));
    Ok(lines.join("; "))
}

async fn inbox_and_journal(view: &Entity<FleetView>, tab: u64, work: &Entity<WorkView>, fake: &FakeWorkman, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let mut lines = Vec::new();
    let (blocked, shipped) = (format!("inbox-ws-blocked:{TASK_2}"), format!("inbox-ws-done_unverified:{SHIPPED}"));
    let question = format!("inbox-ws-question:{TASK_1}");

    click(view, &blocked, cx).await?;
    painted(view, &["item-header-sw2/2"], LOADED_WITHIN, "sw2/2's page from the Inbox", cx).await?;
    let active = view.read_with(cx, |v, _| v.active);
    if active != Some(tab) {
        bail!("the Inbox row activated tab {active:?}, not the workspace's Work tab {tab}");
    }
    lines.push("Inbox row blocked sw2/2 → the Work tab, sw2/2's item page".into());

    click(view, &format!("inbox-ws-dismiss-done_unverified:{SHIPPED}"), cx).await?;
    sleep(cx, Duration::from_millis(100)).await;
    if is_painted(view, &shipped, cx) {
        bail!("× left {shipped} painted");
    }
    if work.read_with(cx, |w, _| w.item.as_ref().map(|o| o.id.clone())).as_deref() != Some(TASK_2) {
        bail!("the × click reached the row under it: the item page left sw2/2");
    }
    let file = view.read_with(cx, |v, _| v.home.clone()).ok_or_else(|| anyhow!("no Supraman home"))?.join(DISMISSED_FILE);
    let deadline = Instant::now() + LIVE_WITHIN;
    loop {
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        if text.contains(&format!("done_unverified:{SHIPPED}")) && text.contains(WORKSPACE) {
            break;
        }
        if Instant::now() > deadline {
            bail!("{} does not hold the dismissal: {text:?}", file.display());
        }
        sleep(cx, Duration::from_millis(50)).await;
    }

    // off the panel first: under a resting pointer the hold keeps the new row back for HOLD_STILL
    let right = view.read_with(cx, |v, _| v.left_width) + gpui::px(200.);
    crate::selftest_left::move_to(gpui::point(right, gpui::px(300.)), cx)?;
    let deadline = Instant::now() + LIVE_WITHIN;
    while view.read_with(cx, |v, _| v.left_held()) {
        if Instant::now() > deadline {
            bail!("the left panel still holds its rows {} ms after the pointer left it", LIVE_WITHIN.as_millis());
        }
        sleep(cx, Duration::from_millis(20)).await;
    }
    fake.set(inbox_target(), Answer::json(inbox(121, true, false)));
    if fake.push(&json!({ "seq": 121, "runId": null, "itemId": null, "kind": "note" }).to_string()) == 0 {
        bail!("the app's stream was gone when the inbox event was pushed");
    }
    painted(view, &[&question, &blocked], LIVE_WITHIN, "the reloaded inbox", cx).await?;
    if is_painted(view, &shipped, cx) {
        bail!("the dismissed {shipped} came back on a reload");
    }
    fake.set(inbox_target(), Answer::json(inbox(122, true, true)));
    fake.push(&json!({ "seq": 122, "runId": DONE_RUN, "itemId": SHIPPED, "kind": "transition" }).to_string());
    painted(view, &[&shipped], LIVE_WITHIN, "sw1/1's row after it started over", cx).await?;
    lines.push(format!("× hid sw1/1's row and wrote {DISMISSED_FILE}; a reload added a question and kept it hidden; a new since showed it again"));

    click(view, "work-journal=closed", cx).await?;
    painted(view, &["work-journal=open", "journal-event-150", "journal-event-151", "journal-event-152", "journal-older"], LOADED_WITHIN, "the journal", cx).await?;
    let event_153 = journal_event(153, Some(item(TASK_1, "sw2/1", "task", "Wire the Work tab", "done")), "verification", "verified: PASS");
    fake.set(journal_target(Some(152), 500), Answer::json(journal(153, false, vec![event_153])));
    let pushed = Instant::now();
    fake.push(&json!({ "seq": 153, "runId": LIVE_RUN, "itemId": TASK_1, "kind": "verification" }).to_string());
    painted(view, &["journal-event-153"], LIVE_WITHIN, "the live journal event", cx).await?;
    let live_after = pushed.elapsed();
    let older = vec![
        journal_event(20, None, "created", "run created"),
        journal_event(40, Some(item(SHIPPED, "sw1/1", "task", "Ship it", "done")), "report", "reported done"),
    ];
    fake.set(journal_target(Some(1), 500), Answer::json(journal(40, false, older)));
    click(view, "journal-older", cx).await?;
    painted(view, &["journal-event-20", "journal-event-40"], LOADED_WITHIN, "the older page", cx).await?;
    sleep(cx, Duration::from_millis(100)).await;
    if is_painted(view, "journal-older", cx) {
        bail!("load older is still offered after a page that reached the start");
    }
    let order: Vec<i64> = work.read_with(cx, |w, _| w.journal.feed.as_ref().map(|f| f.events.iter().map(|e| e.seq).collect()).unwrap_or_default());
    if order != [20, 40, 150, 151, 152, 153] {
        bail!("the journal holds {order:?}, expected 20, 40, 150..153 ascending");
    }
    click(view, "journal-event-40", cx).await?;
    painted(view, &["item-header-sw1/1"], LOADED_WITHIN, "sw1/1 from its journal event", cx).await?;
    lines.push(format!(
        "journal: newest page 150..152 + load older; event 153 painted {} ms after its push; load older → 20, 40, none older offered; event 40 → sw1/1's page",
        live_after.as_millis()
    ));
    crate::selftest_manage::shot("m5-14-work-journal", cx).await;
    if let Some(secs) = std::env::var("SUPRAMAN_SELFTEST_JOURNAL_HOLD_SECS").ok().and_then(|v| v.parse::<u64>().ok()) {
        println!("step 7: holding the journal {secs} s (SUPRAMAN_SELFTEST_JOURNAL_HOLD_SECS)");
        sleep(cx, Duration::from_secs(secs)).await;
    }
    click(view, "work-journal=open", cx).await?;
    painted(view, &["work-run-sw2"], LOADED_WITHIN, "the runs again", cx).await?;
    Ok(lines.join("; "))
}
