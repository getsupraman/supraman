//! `--selftest` step 7b: managing workspaces through the window.

use std::{
    cell::Cell,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Mutex,
    thread::ThreadId,
    time::{Duration, Instant},
};

use anyhow::{Context as _, anyhow, bail};
use supraman_claude::Session;
use supraman_core::Registry;
use gpui::{
    AsyncWindowContext, Entity, KeyDownEvent, Keystroke, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PlatformInput, SharedString,
    point, px,
};

use crate::{
    selftest::{cargo_root, click, press, sleep, until, until_app},
    view::{FleetView, MenuAction, workspace_dialog::WorkspaceDialog},
};

pub const FOLDER: &str = "selftest-add";
pub const SINGLE: &str = "selftest-single";
const ADDED: &str = "selftest-added";
const CUSTOM: &str = "selftest-custom";
const WITHIN: Duration = Duration::from_secs(5);

fn painted(v: &FleetView, key: &str) -> bool {
    v.shown(key)
}

fn dialog<'a>(v: &FleetView, cx: &'a gpui::App) -> Option<&'a WorkspaceDialog> {
    v.add_dialog().map(|d| d.read(cx))
}

/// The dialog's rows as `(name, included, home)`.
fn rows(v: &FleetView, cx: &gpui::App) -> Option<Vec<(String, Option<bool>, bool)>> {
    dialog(v, cx).filter(|d| !d.scanning).map(|d| d.list.rows.iter().map(|r| (r.folder.name.clone(), r.included, r.home)).collect())
}

/// The dialog's name (`name`) or path field set to `text`, and the dialog painted again.
fn set_field(view: &Entity<FleetView>, name: bool, text: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    view.update(cx, |v, cx| {
        let dialog = v.add_dialog().cloned().ok_or_else(|| anyhow!("no Add workspace dialog"))?;
        dialog.update(cx, |d, cx| {
            let field = if name { d.name.clone() } else { d.path.clone() };
            field.update(cx, |f, cx| f.set_text(text, cx));
            cx.notify();
        });
        Ok(())
    })
}

fn typed(cx: &mut AsyncWindowContext, key: &str, key_char: Option<&str>) -> anyhow::Result<()> {
    let keystroke = Keystroke { modifiers: Modifiers::default(), key: key.into(), key_char: key_char.map(Into::into) };
    cx.update(|window, cx| {
        window.dispatch_event(PlatformInput::KeyDown(KeyDownEvent { keystroke, is_held: false, prefer_character_input: false }), cx);
    })?;
    Ok(())
}

/// As the OS queues a real click or key: through gpui's window procedure (and `TranslateMessage`), not straight to `dispatch_event`.
fn post(cx: &mut AsyncWindowContext, messages: &[(u32, usize, isize)]) -> anyhow::Result<()> {
    use windows::Win32::{
        Foundation::{LPARAM, WPARAM},
        UI::WindowsAndMessaging::PostMessageW,
    };
    let hwnd = cx.update(|window, _| crate::tray::hwnd(window))?.ok_or_else(|| anyhow!("no HWND"))?;
    for &(message, wparam, lparam) in messages {
        unsafe { PostMessageW(Some(hwnd), message, WPARAM(wparam), LPARAM(lparam)) }.map_err(|e| anyhow!("PostMessage: {e}"))?;
    }
    Ok(())
}

async fn post_click(view: &Entity<FleetView>, key: &str, dx: f32, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    use windows::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE};
    let bounds = crate::selftest::scrolled_into_view(view, key, cx).await?;
    let scale = cx.update(|window, _| window.scale_factor())?;
    let x = ((f32::from(bounds.left()) + dx) * scale) as isize;
    let y = (f32::from(bounds.center().y) * scale) as isize;
    let at = (y << 16) | (x & 0xffff);
    post(cx, &[(WM_MOUSEMOVE, 0, at), (WM_LBUTTONDOWN, 1, at), (WM_LBUTTONUP, 0, at)])?;
    sleep(cx, Duration::from_millis(150)).await;
    Ok(())
}

/// A key that types goes on to `TranslateMessage`'s WM_CHAR, queued behind it: read what it typed with [`until_app`], never right after this returns (F-04).
pub(crate) async fn post_keys(keys: &[&str], cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    use windows::Win32::UI::{
        Input::KeyboardAndMouse::{
            GetAsyncKeyState, GetKeyState, GetKeyboardState, MAPVK_VK_TO_VSC, MapVirtualKeyW, SetKeyboardState, VK_BACK, VK_CONTROL, VK_END,
            VK_ESCAPE, VK_LCONTROL, VK_LEFT, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU, VK_NEXT, VK_PRIOR, VK_RCONTROL, VK_RETURN, VK_RIGHT, VK_RMENU,
            VK_RSHIFT, VK_RWIN, VK_SHIFT, VkKeyScanW,
        },
        WindowsAndMessaging::{WM_KEYDOWN, WM_KEYUP},
    };
    // gpui takes a key's modifiers from GetKeyState, which a posted message does not set: a held modifier would turn End into Alt+End.
    let held = || {
        [(VK_CONTROL, "Ctrl"), (VK_MENU, "Alt"), (VK_SHIFT, "Shift"), (VK_LWIN, "Win"), (VK_RWIN, "Win")]
            .into_iter()
            .filter(|(vk, _)| unsafe { GetKeyState(vk.0 as i32) } < 0)
            .map(|(_, name)| name)
            .collect::<Vec<_>>()
    };
    let stale = held();
    if !stale.is_empty() {
        let live: Vec<&str> = [(VK_CONTROL, "Ctrl"), (VK_MENU, "Alt"), (VK_SHIFT, "Shift")]
            .into_iter()
            .filter(|(vk, _)| unsafe { GetAsyncKeyState(vk.0 as i32) } as u16 & 0x8000 != 0)
            .map(|(_, name)| name)
            .collect();
        let mut state = [0u8; 256];
        unsafe { GetKeyboardState(&mut state) }.map_err(|e| anyhow!("GetKeyboardState: {e}"))?;
        for vk in [VK_CONTROL, VK_LCONTROL, VK_RCONTROL, VK_MENU, VK_LMENU, VK_RMENU, VK_SHIFT, VK_LSHIFT, VK_RSHIFT, VK_LWIN, VK_RWIN] {
            state[vk.0 as usize] &= !0x80;
        }
        unsafe { SetKeyboardState(&state) }.map_err(|e| anyhow!("SetKeyboardState: {e}"))?;
        println!("keys: {stale:?} held in the UI thread's key state (held on the keyboard: {live:?}); cleared there before posting {keys:?}");
    }
    let started = Instant::now();
    while !held().is_empty() {
        if started.elapsed() > WITHIN {
            bail!("{:?} held in this thread's key state for {} s: posted keys would carry it (another process's input?)", held(), WITHIN.as_secs());
        }
        sleep(cx, Duration::from_millis(50)).await;
    }
    for key in keys {
        let vk = match *key {
            "end" => VK_END.0 as u32,
            "left" => VK_LEFT.0 as u32,
            "right" => VK_RIGHT.0 as u32,
            "pageup" => VK_PRIOR.0 as u32,
            "pagedown" => VK_NEXT.0 as u32,
            "backspace" => VK_BACK.0 as u32,
            "enter" => VK_RETURN.0 as u32,
            "escape" => VK_ESCAPE.0 as u32,
            c => (unsafe { VkKeyScanW(c.encode_utf16().next().unwrap_or_default()) } & 0xff) as u32,
        };
        let scan = unsafe { MapVirtualKeyW(vk, MAPVK_VK_TO_VSC) } as isize;
        post(cx, &[(WM_KEYDOWN, vk as usize, 1 | (scan << 16)), (WM_KEYUP, vk as usize, 1 | (scan << 16) | (0b11 << 30))])?;
    }
    sleep(cx, Duration::from_millis(150)).await;
    Ok(())
}

