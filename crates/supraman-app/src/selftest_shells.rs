//! The ⌄ beside + opens a cmd tab whose prompt shows; under a `cmd` default + starts cmd too, and a [+] claude
//! tab still starts PowerShell. A unit test cannot do it: the prompt shows only from a real PTY running the shell.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context as _, bail};
use gpui::{AsyncWindowContext, Entity};
use supraman_core::Settings;
use supraman_core::shells::ShellSetting;

use crate::{
    selftest::{click, main_repo_of_gate, sleep, until, wait_for_tab_terminal},
    view::FleetView,
};

const WITHIN: Duration = Duration::from_secs(20);
const HAND: &str = "shells selftest claude";

pub(crate) async fn run(view: &Entity<FleetView>, home: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let before: Vec<u64> = view.read_with(cx, |v, _| v.tabs.iter().map(|t| t.id).collect());
    let fake = home.join("fm-c113-claude.cmd");
    std::fs::write(&fake, "@echo fm-c113-hand\r\n").context("the fake claude")?;
    let saved = view.update(cx, |v, _| std::mem::replace(&mut v.claude_command, format!("& '{}'", fake.display())));
    let result = check(view, home, cx).await;
    let restored = Settings::update(home, |s| s.terminal.set_shell(ShellSetting::Auto));
    let mine: Vec<u64> = view.read_with(cx, |v, _| v.tabs.iter().map(|t| t.id).filter(|id| !before.contains(id)).collect());
    view.update(cx, |v, cx| {
        if let Ok((settings, ())) = &restored {
            v.term_look.shell_default.read(settings.terminal.clone());
        }
        v.claude_command = saved;
        v.menu = None;
        mine.into_iter().for_each(|id| v.close_tab(id, cx));
        cx.notify();
    });
    std::fs::remove_file(&fake).ok();
    restored.context("[terminal] shell back to auto")?;
    result
}

fn started(view: &Entity<FleetView>, id: u64, cx: &mut AsyncWindowContext) -> Option<std::path::PathBuf> {
    view.read_with(cx, |v, _| v.tabs.iter().find(|t| t.id == id).and_then(|t| t.started.as_ref()).map(|s| s.program.clone()))
}

fn is_cmd(program: &Path) -> bool {
    program.file_name().is_some_and(|n| n.eq_ignore_ascii_case("cmd.exe"))
}

async fn check(view: &Entity<FleetView>, home: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    until(view, cx, WITHIN, "this machine's shells detected", |v| v.term_look.shell_default.found().map(drop)).await?;
    let (cmd, ()) = Settings::update(home, |s| s.terminal.set_shell(ShellSetting::Cmd)).context("[terminal] shell = cmd")?;
    view.update(cx, |v, _| v.term_look.shell_default.read(cmd.terminal));

    let cwd = view.read_with(cx, |v, _| v.new_shell_cwd()).context("+ has no folder")?;
    click(view, "new-shell-menu", cx).await?;
    let keys = until(view, cx, WITHIN, "the ⌄ menu", |v| v.menu.as_ref().map(|m| m.items.iter().map(|i| (i.key.clone(), i.label.clone())).collect::<Vec<_>>())).await?;
    if !keys.iter().any(|(k, l)| *k == "shell-cmd" && l.ends_with("(default)")) {
        bail!("the ⌄ menu has no Command Prompt marked as the default: {keys:?}");
    }
    let tabs: Vec<u64> = view.read_with(cx, |v, _| v.tabs.iter().map(|t| t.id).collect());
    click(view, "menu-shell-cmd", cx).await?;
    let (id, title) = until(view, cx, WITHIN, "the cmd tab", |v| v.tabs.iter().find(|t| !tabs.contains(&t.id)).map(|t| (t.id, t.title.clone()))).await?;
    if !title.starts_with("cmd ") || view.read_with(cx, |v, _| v.tab(id).and_then(|t| t.cwd.clone())) != Some(cwd.clone()) {
        bail!("the picked cmd tab is {title:?} in {:?}, expected `cmd <id>` in {}", view.read_with(cx, |v, _| v.tab(id).and_then(|t| t.cwd.clone())), cwd.display());
    }
    let terminal = wait_for_tab_terminal(view, id, WITHIN, cx).await?;
    let prompt = format!("{}>", cwd.display()).to_lowercase();
    let deadline = std::time::Instant::now() + WITHIN;
    while !terminal.read_with(cx, |t, _| t.get_content()).to_lowercase().contains(&prompt) {
        if std::time::Instant::now() > deadline {
            bail!("the cmd tab never showed the prompt {prompt:?}: {:?}", terminal.read_with(cx, |t, _| t.get_content()));
        }
        sleep(cx, Duration::from_millis(50)).await;
    }
    let saved = view.read_with(cx, |v, cx| v.saved_tabs(cx));
    let kept = saved.tabs.iter().any(|t| matches!(t, crate::saved_tabs::SavedTab::Shell { title: t, shell: Some(s), .. } if *t == title && is_cmd(&s.program)));
    if !kept {
        bail!("the saved tabs do not keep {title:?} as a cmd tab: {saved:?}");
    }

    let plus = cx.update(|window, cx| view.update(cx, |v, cx| v.new_shell(window, cx)))?;
    let plus_title = view.read_with(cx, |v, _| v.tab(plus).map(|t| t.title.clone())).unwrap_or_default();
    if !plus_title.starts_with("cmd ") || !started(view, plus, cx).is_some_and(|p| is_cmd(&p)) {
        bail!("+ under a cmd default started {plus_title:?} with {:?}", started(view, plus, cx));
    }

    let folder = main_repo_of_gate()?;
    let hand = cx.update(|window, cx| view.update(cx, |v, cx| v.new_claude(folder.clone(), HAND.into(), window, cx)))?;
    let program = until_started(view, hand, cx).await?;
    let powershell = supraman_core::platform::shell::terminal_powershell().path.clone();
    if is_cmd(&program) || program != powershell {
        bail!("a [+] claude tab under a cmd default started {}, not {}", program.display(), powershell.display());
    }
    Ok(format!(
        "⌄ listed {} shells, Command Prompt the default; picked it: tab {title:?} in {} showed its prompt and is saved as cmd; + opened {plus_title:?} on cmd; a [+] claude tab started {} (C113)",
        keys.len(),
        cwd.display(),
        program.display()
    ))
}

async fn until_started(view: &Entity<FleetView>, id: u64, cx: &mut AsyncWindowContext) -> anyhow::Result<std::path::PathBuf> {
    let deadline = std::time::Instant::now() + WITHIN;
    loop {
        if let Some(program) = started(view, id, cx) {
            return Ok(program);
        }
        if std::time::Instant::now() > deadline {
            bail!("the [+] claude tab {id} started no terminal within {} s", WITHIN.as_secs());
        }
        sleep(cx, Duration::from_millis(50)).await;
    }
}
