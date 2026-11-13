//! New task… from the workspace's Task icon: a typed prompt with quotes, `$`, a non-ASCII letter and a new line,
//! Send, and the argv a stand-in claude got (`supraman.exe --selftest-argv`, a native exe as claude.exe is).

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context as _, anyhow, bail};
use gpui::{AsyncWindowContext, Entity, Modifiers};

use crate::selftest::{WORKSPACE, click, main_repo_of_gate, sleep, until};
use crate::selftest::keys::{keystroke, type_text};
use crate::view::{FleetView, TabKind};

const WITHIN: Duration = Duration::from_secs(20);

fn argv_file(home: &Path) -> PathBuf {
    home.join("start-argv.json")
}

pub(crate) async fn run(view: &Entity<FleetView>, home: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let before: Vec<u64> = view.read_with(cx, |v, _| v.tabs.iter().map(|t| t.id).collect());
    let saved = view.read_with(cx, |v, _| v.claude_command.clone());
    let result = check(view, home, cx).await;
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
    let quote = |p: &Path| supraman_core::platform::shell::quote(&p.display().to_string());
    view.update(cx, |v, _| v.claude_command = format!("& {} --selftest-argv {}", quote(&exe), quote(out)));
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
pub(crate) async fn start_tab(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<u64> {
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
    let plugin = view.read_with(cx, |v, _| v.plugin_dir.clone()).context("this build found no plugins/supraman")?;

    click(view, &format!("task-{WORKSPACE}"), cx).await?;
    let tab = start_tab(view, cx).await?;
    let choices = view.read_with(cx, |v, _| v.workspaces.iter().find(|w| w.name == WORKSPACE).map(crate::view::start::folders)).unwrap_or_default();
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
    let out = argv_file(home);
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
    Ok(format!(
        "New task in {WORKSPACE} at {} mode plan: the session tab `{}` got the typed two-line prompt byte for byte after `--`, with the mode and the [+] flags",
        folder.display(),
        lines[0]
    ))
}
