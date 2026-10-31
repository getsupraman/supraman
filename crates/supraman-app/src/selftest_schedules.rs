//! `--selftest` step 7y: the Schedules group in the window, against the fake Task Scheduler (C101). `sm` adds and
//! changes the schedule; only the window approves it (C100).

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::{Context as _, anyhow, bail};
use supraman_core::schedule::{self, FakeRegistrar, Fired, Outcome, RunEvent, RunRecord, Status};
use supraman_core::{Folder, FolderKind, Registry, Workspace};
use gpui::{AsyncWindowContext, Entity, MouseButton};

use crate::selftest::{cargo_root, click, double_click, until};
use crate::selftest_manage::{menu_open, press_painted, shot};
use crate::view::FleetView;

pub const FOLDER: &str = "selftest-sched";
const WS: &str = "selftest-sched";
const ID: &str = "gate";
const WITHIN: Duration = Duration::from_secs(10);
const RUN_WITHIN: Duration = Duration::from_secs(60);

/// The one registrar of every selftest window: nothing reaches Task Scheduler.
pub fn registrar() -> Arc<FakeRegistrar> {
    static FAKE: OnceLock<Arc<FakeRegistrar>> = OnceLock::new();
    FAKE.get_or_init(Arc::default).clone()
}

pub fn folder() -> PathBuf {
    cargo_root().join("target").join(FOLDER)
}

fn sm(args: &[&str]) -> anyhow::Result<String> {
    let exe = std::env::current_exe()?.with_file_name(supraman_core::HOOK_EXE);
    let out = supraman_core::process::run(supraman_core::process::command(&exe).args(args), WITHIN, supraman_core::process::Kill::Tree)
        .with_context(|| format!("run {}", exe.display()))?;
    if !out.success() {
        bail!("sm {}: {}{}", args.join(" "), out.stdout, out.stderr);
    }
    Ok(out.stdout)
}

fn row_state(v: &FleetView) -> Option<(Status, bool)> {
    v.schedule_row(WS, ID).map(|r| (r.status, r.running.is_some()))
}

fn painted(v: &FleetView, key: &str) -> bool {
    v.hits.borrow().contains_key(key)
}

async fn confirm_text(view: &Entity<FleetView>, what: &str, needle: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    until(view, cx, WITHIN, what, |v| v.confirm.as_ref().filter(|c| c.text.contains(needle)).map(|c| c.text.clone()))
        .await
        .map_err(|e| e.context(format!("confirm {:?}, action error {:?}", view.read_with(cx, |v, _| v.confirm.as_ref().map(|c| c.text.clone())), view.read_with(cx, |v, _| v.action_error.clone()))))
}

async fn from_menu(view: &Entity<FleetView>, item: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    press_painted(view, &format!("sched-row-{WS}/{ID}"), MouseButton::Right, cx).await?;
    let keys = menu_open(view, &format!("{WS} · Gate"), cx).await?;
    if !keys.contains(&item) {
        bail!("no {item} in the schedule's menu: {keys:?}");
    }
    click(view, &format!("menu-{item}"), cx).await
}

async fn status_is(view: &Entity<FleetView>, want: Status, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    until(view, cx, WITHIN, &format!("the row {want:?}"), |v| (row_state(v).map(|s| s.0) == Some(want)).then_some(()))
        .await
        .map_err(|e| e.context(format!("row {:?}, errors {:?}", view.read_with(cx, |v, _| row_state(v)), view.read_with(cx, |v, _| (v.action_error.clone(), v.sync_errors.clone())))))
}