fn registry_has(view: &Entity<FleetView>, name: &str, cx: &mut AsyncWindowContext) -> bool {
    view.read_with(cx, |v, _| v.home.as_ref().is_some_and(|h| Registry::at(h).exists(name).unwrap_or(false)))
}

async fn edit_name(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let name = |v: &FleetView, cx: &gpui::App| {
        dialog(v, cx).map(|d| {
            let n = d.name.read(cx);
            (n.text().to_string(), n.line.cursor())
        })
    };
    let now = |cx: &mut AsyncWindowContext| view.read_with(cx, |v, cx| name(v, cx));
    post_click(view, "add-name-text", 1., cx).await?;
    let at_start = Some((FOLDER.to_string(), 0));
    until_app(view, cx, WITHIN, "the caret at the start of the name after a click there", |v, cx| (name(v, cx) == at_start).then_some(()))
        .await
        .map_err(|e| e.context(format!("(name, caret) {:?}, expected ({FOLDER}, 0)", now(cx))))?;
    let seen: std::rc::Rc<std::cell::RefCell<Vec<String>>> = Default::default();
    let observer = cx.update(|_, cx| {
        let (seen, view) = (seen.clone(), view.clone());
        cx.observe_keystrokes(move |e, window, cx| {
            let focused = view.read(cx).input_focused(window, cx);
            let action = e.action.as_ref().map_or("-".to_string(), |a| a.name().to_string());
            seen.borrow_mut().push(format!("{} [{action}{}]", e.keystroke.unparse(), if focused { "" } else { ", input not focused" }));
        })
    })?;
    post_keys(&["end", "backspace", "backspace", "backspace", "a", "d", "d", "e", "d"], cx).await?;
    let added = Some((ADDED.to_string(), ADDED.len()));
    let edited = until_app(view, cx, WITHIN, "the name edited by posted End, 3 Backspace and `added`", |v, cx| (name(v, cx) == added).then_some(())).await;
    drop(observer);
    edited.map_err(|e| e.context(format!("(name, caret) {:?}, expected ({ADDED}, {}); keystrokes {:?}", now(cx), ADDED.len(), seen.borrow())))?;
    let set_scanning = |on: bool| move |v: &mut FleetView, cx: &mut gpui::Context<FleetView>| v.add_dialog().cloned().map(|d| d.update(cx, |d, cx| {
        d.scanning = on;
        cx.notify();
    }));
    view.update(cx, set_scanning(true));
    until_app(view, cx, WITHIN, "Save dimmed while the folder is read", |v, cx| dialog(v, cx).is_some_and(|d| !d.can_save(cx)).then_some(())).await?;
    post_click(view, "add-save", 4., cx).await?;
    let (open, saved) = (view.read_with(cx, |v, _| v.adding.is_some()), registry_has(view, ADDED, cx));
    view.update(cx, set_scanning(false));
    if !open || saved {
        bail!("Save while the folder was still read: dialog open {open}, {ADDED} registered {saved}");
    }
    Ok(format!("name by posted messages: click → caret 0, edited to {ADDED}; Save while reading: dimmed, nothing saved"))
}

/// The move is dispatched: a posted WM_MOUSEMOVE is undone by the WM_MOUSELEAVE the real cursor's position sends at once.
async fn cut_label_tooltip(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let key = format!("ws-root-{ADDED}");
    let full = view.read_with(cx, |v, _| v.workspaces.iter().find(|w| w.name == ADDED).and_then(|w| w.home().map(|h| h.display().to_string())));
    let full = full.ok_or_else(|| anyhow!("{ADDED} is not in the tree"))?;
    let row = crate::selftest::scrolled_into_view(view, &format!("ws-row-{ADDED}"), cx).await?;
    let started = Instant::now();
    let bounds = loop {
        let inside = |b: &gpui::Bounds<gpui::Pixels>| b.size.width > px(40.) && b.center().y > row.top() && b.center().y < row.bottom();
        if let Some(b) = supraman_ui::clipped::cut_at(&key).filter(inside) {
            break b;
        }
        if started.elapsed() > WITHIN {
            bail!("{ADDED}'s root path {full:?} was not painted cut, 40 px or wider: {:?}", supraman_ui::clipped::cut_at(&key));
        }
        sleep(cx, Duration::from_millis(50)).await;
    };
    supraman_ui::tooltip::LAST_SHOWN.with(|l| l.borrow_mut().take());
    let position = bounds.center();
    let started = Instant::now();
    loop {
        cx.update(|window, cx| {
            let event = MouseMoveEvent { position, pressed_button: None, modifiers: Modifiers::default() };
            window.dispatch_event(PlatformInput::MouseMove(event), cx);
        })?;
        sleep(cx, Duration::from_millis(100)).await;
        let shown = supraman_ui::tooltip::LAST_SHOWN.with(|l| l.borrow().clone());
        if shown.as_deref() == Some(full.as_str()) {
            break;
        }
        if started.elapsed() > WITHIN {
            bail!("hovering {ADDED}'s cut root path at {position:?} showed the tooltip {shown:?}, expected {full:?}");
        }
    }
    shot("cut-label-tooltip", cx).await;
    Ok(format!("{ADDED}'s root path painted cut ({:.0} px); hovered, its tooltip reads the whole path", f32::from(bounds.size.width)))
}

