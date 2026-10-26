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

/// `--selftest-supervise` step e2 (m5/10): another Foreman's parked session. A fake `fm-elsewhere-<hex>-park`
/// row, blocked on a permission prompt with its cwd in `workspace`'s repo, is added to every snapshot
/// ([`FleetView::injected`]): over two ticks the panel shows it under that repo, dimmed with "another
/// Foreman", and the Inbox has no row for it, while this home's own park (`own_park`) is still Blocked there.
pub async fn foreign_row(
    view: &Entity<FleetView>,
    workspace: &str,
    cwd: &std::path::Path,
    own_park: &str,
    hex: &str,
    cx: &mut AsyncWindowContext,
) -> anyhow::Result<String> {
    let name = format!("fm-elsewhere-{hex}-park");
    let session_id = format!("ffff{hex}-foreign");
    let json = serde_json::json!([{
        "id": format!("ffff{hex}"),
        "sessionId": session_id,
        "name": name,
        "cwd": cwd,
        "kind": "background",
        "startedAt": foreman_claude::now_ms() - 60_000,
        "state": "blocked",
        "status": "waiting",
        "waitingFor": "permission prompt",
        "pid": std::process::id(),
    }]);
    let rows = foreman_claude::parse(&json.to_string())?;
    view.read_with(cx, |v, _| v.injected.lock().unwrap_or_else(|e| e.into_inner()).extend(rows));
    let result = check_foreign(view, workspace, &name, &session_id, own_park, cx).await;
    view.read_with(cx, |v, _| v.injected.lock().unwrap_or_else(|e| e.into_inner()).clear());
    result
}

async fn check_foreign(
    view: &Entity<FleetView>,
    workspace: &str,
    name: &str,
    session_id: &str,
    own_park: &str,
    cx: &mut AsyncWindowContext,
) -> anyhow::Result<String> {
    let ticks = view.read_with(cx, |v, _| v.ticks);
    let started = Instant::now();
    let row = loop {
        let row = view.read_with(cx, |v, _| {
            let tree = v.tree();
            let ws = tree.workspaces.into_iter().find(|w| w.name == workspace)?;
            ws.repos.into_iter().flat_map(|r| r.sessions.into_iter().map(move |s| (r.name.clone(), s))).find(|(_, s)| s.session.name == name)
        });
        if let Some(row) = row
            && view.read_with(cx, |v, _| v.ticks >= ticks + 2)
        {
            break row;
        }
        if started.elapsed() > Duration::from_secs(30) {
            bail!("{name} not in {workspace}'s tree after two ticks within 30 s");
        }
        sleep(cx, Duration::from_millis(100)).await;
    };
    let (repo, row) = row;
    if !row.foreign {
        bail!("{name} under {workspace}/{repo} is not marked as another Foreman's: {:?}", row.health);
    }
    let label = format!("foreign-{session_id}");
    crate::selftest::scrolled_into_view(view, &label, cx).await?;
    let (needs, own) = view.read_with(cx, |v, _| {
        (
            v.needs.iter().filter(|n| n.name == name).count(),
            v.needs.iter().any(|n| n.name == own_park && n.kind == foreman_supervisor::NeedsKind::Blocked),
        )
    });
    if needs != 0 {
        bail!("{name}, another Foreman's, raised {needs} Inbox row(s)");
    }
    if !own {
        bail!("this home's own {own_park} is no longer Blocked in the Inbox");
    }
    shot("m5-10-another-foreman", cx).await;
    Ok(format!(
        "{name} (blocked on a permission prompt, injected) listed under {workspace}/{repo} as another Foreman's, \"another Foreman\" painted, \
         0 Inbox rows for it over {} ticks; this home's {own_park} still Blocked in the Inbox",
        view.read_with(cx, |v, _| v.ticks) - ticks
    ))
}
