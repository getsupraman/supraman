//! The browser chords in a terminal, a Work and a Git tab, under both Ctrl+T/Ctrl+W modes. A unit test cannot: it needs
//! gpui's keystroke dispatch through the bindings on to a terminal's PTY and the settings window's save; the chord table
//! and the index rules are `view::tab_keys`' and `supraman_ui::tabs`' tests.

use std::time::Duration;

use anyhow::{Context as _, bail};
use gpui::{AsyncWindowContext, Entity, Modifiers};
use supraman_core::{Settings, TabKeys};

use crate::{
    selftest::{click, hex6, home, until, until_app},
    selftest_tabs::{READER, keys, press, reader},
    selftest_work::WORKSPACE,
    settings_window::{SettingsView, page},
    view::{FleetView, TabKind},
};

const WITHIN: Duration = Duration::from_secs(10);

pub(crate) async fn run(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let script = home().join(format!("fm-selftest-tab-keys-{}.ps1", hex6()));
    std::fs::write(&script, READER).context("the key reader script")?;
    let result: anyhow::Result<String> = async {
        close_all(view, cx).await?;
        let tabs = open(view, &script, cx).await?;
        let jumped = jumps(view, tabs, cx).await?;
        let moved = moves(view, tabs, cx).await?;
        let (outside, shell) = outside_terminals(view, tabs, cx).await?;
        let everywhere = everywhere(view, shell, cx).await?;
        Ok(format!("{jumped}; {moved}; {outside}; {everywhere}"))
    }
    .await;
    let restored = set_mode(view, TabKeys::OutsideTerminals, cx).await;
    if let Some(handle) = view.read_with(cx, |v, _| v.settings_window) {
        handle.update(cx, |_, window, _| window.remove_window()).ok();
    }
    close_all(view, cx).await?;
    std::fs::remove_file(&script).ok();
    let result = result?;
    restored?;
    let menu = crate::selftest_tab_menu::run(view, cx).await?;
    Ok(format!("{result}; {menu}"))
}

async fn close_all(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let open: Vec<u64> = view.read_with(cx, |v, _| v.tabs.iter().map(|t| t.id).collect());
    view.update(cx, |v, cx| open.into_iter().for_each(|id| v.close_tab(id, cx)));
    until(view, cx, WITHIN, "every tab closed", |v| v.tabs.is_empty().then_some(())).await
}

fn ctrl() -> Modifiers {
    Modifiers::control()
}

fn ctrl_shift() -> Modifiers {
    Modifiers { control: true, shift: true, ..Default::default() }
}

