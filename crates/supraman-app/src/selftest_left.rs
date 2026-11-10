//! `--selftest` step 7c: the sidebar's action bar, its Settings window opened and closed (by FleetView and by WM_CLOSE) with no teardown line in the log, and the tree clipped under the Inbox with real fonts; real windows a gpui test does not make.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail};
use gpui::{AsyncWindowContext, Bounds, Entity, Modifiers, MouseMoveEvent, PlatformInput, Pixels, WindowHandle};

use crate::{
    selftest::{WORKSPACE, click, drawn, drawn_in, log_offset, teardown_errors_since, until, until_with, win},
    selftest_manage::{shot, shot_window},
    settings_window::{SettingsView, page},
    view::FleetView,
};

const ICONS: [&str; 5] = ["add-workspace", "expand-all", "collapse-all", "refresh", "bar-settings"];
const WITHIN: Duration = Duration::from_secs(5);

pub(crate) fn bounds(v: &FleetView, key: &str) -> Option<Bounds<Pixels>> {
    v.shown(key).then(|| v.hits.borrow().get(key).map(|h| h.bounds)).flatten()
}

fn repo_rows(v: &FleetView) -> usize {
    v.folder_rows(WORKSPACE)
}

fn painted_repo_rows(v: &FleetView) -> usize {
    let prefix = format!("repo-row-{WORKSPACE}/");
    v.hits.borrow().keys().filter(|k| k.starts_with(&prefix)).count()
}

pub async fn action_bar(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let mut sizes = Vec::new();
    for key in ICONS.iter().chain(&["toggle-stopped", "busy-dec", "busy-inc"]) {
        let b = until(view, cx, WITHIN, &format!("{key} painted"), |v| bounds(v, key)).await?;
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

    let open_rows = until(view, cx, WITHIN, "repo rows of the gate workspace", |v| Some(repo_rows(v)).filter(|n| *n > 0)).await?;
    click(view, "collapse-all", cx).await?;
    until(view, cx, WITHIN, "Collapse all hides the repo rows", |v| (repo_rows(v) == 0).then_some(())).await?;
    click(view, "expand-all", cx).await?;
    until(view, cx, WITHIN, "Expand all shows the repo rows again", |v| (repo_rows(v) == open_rows && painted_repo_rows(v) > 0).then_some(())).await?;

    let shown = view.read_with(cx, |v, _| v.show_stopped);
    click(view, "toggle-stopped", cx).await?;
    until(view, cx, WITHIN, "the stopped toggle flips", |v| (v.show_stopped != shown).then_some(())).await?;
    click(view, "toggle-stopped", cx).await?;
    until(view, cx, WITHIN, "the stopped toggle flips back", |v| (v.show_stopped == shown).then_some(())).await?;

    let busy = view.read_with(cx, |v, _| v.max_working);
    // the default is the upper bound (MAX_WORKING_LIMIT): lower first
    click(view, "busy-dec", cx).await?;
    until(view, cx, WITHIN, "busy − lowers the limit", |v| (v.max_working + 1 == busy).then_some(())).await?;
    click(view, "busy-inc", cx).await?;
    until(view, cx, WITHIN, "busy + raises it back", |v| (v.max_working == busy).then_some(())).await?;

    let position = view
        .read_with(cx, |v, _| bounds(v, "collapse-all"))
        .ok_or_else(|| anyhow!("collapse-all gone"))?
        .center();
    cx.update(|window, cx| {
        window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position, pressed_button: None, modifiers: Modifiers::default() }), cx);
    })?;
    if crate::selftest_manage::shooting() {
        supraman_ui::tooltip::LAST_SHOWN.with(|l| l.borrow_mut().take());
        until_with(cx, WITHIN, "collapse-all's tooltip shown", |_| supraman_ui::tooltip::LAST_SHOWN.with(|l| l.borrow().is_some()).then_some(())).await?;
        drawn(cx).await?;
        shot("m5-13-bar-tooltip", cx).await;
    }

    click(view, "bar-settings", cx).await?;
    until(view, cx, WITHIN, "Settings opens its window", |v| v.settings_window).await?;
    view.update(cx, |v, cx| v.open_settings_at(Some(page::ABOUT), cx));
    let handle = view.read_with(cx, |v, _| v.settings_window).ok_or_else(|| anyhow!("the settings window closed"))?;
    let page = handle.read_with(cx, |s, _| s.page()).map_err(|e| anyhow!("settings window: {e}"))?;
    if page != page::ABOUT {
        bail!("open_settings_at(About) left the settings window on {page:?}");
    }
    shown_and_painted(handle, cx).await?;
    shot_window("m5-13-settings-about", Some("Supraman Settings"), cx).await;
    let hwnd = handle.update(cx, |_, window, _| win::hwnd_of(window)).ok().flatten();
    let closing = log_offset();
    view.update(cx, |v, cx| v.close_settings(cx));
    closed(handle, hwnd, cx).await?;
    let errors = teardown_errors_since(closing);
    if !errors.is_empty() {
        bail!("close_settings logged {} window-teardown line(s): {}", errors.len(), errors.join(" | "));
    }
    let closed = close_settings_by_wm_close(view, cx).await?;

    Ok(format!(
        "action bar: {} icons {:.0}×{:.0} px on one line; collapse all hid {open_rows} repo row(s), expand all restored them; \
         stopped toggle and busy ±1 (from {busy}) round-tripped; Settings opened, About shown; {closed}",
        ICONS.len(),
        f32::from(first.size.width),
        f32::from(first.size.height),
    ))
}

