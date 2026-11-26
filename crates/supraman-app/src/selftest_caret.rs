//! `--selftest` step 7k+: the caret rule end to end (UI-KIT §4.3): beside a shell tab, the search box holding the keys
//! paints its caret and the terminal no cursor; the terminal holding them again paints its line cursor. A gpui unit test
//! cannot prove it: it needs Windows' own activation of the window and the real paint of the grid and the field.

use std::time::{Duration, Instant};

use anyhow::bail;
use gpui::{AsyncWindowContext, Entity, KeyDownEvent, Keystroke, Modifiers, PlatformInput};
use supraman_term::Terminal;

use crate::{
    selftest::{cargo_root, drawn, until_with, wait_for_tab_terminal},
    selftest_manage::post_keys,
    view::FleetView,
};

const WITHIN: Duration = Duration::from_secs(15);
/// Two blinks: a cursor still unlit over them is hidden, not between blinks.
const BLINKS: Duration = Duration::from_millis(1200);

pub async fn run(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let cwd = cargo_root().join("target").join("selftest-cwd");
    let shell = view.update(cx, |v, _| std::mem::replace(&mut v.shell_args, vec!["-NoLogo".into(), "-NoProfile".into()]));
    let tab = cx.update(|window, cx| view.update(cx, |v, cx| v.open_shell_in(cwd, "caret".into(), window, cx)));
    view.update(cx, |v, _| v.shell_args = shell);
    let tab = tab?;
    let result = check(view, tab, cx).await;
    view.update(cx, |v, cx| v.close_tab(tab, cx));
    result
}

fn search_holds_keys(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<(bool, bool)> {
    cx.update(|window, cx| {
        let search = view.read(cx).search.read(cx);
        (search.is_focused(window), search.caret_painted.get())
    })
}

/// Whether the terminal painted its cursor in any frame drawn over `time`.
async fn lit_within(terminal: &Entity<Terminal>, time: Duration, cx: &mut AsyncWindowContext) -> anyhow::Result<bool> {
    let started = Instant::now();
    let mut lit = false;
    while started.elapsed() < time {
        drawn(cx).await?;
        lit |= terminal.read_with(cx, |t, _| t.cursor_painted_lit);
    }
    Ok(lit)
}

async fn check(view: &Entity<FleetView>, tab: u64, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let terminal = wait_for_tab_terminal(view, tab, Duration::from_secs(10), cx).await?;
    until_with(cx, WITHIN, "the shell's prompt", |cx| terminal.read_with(cx, |t, _| t.get_content().contains("PS ")).then_some(())).await?;
    let active = cx.update(|window, _| window.is_window_active())?;
    if !active {
        bail!("the main window is not active: the rule only holds in an active window");
    }
    if !lit_within(&terminal, BLINKS, cx).await? {
        bail!("the shown terminal holding the keys painted no cursor over {} ms", BLINKS.as_millis());
    }
    let mut out = vec!["the shown terminal paints its line cursor".to_string()];

    // dispatched: a posted Ctrl is not in the keyboard state Windows reads
    let keystroke = Keystroke { modifiers: Modifiers { control: true, shift: true, ..Default::default() }, key: "e".into(), key_char: None };
    cx.update(|window, cx| window.dispatch_event(PlatformInput::KeyDown(KeyDownEvent { keystroke, is_held: false, prefer_character_input: false }), cx))?;
    until_with(cx, WITHIN, "the search box holding the keys with its caret painted", |cx| search_holds_keys(view, cx).ok().filter(|held| *held == (true, true))).await?;
    if lit_within(&terminal, BLINKS, cx).await? {
        bail!("with the search box holding the keys the terminal still painted its cursor");
    }
    out.push(format!("Ctrl+Shift+E: the search box paints its caret and the terminal no cursor over {} ms", BLINKS.as_millis()));

    post_keys(&["escape", "escape"], cx).await?;
    until_with(cx, WITHIN, "the search box giving the keys back", |cx| search_holds_keys(view, cx).ok().filter(|held| !held.0)).await?;
    if search_holds_keys(view, cx)?.1 {
        bail!("the search box without the keys still painted its caret");
    }
    if !lit_within(&terminal, BLINKS * 2, cx).await? {
        bail!("Esc gave the terminal the keys back but it painted no cursor");
    }
    out.push("Esc Esc: the field paints no caret and the terminal its cursor again".into());
    Ok(out.join("; "))
}
