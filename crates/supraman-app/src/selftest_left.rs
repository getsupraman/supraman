use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail};
use gpui::{AsyncWindowContext, Bounds, Entity, Modifiers, MouseMoveEvent, PlatformInput, Pixels, WindowHandle};

use crate::{
    selftest::{WORKSPACE, click, double_click, log_offset, sleep, teardown_errors_since, until, until_with},
    selftest_manage::{shot, shot_window},
    settings_window::{SettingsView, page},
    view::FleetView,
};

const ICONS: [&str; 5] = ["add-workspace", "expand-all", "collapse-all", "refresh", "bar-settings"];
const WITHIN: Duration = Duration::from_secs(5);

fn bounds(v: &FleetView, key: &str) -> Option<Bounds<Pixels>> {
    v.shown(key).then(|| v.hits.borrow().get(key).map(|h| h.bounds)).flatten()
}

fn repo_rows(v: &FleetView) -> usize {
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
    until(view, cx, WITHIN, "Expand all shows the repo rows again", |v| (repo_rows(v) == open_rows).then_some(())).await?;

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
    sleep(cx, Duration::from_millis(900)).await;
    shot("m5-13-bar-tooltip", cx).await;

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
    let closing = log_offset();
    view.update(cx, |v, cx| v.close_settings(cx));
    sleep(cx, Duration::from_millis(1000)).await;
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
    let painted = std::rc::Rc::new(std::cell::Cell::new(false));
    let flag = painted.clone();
    handle
        .update(cx, |_, window, _| {
            window.on_next_frame(move |_, _| flag.set(true));
            window.refresh();
        })
        .map_err(|e| anyhow!("settings window: {e}"))?;
    until_with(cx, WITHIN, "a frame of the settings window painted", |_| painted.get().then_some(())).await
}

#[cfg(windows)]
async fn close_settings_by_wm_close(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};
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
    unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) }.map_err(|e| anyhow!("WM_CLOSE: {e}"))?;
    let asked = Instant::now();
    while handle.read_with(cx, |_, _| ()).is_ok() {
        if asked.elapsed() > WITHIN {
            bail!("the settings window is still open {} s after WM_CLOSE", WITHIN.as_secs());
        }
        sleep(cx, Duration::from_millis(50)).await;
    }
    let gone = asked.elapsed();
    // gpui destroys the HWND on a later turn of its executor; give that time to log
    sleep(cx, Duration::from_millis(1000)).await;
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

pub async fn foreign_row(
    view: &Entity<FleetView>,
    workspace: &str,
    cwd: &std::path::Path,
    own_park: &str,
    hex: &str,
    cx: &mut AsyncWindowContext,
) -> anyhow::Result<String> {
    let other: String = hex.chars().rev().collect();
    let name = format!("fm-selftest-{other}x-park");
    let session_id = format!("ffff{hex}-foreign");
    let json = serde_json::json!([{
        "id": format!("ffff{hex}"),
        "sessionId": session_id,
        "name": name,
        "cwd": cwd,
        "kind": "background",
        "startedAt": supraman_claude::now_ms() - 60_000,
        "state": "blocked",
        "status": "waiting",
        "waitingFor": "permission prompt",
        "pid": std::process::id(),
    }]);
    let rows = supraman_claude::parse(&json.to_string())?;
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
            let ws = tree.workspaces.clone().into_iter().find(|w| w.name == workspace)?;
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
        bail!("{name} under {workspace}/{repo} is not marked as another Supraman's: {:?}", row.health);
    }
    let label = format!("foreign-{session_id}");
    crate::selftest::scrolled_into_view(view, &label, cx).await?;
    let short = &row.session.id;
    let actions = [format!("attach-{short}"), format!("resume-{short}"), format!("tab-link-{short}")];
    if let Some(key) = view.read_with(cx, |v, _| actions.iter().find(|k| v.hits.borrow().contains_key(*k)).cloned()) {
        bail!("{name}, another Supraman's, paints the action {key}");
    }
    let (tabs, toast) = view.read_with(cx, |v, _| (v.tabs.len(), v.toast.text().map(String::from)));
    double_click(view, &format!("session-row-{session_id}"), cx).await?;
    sleep(cx, Duration::from_millis(500)).await;
    let after = view.read_with(cx, |v, _| (v.tabs.len(), v.toast.text().map(String::from)));
    if after != (tabs, toast.clone()) {
        bail!("a double-click on {name}, another Supraman's, did something: tabs {tabs} → {}, toast {toast:?} → {:?}", after.0, after.1);
    }
    let (needs, own) = view.read_with(cx, |v, _| {
        (
            v.needs.iter().filter(|n| n.name == name).count(),
            v.needs.iter().any(|n| n.name == own_park && n.kind == supraman_supervisor::NeedsKind::Blocked),
        )
    });
    if needs != 0 {
        bail!("{name}, another Supraman's, raised {needs} Inbox row(s)");
    }
    if !own {
        bail!("this home's own {own_park} is no longer Blocked in the Inbox");
    }
    shot("m5-10-another-foreman", cx).await;
    Ok(format!(
        "{name} (blocked on a permission prompt, injected) listed under {workspace}/{repo} as another Supraman's (its name muted under foreign-<id>, no action, a double-click opens nothing), \
         0 Inbox rows for it over {} ticks; this home's {own_park} still Blocked in the Inbox",
        view.read_with(cx, |v, _| v.ticks) - ticks
    ))
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
    sleep(cx, Duration::from_millis(300)).await;
    let mut checked = Vec::new();
    for step in [0., 7., 8., 15., 30., 100.] {
        if step > 0. {
            wheel(-step, cx)?;
            sleep(cx, Duration::from_millis(300)).await;
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

const TABLE: [&str; 5] = [
    "selftest\tproj\topen\tselftest\t—\t0/6 done",
    "  selftest-shout-textkit     run    open    shout: textkit.shout(text)    —    0/1 done",
    "    selftest-shout-textkit-fn  task  open  Add textkit.shout(text) with unit tests  ✗0/3",
    "  selftest-shout-textcli     run    open    shout: textcli greet --shout    —    0/1 done",
    "    selftest-shout-textcli-flag  task  open  greet --shout prints textkit.shout(greet(name))  ✗0/3",
];

static TABLE_FILE: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

pub(crate) fn move_to(position: gpui::Point<Pixels>, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    cx.update(|window, cx| {
        window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position, pressed_button: None, modifiers: Modifiers::default() }), cx);
    })
}