/// Closing a window before Windows has shown, activated and painted it races those messages (F-06b).
pub(crate) async fn shown_and_painted(handle: WindowHandle<SettingsView>, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    until_with(cx, WITHIN, "the settings window shown", |cx| {
        handle.update(cx, |_, window, _| crate::tray::is_shown(window)).ok().filter(|shown| *shown).map(|_| ())
    })
    .await?;
    drawn_in(handle.into(), cx).await.map_err(|e| anyhow!("settings window: {e}"))
}

/// The settings window gone and its HWND destroyed, then a frame of the main window: a teardown's log lines are written by then.
async fn closed(handle: WindowHandle<SettingsView>, hwnd: Option<isize>, cx: &mut AsyncWindowContext) -> anyhow::Result<Duration> {
    let asked = Instant::now();
    until_with(cx, WITHIN, "the settings window closed and its HWND destroyed", |cx| {
        (handle.read_with(cx, |_, _| ()).is_err() && win::destroyed(hwnd)).then_some(())
    })
    .await?;
    let gone = asked.elapsed();
    drawn(cx).await?;
    Ok(gone)
}

#[cfg(windows)]
async fn close_settings_by_wm_close(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    click(view, "bar-settings", cx).await?;
    let handle = until_with(cx, WITHIN, "Settings opens a second time", |cx| {
        view.read_with(cx, |v, _| v.settings_window).filter(|handle| handle.read_with(cx, |_, _| ()).is_ok())
    })
    .await?;
    shown_and_painted(handle, cx).await?;
    let hwnd = handle
        .update(cx, |_, window, _| crate::tray::hwnd(window))
        .ok()
        .flatten()
        .ok_or_else(|| anyhow!("the settings window has no HWND"))?;
    let closing = log_offset();
    win::post_close(hwnd)?;
    let gone = closed(handle, Some(hwnd.0 as isize), cx).await?;
    let errors = teardown_errors_since(closing);
    if !errors.is_empty() {
        bail!("closing the settings window logged {} window-teardown line(s): {}", errors.len(), errors.join(" | "));
    }
    Ok(format!("Settings closed again by WM_CLOSE in {} ms; no window-teardown line in the log", gone.as_millis()))
}

#[cfg(not(windows))]
async fn close_settings_by_wm_close(_view: &Entity<FleetView>, _cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    Ok(String::new())
}

fn painted_part(h: &supraman_ui::hit::Hit) -> Option<Bounds<Pixels>> {
    let b = h.bounds.intersect(&h.visible);
    (b.size.width > Pixels::ZERO && b.size.height > Pixels::ZERO).then_some(b)
}

const SEAM_MIN: f32 = 4.;

fn tree_row(key: &str) -> bool {
    ["ws-row-", "repo-row-", "session-row-"].iter().any(|p| key.starts_with(p))
}

fn overlaps(a: &Bounds<Pixels>, b: &Bounds<Pixels>) -> bool {
    a.left() < b.right() && b.left() < a.right() && a.top() < b.bottom() && b.top() < a.bottom()
}

