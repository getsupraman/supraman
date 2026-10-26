//! `--selftest` step 7c (m5/13): the left panel's action bar. Every icon is painted at one size on one line;
//! Collapse all hides the gate workspace's repo rows and Expand all brings them back; the stopped toggle and
//! the busy stepper change what they say and are put back; hovering an icon shows its tooltip (screenshotted
//! with `FOREMAN_SHOTS`); Settings opens the settings window, which `open_settings_at` turns to About.

use std::time::{Duration, Instant};

use anyhow::{anyhow, bail};
use gpui::{AsyncWindowContext, Bounds, Entity, Modifiers, MouseMoveEvent, PlatformInput, Pixels};

use crate::{
    selftest::{WORKSPACE, click, sleep},
    selftest_manage::{shot, shot_window},
    settings_window::Page,
    view::FleetView,
};

/// Square icon buttons, left to right as painted.
const ICONS: [&str; 4] = ["add-workspace", "expand-all", "collapse-all", "bar-settings"];
const WITHIN: Duration = Duration::from_secs(5);

async fn until<T>(view: &Entity<FleetView>, cx: &mut AsyncWindowContext, what: &str, f: impl Fn(&FleetView) -> Option<T>) -> anyhow::Result<T> {
    let started = Instant::now();
    loop {
        if let Some(t) = view.read_with(cx, |v, _| f(v)) {
            return Ok(t);
        }
        if started.elapsed() > WITHIN {
            bail!("{what}: not within {} s", WITHIN.as_secs());
        }
        sleep(cx, Duration::from_millis(50)).await;
    }
}

fn bounds(v: &FleetView, key: &str) -> Option<Bounds<Pixels>> {
    v.hits.borrow().get(key).map(|h| h.bounds)
}

fn repo_rows(v: &FleetView) -> usize {
    let prefix = format!("repo-row-{WORKSPACE}/");
    v.hits.borrow().keys().filter(|k| k.starts_with(&prefix)).count()
}

pub async fn action_bar(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let mut sizes = Vec::new();
    for key in ICONS.iter().chain(&["toggle-stopped", "busy-dec", "busy-inc"]) {
        let b = until(view, cx, &format!("{key} painted"), |v| bounds(v, key)).await?;
        sizes.push((*key, b));
    }
    let first = sizes[0].1;
    for (key, b) in &sizes {
        let mid = |b: &Bounds<Pixels>| f32::from(b.center().y);
        if (mid(b) - mid(&first)).abs() > 1.0 {
            bail!("{key} is centred at y {:.1}, add-workspace at y {:.1}: the bar is not one line", mid(b), mid(&first));
        }
    }
    for (key, b) in sizes.iter().filter(|(k, _)| ICONS.contains(k)) {
        if b.size != first.size {
            bail!("{key} is {:?}, add-workspace {:?}: the icons differ in size", b.size, first.size);
        }
    }

    let open_rows = until(view, cx, "repo rows of the gate workspace", |v| Some(repo_rows(v)).filter(|n| *n > 0)).await?;
    click(view, "collapse-all", cx).await?;
    until(view, cx, "Collapse all hides the repo rows", |v| (repo_rows(v) == 0).then_some(())).await?;
    click(view, "expand-all", cx).await?;
    until(view, cx, "Expand all shows the repo rows again", |v| (repo_rows(v) == open_rows).then_some(())).await?;

    let shown = view.read_with(cx, |v, _| v.show_stopped);
    click(view, "toggle-stopped", cx).await?;
    until(view, cx, "the stopped toggle flips", |v| (v.show_stopped != shown).then_some(())).await?;
    click(view, "toggle-stopped", cx).await?;
    until(view, cx, "the stopped toggle flips back", |v| (v.show_stopped == shown).then_some(())).await?;

    let busy = view.read_with(cx, |v, _| v.max_working);
    click(view, "busy-inc", cx).await?;
    until(view, cx, "busy + raises the limit", |v| (v.max_working == busy + 1).then_some(())).await?;
    click(view, "busy-dec", cx).await?;
    until(view, cx, "busy − lowers it back", |v| (v.max_working == busy).then_some(())).await?;

    // hover: the tooltip, and the icon's hover fill, in the picture
    let position = view
        .read_with(cx, |v, _| bounds(v, "collapse-all"))
        .ok_or_else(|| anyhow!("collapse-all gone"))?
        .center();
    cx.update(|window, cx| {
        window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position, pressed_button: None, modifiers: Modifiers::default() }), cx);
    })?;
    sleep(cx, Duration::from_millis(900)).await;
    shot("m5-13-bar-tooltip", cx).await;

    click(view, "bar-settings", cx).await?;
    until(view, cx, "Settings opens its window", |v| v.settings_window).await?;
    view.update(cx, |v, cx| v.open_settings_at(Some(Page::About), cx));
    let handle = view.read_with(cx, |v, _| v.settings_window).ok_or_else(|| anyhow!("the settings window closed"))?;
    let page = handle.read_with(cx, |s, _| s.page()).map_err(|e| anyhow!("settings window: {e}"))?;
    if page != Page::About {
        bail!("open_settings_at(About) left the settings window on {page:?}");
    }
    shot_window("m5-13-settings-about", Some("Foreman Settings"), cx).await;
    view.update(cx, |v, cx| v.close_settings(cx));

    Ok(format!(
        "action bar: {} icons {:.0}×{:.0} px on one line; collapse all hid {open_rows} repo row(s), expand all restored them; \
         stopped toggle and busy ±1 (from {busy}) round-tripped; Settings opened, About shown",
        ICONS.len(),
        f32::from(first.size.width),
        f32::from(first.size.height),
    ))
}