pub async fn run(view: &Entity<FleetView>, home: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let fake = registrar();
    let task = format!(r"{}\{WS}\{ID}", schedule::task_root(home));
    let has_task = |fake: &FakeRegistrar| fake.paths().contains(&task);
    let dir = folder();
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("gate.cmd"), "@exit /b 0\r\n")?;
    let registry = Registry::at(home);
    registry.save(&Workspace::new(WS, &dir, vec![Folder { kind: FolderKind::Plain, ..Folder::new("gate", &dir) }]), true)?;
    let mut out = Vec::new();

    sm(&["schedule", "add", WS, "--id", ID, "--name", "Gate", "--every", "daily 09:00", "--script", "gate.cmd", "--prompt", "Fix what the gate found."])?;
    status_is(view, Status::Pending, cx).await?;
    let approve = format!("sched-approve-{WS}/{ID}");
    until(view, cx, WITHIN, "Approve painted on the pending row", |v| painted(v, &approve).then_some(())).await?;
    if !fake.paths().is_empty() {
        bail!("a task before any Approve: {:?}", fake.paths());
    }
    out.push("1 added through sm: pending, no task".to_string());

    click(view, &approve, cx).await?;
    let first = confirm_text(view, "Approve's question", "Never approved. The approval covers:", cx).await?;
    for field in ["files.gate.cmd: ", "trigger.at: 09:00", "permissions.profile: read", "caps.max_tokens: 3000000"] {
        if !first.contains(field) {
            bail!("Approve does not show {field}: {first}");
        }
    }
    crate::selftest::scrolled_into_view(view, "confirm-yes", cx).await?;
    shot("sched-7-approve-first", cx).await;
    click(view, "confirm-yes", cx).await?;
    status_is(view, Status::Active, cx).await?;
    until(view, cx, WITHIN, "the fake holds the task", |_| has_task(&fake).then_some(())).await?;
    out.push(format!("2 approved in the window: {task} registered"));

    let writes = fake.writes().len();
    sm(&["schedule", "edit", WS, ID, "--every", "daily 10:00"])?;
    status_is(view, Status::Pending, cx).await?;
    if fake.writes().len() != writes {
        bail!("sm's edit reached Task Scheduler: {:?}", fake.writes());
    }
    double_click(view, &format!("sched-row-{WS}/{ID}"), cx).await?;
    let diff = confirm_text(view, "Approve's diff", "Changed since the approval of", cx).await?;
    if !diff.contains("\n  trigger.at: 09:00 → 10:00") || diff.contains("files.gate.cmd") {
        bail!("Approve's diff is not the one change: {diff}");
    }
    crate::selftest::scrolled_into_view(view, "confirm-yes", cx).await?;
    shot("sched-7-approve-diff", cx).await;
    click(view, "confirm-yes", cx).await?;
    status_is(view, Status::Active, cx).await?;
    until(view, cx, WITHIN, "the task registered again", |_| (fake.writes().len() > writes && has_task(&fake)).then_some(())).await?;
    out.push("3 changed through sm: pending, the diff shows trigger.at 09:00 → 10:00, approved again".into());

    from_menu(view, "pause", cx).await?;
    status_is(view, Status::Paused, cx).await?;
    until(view, cx, WITHIN, "the paused schedule's task gone", |_| (!has_task(&fake)).then_some(())).await?;
    from_menu(view, "resume", cx).await?;
    status_is(view, Status::Active, cx).await?;
    until(view, cx, WITHIN, "the resumed schedule's task back", |_| has_task(&fake).then_some(())).await?;
    out.push("4 paused (task removed) and resumed (task back)".into());

    let runs_before = schedule::runs(home, WS)?.len();
    click(view, &format!("sched-run-{WS}/{ID}"), cx).await?;
    let ended = until(view, cx, RUN_WITHIN, "Run now's run ended", |v| {
        let row = v.schedule_row(WS, ID)?;
        (row.runs.len() > runs_before).then(|| row.last().map(|(r, o)| (r.run.clone(), r.started.map(|s| s.1), o))).flatten()
    })
    .await
    .map_err(|e| e.context(std::fs::read_to_string(schedule::fire_log(home)).unwrap_or_default()))?;
    if (ended.1, ended.2) != (Some(Fired::RunNow), Outcome::NothingToDo) {
        bail!("Run now ended as {ended:?}, not a run_now with nothing to do");
    }
    shot("sched-7-schedules", cx).await;
    from_menu(view, "history", cx).await?;
    let history = confirm_text(view, "History", "History of Gate", cx).await?;
    if !history.lines().nth(1).is_some_and(|l| l.starts_with("nothing to do · ")) {
        bail!("History's newest run is not 'nothing to do': {history}");
    }
    click(view, "confirm-yes", cx).await?;
    out.push(format!("5 Run now: {} nothing to do, first in History", ended.0));

    from_menu(view, "dry-run", cx).await?;
    let dry = confirm_text(view, "the dry run", "Dry run of", cx).await?;
    if !dry.contains("next fires") {
        bail!("the dry run lacks the next fires: {dry}");
    }
    click(view, "confirm-yes", cx).await?;
    out.push("5d dry run reported".into());

    let failed = format!("{ID}-selftest-failed");
    let at = supraman_core::time::now_ms();
    for (ts, event) in [
        (at, RunEvent::Started { trigger: Fired::Task, fingerprint: String::new(), fm_pid: 1 }),
        (at + 1, RunEvent::Ended { outcome: Outcome::Failed, why: "exit 1".into(), tokens: None, ms: 1 }),
    ] {
        schedule::append_run(home, WS, &RunRecord { ts, run: failed.clone(), schedule: ID.into(), event })?;
    }
    let dismiss = format!("inbox-sched-dismiss-{failed}");
    until(view, cx, WITHIN, "the failed run in the Inbox", |v| painted(v, &dismiss).then_some(())).await?;
    let tip = view.read_with(cx, |v, _| v.tray_tooltip());
    if tip != "Supraman · 1 scheduled run needs you" {
        bail!("the tray says {tip:?}");
    }
    click(view, &dismiss, cx).await?;
    until(view, cx, WITHIN, "the dismissed notice gone", |v| (!painted(v, &dismiss) && v.tray_tooltip() == "Supraman").then_some(())).await?;
    out.push("6 a failed run in the Inbox and the tray, dismissed".into());

    from_menu(view, "edit-schedule", cx).await?;
    until(view, cx, WITHIN, "the Edit dialog", |v| painted(v, "sched-edit-kind-weekly").then_some(())).await?;
    click(view, "sched-edit-kind-weekly", cx).await?;
    until(view, cx, WITHIN, "the days", |v| painted(v, "sched-edit-day-fri").then_some(())).await?;
    click(view, "sched-edit-day-fri", cx).await?;
    click(view, "sched-edit-save", cx).await?;
    status_is(view, Status::Pending, cx).await?;
    let saved = schedule::load(home, WS)?.into_iter().find(|s| s.id == ID).ok_or_else(|| anyhow!("the schedule is gone"))?;
    if saved.trigger.to_string() != "weekly mon,fri 10:00" {
        bail!("the edit saved {}", saved.trigger);
    }
    until(view, cx, WITHIN, "the pending schedule's task gone", |_| (!has_task(&fake)).then_some(())).await?;
    out.push("7 edited in the dialog: weekly mon,fri 10:00, pending, task removed".into());

    from_menu(view, "remove-schedule", cx).await?;
    confirm_text(view, "Remove's question", "nothing in any folder is touched", cx).await?;
    click(view, "confirm-yes", cx).await?;
    until(view, cx, WITHIN, "the schedule removed", |v| v.schedule_row(WS, ID).is_none().then_some(())).await?;
    if !schedule::load(home, WS)?.is_empty() || has_task(&fake) || fake.has_folder(&format!(r"{}\{WS}", schedule::task_root(home))) {
        bail!("Remove left the entry or the task: {:?}", fake.paths());
    }
    if !dir.join("gate.cmd").is_file() {
        bail!("Remove touched the folder");
    }
    out.push("8 removed: entry, task and task folder gone, the folder untouched".into());

    registry.remove(WS)?;
    if fake.writes().iter().any(|w| !w.contains(&schedule::task_root(home))) {
        bail!("a write outside this home's task folder: {:?}", fake.writes());
    }
    Ok(out.join("; "))
}