/// Resting on a workspace, a folder and a session row shows no card while each row's icon keeps its tooltip;
/// Details in the row's menu opens the row's Details tab, and a second Details focuses it.
async fn row_details(view: &Entity<FleetView>, own: &Session, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let folder = until(view, cx, WITHIN, &format!("a folder row of {ADDED}"), |v| {
        v.hits.borrow().keys().find_map(|k| k.strip_prefix(&format!("repo-row-{ADDED}/")).map(str::to_string))
    })
    .await?;
    let ws_row = format!("ws-row-{ADDED}");
    let session_row = format!("session-row-{}", own.session_id);
    let rows = [
        (ws_row.clone(), format!("work-{ADDED}")),
        (format!("repo-row-{ADDED}/{folder}"), format!("terminal-{ADDED}/{folder}")),
        (session_row.clone(), format!("star-{}", own.session_id)),
    ];
    for (row, icon) in &rows {
        no_card_on(view, row, icon, cx).await?;
    }

    let home = view.read_with(cx, |v, _| v.home.clone()).ok_or_else(|| anyhow!("no Supraman home"))?;
    let (ws_tab, ws) = details_tab(view, &ws_row, ADDED, cx).await?;
    let wants = [
        format!("{ADDED}\n"),
        format!("\nName  {ADDED}\n"),
        "\nState  Never started\n".into(),
        "\nalpha  ".into(),
        " (disabled)\n".into(),
        "\nWorkspace session\nNot wanted.".into(),
    ];
    for want in wants {
        if !ws.starts_with(ADDED) || !format!("{ws}\n").contains(&want) {
            bail!("the Details tab of {ADDED} reads {ws:?}, without {want:?}");
        }
    }
    crate::selftest_schedules::themed_shots(&home, "sched2-5-details-workspace", cx).await;
    let (session_tab, session) = details_tab(view, &session_row, &own.name, cx).await?;
    let name = format!("\nName  {}\n", own.name);
    let cwd = format!("\nWorking directory  {}\n", own.cwd.display());
    let folder_line = session.lines().find(|l| l.starts_with("Folder  ")).unwrap_or_default().to_string();
    if !session.contains(&name) || !session.contains(&cwd) || folder_line.is_empty() || folder_line.contains("none of") || !session.contains("\nState  ") {
        bail!("the Details tab of {} reads {session:?}, not its name, its folder and {cwd:?}", own.name);
    }
    if own.id != own.session_id && !session.contains(&format!("\nUUID  {}\n", own.session_id)) {
        bail!("the Details tab of {} reads {session:?}, without its UUID", own.name);
    }
    crate::selftest_schedules::themed_shots(&home, "sched2-5-details-session", cx).await;
    let tabs = view.read_with(cx, |v, _| v.tabs.len());
    let (again, _) = details_tab(view, &session_row, &own.name, cx).await?;
    if again != session_tab || view.read_with(cx, |v, _| v.tabs.len()) != tabs {
        bail!("a second Details of {} opened tab {again} ({} tabs), not its tab {session_tab} again ({tabs})", own.name, view.read_with(cx, |v, _| v.tabs.len()));
    }
    let starred = view.read_with(cx, |v, _| v.stars.ids().contains(&own.session_id));
    let star = format!("details-session-{}-star", own.session_id);
    for want in [!starred, starred] {
        crate::selftest::scrolled_into_view(view, &star, cx).await?;
        click(view, &star, cx).await?;
        until(view, cx, WITHIN, &format!("the page's Star made {} starred {want}", own.name), |v| (v.stars.ids().contains(&own.session_id) == want).then_some(())).await?;
        let label = if want { "Unstar" } else { "Star" };
        until(view, cx, Duration::from_secs(15), &format!("the page's action reads {label} after a poll"), |v| {
            let page = v.tab(session_tab).and_then(|t| match &t.kind { crate::view::TabKind::Details(p) => p.seen.as_ref(), _ => None })?;
            (page.starred == want && painted(v, &star)).then_some(())
        })
        .await?;
    }
    if let Some(card) = supraman_ui::hover_card::LAST_CARD.with(|l| l.borrow_mut().take()) {
        bail!("a hover card showed during the Details checks: {card:?}");
    }
    view.update(cx, |v, cx| {
        v.close_tab(session_tab, cx);
        v.close_tab(ws_tab, cx);
    });
    Ok(format!(
        "no card on {ADDED}'s row, folder {folder} or {}'s row, their icons' tooltips shown; Details opened tab {ws_tab}: {:?}; tab {session_tab}: {:?}; a second Details focused tab {session_tab}; its Star starred and unstarred it",
        own.name,
        ws.replace('\n', " | "),
        session.replace('\n', " | ")
    ))
}

