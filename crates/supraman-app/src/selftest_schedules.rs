//! `--selftest` step 7y: the Schedules group and a schedule's Details tab in the window, against the fake Task
//! Scheduler (C101). `sm` adds and changes the schedule; only the page's Approve approves it (C100).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use anyhow::{Context as _, anyhow, bail};
use supraman_core::schedule::{self, FakeRegistrar, Fired, Outcome, RunEvent, RunRecord, Status, TaskState};
use supraman_core::{Folder, FolderKind, Registry, Workspace};
use gpui::{AsyncWindowContext, Entity, MouseButton};

use crate::selftest::{cargo_root, click, double_click, until};
use crate::selftest_manage::{menu_open, press_painted, shot};
use crate::schedules;
use crate::view::FleetView;
use crate::view::MenuAction;

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

pub(crate) fn sm(args: &[&str]) -> anyhow::Result<String> {
    let exe = supraman_core::install::sm_exe()?;
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
    v.shown(key)
}

/// The schedule's Details tab as text, once it is the active tab and holds `needle`.
async fn page_text(view: &Entity<FleetView>, what: &str, needle: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    until(view, cx, WITHIN, what, |v| {
        let (tab, page) = v.schedule_page(WS, ID)?;
        let row = v.schedule_row(WS, ID)?;
        let text = supraman_ui::details::text(row.title(), &v.page_sections(row, Some(page)));
        (v.active == Some(tab) && text.contains(needle)).then_some(text)
    })
    .await
}

/// `<name>-dark` in the dark default and `<name>-light` in a light theme planted in the selftest home, when
/// `FOREMAN_SHOTS` asks.
pub(crate) async fn themed_shots(home: &Path, name: &str, cx: &mut AsyncWindowContext) {
    if std::env::var_os("FOREMAN_SHOTS").is_none() {
        return;
    }
    shot(&format!("{name}-dark"), cx).await;
    let dir = supraman_ui::themes::themes_dir(home);
    if std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(dir.join("selftest-light.json"), LIGHT)).is_err() {
        return;
    }
    let apply = |name: Option<&str>, cx: &mut AsyncWindowContext| {
        let home = home.to_path_buf();
        cx.update(|_, cx| {
            supraman_ui::themes::apply(cx, name, supraman_ui::themes::TerminalColors::Theme, Some(&home));
        })
        .ok();
    };
    apply(Some("Selftest Light"), cx);
    shot(&format!("{name}-light"), cx).await;
    apply(None, cx);
    std::fs::remove_file(dir.join("selftest-light.json")).ok();
}

static OPENED: Mutex<Option<PathBuf>> = Mutex::new(None);

fn recording_open(path: &Path) -> Result<(), String> {
    *OPENED.lock().unwrap_or_else(|e| e.into_inner()) = Some(path.to_path_buf());
    Ok(())
}

fn other_task(folder: &str, name: &str, exec: &str, now: i64) -> TaskState {
    TaskState {
        folder: folder.into(),
        name: name.into(),
        xml: format!(
            r#"<?xml version="1.0" encoding="UTF-16"?><Task xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task"><Triggers><CalendarTrigger><StartBoundary>2026-11-01T07:00:00</StartBoundary><ScheduleByDay><DaysInterval>1</DaysInterval></ScheduleByDay></CalendarTrigger></Triggers><Actions>{exec}</Actions></Task>"#
        ),
        enabled: true,
        last_run_ms: Some(now - 3_600_000),
        last_result: 0,
        next_run_ms: Some(now + 3_600_000),
    }
}

