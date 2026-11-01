//! New task… from the workspace menu and New scheduled task… from a folder's: the prompt tab's folder and mode,
//! a typed prompt with quotes, `$`, a non-ASCII letter and a new line, Send, and the argv a stand-in claude got
//! (`supraman.exe --selftest-argv`, a native exe as claude.exe is); then the schedule the session adds raises its
//! banner, whose Open opens the schedule's tab.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context as _, anyhow, bail};
use gpui::{AsyncWindowContext, Entity, Modifiers, MouseButton};

use crate::selftest::{WORKSPACE, click, main_repo_of_gate, sleep, until};
use crate::selftest_manage::{menu_open, press_painted};
use crate::selftest_supervise::{keystroke, type_text};
use crate::view::start::SCHEDULE_ASK;
use crate::view::{FleetView, TabKind};

const WITHIN: Duration = Duration::from_secs(20);
const ASKED: &str = "selftest-asked";

fn argv_file(home: &Path, n: u32) -> PathBuf {
    home.join(format!("start-argv-{n}.json"))
}

pub(crate) async fn run(view: &Entity<FleetView>, home: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let before: Vec<u64> = view.read_with(cx, |v, _| v.tabs.iter().map(|t| t.id).collect());
    let saved = view.read_with(cx, |v, _| v.claude_command.clone());
    let result = check(view, home, cx).await;
    crate::selftest_schedules::sm(&["schedule", "remove", WORKSPACE, ASKED]).ok();
    view.update(cx, |v, cx| {
        v.claude_command = saved;
        let mine: Vec<u64> = v.tabs.iter().map(|t| t.id).filter(|id| !before.contains(id)).collect();
        mine.into_iter().for_each(|id| v.close_tab(id, cx));
        cx.notify();
    });
    result
}

fn fake(view: &Entity<FleetView>, out: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    std::fs::remove_file(out).ok();
    let exe = std::env::current_exe()?;
    let quote = |p: &Path| p.display().to_string().replace('\'', "''");
    view.update(cx, |v, _| v.claude_command = format!("& '{}' --selftest-argv '{}'", quote(&exe), quote(out)));
    Ok(())
}

async fn recorded(out: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<Vec<String>> {
    let deadline = std::time::Instant::now() + WITHIN;
    loop {
        if let Some(argv) = std::fs::read_to_string(out).ok().and_then(|t| serde_json::from_str(&t).ok()) {
            return Ok(argv);
        }
        if std::time::Instant::now() > deadline {
            bail!("the stand-in claude wrote no argv to {} within {} s", out.display(), WITHIN.as_secs());
        }
        sleep(cx, Duration::from_millis(100)).await;
    }
}

/// The page's input, painted focused: only then do typed characters reach it.
async fn start_tab(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<u64> {
    let tab = until(view, cx, WITHIN, "a prompt tab active", |v| v.active.filter(|id| v.start_page(*id).is_some())).await?;
    let deadline = std::time::Instant::now() + WITHIN;
    loop {
        let ready = cx.update(|window, cx| {
            view.read(cx).start_page(tab).is_some_and(|p| {
                let input = p.input.read(cx);
                input.is_focused(window) && input.caret_painted.get()
            })
        })?;
        if ready {
            return Ok(tab);
        }
        if std::time::Instant::now() > deadline {
            bail!("the prompt box was not painted focused within {} s", WITHIN.as_secs());
        }
        sleep(cx, Duration::from_millis(50)).await;
    }
}

fn type_lines(cx: &mut AsyncWindowContext, lines: &[&str]) -> anyhow::Result<()> {
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            keystroke(cx, "enter", None, Modifiers { shift: true, ..Default::default() })?;
        }
        type_text(cx, line)?;
    }
    Ok(())
}

fn has_pair(argv: &[String], flag: &str, value: &str) -> bool {
    argv.windows(2).any(|w| w[0] == flag && w[1] == value)
}