/// Details in the row's menu opens or focuses a Details tab; its id and text once it is the active tab.
async fn details_tab(view: &Entity<FleetView>, row: &str, title: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<(u64, String)> {
    press_painted(view, row, MouseButton::Right, cx).await?;
    let keys = menu_open(view, title, cx).await?;
    if !keys.iter().any(|k| k == "details") {
        bail!("the menu of {title} has {keys:?}, no details");
    }
    click(view, "menu-details", cx).await?;
    let found = until_app(view, cx, WITHIN, &format!("the Details tab of {title}"), |v, cx| {
        let tab = v.active.and_then(|id| v.tab(id))?;
        let crate::view::TabKind::Details(page) = &tab.kind else { return None };
        let text = v.details_text(&page.target, cx)?;
        (v.confirm.is_none() && text.starts_with(title)).then_some((tab.id, text))
    })
    .await?;
    Ok(found)
}

/// Rests the pointer near the row's left edge, off its icons and labels, long past the tooltip delay: no card shows.
/// Then on the row's `icon`, whose tooltip shows, so the rest was a real hover.
async fn no_card_on(view: &Entity<FleetView>, row: &str, icon: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let bounds = crate::selftest::scrolled_into_view(view, row, cx).await?;
    let position = point(bounds.left() + px(4.), bounds.top() + px(6.));
    crate::selftest_left::move_to(point(px(1.), px(1.)), cx)?;
    sleep(cx, Duration::from_millis(100)).await;
    supraman_ui::hover_card::LAST_CARD.with(|l| l.borrow_mut().take());
    let started = Instant::now();
    // past the card's 500 ms delay; no gpui test builds FleetView's sidebar, so only the window can show a card's absence
    while started.elapsed() < Duration::from_millis(800) {
        crate::selftest_left::move_to(position, cx)?;
        sleep(cx, Duration::from_millis(50)).await;
    }
    if let Some(card) = supraman_ui::hover_card::LAST_CARD.with(|l| l.borrow_mut().take()) {
        bail!("resting on {row} at {position:?} showed the card {card:?}");
    }
    let at = crate::selftest::scrolled_into_view(view, icon, cx).await?.center();
    supraman_ui::tooltip::LAST_SHOWN.with(|l| l.borrow_mut().take());
    let started = Instant::now();
    loop {
        crate::selftest_left::move_to(at, cx)?;
        sleep(cx, Duration::from_millis(50)).await;
        if supraman_ui::tooltip::LAST_SHOWN.with(|l| l.borrow().is_some()) {
            break;
        }
        if started.elapsed() > WITHIN {
            bail!("resting on {icon} at {at:?} showed no tooltip");
        }
    }
    crate::selftest_left::move_to(point(px(1.), px(1.)), cx)?;
    if let Some(card) = supraman_ui::hover_card::LAST_CARD.with(|l| l.borrow_mut().take()) {
        bail!("resting on {icon} showed the card {card:?}");
    }
    Ok(())
}

/// Every Edit folders… row keeps its buttons inside the dialog at the left panel's minimum width; Cancel saves nothing.
async fn edit_folders_fit(view: &Entity<FleetView>, registry: &Registry, workspace: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    const LONG: &str = "a-folder-with-a-rather-long-name-for-edit-folders";
    const BRANCH: &str = "copilot/fix-the-edit-folders-dialog-overflowing-its-box";
    let long = cargo_root().join("target").join("selftest-edit-long").join(LONG);
    std::fs::create_dir_all(long.join(".git"))?;
    std::fs::write(long.join(".git").join("HEAD"), format!("ref: refs/heads/{BRANCH}\n"))?;
    let before = registry.load(workspace)?;
    let width = view.read_with(cx, |v, _| v.left_width);
    cx.update(|window, cx| {
        view.update(cx, |v, cx| {
            v.left_width = crate::view::MIN_LEFT_WIDTH;
            v.edit_folders(workspace, window, cx);
        })
    })?;
    until(view, cx, WITHIN, "Edit folders…'s path field", |v| painted(v, "edit-new-path-text").then_some(())).await?;
    set_field(view, false, &long.display().to_string(), cx)?;
    click(view, "edit-add", cx).await?;
    let folders = until_app(view, cx, WITHIN, "the long folder in the list", |v, cx| {
        let d = dialog(v, cx)?;
        d.list.rows.iter().any(|r| r.folder.path == long).then(|| d.list.rows.iter().map(|r| r.folder.name.clone()).collect::<Vec<_>>())
    })
    .await?;
    let long_name = folders.last().cloned().ok_or_else(|| anyhow!("no long folder"))?;
    let fits = until(view, cx, WITHIN, "every Edit folders row inside the dialog", |v| {
        let hits = v.hits.borrow();
        let dialog = hits.get("edit-dialog")?.bounds;
        let mut rights = Vec::new();
        for f in &folders {
            for key in [format!("edit-home-{f}"), format!("edit-remove-{f}")] {
                let b = hits.get(&key)?.bounds;
                if b.left() < dialog.left() || b.right() > dialog.right() + px(0.5) {
                    return None;
                }
            }
            rights.push(hits.get(&format!("edit-remove-{f}"))?.bounds.right());
        }
        let same = rights.windows(2).all(|p| (f32::from(p[0]) - f32::from(p[1])).abs() < 0.5);
        same.then_some((f32::from(dialog.size.width), rights.first().copied().map(f32::from)))
    })
    .await;
    if fits.is_err() {
        shot("m5-63-edit-folders-min", cx).await;
    }
    let fits = fits.map_err(|e| {
        let found = view.read_with(cx, |v, _| {
            let hits = v.hits.borrow();
            let mut keys: Vec<String> = hits.keys().filter(|k| k.starts_with("edit-")).map(|k| format!("{k} {:?}", hits[k].bounds)).collect();
            keys.sort();
            keys
        });
        e.context(format!("painted {found:?}"))
    })?;
    let cut = [format!("edit-folder-path-{long_name}"), format!("edit-branch-{long_name}")];
    for key in &cut {
        until(view, cx, WITHIN, &format!("{key} painted cut"), |_| supraman_ui::clipped::cut_at(key).map(|_| ())).await?;
    }
    if !view.read_with(cx, |v, _| painted(v, "edit-elsewhere-alpha")) {
        bail!("alpha in {workspace} shows no `supervised in` badge");
    }
    shot("m5-63-edit-folders-min", cx).await;
    view.update(cx, |v, cx| {
        v.left_width = width;
        cx.notify();
    });
    click(view, "edit-cancel", cx).await?;
    until(view, cx, WITHIN, "Edit folders… closed by Cancel", |v| v.adding.is_none().then_some(())).await?;
    if registry.load(workspace)? != before {
        bail!("Cancel in Edit folders… changed {workspace}");
    }
    Ok(format!(
        "Edit folders… of {workspace} at the minimum left width ({:.0} px dialog): {folders:?} keep their buttons inside, × at x {:?} in every row, {LONG}'s path and branch {BRANCH} clipped with a tooltip; Cancel saved nothing",
        fits.0, fits.1
    ))
}

/// Edit folders…: beta removed by ×, a folder added by its typed path, home moved to alpha; Save writes all three.
async fn edit_folders(view: &Entity<FleetView>, registry: &Registry, root: &Path, ws_key: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let extra = cargo_root().join("target").join("selftest-edit-extra");
    std::fs::create_dir_all(&extra)?;
    press_painted(view, ws_key, MouseButton::Right, cx).await?;
    menu_open(view, ADDED, cx).await?;
    click(view, "menu-edit-folders", cx).await?;
    until(view, cx, WITHIN, "Edit folders… with beta's ×", |v| painted(v, "edit-remove-beta").then_some(())).await?;
    shot("edit-folders", cx).await;
    click(view, "edit-remove-beta", cx).await?;
    until_app(view, cx, WITHIN, "beta out of the list", |v, cx| rows(v, cx).filter(|r| r.len() == 1 && r[0].0 == "alpha").map(|_| ())).await?;
    set_field(view, false, &extra.display().to_string(), cx)?;
    click(view, "edit-add", cx).await?;
    until(view, cx, WITHIN, "the typed folder's home button", |v| painted(v, "edit-home-selftest-edit-extra").then_some(())).await?;
    click(view, "edit-home-alpha", cx).await?;
    let want = vec![("alpha".to_string(), None, true), ("selftest-edit-extra".to_string(), None, false)];
    until_app(view, cx, WITHIN, "home moved to alpha", |v, cx| rows(v, cx).filter(|r| r == &want).map(|_| ())).await?;
    if registry.load(ADDED)?.folders.iter().any(|f| f.name != "alpha" && f.name != "beta") {
        bail!("Edit folders… wrote {ADDED} before Save");
    }
    click(view, "edit-save", cx).await?;
    until(view, cx, WITHIN, "Edit folders… closed on Save", |v| v.adding.is_none().then_some(())).await?;
    let saved = until(view, cx, WITHIN, "the edit in the registry", |_| registry.load(ADDED).ok().filter(|w| w.folders.len() == 2 && w.home.as_deref() == Some(root.join("alpha").as_path()))).await;
    let saved = saved.map_err(|e| e.context(format!("registered {:?}", registry.load(ADDED))))?;
    let folders: Vec<&Path> = saved.folders.iter().map(|f| f.path.as_path()).collect();
    let excluded: Vec<&Path> = saved.scans.iter().flat_map(|s| s.exclude.iter().map(PathBuf::as_path)).collect();
    if folders != [root.join("alpha").as_path(), extra.as_path()] || excluded != [root.join("notes").as_path(), root.join("beta").as_path()] {
        bail!("Edit folders… saved folders {folders:?} with the parent excluding {excluded:?}; expected alpha and the typed folder, notes and beta excluded");
    }
    let extra_row = format!("repo-row-{ADDED}/selftest-edit-extra");
    until(view, cx, WITHIN, "the typed folder in the tree", |v| painted(v, &extra_row).then_some(())).await?;
    Ok(format!("Edit folders… of {ADDED}: beta removed by × (into the parent's exclude), {} added by its typed path, home moved to alpha; saved at once by Save (no wanted session in the fixture, so no question)", extra.display()))
}

/// Rescan offers a new subfolder unticked; Save leaves it unticked and unskipped, so the next Rescan offers it again;
/// ticked and saved, it is a folder and the third Rescan offers nothing. Beta (×) and notes are never offered.
async fn rescan(view: &Entity<FleetView>, registry: &Registry, root: &Path, ws_key: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let fresh = root.join("fresh");
    std::fs::create_dir_all(&fresh)?;
    let open = async |view: &Entity<FleetView>, cx: &mut AsyncWindowContext| -> anyhow::Result<Vec<(String, Option<bool>, bool)>> {
        until(view, cx, WITHIN, "no dialog", |v| v.adding.is_none().then_some(())).await?;
        press_painted(view, ws_key, MouseButton::Right, cx).await?;
        menu_open(view, ADDED, cx).await?;
        click(view, "menu-rescan", cx).await?;
        let listed = until_app(view, cx, WITHIN, "Rescan's dialog", |v, cx| rows(v, cx)).await?;
        until(view, cx, WITHIN, "Rescan's Save painted", |v| painted(v, "edit-save").then_some(())).await?;
        Ok(listed)
    };
    let offered = |listed: &[(String, Option<bool>, bool)]| listed.iter().filter(|r| r.1.is_some()).map(|r| (r.0.clone(), r.1)).collect::<Vec<_>>();
    let listed = open(view, cx).await?;
    if offered(&listed) != [("fresh".to_string(), Some(false))] {
        bail!("Rescan listed {listed:?}; expected fresh offered unticked, beta and notes not");
    }
    if registry.load(ADDED)?.folders.iter().any(|f| f.path == fresh) {
        bail!("Rescan saved fresh before Save");
    }
    shot("rescan-offer", cx).await;
    click(view, "edit-save", cx).await?;
    until(view, cx, WITHIN, "Rescan's dialog closed on Save", |v| v.adding.is_none().then_some(())).await?;
    let w = registry.load(ADDED)?;
    if w.folders.iter().any(|f| f.path == fresh) || w.scans.iter().any(|s| s.exclude.contains(&fresh)) {
        bail!("an unticked fresh was saved: folders {:?}, scans {:?}", w.folders, w.scans);
    }
    let listed = open(view, cx).await?;
    if offered(&listed) != [("fresh".to_string(), Some(false))] {
        bail!("the second Rescan listed {listed:?}; expected fresh offered again");
    }
    click(view, "edit-include-fresh", cx).await?;
    until_app(view, cx, WITHIN, "fresh ticked", |v, cx| rows(v, cx).filter(|r| r.iter().any(|r| r.0 == "fresh" && r.1 == Some(true))).map(|_| ())).await?;
    click(view, "edit-save", cx).await?;
    until(view, cx, WITHIN, "fresh in the registry", |_| registry.load(ADDED).ok()?.folders.iter().any(|f| f.path == fresh).then_some(())).await?;
    let listed = open(view, cx).await?;
    if !offered(&listed).is_empty() {
        bail!("the third Rescan listed {listed:?}; expected nothing offered");
    }
    until(view, cx, WITHIN, "Rescan's Cancel painted", |v| painted(v, "edit-cancel").then_some(())).await?;
    click(view, "edit-cancel", cx).await?;
    until(view, cx, WITHIN, "Rescan's dialog closed on Cancel", |v| v.adding.is_none().then_some(())).await?;
    Ok("Rescan offered the new fresh unticked (beta, removed by ×, and notes not); Save left it out and unskipped, the second Rescan offered it again; ticked and saved it is a folder, and the third Rescan offers nothing".into())
}

pub(crate) fn fake_repo(root: &Path, name: &str) -> anyhow::Result<()> {
    let git = root.join(name).join(".git");
    std::fs::create_dir_all(&git)?;
    std::fs::write(git.join("HEAD"), "ref: refs/heads/main\n")?;
    Ok(())
}

/// `SUPRAMAN_SHOTS` is set: the run takes screenshots, and the rests that only serve them are taken too.
pub(crate) fn shooting() -> bool {
    std::env::var_os("SUPRAMAN_SHOTS").is_some()
}

pub(crate) async fn shot(name: &str, cx: &mut AsyncWindowContext) {
    shot_window(name, None, cx).await
}

pub(crate) async fn shot_window(name: &str, title: Option<&str>, cx: &mut AsyncWindowContext) {
    let Some(dir) = std::env::var_os("SUPRAMAN_SHOTS").map(PathBuf::from) else {
        return;
    };
    sleep(cx, Duration::from_millis(300)).await;
    match capture(dir.join(format!("{name}.png")), title, cx).await {
        Ok(out) => println!("shot: {}", out.display()),
        Err(e) => println!("shot: {name} failed: {e:#}"),
    }
}

/// A PNG of this process's main window, or its window titled `title`, as the window paints itself.
pub(crate) async fn capture(out: PathBuf, title: Option<&str>, cx: &mut AsyncWindowContext) -> anyhow::Result<PathBuf> {
    let title = title.map(str::to_string);
    cx.background_executor()
        .spawn(async move {
            let hwnd = crate::selftest_capture::window(title.as_deref())?;
            crate::selftest_capture::print_window(hwnd, &out)?;
            Ok(out)
        })
        .await
}

pub(crate) async fn menu_open(view: &Entity<FleetView>, title: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<Vec<SharedString>> {
    let keys = until(view, cx, WITHIN, &format!("the menu of {title}"), |v| {
        v.menu.as_ref().filter(|m| m.title == title).map(|m| m.items.iter().map(|i| i.key.clone()).collect::<Vec<_>>())
    })
    .await
    .map_err(|e| {
        let state = view.read_with(cx, |v, _| {
            format!("menu {:?}, confirm {:?}", v.menu.as_ref().map(|m| m.title.clone()), v.confirm.as_ref().map(|c| c.text.clone()))
        });
        e.context(state)
    })?;
    let first = format!("menu-{}", keys.first().ok_or_else(|| anyhow!("the menu of {title} is empty"))?);
    until(view, cx, WITHIN, &format!("{first} painted"), |v| painted(v, &first).then_some(())).await?;
    Ok(keys)
}

/// [`press`] once `key` is painted (a row can be missing from the frame being drawn).
pub(crate) async fn press_painted(view: &Entity<FleetView>, key: &str, button: MouseButton, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    until(view, cx, WITHIN, &format!("{key} painted"), |v| painted(v, key).then_some(())).await.map_err(|e| {
        let (keys, rows) = view.read_with(cx, |v, _| {
            let mut keys: Vec<String> = v.hits.borrow().keys().filter(|k| k.starts_with("ws-row-")).cloned().collect();
            keys.sort();
            (keys, v.workspaces.iter().map(|w| w.name.clone()).collect::<Vec<_>>())
        });
        e.context(format!("painted rows {keys:?}; workspaces {rows:?}"))
    })?;
    // a menu closed just now still occludes the row in the last frame's hitboxes
    crate::selftest::drawn(cx).await?;
    press(view, key, button, cx).await
}

static OPENED_ON: Mutex<Option<ThreadId>> = Mutex::new(None);

/// As ShellExecuteW does while Explorer answers a folder's open (shell32, then gpui's window procedure
/// running a foreground task): the calling thread's messages are dispatched. Then the real call.
fn pumping_open(path: &Path) -> Result<(), String> {
    use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage};
    *OPENED_ON.lock().unwrap_or_else(|e| e.into_inner()) = Some(std::thread::current().id());
    let until = Instant::now() + Duration::from_millis(300);
    let mut msg = MSG::default();
    while Instant::now() < until {
        // SAFETY: a plain message loop on this thread's own queue.
        unsafe {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    supraman_ui::open::open(path.as_os_str())
}

/// Open folder from the workspace menu while foreground tasks keep arriving, as they do in a working Supraman; the
/// folder does not exist, so no Explorer opens and the refusal must come back as the action error.
async fn open_folder(view: &Entity<FleetView>, ws_key: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let missing = cargo_root().join("target").join("selftest-open-folder-missing");
    let _ = std::fs::remove_dir_all(&missing);
    press_painted(view, ws_key, MouseButton::Right, cx).await?;
    menu_open(view, ADDED, cx).await?;
    let real = view.update(cx, |v, cx| {
        for item in v.menu.iter_mut().flat_map(|m| m.items.iter_mut()).filter(|i| i.key == "open-folder") {
            item.action = MenuAction::OpenFolder(missing.clone());
        }
        v.action_error = None;
        cx.notify();
        std::mem::replace(&mut v.open_with, pumping_open)
    });
    // the click reaches the closures of the next frame
    sleep(cx, Duration::from_millis(200)).await;

    let (ticks, stop) = (Rc::new(Cell::new(0u32)), Rc::new(Cell::new(false)));
    let busy = (view.clone(), ticks.clone(), stop.clone());
    let foreground = cx.spawn(async move |cx| {
        while !busy.2.get() {
            cx.background_executor().timer(Duration::from_millis(1)).await;
            busy.0.read_with(cx, |_, _| ());
            busy.1.set(busy.1.get() + 1);
        }
    });
    post_click(view, "menu-open-folder", 4., cx).await?;
    let want = format!("Cannot open {}: ShellExecuteW returned 2", missing.display());
    let error = until(view, cx, WITHIN, "Open folder's refusal as the action error", |v| v.action_error.clone()).await;
    stop.set(true);
    foreground.await;
    let error = error?;
    view.update(cx, |v, _| {
        v.open_with = real;
        v.action_error = None;
    });
    if error != want {
        bail!("Open folder of a missing folder reads {error:?}, expected {want:?}");
    }
    let opened_on = OPENED_ON.lock().unwrap_or_else(|e| e.into_inner()).take();
    let ui = std::thread::current().id();
    if opened_on.is_none_or(|t| t == ui) {
        bail!("Open folder ran its ShellExecuteW on {opened_on:?}, the UI thread is {ui:?}");
    }
    Ok(format!(
        "Open folder in the workspace menu, its ShellExecuteW pumping messages off the UI thread ({opened_on:?}, UI {ui:?}) with {} foreground tasks run meanwhile: alive, \"{error}\"",
        ticks.get()
    ))
}

pub async fn run(view: &Entity<FleetView>, own: &Session, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let root = cargo_root().join("target").join(FOLDER);
    fake_repo(&root, "alpha")?;
    fake_repo(&root, "beta")?;
    let home = view.read_with(cx, |v, _| v.home.clone()).ok_or_else(|| anyhow!("no Supraman home"))?;
    let registry = Registry::at(&home);

    std::fs::create_dir_all(root.join("notes"))?;
    let _ = std::fs::remove_dir_all(root.join("fresh"));
    let all_in = |names: &[&str]| names.iter().map(|n| (n.to_string(), Some(true), false)).collect::<Vec<_>>();

    view.update(cx, |v, _| v.pick_folder = Some(root.clone()));
    click(view, "add-workspace", cx).await?;
    click(view, "menu-add-from-folder", cx).await?;
    let listed = until_app(view, cx, WITHIN, "the dialog's folders", |v, cx| rows(v, cx)).await?;
    let name = view.read_with(cx, |v, cx| dialog(v, cx).map(|d| d.name_text(cx)).unwrap_or_default());
    if name != FOLDER || listed != all_in(&["alpha", "beta", "notes"]) {
        bail!("the dialog offers name {name} with rows {listed:?}, expected {FOLDER} with alpha, beta and the plain notes, all ticked, no row home");
    }
    until(view, cx, WITHIN, "Save and the parent's path painted", |v| (painted(v, "add-save") && painted(v, "add-parent")).then_some(())).await?;
    click(view, "add-include-notes", cx).await?;
    until_app(view, cx, WITHIN, "notes unticked", |v, cx| rows(v, cx).filter(|r| r[2].1 == Some(false)).map(|_| ())).await?;
    let name_edit = edit_name(view, cx).await?;
    shot("add-review", cx).await;
    post_keys(&["enter"], cx).await?;
    until(view, cx, WITHIN, "the dialog closed on Enter", |v| v.adding.is_none().then_some(())).await?;
    let saved = registry.load(ADDED).context("the registry after Save")?;
    let flags: Vec<(String, bool)> = saved.folders.iter().map(|r| (r.name.clone(), r.enabled)).collect();
    if flags != [("alpha".to_string(), true), ("beta".to_string(), true)] {
        bail!("registered {ADDED} with folders {flags:?}, expected alpha and beta, enabled");
    }
    let excluded: Vec<&Path> = saved.scans.iter().flat_map(|s| s.exclude.iter().map(PathBuf::as_path)).collect();
    if saved.home() != Some(root.as_path()) || saved.scans.len() != 1 || excluded != [root.join("notes").as_path()] {
        bail!("registered {ADDED} with home {:?} and scans {:?}, expected home {} and one scan of it excluding notes", saved.home, saved.scans, root.display());
    }
    let alpha = format!("repo-row-{ADDED}/alpha");
    let in_tree = |v: &FleetView, repo: &str| v.tree().workspaces.iter().find(|w| w.name == ADDED).is_some_and(|w| w.repos.iter().any(|r| r.name == repo));
    until(view, cx, WITHIN, "alpha in the tree, the unticked notes not", |v| (painted(v, &alpha) && !in_tree(v, "notes")).then_some(())).await?;
    let mut out = vec![format!("added {ADDED} from {} (alpha and beta; notes unticked, in its scan's exclude), home the parent", root.display()), name_edit];
    out.push(cut_label_tooltip(view, cx).await?);

    view.update(cx, |v, _| v.pick_folder = Some(root.clone()));
    click(view, "add-workspace", cx).await?;
    click(view, "menu-add-from-folder", cx).await?;
    until_app(view, cx, WITHIN, "the second dialog's folders", |v, cx| rows(v, cx).filter(|r| r.len() == 3)).await?;
    until(view, cx, WITHIN, "alpha's `supervised in` painted", |v| painted(v, "add-elsewhere-alpha").then_some(())).await?;
    let again = view.read_with(cx, |v, cx| dialog(v, cx).map(|d| d.name_text(cx))).unwrap_or_default();
    click(view, "add-save", cx).await?;
    until(view, cx, WITHIN, "the second dialog closed on Save", |v| v.adding.is_none().then_some(())).await?;
    let second = registry.load(&again).with_context(|| format!("the same folder again, saved as {again}"))?;
    let flags: Vec<(String, bool)> = second.folders.iter().map(|r| (r.name.clone(), r.enabled)).collect();
    if flags != [("alpha".to_string(), false), ("beta".to_string(), false), ("notes".to_string(), true)] {
        bail!("the same folder again as {again}: folders {flags:?}, expected alpha and beta disabled (supervised in {ADDED}) and notes enabled");
    }
    let in_again = |v: &FleetView, repo: &str| v.tree().workspaces.iter().find(|w| w.name == again).is_some_and(|w| w.repos.iter().any(|r| r.name == repo));
    until(view, cx, WITHIN, "notes in the second workspace, alpha (supervised in the first) not", |v| {
        (painted(v, &format!("repo-row-{again}/notes")) && !in_again(v, "alpha") && !painted(v, &format!("supervised-in-{again}/alpha"))).then_some(())
    })
    .await?;
    view.update(cx, |v, cx| v.search.update(cx, |s, cx| s.set_text("alpha", cx)));
    let found = format!("result-supervised-in-{again}/alpha");
    until(view, cx, WITHIN, "search finds alpha in the second workspace, supervised in the first", |v| painted(v, &found).then_some(())).await?;
    view.update(cx, |v, cx| v.clear_search(cx));
    out.push(edit_folders_fit(view, &registry, &again, cx).await?);
    registry.remove(&again)?;
    until(view, cx, Duration::from_secs(15), "the second workspace gone", |v| (!v.workspaces.iter().any(|w| w.name == again)).then_some(())).await?;
    out.push(format!("the same folder again registers as {again} with alpha and beta disabled, alpha out of its tree and found by search as supervised in {ADDED}; removed"));
    view.update(cx, |v, cx| v.apply_edit(ADDED, supraman_core::Edit::SetEnabled { folder: "beta".into(), enabled: false }, cx));
    let beta_row = format!("repo-row-{ADDED}/beta");
    until(view, cx, WITHIN, "beta disabled, out of the tree and its painted rows", |v| (!in_tree(v, "beta") && !v.hits.borrow().contains_key(&beta_row)).then_some(())).await?;
    out.push(row_details(view, own, cx).await?);

    let session_key = format!("session-row-{}", own.session_id);
    press_painted(view, &session_key, MouseButton::Right, cx).await?;
    let keys = menu_open(view, &own.name, cx).await?;
    shot("menu-session", cx).await;
    typed(cx, "escape", None)?;
    until(view, cx, WITHIN, "the session menu closed on Esc", |v| v.menu.is_none().then_some(())).await?;
    out.push(format!("session menu {keys:?} closed by Esc"));

    let ws_key = format!("ws-row-{ADDED}");
    press_painted(view, &ws_key, MouseButton::Right, cx).await?;
    let keys = menu_open(view, ADDED, cx).await?;
    shot("menu-workspace", cx).await;
    cx.update(|window, cx| {
        // the right panel's bottom-right corner, far from the menu and from any button
        let size = window.viewport_size();
        let outside = point(size.width - px(24.), size.height - px(24.));
        let modifiers = Modifiers::default();
        let down = MouseDownEvent { button: MouseButton::Left, position: outside, modifiers, click_count: 1, first_mouse: false };
        window.dispatch_event(PlatformInput::MouseDown(down), cx);
        let up = MouseUpEvent { button: MouseButton::Left, position: outside, modifiers, click_count: 1 };
        window.dispatch_event(PlatformInput::MouseUp(up), cx);
    })?;
    until(view, cx, WITHIN, "the workspace menu closed by a click outside", |v| v.menu.is_none().then_some(())).await?;
    out.push(format!("workspace menu {keys:?} closed by a click outside"));

    // a menu closed but still painted takes the next press as a click outside it
    until(view, cx, WITHIN, "a frame without the closed menu", |v| (!painted(v, "menu-remove")).then_some(())).await?;
    out.push(edit_folders(view, &registry, &root, &ws_key, cx).await?);
    out.push(rescan(view, &registry, &root, &ws_key, cx).await?);

    out.push(open_folder(view, &ws_key, cx).await?);

    press_painted(view, &ws_key, MouseButton::Right, cx).await?;
    menu_open(view, ADDED, cx).await?;
    click(view, "menu-remove", cx).await?;
    until(view, cx, WITHIN, "the Remove question", |v| painted(v, "confirm-yes").then_some(())).await?;
    let question = view.read_with(cx, |v, _| v.confirm.as_ref().map(|c| c.text.clone())).unwrap_or_default();
    if !question.contains("No files are deleted") {
        bail!("the Remove question reads: {question}");
    }
    shot("confirm-remove", cx).await;
    click(view, "confirm-yes", cx).await?;
    until(view, cx, WITHIN, "the workspace gone from the tree", |v| {
        (!v.workspaces.iter().any(|w| w.name == ADDED) && !painted(v, &ws_key)).then_some(())
    })
    .await?;
    if registry.exists(ADDED)? {
        bail!("{ADDED} is still registered after Remove");
    }
    if !root.join("alpha").join(".git").join("HEAD").is_file() || !root.join("beta").is_dir() {
        bail!("Remove touched the folder {}", root.display());
    }
    out.push(format!("removed via its menu; {} and its repos still on disk", root.display()));
    out.push(custom(view, &registry, &root, cx).await?);
    Ok(out.join("; "))
}

/// Custom workspace…: two folders typed (Enter, then Add), home moved to the second, saved with that home.
async fn custom(view: &Entity<FleetView>, registry: &Registry, root: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let (alpha, beta) = (root.join("alpha"), root.join("beta"));
    click(view, "add-workspace", cx).await?;
    click(view, "menu-add-custom", cx).await?;
    until(view, cx, WITHIN, "the custom dialog's path field", |v| painted(v, "add-new-path-text").then_some(())).await?;
    if view.read_with(cx, |v, cx| dialog(v, cx).is_none_or(|d| d.can_save(cx) || !d.list.rows.is_empty())) {
        bail!("a custom dialog opened with folders or Save enabled");
    }
    set_field(view, false, &alpha.display().to_string(), cx)?;
    click(view, "add-new-path-text", cx).await?;
    post_keys(&["enter"], cx).await?;
    until_app(view, cx, WITHIN, "alpha added by Enter, home", |v, cx| rows(v, cx).filter(|r| r == &[("alpha".to_string(), None, true)]).map(|_| ())).await?;
    set_field(view, false, &beta.display().to_string(), cx)?;
    click(view, "add-add", cx).await?;
    until_app(view, cx, WITHIN, "beta added by Add", |v, cx| rows(v, cx).filter(|r| r.len() == 2).map(|_| ())).await?;
    click(view, "add-home-beta", cx).await?;
    until_app(view, cx, WITHIN, "home moved to beta", |v, cx| rows(v, cx).filter(|r| !r[0].2 && r[1].2).map(|_| ())).await?;
    set_field(view, true, CUSTOM, cx)?;
    until_app(view, cx, WITHIN, "Save enabled", |v, cx| dialog(v, cx).is_some_and(|d| d.can_save(cx)).then_some(())).await?;
    shot("add-custom", cx).await;
    click(view, "add-save", cx).await?;
    until(view, cx, WITHIN, "the custom dialog closed on Save", |v| v.adding.is_none().then_some(())).await?;
    let saved = registry.load(CUSTOM).context("the custom workspace after Save")?;
    let folders: Vec<&Path> = saved.folders.iter().map(|f| f.path.as_path()).collect();
    if folders != [alpha.as_path(), beta.as_path()] || saved.home.as_deref() != Some(beta.as_path()) || !saved.scans.is_empty() {
        bail!("registered {CUSTOM} with folders {folders:?}, home {:?}, scans {:?}; expected alpha and beta, home beta, no scan", saved.home, saved.scans);
    }
    registry.remove(CUSTOM)?;
    until(view, cx, Duration::from_secs(15), "the custom workspace gone", |v| (!v.workspaces.iter().any(|w| w.name == CUSTOM)).then_some(())).await?;
    Ok(format!("{CUSTOM}: alpha typed and added by Enter, beta by Add, home moved to beta; saved with home beta and no scan; removed"))
}