/// sched2/7: Task Scheduler's tasks that run claude outside Supraman, under the workspace holding their working
/// directory or at the top, read only: a menu of Open in Task Scheduler and Copy action, and no write.
async fn others(view: &Entity<FleetView>, fake: &FakeRegistrar, dir: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    const FOLDER: &str = r"\SelftestOther";
    let now = supraman_core::time::now_ms();
    let owned = format!(r"{FOLDER}\Nightly claude");
    let loose = format!(r"{FOLDER}\Loose claude");
    let planted = [
        other_task(FOLDER, "Nightly claude", &format!("<Exec><Command>claude.exe</Command><Arguments>-p review</Arguments><WorkingDirectory>{}</WorkingDirectory></Exec>", dir.display()), now),
        other_task(FOLDER, "Loose claude", r"<Exec><Command>pwsh.exe</Command><Arguments>-c claude -p hi</Arguments><WorkingDirectory>C:\selftest-nowhere</WorkingDirectory></Exec>", now),
        other_task(FOLDER, "Backup", r"<Exec><Command>robocopy.exe</Command><Arguments>C:\a D:\a</Arguments></Exec>", now),
        other_task(r"\Supraman-selftest\x", "own", "<Exec><Command>claude.exe</Command></Exec>", now),
    ];
    let paths: Vec<String> = planted.iter().map(|t| format!(r"{}\{}", t.folder, t.name)).collect();
    for t in planted {
        fake.plant_task(t);
    }
    let writes = fake.writes().len();
    let (group, owned_row, top, loose_row) = (format!("others-row-{WS}"), format!("other-row-{owned}"), "others-row".to_string(), format!("other-row-{loose}"));
    view.update(cx, |v, cx| v.scan_other_schedules(cx));
    until(view, cx, WITHIN, "the other schedules painted", |v| [&group, &owned_row, &top, &loose_row].iter().all(|k| painted(v, k)).then_some(())).await.map_err(|e| {
        let state = view.read_with(cx, |v, _| {
            let mut keys: Vec<String> = v.hits.borrow().keys().filter(|k| k.contains("other") || k.starts_with("schedules-row")).cloned().collect();
            keys.sort();
            let found = v.other_schedules.as_ref().map(|f| f.tasks.iter().map(|t| t.path()).collect::<Vec<_>>());
            format!("painted {keys:?}; scanned {found:?}, error {:?}; under {WS} {:?}", v.other_scan_error, v.others().of_workspace(WS).len())
        });
        e.context(state)
    })?;
    let (summary, stray) = view.read_with(cx, |v, _| {
        let others = v.others();
        let stray: Vec<String> = v.hits.borrow().keys().filter(|k| k.starts_with("other-row-") && (k.contains("Backup") || k.contains("Supraman-selftest"))).cloned().collect();
        (others.of_workspace(WS).first().map(|t| schedules::other_summary(t, now)), stray)
    });
    if !stray.is_empty() {
        bail!("rows for tasks that are not other schedules: {stray:?}");
    }
    let summary = summary.ok_or_else(|| anyhow!("no other schedule under {WS}"))?;
    if !summary.starts_with("daily at 07:00 · next ") || !summary.ends_with("last OK") {
        bail!("the row reads {summary:?}");
    }

    press_painted(view, &owned_row, MouseButton::Right, cx).await?;
    let keys = menu_open(view, &owned, cx).await?;
    if keys != ["open-task-scheduler", "copy-action"] {
        bail!("the other schedule's menu is {keys:?}");
    }
    let copies = view.read_with(cx, |v, _| v.menu.as_ref().and_then(|m| m.items.iter().find(|i| i.key == "copy-action")).map(|i| i.action.clone()));
    if copies != Some(MenuAction::Copy("claude.exe -p review".into())) {
        bail!("Copy action copies {copies:?}");
    }
    *OPENED.lock().unwrap_or_else(|e| e.into_inner()) = None;
    let real = view.update(cx, |v, _| std::mem::replace(&mut v.open_with, recording_open));
    let clicked = click(view, "menu-open-task-scheduler", cx).await;
    let opened = until(view, cx, WITHIN, "taskschd.msc opened", |_| OPENED.lock().unwrap_or_else(|e| e.into_inner()).clone()).await;
    view.update(cx, |v, _| v.open_with = real);
    clicked?;
    if opened? != Path::new("taskschd.msc") {
        bail!("Open in Task Scheduler opened something else");
    }
    if fake.writes().len() != writes {
        bail!("reading other schedules wrote to Task Scheduler: {:?}", fake.writes());
    }

    for p in &paths {
        fake.unplant(p);
    }
    view.update(cx, |v, cx| v.scan_other_schedules(cx));
    until(view, cx, WITHIN, "the other schedules gone", |v| (!painted(v, &group) && !painted(v, &top)).then_some(())).await?;
    Ok("9 other schedules: one under the workspace, one at the top, the rest left out; Open in Task Scheduler and Copy action, no write".into())
}

