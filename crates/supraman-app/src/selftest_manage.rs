//! `--selftest` step 7b: managing workspaces through the window.

use std::{
    cell::Cell,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Mutex, atomic::Ordering},
    thread::ThreadId,
    time::{Duration, Instant},
};

use anyhow::{Context as _, anyhow, bail};
use supraman_claude::Session;
use supraman_core::Registry;
use gpui::{
    AsyncWindowContext, Entity, KeyDownEvent, Keystroke, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PlatformInput, point,
    px,
};

use crate::{
    selftest::{cargo_root, click, press, sleep, until, until_app},
    view::{FleetView, MenuAction},
};

pub const FOLDER: &str = "selftest-add";
pub const SINGLE: &str = "selftest-single";
const ADDED: &str = "selftest-added";
const WITHIN: Duration = Duration::from_secs(5);

fn painted(v: &FleetView, key: &str) -> bool {
    v.shown(key)
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
            VK_ESCAPE, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU, VK_RCONTROL, VK_RETURN, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT,
            VkKeyScanW,
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

async fn edit_name(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let name = |v: &FleetView, cx: &gpui::App| {
        v.adding.as_ref().map(|d| {
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
    view.update(cx, |v, _| v.adding.as_mut().map(|d| d.scanning = true));
    post_click(view, "add-save", 4., cx).await?;
    let said = view.update(cx, |v, _| {
        let d = v.adding.as_mut()?;
        d.scanning = false;
        d.error.take()
    });
    let Some(said) = said else {
        bail!("Save while the folder was still read said nothing");
    };
    Ok(format!("name by posted messages: click → caret 0, edited to {ADDED}; Save while reading: \"{said}\""))
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
        if let Some(b) = crate::clipped::cut_at(&key).filter(inside) {
            break b;
        }
        if started.elapsed() > WITHIN {
            bail!("{ADDED}'s root path {full:?} was not painted cut, 40 px or wider: {:?}", crate::clipped::cut_at(&key));
        }
        sleep(cx, Duration::from_millis(50)).await;
    };
    crate::view::icon::LAST_SHOWN.with(|l| l.borrow_mut().take());
    let position = bounds.center();
    let started = Instant::now();
    loop {
        cx.update(|window, cx| {
            let event = MouseMoveEvent { position, pressed_button: None, modifiers: Modifiers::default() };
            window.dispatch_event(PlatformInput::MouseMove(event), cx);
        })?;
        sleep(cx, Duration::from_millis(100)).await;
        let shown = crate::view::icon::LAST_SHOWN.with(|l| l.borrow().clone());
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
/// Details in the row's menu opens the row's Details tab, and a second Details focuses it (m5/71, sched2/5).
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
    if !keys.contains(&"details") {
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
    // past the card's 500 ms delay: a row that would show one has shown it by then
    while started.elapsed() < Duration::from_millis(800) {
        crate::selftest_left::move_to(position, cx)?;
        sleep(cx, Duration::from_millis(50)).await;
    }
    if let Some(card) = supraman_ui::hover_card::LAST_CARD.with(|l| l.borrow_mut().take()) {
        bail!("resting on {row} at {position:?} showed the card {card:?}");
    }
    let at = crate::selftest::scrolled_into_view(view, icon, cx).await?.center();
    crate::view::icon::LAST_SHOWN.with(|l| l.borrow_mut().take());
    let started = Instant::now();
    loop {
        crate::selftest_left::move_to(at, cx)?;
        sleep(cx, Duration::from_millis(50)).await;
        if crate::view::icon::LAST_SHOWN.with(|l| l.borrow().is_some()) {
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

/// Details from the row's menu: its report's text, once painted; OK closes it.
/// Every Edit folders… row keeps its box and buttons inside the dialog at the left panel's minimum width (m5/63).
async fn edit_folders_fit(view: &Entity<FleetView>, registry: &Registry, workspace: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    const LONG: &str = "a-folder-with-a-rather-long-name-for-edit-folders";
    const BRANCH: &str = "copilot/fix-the-edit-folders-dialog-overflowing-its-box";
    let long = cargo_root().join("target").join("selftest-edit-long").join(LONG);
    std::fs::create_dir_all(long.join(".git"))?;
    std::fs::write(long.join(".git").join("HEAD"), format!("ref: refs/heads/{BRANCH}
"))?;
    let width = view.read_with(cx, |v, _| v.left_width);
    cx.update(|window, cx| {
        view.update(cx, |v, cx| {
            v.left_width = crate::view::MIN_LEFT_WIDTH;
            v.edit_folders(workspace, window, cx);
        })
    })?;
    view.update(cx, |v, _| v.pick_folder = Some(long.clone()));
    click(view, "edit-add-folder", cx).await?;
    let folders = until(view, cx, WITHIN, "the long folder added", |_| {
        let w = registry.load(workspace).ok()?;
        w.folders.iter().any(|f| f.path == long).then(|| w.folders.iter().map(|f| f.name.clone()).collect::<Vec<_>>())
    })
    .await?;
    let long_name = registry.load(workspace)?.folders.into_iter().find(|f| f.path == long).map(|f| f.name).ok_or_else(|| anyhow!("no long folder"))?;
    let fits = until(view, cx, WITHIN, "every Edit folders row inside the dialog", |v| {
        let hits = v.hits.borrow();
        let dialog = hits.get("edit-folders")?.bounds;
        let mut rights = Vec::new();
        for f in &folders {
            let state = [format!("edit-enable-{f}"), format!("edit-elsewhere-{f}")].into_iter().find(|k| hits.contains_key(k))?;
            let mut keys = vec![state, format!("edit-remove-{f}")];
            keys.extend(Some(format!("edit-home-{f}")).filter(|k| hits.contains_key(k)));
            for key in &keys {
                let b = hits.get(key)?.bounds;
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
        until(view, cx, WITHIN, &format!("{key} painted cut"), |_| crate::clipped::cut_at(key).map(|_| ())).await?;
    }
    if !view.read_with(cx, |v, _| painted(v, "edit-elsewhere-alpha")) {
        bail!("alpha in {workspace} shows no `supervised in` badge");
    }
    shot("m5-63-edit-folders-min", cx).await;
    view.update(cx, |v, cx| {
        v.left_width = width;
        cx.notify();
    });
    click(view, "edit-done", cx).await?;
    until(view, cx, WITHIN, "Edit folders… closed by Done", |v| v.editing.is_none().then_some(())).await?;
    Ok(format!(
        "Edit folders… of {workspace} at the minimum left width ({:.0} px dialog): {folders:?} keep their boxes and buttons inside, Remove at x {:?} in every row, {LONG}'s path and branch {BRANCH} clipped with a tooltip",
        fits.0, fits.1
    ))
}

pub(crate) fn fake_repo(root: &Path, name: &str) -> anyhow::Result<()> {
    let git = root.join(name).join(".git");
    std::fs::create_dir_all(&git)?;
    std::fs::write(git.join("HEAD"), "ref: refs/heads/main\n")?;
    Ok(())
}

/// `FOREMAN_SHOTS` is set: the run takes screenshots, and the rests that only serve them are taken too.
pub(crate) fn shooting() -> bool {
    std::env::var_os("FOREMAN_SHOTS").is_some()
}

pub(crate) async fn shot(name: &str, cx: &mut AsyncWindowContext) {
    shot_window(name, None, cx).await
}

pub(crate) async fn shot_window(name: &str, title: Option<&str>, cx: &mut AsyncWindowContext) {
    let Some(dir) = std::env::var_os("FOREMAN_SHOTS").map(PathBuf::from) else {
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

pub(crate) async fn menu_open(view: &Entity<FleetView>, title: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<Vec<&'static str>> {
    let keys = until(view, cx, WITHIN, &format!("the menu of {title}"), |v| {
        v.menu.as_ref().filter(|m| m.title == title).map(|m| m.items.iter().map(|i| i.key).collect::<Vec<_>>())
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
    sleep(cx, Duration::from_millis(100)).await;
    press(view, key, button, cx).await
}

static OPENED_ON: Mutex<Option<ThreadId>> = Mutex::new(None);

/// As ShellExecuteW does while Explorer answers a folder's open (m5/68: shell32, then gpui's window procedure
/// running a foreground task): the calling thread's messages are dispatched. Then the real call.
fn pumping_open(path: &Path) -> Result<(), isize> {
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
    supraman_core::process::shell_execute("open", path.as_os_str(), None, None)
}

/// Open folder from the workspace menu while foreground tasks keep arriving, as they do in a working Foreman; the
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

    view.update(cx, |v, _| v.pick_folder = Some(root.clone()));
    click(view, "add-workspace", cx).await?;
    click(view, "menu-add-from-folder", cx).await?;
    let repos = until(view, cx, WITHIN, "the review's repos", |v| {
        let d = v.adding.as_ref().filter(|d| !d.scanning)?;
        Some(d.repos.iter().map(|(r, _)| r.name.clone()).collect::<Vec<_>>())
    })
    .await?;
    let name = view.read_with(cx, |v, cx| v.adding.as_ref().map(|d| d.name_text(cx)).unwrap_or_default());
    if name != FOLDER || repos != ["alpha", "beta"] {
        bail!("the review offers name {name} with repos {repos:?}, expected {FOLDER} with [alpha, beta]");
    }
    until(view, cx, WITHIN, "Save painted", |v| painted(v, "add-save").then_some(())).await?;
    click(view, "add-repo-beta", cx).await?;
    until(view, cx, WITHIN, "beta unticked", |v| v.adding.as_ref().is_some_and(|d| !d.repos[1].1).then_some(())).await?;
    let name_edit = edit_name(view, cx).await?;
    shot("add-review", cx).await;
    let hold = view.read_with(cx, |v, _| v.poll_hold.clone());
    hold.armed.store(true, Ordering::SeqCst);
    until(view, cx, WITHIN, "a poll held after its registry read", |_| hold.holding().then_some(())).await?;
    post_keys(&["enter"], cx).await?;
    until(view, cx, WITHIN, "the review closed on Enter", |v| v.adding.is_none().then_some(())).await?;
    let saved = registry.load(ADDED).context("the registry after Save")?;
    let flags: Vec<(String, bool)> = saved.folders.iter().map(|r| (r.name.clone(), r.enabled)).collect();
    if flags != [("alpha".to_string(), true), ("beta".to_string(), false)] {
        bail!("registered {ADDED} with repos {flags:?}, expected alpha enabled and beta disabled");
    }
    let alpha = format!("repo-row-{ADDED}/alpha");
    let in_tree = |v: &FleetView, repo: &str| v.tree().workspaces.iter().find(|w| w.name == ADDED).is_some_and(|w| w.repos.iter().any(|r| r.name == repo));
    until(view, cx, WITHIN, "alpha in the tree, the disabled beta not", |v| (painted(v, &alpha) && !in_tree(v, "beta")).then_some(())).await?;
    let applied = view.read_with(cx, |v, _| v.refreshes);
    hold.release();
    until(view, cx, WITHIN, "the held poll applied", |v| (v.refreshes > applied).then_some(())).await?;
    if !view.read_with(cx, |v, _| v.workspaces.iter().any(|w| w.name == ADDED)) {
        bail!("a poll that read the registry before Save took {ADDED} out of the tree when it applied");
    }
    let mut out = vec![format!("added {ADDED} from {} (alpha enabled, beta disabled), only alpha in the tree", root.display()), name_edit];
    out.push(cut_label_tooltip(view, cx).await?);

    view.update(cx, |v, _| v.pick_folder = Some(root.clone()));
    click(view, "add-workspace", cx).await?;
    click(view, "menu-add-from-folder", cx).await?;
    until(view, cx, WITHIN, "the second review", |v| v.adding.as_ref().filter(|d| !d.scanning).map(|_| ())).await?;
    until(view, cx, WITHIN, "Save painted", |v| painted(v, "add-save").then_some(())).await?;
    let again = view.read_with(cx, |v, cx| v.adding.as_ref().map(|d| d.name_text(cx))).unwrap_or_default();
    click(view, "add-save", cx).await?;
    until(view, cx, WITHIN, "the second review closed on Save", |v| v.adding.is_none().then_some(())).await?;
    let second = registry.load(&again).with_context(|| format!("the same folder again, saved as {again}"))?;
    let flags: Vec<(String, bool)> = second.folders.iter().map(|r| (r.name.clone(), r.enabled)).collect();
    if flags != [("alpha".to_string(), false), ("beta".to_string(), true)] {
        bail!("the same folder again as {again}: folders {flags:?}, expected alpha disabled (supervised in {ADDED}) and beta enabled");
    }
    let in_again = |v: &FleetView, repo: &str| v.tree().workspaces.iter().find(|w| w.name == again).is_some_and(|w| w.repos.iter().any(|r| r.name == repo));
    until(view, cx, WITHIN, "beta in the second workspace, alpha (supervised in the first) not", |v| {
        (painted(v, &format!("repo-row-{again}/beta")) && !in_again(v, "alpha") && !painted(v, &format!("supervised-in-{again}/alpha"))).then_some(())
    })
    .await?;
    view.update(cx, |v, cx| v.search.update(cx, |s, cx| s.set_text("alpha", cx)));
    let found = format!("result-supervised-in-{again}/alpha");
    until(view, cx, WITHIN, "search finds alpha in the second workspace, supervised in the first", |v| painted(v, &found).then_some(())).await?;
    view.update(cx, |v, cx| v.clear_search(cx));
    out.push(edit_folders_fit(view, &registry, &again, cx).await?);
    registry.remove(&again)?;
    until(view, cx, Duration::from_secs(15), "the second workspace gone", |v| (!v.workspaces.iter().any(|w| w.name == again)).then_some(())).await?;
    out.push(format!("the same folder again registers as {again} with alpha disabled, out of its tree and found by search as supervised in {ADDED}; removed"));
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
    press_painted(view, &ws_key, MouseButton::Right, cx).await?;
    menu_open(view, ADDED, cx).await?;
    click(view, "menu-edit-folders", cx).await?;
    until(view, cx, WITHIN, "Edit folders… with beta's box", |v| painted(v, "edit-enable-beta").then_some(())).await?;
    shot("edit-folders", cx).await;
    click(view, "edit-enable-beta", cx).await?;
    until(view, cx, WITHIN, "beta enabled in the tree", |v| {
        v.tree().workspaces.iter().find(|w| w.name == ADDED)?.repos.iter().find(|r| r.name == "beta" && r.enabled).map(|_| ())
    })
    .await?;
    // painted: the next press must see this frame's layout, not the one before Enable
    let plus = format!("new-claude-{ADDED}/beta");
    until(view, cx, WITHIN, "beta's + painted", |v| painted(v, &plus).then_some(())).await?;
    if !registry.load(ADDED)?.folders.iter().all(|r| r.enabled) {
        bail!("Enable did not reach the registry");
    }
    click(view, "edit-done", cx).await?;
    until(view, cx, WITHIN, "Edit folders… closed by Done", |v| v.editing.is_none().then_some(())).await?;
    out.push("beta enabled in Edit folders…, then in the tree with its +".into());

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
    Ok(out.join("; "))
}
