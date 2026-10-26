//! `--selftest` step 7: the Work tab against a [`FakeWorkState`] on 127.0.0.1 (never the real server,
//! C21). [`prepare`] registers workspace [`WORKSPACE`] (an empty scratch dir), starts the fake with a
//! project of that ref (one live run, one done run) and points the scratch home's `settings.toml` at it
//! and at a fake `.env`. [`run`] opens the tab with the row's Work button, checks what it paints,
//! expands the done run, selects a row, then changes a row on the fake, pushes stream events and waits
//! for the new status to be painted.

use std::{
    cell::RefCell,
    path::Path,
    rc::Rc,
    sync::OnceLock,
    time::{Duration, Instant},
};

use anyhow::{anyhow, bail};
use foreman_core::{Registry, Settings, Workspace};
use foreman_workstate::{
    ItemStatus,
    fake::{Answer, FakeWorkState},
};
use gpui::{AsyncWindowContext, Entity, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, PlatformInput};
use serde_json::{Value, json};

use crate::{
    selftest::{click, scrolled_into_view, sleep},
    view::{FleetView, TabKind},
    work_view::{WorkEvent, WorkView, status_key},
};

pub const WORKSPACE: &str = "selftest-work";
const TOKEN: &str = "fake-selftest-reader";
const PROJECT: &str = "5e1f0000-0000-4000-8000-000000000001";
const LIVE_RUN: &str = "5e1f0000-0000-4000-8000-0000000000a2";
const DONE_RUN: &str = "5e1f0000-0000-4000-8000-0000000000a1";
const TASK_1: &str = "5e1f0000-0000-4000-8000-0000000002a1";
const TASK_2: &str = "5e1f0000-0000-4000-8000-0000000002a2";
const LOADED_WITHIN: Duration = Duration::from_secs(5);
const LIVE_WITHIN: Duration = Duration::from_secs(2);

static FAKE: OnceLock<FakeWorkState> = OnceLock::new();

fn item(id: &str, qref: &str, kind: &str, title: &str, status: &str) -> Value {
    let ref_ = qref.rsplit('/').next().unwrap_or(qref);
    json!({ "id": id, "ref": ref_, "qualifiedRef": qref, "kind": kind, "title": title, "status": status })
}

fn rollup(total: u32, done: u32, in_progress: u32, open: u32, cancelled: u32) -> Value {
    json!({ "total": total, "done": done, "inProgress": in_progress, "review": 0, "blocked": 0, "open": open, "cancelled": cancelled, "superseded": 0 })
}

fn project() -> Value {
    item(PROJECT, WORKSPACE, "project", "Foreman selftest project", "in_progress")
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

/// The live run's board; `finished` is after the change the stream announces: sw2/1 done and verified.
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
    // The server lists the run itself first, with a numeric ref (observed live in m4/2's brief).
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
    let mut r1 = row(item("5e1f0000-0000-4000-8000-0000000001a1", "sw1/1", "task", "Ship it", "done"), DONE_RUN, 1, "1");
    r1["criteria"] = json!({ "checked": 2, "total": 2 });
    r1["verification"] = json!({ "passed": true, "runBy": "controller", "at": "2026-10-25T17:55:00+00:00" });
    r1["commit"] = json!("abc1234def567890");
    let mut r2 = row(item("5e1f0000-0000-4000-8000-0000000001a2", "sw1/2", "task", "Dropped idea", "cancelled"), DONE_RUN, 1, "2");
    r2["flags"] = json!(["done_unverified"]);
    json!({
        "asOf": "2026-10-26T12:00:00+00:00", "seq": 100, "run": done_run(), "project": project(), "intent": "selftest",
        "rollup": rollup(3, 2, 0, 0, 1), "rows": [r1, r2]
    })
    .to_string()
}

fn board_target(run: &str) -> String {
    format!("/api/v1/runs/{run}/board")
}

