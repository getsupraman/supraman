use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use anyhow::bail;
use gpui::{AnyWindowHandle, App, AsyncWindowContext, Entity};

use super::sleep;
use crate::view::FleetView;

static FRAME_ASKED: AtomicBool = AtomicBool::new(false);

/// [`FleetView::shown`] scrolled the sidebar; the next wait redraws the window so the row is painted.
pub(crate) fn ask_frame() {
    FRAME_ASKED.store(true, Ordering::SeqCst);
}

pub(crate) fn frame_if_asked(cx: &mut AsyncWindowContext) {
    if FRAME_ASKED.swap(false, Ordering::SeqCst) {
        cx.update(|window, _| window.refresh()).ok();
    }
}

pub(crate) async fn until_with<T>(
    cx: &mut AsyncWindowContext,
    within: Duration,
    what: &str,
    mut check: impl FnMut(&mut AsyncWindowContext) -> Option<T>,
) -> anyhow::Result<T> {
    let started = Instant::now();
    loop {
        if let Some(t) = check(cx) {
            return Ok(t);
        }
        if started.elapsed() > within {
            bail!("{what}: not within {within:?}");
        }
        frame_if_asked(cx);
        sleep(cx, Duration::from_millis(25)).await;
    }
}

/// The main window drawn once more from now: a hit key forgotten before it is back only if that frame painted it.
pub(crate) async fn drawn(cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let main = cx.window_handle();
    drawn_in(main, cx).await
}

/// `window` (the main one, a settings window) drawn once more from now.
pub(crate) async fn drawn_in(window: AnyWindowHandle, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let flag = std::rc::Rc::new(std::cell::Cell::new(false));
    let set = flag.clone();
    window.update(cx, |_, window, _| {
        window.on_next_frame(move |_, _| set.set(true));
        window.refresh();
    })?;
    until_with(cx, Duration::from_secs(5), "a frame drawn", |_| flag.get().then_some(())).await
}

pub(crate) async fn until<T>(
    view: &Entity<FleetView>,
    cx: &mut AsyncWindowContext,
    within: Duration,
    what: &str,
    f: impl Fn(&FleetView) -> Option<T>,
) -> anyhow::Result<T> {
    until_with(cx, within, what, |cx| view.read_with(cx, |v, _| f(v))).await
}

pub(crate) async fn until_app<T>(
    view: &Entity<FleetView>,
    cx: &mut AsyncWindowContext,
    within: Duration,
    what: &str,
    f: impl Fn(&FleetView, &App) -> Option<T>,
) -> anyhow::Result<T> {
    until_with(cx, within, what, |cx| view.read_with(cx, |v, cx| f(v, cx))).await
}