fn order(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> (Vec<u64>, Option<u64>) {
    view.read_with(cx, |v, _| (v.tabs.iter().map(|t| t.id).collect(), v.active))
}

/// A key reader shell, a Work tab and a Git tab, in that order.
async fn open(view: &Entity<FleetView>, script: &std::path::Path, cx: &mut AsyncWindowContext) -> anyhow::Result<[u64; 3]> {
    let args = ["-NoLogo", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File"].map(String::from).into_iter();
    let args: Vec<String> = args.chain([script.display().to_string()]).collect();
    let shell = cx.update(|window, cx| {
        view.update(cx, |v, cx| {
            let saved = std::mem::replace(&mut v.shell_args, args);
            let id = v.open_shell_in(home(), "tab keys".into(), window, cx);
            v.shell_args = saved;
            id
        })
    })?;
    let work = view.update(cx, |v, cx| v.open_work(WORKSPACE, cx));
    let git = view.update(cx, |v, cx| v.open_git(home(), "Git · tab keys".into(), &Default::default(), cx));
    if order(view, cx).0 != [shell, work, git] {
        bail!("expected tabs [{shell}, {work}, {git}], found {:?}", order(view, cx).0);
    }
    reader(view, shell, cx).await?;
    click(view, &format!("tab-{shell}"), cx).await?;
    Ok([shell, work, git])
}

/// Ctrl+1..8 and Ctrl+9 from each kind of tab; the strip reveals what they activate (`selftest_overflow` with 30 tabs).
async fn jumps(view: &Entity<FleetView>, [shell, work, git]: [u64; 3], cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let steps = [("2", work), ("3", git), ("1", shell), ("9", git), ("8", git), ("2", work), ("1", shell)];
    let count = steps.len();
    for (i, (key, expected)) in steps.into_iter().enumerate() {
        let from = view.read_with(cx, |v, _| v.active);
        press(cx, key, ctrl())?;
        let now = view.read_with(cx, |v, _| v.active);
        if now != Some(expected) {
            bail!("press {} (ctrl-{key}) on tab {from:?}: active is {now:?}, expected {expected}", i + 1);
        }
    }
    Ok(format!("{} Ctrl+digit jumps from the shell, Work and Git tabs; Ctrl+8 of 3 tabs did nothing", count))
}

async fn moves(view: &Entity<FleetView>, [shell, work, git]: [u64; 3], cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let steps = [
        ("pageup", vec![shell, work, git]),
        ("pagedown", vec![work, shell, git]),
        ("pagedown", vec![work, git, shell]),
        ("pagedown", vec![work, git, shell]),
        ("pageup", vec![work, shell, git]),
        ("pageup", vec![shell, work, git]),
    ];
    let count = steps.len();
    for (i, (key, expected)) in steps.into_iter().enumerate() {
        press(cx, key, ctrl_shift())?;
        let now = order(view, cx);
        if now != (expected.clone(), Some(shell)) {
            bail!("press {} (ctrl-shift-{key}): (order, active) {now:?}, expected ({expected:?}, {shell})", i + 1);
        }
    }
    Ok(format!("{} Ctrl+Shift+PageUp/PageDown moved the shell tab one place each, never past an end", count))
}

async fn until_new(view: &Entity<FleetView>, before: &[u64], what: &str, cx: &mut AsyncWindowContext, kind: fn(&TabKind) -> bool) -> anyhow::Result<u64> {
    let before = before.to_vec();
    until(view, cx, WITHIN, what, move |v| {
        let id = v.active.filter(|id| !before.contains(id))?;
        v.tab(id).is_some_and(|t| kind(&t.kind)).then_some(id)
    })
    .await
}

/// The default mode: the shell's program gets Ctrl+T and Ctrl+W, the Git tab opens and closes tabs with them, and
/// Ctrl+Shift+T/W and Ctrl+Alt+T act in the shell too. Returns the shell reopened by the last Ctrl+Alt+T.
async fn outside_terminals(view: &Entity<FleetView>, [shell, work, git]: [u64; 3], cx: &mut AsyncWindowContext) -> anyhow::Result<(String, u64)> {
    let terminal = reader(view, shell, cx).await?;
    press(cx, "t", ctrl())?;
    press(cx, "w", ctrl())?;
    // ConsoleModifiers: Control = 4
    let got = keys(&terminal, 2, cx).await.context("the shell's reader")?;
    if got != ["fm-key T 4", "fm-key W 4"] || order(view, cx) != (vec![shell, work, git], Some(shell)) {
        bail!("Ctrl+T, Ctrl+W in the shell: the reader got {got:?}, tabs {:?}; expected both read and the tabs unchanged", order(view, cx));
    }

    press(cx, "3", ctrl())?;
    press(cx, "t", ctrl())?;
    let start = until_new(view, &[shell, work, git], "Ctrl+T in the Git tab: a New task tab active", cx, |k| matches!(k, TabKind::Start(_))).await?;
    press(cx, "w", ctrl())?;
    until(view, cx, WITHIN, "Ctrl+W: the New task tab closed", |v| v.tab(start).is_none().then_some(())).await?;
    press(cx, "3", ctrl())?;
    press(cx, "w", ctrl())?;
    until(view, cx, WITHIN, "Ctrl+W: the Git tab closed", |v| v.tab(git).is_none().then_some(())).await?;
    press(cx, "t", ctrl_alt())?;
    let git_again = until_new(view, &[shell, work], "Ctrl+Alt+T: the Git tab reopened and active", cx, |k| matches!(k, TabKind::Git(_))).await?;

    press(cx, "1", ctrl())?;
    press(cx, "t", ctrl_shift())?;
    let start = until_new(view, &[shell, work, git_again], "Ctrl+Shift+T in the shell: a New task tab active", cx, |k| matches!(k, TabKind::Start(_))).await?;
    press(cx, "1", ctrl())?;
    press(cx, "tab", Modifiers::default())?;
    let got = keys(&terminal, 3, cx).await.context("the shell's reader")?;
    if got != ["fm-key T 4", "fm-key W 4", "fm-key Tab 0"] {
        bail!("after Ctrl+Shift+T in the shell the reader got {got:?}; expected nothing more than Tab");
    }
    press(cx, "w", ctrl_shift())?;
    until(view, cx, WITHIN, "Ctrl+Shift+W: the shell closed", |v| v.tab(shell).is_none().then_some(())).await?;
    view.update(cx, |v, cx| v.close_tab(start, cx));
    let open: Vec<u64> = order(view, cx).0;
    press(cx, "t", ctrl_alt())?;
    let shell_again = until_new(view, &open, "Ctrl+Alt+T: the shell reopened and active", cx, |k| matches!(k, TabKind::Shell)).await?;
    Ok((
        "outside terminals: the shell's program read Ctrl+T and Ctrl+W; in the Git tab Ctrl+T opened and Ctrl+W closed a task tab and \
         closed the Git tab, Ctrl+Alt+T reopened it; in the shell Ctrl+Shift+T opened a task tab, Ctrl+Shift+W closed the shell and \
         Ctrl+Alt+T reopened it"
            .into(),
        shell_again,
    ))
}

fn ctrl_alt() -> Modifiers {
    Modifiers { control: true, alt: true, ..Default::default() }
}

/// The other mode, chosen in the settings window and applied without a restart: Ctrl+T and Ctrl+W never reach the shell.
async fn everywhere(view: &Entity<FleetView>, shell: u64, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    set_mode(view, TabKeys::Everywhere, cx).await?;
    let terminal = reader(view, shell, cx).await?;
    click(view, &format!("tab-{shell}"), cx).await?;
    let open: Vec<u64> = order(view, cx).0;
    press(cx, "t", ctrl())?;
    until_new(view, &open, "Ctrl+T in the shell: a New task tab active", cx, |k| matches!(k, TabKind::Start(_))).await?;
    click(view, &format!("tab-{shell}"), cx).await?;
    press(cx, "tab", Modifiers::default())?;
    let got = keys(&terminal, 1, cx).await.context("the reopened shell's reader")?;
    if got != ["fm-key Tab 0"] {
        bail!("everywhere: the reader got {got:?}; expected only Tab");
    }
    press(cx, "w", ctrl())?;
    until(view, cx, WITHIN, "Ctrl+W: the shell closed", |v| v.tab(shell).is_none().then_some(())).await?;
    Ok("everywhere (set in Settings, applied at once): Ctrl+T opened a task tab from the shell, Ctrl+W closed it, its program read neither".into())
}

async fn set_mode(view: &Entity<FleetView>, mode: TabKeys, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    view.update(cx, |v, cx| v.open_settings_at(Some(page::APPEARANCE), cx));
    let handle: gpui::WindowHandle<SettingsView> = crate::selftest::until_with(cx, WITHIN, "Settings open and loaded", |cx| {
        let handle = view.read_with(cx, |v, _| v.settings_window)?;
        handle.read_with(cx, |s, _| s.loaded()).ok()?.then_some(handle)
    })
    .await?;
    handle.update(cx, |s, _, cx| s.set_tab_keys(mode, cx)).map_err(|e| anyhow::anyhow!("settings window: {e}"))?;
    until_app(view, cx, WITHIN, &format!("{mode:?} applied to the main window"), |v, cx| (v.tab_keys.read(cx).mode == mode).then_some(())).await?;
    let saved = Settings::load(&home())?.window.tab_keys();
    if saved != mode {
        bail!("settings.toml holds {saved:?}, the window {mode:?}");
    }
    Ok(())
}