const LIGHT: &str =r##"{"themes":[{"name":"Selftest Light","appearance":"light","style":{
"background":"#F3F3F3","editor.background":"#FFFFFF","panel.background":"#F3F3F3","surface.background":"#F3F3F3","elevated_surface.background":"#FFFFFF",
"element.background":"#F0F0F0","element.hover":"#E4E4E4","element.active":"#D8D8D8","element.selected":"#DDE8F5",
"ghost_element.hover":"#E8E8E8","ghost_element.active":"#DCDCDC","ghost_element.selected":"#DDE8F5",
"text":"#1F1F1F","text.muted":"#5A5A5A","text.placeholder":"#6E6E6E","border":"#D0D0D0","border.variant":"#E2E2E2","border.focused":"#005FB8","border.selected":"#005FB8",
"tab_bar.background":"#F3F3F3","tab.active_background":"#FFFFFF","tab.inactive_background":"#F3F3F3","title_bar.background":"#F3F3F3","status_bar.background":"#F3F3F3",
"terminal.background":"#FFFFFF","terminal.foreground":"#1F1F1F",
"warning":"#8A5A00","success":"#1E7A35","error":"#B3261E","info":"#005FB8","hint":"#5A5A5A"}}]}"##;

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

/// 3c: an approved schedule whose slash command file is edited says so, offers Approve and asks in the Inbox; Approve
/// takes the new file and the entry goes. Its own schedule, removed again, so the later steps see one.
async fn changed_command(view: &Entity<FleetView>, dir: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    const CMD: &str = "gate-command";
    let commands = dir.join(".claude").join("commands");
    std::fs::create_dir_all(&commands)?;
    std::fs::write(commands.join("selftest-gate.md"), "Run the gate.\n")?;
    sm(&["schedule", "add", WS, "--id", CMD, "--name", "Gate command", "--every", "weekly mon 09:00", "--command", "/selftest-gate", "--profile", "edit"])?;
    let open = format!("inbox-sched-open-{WS}/{CMD}");
    until(view, cx, WITHIN, "the command schedule's Inbox entry", |v| painted(v, &open).then_some(())).await?;
    click(view, &open, cx).await?;
    let approve = format!("sched-page-approve-{WS}/{CMD}");
    crate::selftest::scrolled_into_view(view, &approve, cx).await?;
    click(view, &approve, cx).await?;
    until(view, cx, WITHIN, "the command schedule active", |v| {
        v.schedule_row(WS, CMD).filter(|r| r.status == Status::Active && !r.command_changed).map(|_| ())
    })
    .await?;
    until(view, cx, WITHIN, "its Inbox entry gone", |v| (!painted(v, &open)).then_some(())).await?;

    std::fs::write(commands.join("selftest-gate.md"), "Run the gate, then fix what it found.\n")?;
    let row = until(view, cx, WITHIN, "the row says its command changed", |v| {
        v.schedule_row(WS, CMD).filter(|r| r.status == Status::Active && r.command_changed).map(|r| r.status_text())
    })
    .await?;
    until(view, cx, WITHIN, "the changed command's Inbox entry", |v| painted(v, &open).then_some(())).await?;
    let tip = view.read_with(cx, |v, _| v.tray_tooltip());
    click(view, &open, cx).await?;
    let page = until(view, cx, WITHIN, "the page with the changed file", |v| {
        let (tab, page) = v.schedule_page(WS, CMD)?;
        let row = v.schedule_row(WS, CMD)?;
        let text = supraman_ui::details::text(row.title(), &v.page_sections(row, Some(page)));
        (v.active == Some(tab) && text.contains("File now")).then_some(text)
    })
    .await?;
    for words in ["Slash command  .claude/commands/selftest-gate.md", "Approved file  sha256 ", "File now  sha256 "] {
        if !page.contains(words) {
            bail!("the page does not say {words:?}: {page}");
        }
    }
    crate::selftest::scrolled_into_view(view, &approve, cx).await?;
    click(view, &approve, cx).await?;
    until(view, cx, WITHIN, "approved again with the new file", |v| {
        v.schedule_row(WS, CMD).filter(|r| r.status == Status::Active && !r.command_changed).map(|_| ())
    })
    .await?;
    until(view, cx, WITHIN, "the entry gone after Approve", |v| (!painted(v, &open) && v.tray_tooltip() == "Supraman").then_some(())).await?;

    view.update(cx, |v, cx| {
        if let Some((tab, _)) = v.schedule_page(WS, CMD) {
            v.close_tab(tab, cx);
        }
        v.remove_schedule(WS.into(), CMD.into(), cx);
    });
    until(view, cx, WITHIN, "the command schedule removed", |v| v.schedule_row(WS, CMD).is_none().then_some(())).await?;
    Ok(format!("3c /selftest-gate edited after Approve: the row says {row:?}, the Inbox asks, the tray says {tip:?}, the page shows the approved and the new file's sha256 and offers Approve; approved again, the entry is gone"))
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

    sm(&["schedule", "add", WS, "--id", ID, "--name", "Gate", "--every", "weekly mon 09:00", "--script", "gate.cmd", "--prompt", "Fix what the gate found.", "--profile", "edit"])?;
    status_is(view, Status::Pending, cx).await?;
    let approve = format!("sched-approve-{WS}/{ID}");
    until(view, cx, WITHIN, "Approve painted on the pending row", |v| painted(v, &approve).then_some(())).await?;
    if !fake.paths().is_empty() {
        bail!("a task before any Approve: {:?}", fake.paths());
    }
    let open = format!("inbox-sched-open-{WS}/{ID}");
    until(view, cx, WITHIN, "the pending schedule's Inbox entry", |v| painted(v, &open).then_some(())).await?;
    let tip = view.read_with(cx, |v, _| v.tray_tooltip());
    if tip != "Supraman · 1 schedule waits for your approval" {
        bail!("the tray says {tip:?}");
    }
    out.push("1 added through sm: pending, no task, an Inbox entry and the tray say it waits".to_string());

    click(view, &open, cx).await?;
    let first = page_text(view, "the Details tab from the Inbox", "Never approved.", cx).await?;
    for words in ["Runs  Every Monday at 09:00", "Profile  edit", "Script  gate.cmd", "Fix what the gate found.", "In the folder's checkout", "Max tokens  3,000,000"] {
        if !first.contains(words) {
            bail!("the page does not say {words:?}: {first}");
        }
    }
    if first.contains("worktree-edit") || first.contains("trigger.at") {
        bail!("the page shows keys: {first}");
    }
    let page_approve = format!("sched-page-approve-{WS}/{ID}");
    crate::selftest::scrolled_into_view(view, &page_approve, cx).await?;
    themed_shots(home, "sched-7-details", cx).await;
    click(view, &page_approve, cx).await?;
    status_is(view, Status::Active, cx).await?;
    until(view, cx, WITHIN, "the fake holds the task", |_| has_task(&fake).then_some(())).await?;
    until(view, cx, WITHIN, "the Inbox entry gone", |v| (!painted(v, &open) && v.tray_tooltip() == "Supraman").then_some(())).await?;
    out.push(format!("2 opened from the Inbox: the page says Every Monday and edit; approved there: {task} registered, the entry gone"));

    let writes = fake.writes().len();
    sm(&["schedule", "edit", WS, ID, "--every", "weekly mon 10:00"])?;
    status_is(view, Status::Pending, cx).await?;
    if fake.writes().len() != writes {
        bail!("sm's edit reached Task Scheduler: {:?}", fake.writes());
    }
    let later = format!("inbox-sched-later-{WS}/{ID}");
    until(view, cx, WITHIN, "the changed schedule's Inbox entry", |v| painted(v, &later).then_some(())).await?;
    click(view, &later, cx).await?;
    until(view, cx, WITHIN, "the dismissed entry gone", |v| (!painted(v, &later) && v.tray_tooltip() == "Supraman").then_some(())).await?;
    let kept = std::fs::read_to_string(home.join("schedules").join("waiting-dismissed.json")).unwrap_or_default();
    if !kept.contains(&format!("{WS}/{ID}@")) {
        bail!("the dismissal is not kept in the home for a restart: {kept:?}");
    }
    sm(&["schedule", "edit", WS, ID, "--every", "weekly mon 10:30"])?;
    until(view, cx, WITHIN, "the entry back once the schedule changed again", |v| painted(v, &later).then_some(())).await?;
    double_click(view, &format!("sched-row-{WS}/{ID}"), cx).await?;
    let diff = page_text(view, "the change since the approval", "Changed since your last approval", cx).await?;
    if !diff.contains("\nTrigger time: 09:00 → 10:30") || diff.contains("File gate.cmd") {
        bail!("the page's change is not the one change: {diff}");
    }
    click(view, &page_approve, cx).await?;
    status_is(view, Status::Active, cx).await?;
    until(view, cx, WITHIN, "the task registered again", |_| (fake.writes().len() > writes && has_task(&fake)).then_some(())).await?;
    out.push("3 changed through sm: an entry, dismissed (kept in schedules/waiting-dismissed.json), back after the next change; the page shows Trigger time 09:00 → 10:30, approved again".into());
    out.push(changed_command(view, &dir, cx).await?);

    let tabs = view.read_with(cx, |v, _| v.tabs.len());
    from_menu(view, "details", cx).await?;
    page_text(view, "Details from the row's menu", "Runs  Every Monday at 10:30", cx).await?;
    if view.read_with(cx, |v, _| v.tabs.len()) != tabs {
        bail!("Details opened a second tab for the same schedule");
    }
    from_menu(view, "pause", cx).await?;
    status_is(view, Status::Paused, cx).await?;
    until(view, cx, WITHIN, "the paused schedule's task gone", |_| (!has_task(&fake)).then_some(())).await?;
    // the page's button turns into Resume with the frame after the poll
    crate::selftest::sleep(cx, Duration::from_millis(300)).await;
    click(view, &format!("sched-page-pause-{WS}/{ID}"), cx).await?;
    status_is(view, Status::Active, cx).await?;
    until(view, cx, WITHIN, "the resumed schedule's task back", |_| has_task(&fake).then_some(())).await?;
    out.push("4 Details from the row's menu focused the one tab; paused from the menu (task removed), resumed on the page (task back)".into());

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
    let history = page_text(view, "Last runs", "Last runs", cx).await?;
    if !history.lines().skip_while(|l| *l != "Last runs").nth(1).is_some_and(|l| l.contains("  Run now · nothing to do")) {
        bail!("the newest of Last runs is not Run now's 'nothing to do': {history}");
    }
    out.push(format!("5 Run now: {} nothing to do, first in Last runs", ended.0));

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

    click(view, &format!("sched-page-edit-{WS}/{ID}"), cx).await?;
    until(view, cx, WITHIN, "the Edit dialog", |v| painted(v, "sched-edit-kind-weekly").then_some(())).await?;
    click(view, "sched-edit-kind-weekly", cx).await?;
    until(view, cx, WITHIN, "the days", |v| painted(v, "sched-edit-day-fri").then_some(())).await?;
    click(view, "sched-edit-day-fri", cx).await?;
    click(view, "sched-edit-save", cx).await?;
    status_is(view, Status::Pending, cx).await?;
    let saved = schedule::load(home, WS)?.into_iter().find(|s| s.id == ID).ok_or_else(|| anyhow!("the schedule is gone"))?;
    if saved.trigger.to_string() != "weekly mon,fri 10:30" {
        bail!("the edit saved {}", saved.trigger);
    }
    until(view, cx, WITHIN, "the pending schedule's task gone", |_| (!has_task(&fake)).then_some(())).await?;
    out.push("7 edited in the dialog from the page: weekly mon,fri 10:30, pending, task removed".into());

    click(view, &format!("sched-page-remove-{WS}/{ID}"), cx).await?;
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
    if let Some(tab) = view.read_with(cx, |v, _| v.schedule_page(WS, ID).map(|(t, _)| t)) {
        view.update(cx, |v, cx| v.close_tab(tab, cx));
    }

    out.push(others(view, &fake, &dir, cx).await?);

    registry.remove(WS)?;
    if fake.writes().iter().any(|w| !w.contains(&schedule::task_root(home))) {
        bail!("a write outside this home's task folder: {:?}", fake.writes());
    }
    Ok(out.join("; "))
}