pub async fn tree_clip(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let inbox = until(view, cx, WITHIN, "the Inbox painted", |v| v.hits.borrow().get("inbox-area").map(|h| h.visible)).await?;
    let tree = until(view, cx, WITHIN, "the tree painted", |v| v.hits.borrow().get("tree-area").map(|h| h.visible)).await?;
    let sections: Pixels = view.read_with(cx, |v, _| {
        let hits = v.hits.borrow();
        ["messages-area", "notices-area", "search-box"]
            .iter()
            .filter_map(|k| hits.get(*k).map(|h| h.bounds))
            .filter(|b| b.top() >= inbox.bottom() && b.bottom() <= tree.top())
            .fold(Pixels::ZERO, |sum, b| sum + b.size.height)
    });
    let between = tree.top() - inbox.bottom() - sections;
    if f32::from(between) > 20. {
        let mut inside: Vec<String> = view.read_with(cx, |v, _| {
            v.hits.borrow().iter().filter(|(_, h)| h.bounds.top() >= inbox.bottom() && h.bounds.bottom() <= tree.top()).map(|(k, h)| format!("{k} {:.0}", f32::from(h.bounds.size.height))).collect()
        });
        inside.sort();
        bail!(
            "the tree starts at y {:.1}, {:.1} px under the Inbox ({:.1} px besides the Messages, notices and search box): something sits between them: {inside:?}",
            f32::from(tree.top()),
            f32::from(tree.top() - inbox.bottom()),
            f32::from(between)
        );
    }
    let position = tree.center();
    let wheel = |dy: f32, cx: &mut AsyncWindowContext| {
        cx.update(|window, cx| {
            window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position, pressed_button: None, modifiers: Modifiers::default() }), cx);
            let event = gpui::ScrollEvent { position, delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(dy))), ..Default::default() };
            window.dispatch_event(PlatformInput::Scroll(event), cx);
        })
    };
    wheel(100_000., cx)?;
    drawn(cx).await?;
    let mut checked = Vec::new();
    for step in [0., 7., 8., 15., 30., 100.] {
        if step > 0. {
            wheel(-step, cx)?;
            drawn(cx).await?;
        }
        let (offset, last, area, rows) = view.read_with(cx, |v, _| {
            let hits = v.hits.borrow();
            let last = hits
                .iter()
                .filter(|(k, _)| k.starts_with("inbox-ws-"))
                .filter_map(|(k, h)| painted_part(h).map(|p| (k.clone(), p)))
                .max_by(|a, b| a.1.bottom().partial_cmp(&b.1.bottom()).unwrap_or(std::cmp::Ordering::Equal));
            let rows: Vec<(String, Bounds<Pixels>)> =
                hits.iter().filter(|(k, _)| tree_row(k)).filter_map(|(k, h)| painted_part(h).map(|p| (k.clone(), p))).collect();
            let top = hits.iter().filter(|(k, _)| tree_row(k)).map(|(_, h)| f32::from(h.bounds.top())).fold(f32::MAX, f32::min);
            (f32::from(tree.top()) - top, last, hits.get("inbox-area").map(|h| h.visible), rows)
        });
        let (last_key, last) = last.ok_or_else(|| anyhow!("no Inbox row painted"))?;
        let area = area.ok_or_else(|| anyhow!("the Inbox is gone"))?;
        if let Some((key, b)) = rows.iter().find(|(_, b)| overlaps(b, &last) || overlaps(b, &area)) {
            bail!(
                "tree scrolled {offset:.1} px: {key} paints {:.1}..{:.1}, over the Inbox ({:.1}..{:.1}, last row {last_key} {:.1}..{:.1})",
                f32::from(b.top()),
                f32::from(b.bottom()),
                f32::from(area.top()),
                f32::from(area.bottom()),
                f32::from(last.top()),
                f32::from(last.bottom())
            );
        }
        // and apart: a row cut by the tree's clip right under one cut by the Inbox's reads as one
        if let Some((key, b)) = rows.iter().filter(|(_, b)| b.top() >= last.bottom()).min_by(|a, b| a.1.top().partial_cmp(&b.1.top()).unwrap_or(std::cmp::Ordering::Equal))
            && f32::from(b.top() - last.bottom()) < SEAM_MIN
        {
            bail!(
                "tree scrolled {offset:.1} px: {key} paints from {:.1}, {:.1} px under the last Inbox row {last_key} (ends {:.1}); at least {SEAM_MIN} px expected",
                f32::from(b.top()),
                f32::from(b.top() - last.bottom()),
                f32::from(last.bottom())
            );
        }
        checked.push(format!("{offset:.0}"));
    }
    shot("m5-16-tree-scrolled", cx).await;
    Ok(format!(
        "Inbox {:.1}..{:.1}, tree {:.1}..{:.1}: at tree offsets {} px no tree row painted over the Inbox or its last row",
        f32::from(inbox.top()),
        f32::from(inbox.bottom()),
        f32::from(tree.top()),
        f32::from(tree.bottom()),
        checked.join(", ")
    ))
}

