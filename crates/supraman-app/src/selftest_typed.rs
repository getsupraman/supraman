//! `--selftest` step 7k++: typed text reaches a shell tab through the terminal's input handler (UI-KIT §8). A posted
//! WM_KEYDOWN of `a` goes through gpui_windows' key down, the terminal leaving it untaken and `TranslateMessage`'s
//! WM_CHAR bringing it, once; then the stream a dead key queues on a layout with dead keys (WM_DEADCHAR `, WM_CHAR é),
//! posted as such since loading such a layout would touch the user's input settings. A gpui unit test cannot prove
//! it: it needs Windows' message path into gpui_windows and the shell echoing what it read.

use std::time::Duration;

use anyhow::{anyhow, bail};
use gpui::{AsyncWindowContext, Entity};
use supraman_term::Terminal;

use crate::{
    selftest::{cargo_root, until_with, wait_for_tab_terminal, win},
    selftest_manage::post_keys,
    view::FleetView,
};

const WITHIN: Duration = Duration::from_secs(15);

pub async fn run(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let cwd = cargo_root().join("target").join("selftest-cwd");
    let shell = view.update(cx, |v, _| std::mem::replace(&mut v.shell_args, vec!["-NoLogo".into(), "-NoProfile".into()]));
    let tab = cx.update(|window, cx| view.update(cx, |v, cx| v.open_shell_in(cwd, "typed".into(), window, cx)));
    view.update(cx, |v, _| v.shell_args = shell);
    let tab = tab?;
    let result = check(view, tab, cx).await;
    view.update(cx, |v, cx| v.close_tab(tab, cx));
    result
}

/// The text after the prompt on the last line that holds one.
fn typed_line(terminal: &Entity<Terminal>, cx: &mut AsyncWindowContext) -> Option<String> {
    let content = terminal.read_with(cx, |t, _| t.get_content());
    let line = content.lines().rev().find(|l| l.contains("PS ") && l.contains('>'))?;
    Some(line.split_once('>')?.1.trim().to_string())
}

async fn check(view: &Entity<FleetView>, tab: u64, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let terminal = wait_for_tab_terminal(view, tab, Duration::from_secs(10), cx).await?;
    until_with(cx, WITHIN, "the shell's prompt", |cx| typed_line(&terminal, cx).filter(String::is_empty).map(|_| ())).await?;
    cx.update(|window, _| window.activate_window())?;
    until_with(cx, WITHIN, "the main window active", |cx| cx.update(|window, _| window.is_window_active()).ok().filter(|a| *a).map(|_| ())).await?;
    until_with(cx, WITHIN, "the terminal holding the keys", |cx| cx.update(|window, cx| view.read(cx).terminal_holds_keys(window)).ok().filter(|h| *h).map(|_| ())).await?;

    post_keys(&["a"], cx).await?;
    until_with(cx, WITHIN, "the shell echoing a", |cx| typed_line(&terminal, cx).filter(|l| !l.is_empty())).await?;
    let after_key = typed_line(&terminal, cx).unwrap_or_default();
    if after_key != "a" {
        bail!("a posted key a typed {after_key:?}, not \"a\" once");
    }

    let hwnd = cx.update(|window, _| crate::tray::hwnd(window))?.ok_or_else(|| anyhow!("no HWND"))?;
    win::post(hwnd, &win::dead_key_messages('`', 'é'))?;
    until_with(cx, WITHIN, "the shell echoing é", |cx| typed_line(&terminal, cx).filter(|l| l.chars().count() > 1)).await?;
    let after_dead = typed_line(&terminal, cx).unwrap_or_default();
    if after_dead != "aé" {
        bail!("WM_DEADCHAR ` then WM_CHAR é typed {after_dead:?} after the a, not \"aé\"");
    }
    post_keys(&["escape"], cx).await?;
    Ok("a posted key a typed a once through WM_CHAR; WM_DEADCHAR ` and WM_CHAR é typed é, no bare accent".into())
}