pub async fn steady_click(
    view: &Entity<FleetView>,
    key: &str,
    outbox: &std::path::Path,
    from_name: &str,
    cx: &mut AsyncWindowContext,
) -> anyhow::Result<(String, Bounds<Pixels>)> {
    let mut void = Vec::new();
    for _ in 0..3 {
        match held_arrival(view, key, outbox, from_name, cx).await? {
            Ok((said, aimed)) if void.is_empty() => return Ok((said, aimed)),
            Ok((said, aimed)) => return Ok((format!("{said} (retaken: {})", void.join("; ")), aimed)),
            Err(why) => void.push(why),
        }
    }
    bail!("the hold under the pointer was released and armed again before the message was read, in each of 3 tries: {}", void.join("; "))
}

/// `Err` when the hold was armed again meanwhile: the panel saw the pointer leave (an OS mouse message, or a rest), which shows held rows by design.
async fn held_arrival(
    view: &Entity<FleetView>,
    key: &str,
    outbox: &std::path::Path,
    from_name: &str,
    cx: &mut AsyncWindowContext,
) -> anyhow::Result<Result<(String, Bounds<Pixels>), String>> {
    let aimed = crate::selftest::scrolled_into_view(view, key, cx).await?;
    let target = aimed.center();
    move_to(target, cx)?;
    let armed = view.read_with(cx, |v, _| v.left_holds);
    let file = format!("{:013}-selftest-table.json", supraman_claude::now_ms());
    let mut body = vec!["Plan for the selftest (a work_tree, not a question):".to_string()];
    body.extend(TABLE.iter().map(|l| l.to_string()));
    let text = serde_json::json!({"from": "uds:selftest", "from_name": from_name, "body": body.join("\n")}).to_string();
    std::fs::write(outbox.join(format!("{file}.tmp")), text)?;
    std::fs::rename(outbox.join(format!("{file}.tmp")), outbox.join(&file))?;
    *TABLE_FILE.lock().unwrap_or_else(|e| e.into_inner()) = Some(file.clone());
    // a hand still aiming moves a little; the hold lasts while it does
    let started = Instant::now();
    let mut nudge = 0.;
    loop {
        let read = view.read_with(cx, |v, _| v.messages.values().flatten().any(|m| m.file == file));
        if read {
            break;
        }
        if started.elapsed() > Duration::from_secs(15) {
            bail!("the app did not read {file} from the outbox within 15 s");
        }
        nudge = if nudge == 0. { 1. } else { 0. };
        move_to(gpui::point(target.x + gpui::px(nudge), target.y), cx)?;
        sleep(cx, Duration::from_millis(250)).await;
    }
    let read_after = started.elapsed();
    move_to(target, cx)?;
    sleep(cx, Duration::from_millis(300)).await;
    let (now, held, header, holds) =
        view.read_with(cx, |v, _| (bounds(v, key), v.left_held(), v.hits.borrow().contains_key("messages-area"), v.left_holds));
    if holds != armed {
        return Ok(Err(format!("{file} read {} ms in, the hold armed {} more time(s)", read_after.as_millis(), holds - armed)));
    }
    let now = now.ok_or_else(|| anyhow!("{key} is no longer painted after the message arrived"))?;
    shot("m5-18-hover-hold", cx).await;
    if now != aimed {
        bail!(
            "a Messages row arrived {} ms in and moved {key} from y {:.1} to y {:.1} under the pointer (held: {held})",
            read_after.as_millis(),
            f32::from(aimed.top()),
            f32::from(now.top())
        );
    }
    cx.update(|window, cx| {
        let modifiers = Modifiers::default();
        window.dispatch_event(
            PlatformInput::MouseDown(gpui::MouseDownEvent { button: gpui::MouseButton::Left, position: target, modifiers, click_count: 1, first_mouse: false }),
            cx,
        );
        window.dispatch_event(PlatformInput::MouseUp(gpui::MouseUpEvent { button: gpui::MouseButton::Left, position: target, modifiers, click_count: 1 }), cx);
    })?;
    let said = format!(
        "pointer on {key} at y {:.1}; {file} read {} ms later and held back (Messages section {}), {key} still at y {:.1}; clicked there",
        f32::from(aimed.top()),
        read_after.as_millis(),
        if header { "showing older rows only" } else { "not shown" },
        f32::from(now.top())
    );
    Ok(Ok((said, aimed)))
}

