//! A tab's right-click menu on a shell tab in a fixture repo's subfolder (standing in for a session tab: no claude
//! session runs there): opened at the pointer, closed by Esc and by a click outside, Open Git opening the
//! repository's Git tab, Copy folder path, Open folder, Close tabs to the right, Reopen closed tab,
//! Close other tabs and Close. A unit test cannot: it needs the app's real terminal and Git tabs, gpui's mouse dispatch
//! through the strip and the menu, and the repository read landing off the UI thread in the open menu; the items per
//! tab are `view::right::tab_menu`'s tests, one Git tab per folder `view::git_tab`'s.

use std::{
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};

use anyhow::bail;
use gpui::{AsyncWindowContext, Entity, Modifiers, MouseButton, Pixels, Point};
use supraman_core::paths::path_key;

use crate::{
    selftest::{home, key, until, until_app},
    selftest_git::git_tabs,
    selftest_manage::press_painted,
    view::{FleetView, MenuAction},
};

const WITHIN: Duration = Duration::from_secs(20);

static OPENED: Mutex<Option<PathBuf>> = Mutex::new(None);

fn recorded_open(path: &Path) -> Result<(), String> {
    *OPENED.lock().unwrap_or_else(|e| e.into_inner()) = Some(path.to_path_buf());
    Ok(())
}

pub(crate) async fn run(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let parent = home().join("tab-menu-fixture");
    std::fs::create_dir_all(&parent)?;
    let fixture = cx.background_executor().spawn(async move { supraman_gitman::fixture::Fixture::new_in(&parent) }).await;
    let sub = fixture.dir.join("src").join("menu");
    std::fs::create_dir_all(&sub)?;
    let real_open = view.update(cx, |v, _| std::mem::replace(&mut v.open_with, recorded_open));
    let result = steps(view, &fixture.dir, &sub, cx).await;
    view.update(cx, |v, _| v.open_with = real_open);
    let open: Vec<u64> = view.read_with(cx, |v, _| v.tabs.iter().map(|t| t.id).collect());
    view.update(cx, |v, cx| open.into_iter().for_each(|id| v.close_tab(id, cx)));
    until(view, cx, WITHIN, "every tab closed", |v| v.tabs.is_empty().then_some(())).await?;
    drop(fixture);
    result
}

fn ids(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> Vec<u64> {
    view.read_with(cx, |v, _| v.tabs.iter().map(|t| t.id).collect())
}

/// The menu of `tab` open, by a right click on it, once its repository read landed; where it opened.
async fn right_click(view: &Entity<FleetView>, tab: u64, cx: &mut AsyncWindowContext) -> anyhow::Result<Point<Pixels>> {
    press_painted(view, &format!("tab-{tab}"), MouseButton::Right, cx).await?;
    until(view, cx, WITHIN, &format!("tab {tab}'s menu with its repository read"), |v| {
        let menu = v.menu.as_ref().filter(|m| m.items.iter().any(|i| i.action == MenuAction::CloseTab(tab)))?;
        menu.items.iter().any(|i| i.key == "tab-open-git" && i.dimmed.is_none()).then_some(menu.position)
    })
    .await
}

async fn pick(view: &Entity<FleetView>, tab: u64, item: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    right_click(view, tab, cx).await?;
    press_painted(view, &format!("menu-{item}"), MouseButton::Left, cx).await
}

async fn steps(view: &Entity<FleetView>, top: &Path, sub: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let (shell, right) = cx.update(|window, cx| {
        view.update(cx, |v, cx| (v.open_shell_in(sub.to_path_buf(), "tab menu".into(), window, cx), v.open_shell_in(home(), "tab menu right".into(), window, cx)))
    })?;
    press_painted(view, &format!("tab-{shell}"), MouseButton::Left, cx).await?;

    let at = right_click(view, shell, cx).await?;
    let center = view.read_with(cx, |v, _| v.hits.borrow().get(&format!("tab-{shell}")).map(|h| h.bounds.center()));
    if Some(at) != center {
        bail!("the menu opened at {at:?}, the right click was at {center:?}");
    }
    key(cx, "escape", Modifiers::default())?;
    until(view, cx, WITHIN, "Esc closed the menu", |v| v.menu.is_none().then_some(())).await?;
    right_click(view, shell, cx).await?;
    press_painted(view, &format!("terminal-{shell}"), MouseButton::Left, cx).await?;
    until(view, cx, WITHIN, "a click outside closed the menu", |v| v.menu.is_none().then_some(())).await?;

    pick(view, shell, "tab-open-git", cx).await?;
    let git = until_app(view, cx, WITHIN, "the repository's Git tab, active", |v, cx| match git_tabs(v, cx, top)[..] {
        [(id, _)] if v.active == Some(id) => Some(id),
        _ => None,
    })
    .await?;
    let in_sub = view.read_with(cx, |v, cx| git_tabs(v, cx, sub).len());
    if in_sub != 0 || ids(view, cx) != [shell, right, git] {
        bail!("tabs {:?} with {in_sub} Git tab(s) of the subfolder; expected [{shell}, {right}, {git}]", ids(view, cx));
    }

    pick(view, shell, "tab-copy-path", cx).await?;
    let copied = until_with_clipboard(sub, cx).await?;
    *OPENED.lock().unwrap_or_else(|e| e.into_inner()) = None;
    pick(view, shell, "tab-open-folder", cx).await?;
    crate::selftest::until_with(cx, WITHIN, "Open folder's path", |_| OPENED.lock().unwrap_or_else(|e| e.into_inner()).clone().filter(|p| path_key(p) == path_key(sub))).await?;

    pick(view, shell, "tab-close-right", cx).await?;
    until(view, cx, WITHIN, "the tabs to its right closed", |v| (v.tabs.len() == 1 && v.tabs[0].id == shell).then_some(())).await?;
    pick(view, shell, "tab-reopen", cx).await?;
    let reopened = until_app(view, cx, WITHIN, "the last closed tab, its Git tab, reopened", |v, cx| git_tabs(v, cx, top).first().map(|(id, _)| *id)).await?;
    pick(view, shell, "tab-close-others", cx).await?;
    until(view, cx, WITHIN, "the other tabs closed", |v| (v.tabs.len() == 1 && v.tabs[0].id == shell).then_some(())).await?;
    pick(view, shell, "tab-close", cx).await?;
    until(view, cx, WITHIN, "the tab closed", |v| v.tabs.is_empty().then_some(())).await?;
    Ok(format!(
        "tab menu at the pointer, closed by Esc and a click outside; Open Git from {} opened Git tab {git} of its top; copied {copied:?}; Open folder; Close tabs to the right, Reopen closed tab (Git tab {reopened}), Close other tabs, Close",
        sub.display()
    ))
}

async fn until_with_clipboard(sub: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let want = sub.display().to_string();
    crate::selftest::until_with(cx, WITHIN, "the folder path on the clipboard", |cx| {
        cx.update(|_, cx| cx.read_from_clipboard().and_then(|c| c.text())).ok().flatten().filter(|t| *t == want)
    })
    .await
}
