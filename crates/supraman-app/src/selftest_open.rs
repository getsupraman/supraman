//! A right-clicked row keeps its open fill while the pointer is over its menu, and loses it when the menu closes
//! (UI-KIT §4.2). A unit test cannot do it: the pointer leaves the row for the occluding menu only through the
//! window's own mouse dispatch over a painted window, which drops the row's hover.

use std::time::Duration;

use anyhow::bail;
use gpui::{AsyncWindowContext, Entity, Modifiers, MouseButton};

use crate::selftest::{drawn, key, press_at, until};
use crate::view::FleetView;

const WITHIN: Duration = Duration::from_secs(10);

/// How many frames painted `anchor` open over `frames` more frames.
async fn open_frames(anchor: &str, frames: usize, cx: &mut AsyncWindowContext) -> anyhow::Result<u64> {
    let before = supraman_ui::popup::painted_open(anchor);
    for _ in 0..frames {
        drawn(cx).await?;
    }
    Ok(supraman_ui::popup::painted_open(anchor) - before)
}

/// The menu a right-click at `at` opens; again while none comes, since a Git tab's row asked before its writer opened
/// says why in a toast and opens none.
async fn menu_of(view: &Entity<FleetView>, row: &str, at: gpui::Point<gpui::Pixels>, cx: &mut AsyncWindowContext) -> anyhow::Result<gpui::Bounds<gpui::Pixels>> {
    let mut last = None;
    for _ in 0..5 {
        press_at(at, MouseButton::Right, 1, cx)?;
        match until(view, cx, Duration::from_secs(2), &format!("{row}'s menu painted"), |v| v.hits.borrow().get("menu").map(|h| h.bounds)).await {
            Ok(menu) => return Ok(menu),
            Err(e) => last = Some(e),
        }
    }
    Err(last.expect("five tries").context(format!("{row} right-clicked five times at {at:?}")))
}

/// `row` (its hit key, its menu's anchor) right-clicked, the pointer moved onto the menu (`menu`): the row paints open
/// in every frame; Esc closes the menu and the row paints open no more.
pub(crate) async fn row(view: &Entity<FleetView>, row: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    view.update(cx, |v, cx| {
        v.hits.borrow_mut().clear();
        cx.notify();
    });
    let at = until(view, cx, WITHIN, &format!("{row} painted"), |v| v.hits.borrow().get(row).map(|h| h.bounds.center())).await?;
    let menu = menu_of(view, row, at, cx).await?;
    crate::selftest_left::move_to(menu.center(), cx)?;
    drawn(cx).await?;
    let open = open_frames(row, 3, cx).await?;
    if open < 3 {
        bail!("{row}: painted open in {open} of 3 frames with the pointer over its menu at {menu:?}");
    }
    key(cx, "escape", Modifiers::default())?;
    let window = cx.window_handle();
    let closed = |cx: &mut AsyncWindowContext| window.update(cx, |_, window, cx| !supraman_ui::popup::is_open(row, window, cx)).unwrap_or(false);
    crate::selftest::until_with(cx, WITHIN, &format!("{row} no longer open"), |cx| closed(cx).then_some(())).await?;
    drawn(cx).await?;
    let after = open_frames(row, 2, cx).await?;
    if after > 0 {
        bail!("{row}: still painted open in {after} frames after its menu closed");
    }
    Ok(format!("{row} painted open in every frame with the pointer over its menu, and not once it closed"))
}