pub async fn table_lines(view: &Entity<FleetView>, key: &str, aimed: Bounds<Pixels>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let button = key.to_string();
    let file = TABLE_FILE.lock().unwrap_or_else(|e| e.into_inner()).clone().ok_or_else(|| anyhow!("no table message written"))?;
    let right = view.read_with(cx, |v, _| v.left_width) + gpui::px(200.);
    move_to(gpui::point(right, gpui::px(300.)), cx)?;
    let key = until(view, cx, WITHIN, "the held message once the pointer left", |v| {
        (!v.left_held()).then(|| v.messages.iter().find_map(|(w, msgs)| msgs.iter().any(|m| m.file == file).then(|| format!("{w}/{file}")))).flatten()
    })
    .await?;
    sleep(cx, Duration::from_millis(300)).await;
    let moved = view.read_with(cx, |v, _| bounds(v, &button)).ok_or_else(|| anyhow!("{button} gone once the pointer left"))?;
    if moved == aimed {
        bail!("{button} stayed at y {:.1} after the held row appeared too: the arrival never moved it, so the hold was not tested", f32::from(aimed.top()));
    }
    view.update(cx, |v, cx| {
        v.expanded.insert(key.clone());
        cx.notify();
    });
    let prefix = format!("msg-table-{key}#");
    let lines = until(view, cx, WITHIN, "the table's lines painted", |v| {
        let hits = v.hits.borrow();
        let mut lines: Vec<(String, Bounds<Pixels>)> = hits.iter().filter(|(k, _)| k.starts_with(&prefix)).map(|(k, h)| (k.clone(), h.bounds)).collect();
        (lines.len() >= TABLE.len()).then(|| {
            lines.sort_by(|a, b| a.1.top().partial_cmp(&b.1.top()).unwrap_or(std::cmp::Ordering::Equal));
            lines
        })
    })
    .await?;
    crate::selftest::scrolled_into_view(view, &lines[0].0, cx).await?;
    shot("m5-18-messages-table", cx).await;
    if lines.len() != TABLE.len() {
        bail!("{} table lines painted for {} rows: {:?}", lines.len(), TABLE.len(), lines.iter().map(|l| &l.0).collect::<Vec<_>>());
    }
    let heights: Vec<f32> = lines.iter().map(|(_, b)| f32::from(b.size.height)).collect();
    let (low, high) = heights.iter().fold((f32::MAX, 0f32), |(l, h), x| (l.min(*x), h.max(*x)));
    if high - low > 0.5 || high > 24. {
        bail!("table rows painted {heights:?} px high: a row wrapped");
    }
    for pair in lines.windows(2) {
        if (f32::from(pair[1].1.top() - pair[0].1.bottom())).abs() > 0.5 {
            bail!("table rows {} and {} are not consecutive lines", pair[0].0, pair[1].0);
        }
    }
    Ok(format!(
        "pointer left: the held row appeared and moved {button} to y {:.1} (what the hold kept from under the pointer); its {}-row table painted {} lines, each {low:.1} px, unwrapped",
        f32::from(moved.top()),
        TABLE.len(),
        lines.len()
    ))
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

/// Every painted row of the gate workspace is laid out on the one grid, levels 0 to 2 at least.
pub async fn tree_grid(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    view.update(cx, |v, cx| v.collapse_all(false, cx));
    let rows = until(view, cx, WITHIN, "the gate workspace's rows at three levels", |v| {
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