/// Before the window: the workspace, the fake server, and the scratch home's settings and `.env`.
pub fn prepare(home: &Path, root: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(root)?;
    Registry::from_env()?.save(&Workspace::new(WORKSPACE, root, Vec::new()), true)?;
    let resolved = json!({ "asOf": "2026-10-26T12:00:00+00:00", "seq": 100, "matches": [project()] }).to_string();
    let fake = FakeWorkState::start(vec![
        (format!("/api/v1/resolve?ref={WORKSPACE}"), Answer::json(resolved)),
        ("/api/v1/overview".into(), Answer::json(overview(100))),
        (board_target(LIVE_RUN), Answer::json(live_board(100, false))),
        (board_target(DONE_RUN), Answer::json(done_board())),
    ]);
    let env = home.join("workstate-fake.env");
    std::fs::write(&env, format!("WORKSTATE_READER_TOKEN={TOKEN}\n"))?;
    let mut settings = Settings::load(home)?;
    settings.workstate_env = Some(env);
    settings.workstate_url = Some(fake.mcp_url());
    settings.save(home)?;
    println!("step 1w: workspace {WORKSPACE} = {}; fake WorkState at {} (project {WORKSPACE}, runs sw2 live, sw1 done)", root.display(), fake.base());
    FAKE.set(fake).map_err(|_| anyhow!("the fake WorkState was started twice"))
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
    view.read_with(cx, |v, _| v.hits.borrow().contains_key(key))
}

pub async fn run(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let fake = FAKE.get().ok_or_else(|| anyhow!("no fake WorkState (prepare did not run)"))?;
    let mut lines = Vec::new();

    // Open: the Work button on the workspace row; frames measured while the overview and boards load.
    view.update(cx, |v, cx| {
        v.probe.start();
        cx.notify();
    });
    click(view, &format!("work-{WORKSPACE}"), cx).await?;
    let (tab, work) = view
        .read_with(cx, |v, _| {
            v.tabs.iter().find_map(|t| match &t.kind {
                TabKind::Work(w) if v.active == Some(t.id) => Some((t.id, w.clone())),
                _ => None,
            })
        })
        .ok_or_else(|| anyhow!("clicking work-{WORKSPACE} opened no active Work tab"))?;
    let loaded = painted(view, &["work-run-sw2", "work-run-sw1", "work-row-sw2/1", "work-row-sw2/2", "work-row-sw2/2.1"], LOADED_WITHIN, "runs and the live run's board", cx).await;
    sleep(cx, Duration::from_millis(200)).await;
    let stats = view.update(cx, |v, _| v.probe.stop());
    let loaded = loaded?;
    if stats.over_50 > 0 {
        bail!("frames over 50 ms while the board loaded: {stats}");
    }
    for absent in ["work-row-13", "work-row-sw1/1"] {
        if is_painted(view, absent, cx) {
            bail!("{absent} is painted: the run's own row must be the header, and the done run must start collapsed");
        }
    }
    let title = view.read_with(cx, |v, _| v.tab(tab).map(|t| t.title.clone()).unwrap_or_default());
    lines.push(format!("tab \"{title}\" painted 2 runs and sw2's 3 rows (depth 1, 1, 2) {} ms after the click; run row as header, sw1 collapsed; frames while loading: {stats}", loaded.as_millis()));

    // Expand the done run.
    click(view, "work-run-sw1", cx).await?;
    let expanded = painted(view, &["work-row-sw1/1", "work-row-sw1/2", &status_key("sw1/1", ItemStatus::Done)], LOADED_WITHIN, "sw1 expanded", cx).await?;
    lines.push(format!("clicked sw1: its 2 rows painted after {} ms", expanded.as_millis()));

    // Select a row: highlighted, and the seam m4/3 hooks fires.
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

    // Live: the fake's board changes, three events announce it; one re-read paints the new status.
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

    if let Some(secs) = std::env::var("FOREMAN_SELFTEST_HOLD_SECS").ok().and_then(|v| v.parse::<u64>().ok()) {
        println!("step 7: holding the Work tab {secs} s (FOREMAN_SELFTEST_HOLD_SECS)");
        sleep(cx, Duration::from_secs(secs)).await;
    }

    // Read only, with the fake token, and nothing but the read API.
    let seen = fake.seen();
    let bearer = format!("Bearer {TOKEN}");
    if let Some(bad) = seen.iter().find(|s| s.method != "GET" || s.authorization.as_deref() != Some(bearer.as_str())) {
        bail!("a request that is not a GET with the fake reader token: {} {}", bad.method, bad.target);
    }
    if let Some(bad) = seen.iter().find(|s| !(s.target.starts_with("/api/v1/") || s.target == "/api/stream")) {
        bail!("a request outside the read API: {}", bad.target);
    }
    lines.push(format!("the fake saw {} requests, every one a GET with the fake reader token", seen.len()));

    // Middle-click closes it like any tab.
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