async fn check(view: &Entity<FleetView>, home: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let folder = main_repo_of_gate()?;
    let repo = folder.file_name().context("the main repo has no folder name")?.to_string_lossy().into_owned();
    let plugin = view.read_with(cx, |v, _| v.plugin_dir.clone()).context("this build found no plugins/supraman")?;

    press_painted(view, &format!("ws-row-{WORKSPACE}"), MouseButton::Right, cx).await?;
    let keys = menu_open(view, WORKSPACE, cx).await?;
    if keys.contains(&"task") || !keys.contains(&"new-task") || !keys.contains(&"new-scheduled-task") {
        bail!("the workspace menu holds {keys:?}: New task… and New scheduled task…, no Task…");
    }
    click(view, "menu-new-task", cx).await?;
    let tab = start_tab(view, cx).await?;
    let choices = view.read_with(cx, |v, _| v.workspaces.iter().find(|w| w.name == WORKSPACE).map(crate::view::start::folders)).unwrap_or_default();
    let heading = view.read_with(cx, |v, _| v.start_page(tab).map(|p| (p.scheduled, p.folder.clone(), p.mode)));
    if heading != choices.first().map(|(_, p)| (false, p.clone(), "default")) {
        bail!("New task opened {heading:?}, not a task in {WORKSPACE}'s first folder of {choices:?} in mode default");
    }
    let index = choices
        .iter()
        .position(|(_, p)| supraman_core::paths::path_key(p) == supraman_core::paths::path_key(&folder))
        .ok_or_else(|| anyhow!("{} is not among {WORKSPACE}'s folders {choices:?}", folder.display()))?;
    click(view, &format!("start-folder-{index}"), cx).await?;
    click(view, "start-mode-plan", cx).await?;
    crate::selftest_schedules::themed_shots(home, "sched2-6-start", cx).await;
    let lines = ["Say \"hello\" to 'René' for $5 & 100% `now`", "then stop."];
    let want = lines.join("\n");
    start_tab(view, cx).await?;
    type_lines(cx, &lines)?;
    let typed = view.read_with(cx, |v, cx| v.start_page(tab).map(|p| p.input.read(cx).text().to_string()));
    if typed.as_deref() != Some(want.as_str()) {
        bail!("the prompt box holds {typed:?} after typing {want:?}");
    }
    let out = argv_file(home, 1);
    fake(view, &out, cx)?;
    keystroke(cx, "enter", None, Modifiers::default())?;
    let argv = recorded(&out, cx).await?;
    let (kind_shell, title, cwd) = view.read_with(cx, |v, _| v.tab(tab).map(|t| (matches!(t.kind, TabKind::Shell), t.title.clone(), t.cwd.clone()))).unwrap_or_default();
    if !kind_shell || title != lines[0] || cwd.as_deref() != Some(folder.as_path()) {
        bail!("after Send the tab is shell {kind_shell}, titled {title:?}, in {cwd:?}; expected a session tab titled by the first line in {}", folder.display());
    }
    if argv.last() != Some(&want) || argv.get(argv.len().saturating_sub(2)).map(String::as_str) != Some("--") {
        bail!("the claude argv ends {:?}, not `--` and the prompt {want:?}", argv.iter().rev().take(2).collect::<Vec<_>>());
    }
    let tools = supraman_core::sm_allowed_tools(&plugin);
    if !has_pair(&argv, "--permission-mode", "plan") || !has_pair(&argv, "--plugin-dir", &plugin.display().to_string()) || !argv.windows(tools.len() + 1).any(|w| w[0] == "--allowedTools" && w[1..] == tools[..]) {
        bail!("the claude argv lacks the mode or the [+] flags: {argv:?}");
    }

    press_painted(view, &format!("repo-row-{WORKSPACE}/{repo}"), MouseButton::Right, cx).await?;
    let keys = menu_open(view, &format!("{WORKSPACE} · ./{repo}"), cx).await?;
    if !keys.contains(&"new-scheduled-task") {
        bail!("the folder's menu holds {keys:?}, without New scheduled task…");
    }
    click(view, "menu-new-scheduled-task", cx).await?;
    let tab = start_tab(view, cx).await?;
    let preselected = view.read_with(cx, |v, _| v.start_page(tab).map(|p| (p.scheduled, p.folder.clone(), p.mode)));
    if preselected != Some((true, folder.clone(), "default")) {
        bail!("New scheduled task from {repo}'s menu opened {preselected:?}, not a scheduled task in {} in mode default", folder.display());
    }
    let job = "every monday at 07:00 run the gate";
    type_text(cx, job)?;
    let out = argv_file(home, 2);
    fake(view, &out, cx)?;
    click(view, "start-send", cx).await?;
    let argv = recorded(&out, cx).await?;
    let asked = format!("{SCHEDULE_ASK}\n{job}");
    if argv.last() != Some(&asked) || !has_pair(&argv, "--permission-mode", "default") {
        bail!("the scheduled task's claude got {argv:?}, not mode default and {asked:?} last");
    }
    crate::selftest_schedules::sm(&["schedule", "add", WORKSPACE, "--id", ASKED, "--name", "Selftest asked", "--folder", &repo, "--every", "weekly mon 07:00", "--prompt", job])?;
    let open = format!("start-schedule-open-{tab}");
    until(view, cx, WITHIN, "the banner of the schedule the session added", |v| v.hits.borrow().contains_key(&open).then_some(())).await?;
    let found = view.read_with(cx, |v, _| v.tab(tab).and_then(|t| t.asks_schedule.as_ref()).and_then(|a| a.found.clone()));
    if found.as_ref().map(|f| f.0.as_str()) != Some(ASKED) {
        bail!("the banner names {found:?}, not {ASKED}");
    }
    click(view, &open, cx).await?;
    until(view, cx, WITHIN, "the schedule's tab active", |v| v.schedule_page(WORKSPACE, ASKED).filter(|(id, _)| v.active == Some(*id)).map(drop)).await?;
    Ok(format!(
        "New task in {WORKSPACE}/{repo} mode plan: the session tab `{}` got the typed two-line prompt byte for byte after `--`, with the mode and the [+] flags; \
         New scheduled task from the folder's menu: `{}` first; the session's schedule raised `Schedule Selftest asked waits for your approval`, Open opened its tab",
        lines[0], SCHEDULE_ASK
    ))
}
