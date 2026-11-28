//! `--selftest` step 7f+: a shell tab with a Git panel; the panel's title bar dragged to the strip, then its tab onto the
//! shell's right edge through gpui's real drag, the dock's ghost and the placed panel read from the hit keys. The drop
//! rules are `view::panel_drop`'s tests; a unit test has no window whose drag paints the ghost between move and release.

use std::time::Duration;

use anyhow::{anyhow, bail};
use gpui::{AsyncWindowContext, Bounds, Entity, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, PlatformInput, Point, point};

use crate::selftest::{drawn, home, scrolled_into_view, until, wait_for_tab_terminal};
use crate::view::FleetView;

const WITHIN: Duration = Duration::from_secs(20);

pub(crate) async fn run(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let parent = home().join("panel-drag-fixture");
    std::fs::create_dir_all(&parent)?;
    let fixture = cx.background_executor().spawn(async move { supraman_gitman::fixture::Fixture::new_in(&parent) }).await;
    let result = steps(view, &fixture.dir, cx).await;
    let open: Vec<u64> = view.read_with(cx, |v, _| v.tabs.iter().map(|t| t.id).collect());
    view.update(cx, |v, cx| open.into_iter().for_each(|id| v.close_tab(id, cx)));
    until(view, cx, WITHIN, "every tab closed", |v| v.tabs.is_empty().then_some(())).await?;
    drop(fixture);
    result
}

async fn steps(view: &Entity<FleetView>, repo: &std::path::Path, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let shell = cx.update(|window, cx| view.update(cx, |v, cx| v.open_shell_in(home(), "panel drag".into(), window, cx)))?;
    wait_for_tab_terminal(view, shell, WITHIN, cx).await?;
    let git = view.update(cx, |v, cx| v.open_git(repo.to_path_buf(), "Git · panel drag".into(), cx));
    if !view.update(cx, |v, cx| v.add_panel(shell, git, cx)) {
        bail!("the Git panel {git} did not join the shell tab {shell}");
    }
    view.update(cx, |v, cx| v.activate_tab(shell, cx));
    let title = scrolled_into_view(view, &format!("dock-{shell}-title-1"), cx).await?.center();

    let header = scrolled_into_view(view, &format!("tab-{shell}"), cx).await?;
    let right_half = point(header.right() - header.size.width * 0.2, header.center().y);
    drag(title, right_half, cx).await?;
    release(right_half, cx)?;
    until(view, cx, WITHIN, "the Git panel a tab after the shell, the dock closed up", |v| {
        (heads(v) == [shell, git] && v.docks.of(shell).is_none() && v.active == Some(shell)).then_some(())
    })
    .await?;
    scrolled_into_view(view, &format!("tab-{git}"), cx).await?;

    let content = scrolled_into_view(view, &format!("terminal-{shell}"), cx).await?;
    let right_edge = point(content.right() - content.size.width * 0.08, content.center().y);
    let from = scrolled_into_view(view, &format!("tab-{git}"), cx).await?.center();
    drag(from, right_edge, cx).await?;
    let ghost = hit(view, &format!("dock-{shell}-ghost"), cx).ok_or_else(|| anyhow!("no ghost over the shell's right edge"));
    crate::selftest_manage::shot("dk9-panel-drag-ghost", cx).await;
    release(right_edge, cx)?;
    let ghost = ghost?;
    if ghost.left() < content.center().x - Pixels::from(2.) {
        bail!("the ghost at {ghost:?} is not the right half of {content:?}");
    }
    let placed = until(view, cx, WITHIN, "the Git panel placed on the shell's right", |v| {
        let laid = v.docks.of(shell).map(|d| d.layout.panels());
        let title = v.hits.borrow().get(&format!("dock-{shell}-slot-1")).map(|h| h.bounds);
        (laid.as_deref() == Some(&[shell, git][..]) && heads(v) == [shell] && !v.hits.borrow().contains_key(&format!("dock-{shell}-ghost"))).then_some(title).flatten()
    })
    .await?;
    if placed.left() < content.center().x - Pixels::from(2.) {
        bail!("the placed Git panel at {placed:?} is not the right half of {content:?}");
    }
    Ok(format!("the Git panel's title bar let go on the strip became tab {git} after {shell}; its tab let go on the shell's right edge painted the ghost at {ghost:?}, then the panel at {placed:?}"))
}

fn hit(view: &Entity<FleetView>, key: &str, cx: &mut AsyncWindowContext) -> Option<Bounds<Pixels>> {
    view.read_with(cx, |v, _| v.hits.borrow().get(key).map(|h| h.bounds))
}

/// Left button down at `from` and moved in steps to `to`, held there once drawn.
async fn drag(from: Point<Pixels>, to: Point<Pixels>, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let none = Modifiers::default();
    cx.update(|window, cx| {
        window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position: from, pressed_button: None, modifiers: none }), cx);
        window.dispatch_event(PlatformInput::MouseDown(MouseDownEvent { button: MouseButton::Left, position: from, modifiers: none, click_count: 1, first_mouse: false }), cx);
        for step in 1..=8 {
            let t = step as f32 / 8.;
            let position = point(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t);
            window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position, pressed_button: Some(MouseButton::Left), modifiers: none }), cx);
        }
    })?;
    drawn(cx).await?;
    cx.update(|window, cx| window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position: to, pressed_button: Some(MouseButton::Left), modifiers: none }), cx))?;
    drawn(cx).await
}

fn release(at: Point<Pixels>, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    cx.update(|window, cx| {
        window.dispatch_event(PlatformInput::MouseUp(MouseUpEvent { button: MouseButton::Left, position: at, modifiers: none(), click_count: 1 }), cx);
    })?;
    Ok(())
}

fn none() -> Modifiers {
    Modifiers::default()
}

fn heads(v: &FleetView) -> Vec<u64> {
    v.headers().map(|t| t.id).collect()
}