pub(crate) fn move_to(position: gpui::Point<Pixels>, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    cx.update(|window, cx| {
        window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position, pressed_button: None, modifiers: Modifiers::default() }), cx);
    })
}

/// The painted icon slot and label of each `(key, level)` row sit on `view::grid`.
pub fn on_grid(v: &FleetView, rows: &[(String, usize)]) -> anyhow::Result<String> {
    use crate::view::grid::{GAP, ICON, STEP};
    let hits = v.hits.borrow();
    let mut levels: BTreeMap<usize, (f32, f32)> = BTreeMap::new();
    let mut slot = None;
    for (key, level) in rows {
        let icon = hits.get(&format!("tree-icon-{key}")).ok_or_else(|| anyhow!("{key}: no icon slot painted"))?.bounds;
        let label = hits.get(&format!("tree-label-{key}")).ok_or_else(|| anyhow!("{key}: no label painted"))?.bounds;
        let size = (f32::from(icon.size.width), f32::from(icon.size.height));
        if *slot.get_or_insert(size) != size {
            bail!("{key}: icon slot {size:?}, another row's {:?}", slot.unwrap());
        }
        let (x, at) = (f32::from(icon.left()), f32::from(label.left()));
        if (at - x - ICON - GAP).abs() > 0.5 {
            bail!("{key}: label at x {at:.1}, {:.1} px after its icon at {x:.1}, not {}", at - x, ICON + GAP);
        }
        let (first, _) = *levels.entry(*level).or_insert((x, at));
        if (x - first).abs() > 0.5 {
            bail!("{key}: icon at x {x:.1}, another level-{level} row's at {first:.1}");
        }
    }
    for (level, (x, at)) in &levels {
        if let Some((child, _)) = levels.get(&(level + 1)) {
            if (child - at).abs() > 0.5 || (child - x - STEP).abs() > 0.5 {
                bail!("level {} icons at x {child:.1}, level {level} labels at {at:.1}: a child's icon does not start under its parent's label", level + 1);
            }
        }
    }
    let xs: Vec<String> = levels.iter().map(|(l, (x, at))| format!("L{l} icon {x:.0} label {at:.0}")).collect();
    Ok(format!("{} rows on the grid ({})", rows.len(), xs.join(", ")))
}

/// Every painted row of the gate workspace is laid out on the one grid, levels 0 to 2 at least; the gate's own
/// session row (`own`, level 2) is scrolled into view, so the levels are painted wherever the rows sort.
pub async fn tree_grid(view: &Entity<FleetView>, own: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    view.update(cx, |v, cx| v.collapse_all(false, cx));
    let own = format!("session-row-{own}");
    let rows = until(view, cx, WITHIN, "the gate workspace's rows at three levels", |v| {
        v.shown(&own);
        let rows = tree_rows(v);
        let levels: std::collections::HashSet<usize> = rows.iter().map(|(_, l)| *l).collect();
        (levels.len() >= 3).then_some(rows)
    })
    .await
    .map_err(|e| e.context(format!("rows painted {:?}", view.read_with(cx, |v, _| tree_rows(v)))))?;
    let checked = view.read_with(cx, |v, _| on_grid(v, &rows))?;
    shot("m5-70-tree", cx).await;
    Ok(checked)
}

fn tree_rows(v: &FleetView) -> Vec<(String, usize)> {
    let hits = v.hits.borrow();
    let painted = |key: &str| hits.contains_key(&format!("tree-icon-{key}")).then(|| key.to_string());
    let mut rows = Vec::new();
    for w in v.tree().workspaces.iter().filter(|w| w.name == WORKSPACE) {
        rows.extend(painted(&format!("ws-{}", w.name)).map(|k| (k, 0)));
        for s in &w.supervised {
            rows.extend(painted(&format!("session-{}{}", s.session.session_id, s.scope(&w.name))).map(|k| (k, 1)));
        }
        for r in &w.repos {
            rows.extend(painted(&format!("repo-{}/{}", w.name, r.name)).map(|k| (k, 1)));
            for s in &r.sessions {
                rows.extend(painted(&format!("session-{}{}", s.session.session_id, s.scope(&w.name))).map(|k| (k, 2)));
            }
        }
    }
    rows
}
