//! The tray icon that keeps Foreman reachable while its window is hidden (m5/1): closing the window
//! only hides it, so the supervisor, the relay watcher, hook ingest and the Inbox keep running in this
//! process. The icon's menu: Open, Stop all workspaces, Quit. A left click opens the window.
//! The menu and click handlers run inside the tray's own window procedure on gpui's thread; they only
//! queue a [`Command`], which `main` applies with the app in hand.

use std::sync::atomic::{AtomicBool, Ordering};

use futures::channel::mpsc::UnboundedSender;
use gpui::{App, Entity, PromptLevel, Window};

use crate::view::FleetView;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Open,
    StopAll,
    Quit,
}

/// What the tray's menu, its left click and a second launch ask of the running app.
pub fn apply(command: Command, fleet: &Entity<FleetView>, window: &mut Window, cx: &mut App) {
    match command {
        Command::Open => show(window, cx),
        Command::StopAll => {
            let started = started(fleet, cx);
            fleet.update(cx, |view, cx| started.iter().for_each(|name| view.stop_workspace(name, cx)));
        }
        Command::Quit => quit(fleet, window, cx),
    }
}

/// gpui allows one prompt at a time.
static CONFIRMING: AtomicBool = AtomicBool::new(false);

/// With a workspace started, only after a confirm: supervision stops with the app; the sessions keep
/// running, as they always did when Foreman exits.
fn quit(fleet: &Entity<FleetView>, window: &mut Window, cx: &mut App) {
    let started = started(fleet, cx);
    if started.is_empty() {
        fleet.update(cx, |view, cx| view.quit(window, cx));
        return;
    }
    show(window, cx);
    if CONFIRMING.swap(true, Ordering::SeqCst) {
        return;
    }
    let detail = format!(
        "{} started ({}): nothing restarts or delivers for them until Foreman runs again. Their sessions keep running.",
        if started.len() == 1 { "1 workspace is".to_string() } else { format!("{} workspaces are", started.len()) },
        started.join(", ")
    );
    let answer = window.prompt(PromptLevel::Warning, "Quit Foreman and stop supervising?", Some(&detail), &["Quit", "Cancel"], cx);
    let fleet = fleet.clone();
    window
        .spawn(cx, async move |cx| {
            let quit = matches!(answer.await, Ok(0));
            CONFIRMING.store(false, Ordering::SeqCst);
            if quit {
                cx.update(|window, cx| fleet.update(cx, |view, cx| view.quit(window, cx))).ok();
            }
        })
        .detach();
}

fn started(fleet: &Entity<FleetView>, cx: &App) -> Vec<String> {
    fleet.read(cx).desired.iter().filter(|(_, (started, _))| *started).map(|(name, _)| name.clone()).collect()
}

/// A placeholder; m5/14 replaces this one file with the real icon.
#[cfg(windows)]
const ICON: &[u8] = include_bytes!("../assets/tray.ico");

#[cfg(windows)]
pub struct Tray(#[allow(dead_code)] tray_icon::TrayIcon);

#[cfg(windows)]
pub fn install(commands: UnboundedSender<Command>) -> anyhow::Result<Tray> {
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let menu = Menu::new();
    menu.append_items(&[
        &MenuItem::with_id("open", "Open Foreman", true, None),
        &PredefinedMenuItem::separator(),
        &MenuItem::with_id("stop-all", "Stop all workspaces", true, None),
        &PredefinedMenuItem::separator(),
        &MenuItem::with_id("quit", "Quit Foreman", true, None),
    ])?;
    let menu_commands = commands.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let command = match event.id.0.as_str() {
            "open" => Command::Open,
            "stop-all" => Command::StopAll,
            "quit" => Command::Quit,
            _ => return,
        };
        menu_commands.unbounded_send(command).ok();
    }));
    TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
        if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
            commands.unbounded_send(Command::Open).ok();
        }
    }));
    let decoded = image::load_from_memory_with_format(ICON, image::ImageFormat::Ico)?.into_rgba8();
    let (width, height) = decoded.dimensions();
    let icon = tray_icon::Icon::from_rgba(decoded.into_raw(), width, height)?;
    let tray = TrayIconBuilder::new()
        .with_tooltip("Foreman")
        .with_icon(icon)
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .build()?;
    Ok(Tray(tray))
}

#[cfg(not(windows))]
pub struct Tray;

#[cfg(not(windows))]
pub fn install(_commands: UnboundedSender<Command>) -> anyhow::Result<Tray> {
    Ok(Tray)
}

#[cfg(windows)]
pub(crate) fn hwnd(window: &Window) -> Option<windows::Win32::Foundation::HWND> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(windows::Win32::Foundation::HWND(handle.hwnd.get() as *mut _)),
        _ => None,
    }
}

// `ShowWindow` sends the window its activation, focus and position messages before it returns, and
// gpui takes a window out of the app while it updates it: called from inside an update (every caller
// here is), each of those messages finds "window not found" (m5/22, LEARNINGS). So these run as a
// task of their own, as gpui's own `activate` does.

/// Off the screen and the taskbar; the window, its tabs and everything it drives stay alive.
#[cfg(windows)]
pub fn hide(window: &Window, cx: &mut App) {
    use windows::Win32::UI::WindowsAndMessaging::{SW_HIDE, ShowWindow};
    if let Some(hwnd) = hwnd(window) {
        cx.spawn(async move |_| unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        })
        .detach();
    }
}

#[cfg(windows)]
pub fn show(window: &Window, cx: &mut App) {
    use windows::Win32::UI::WindowsAndMessaging::{SW_SHOW, SetForegroundWindow, ShowWindow};
    if let Some(hwnd) = hwnd(window) {
        cx.spawn(async move |_| unsafe {
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);
        })
        .detach();
    }
    window.activate_window();
}

#[cfg(windows)]
pub fn is_shown(window: &Window) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::IsWindowVisible;
    hwnd(window).is_some_and(|hwnd| unsafe { IsWindowVisible(hwnd) }.as_bool())
}

/// Closes a window for good: hidden first, outside any update, so the deactivation reaches a window
/// gpui still has; removed once what Windows posted it on the way (a mouse leave, a paint) has been
/// handled; gpui destroys the HWND after. Every close goes this way, a caption ✕ included
/// (`on_window_should_close` → `false`, so Windows never destroys the HWND under gpui, which then
/// logs "Invalid window handle" for it).
pub fn close(window: &Window, cx: &mut App) {
    let handle = window.window_handle();
    #[cfg(windows)]
    let hwnd = hwnd(window);
    cx.spawn(async move |cx| {
        #[cfg(windows)]
        if let Some(hwnd) = hwnd {
            use windows::Win32::UI::WindowsAndMessaging::{SW_HIDE, ShowWindow};
            unsafe {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
            let started = std::time::Instant::now();
            loop {
                cx.background_executor().timer(DRAIN_EVERY).await;
                if drained(hwnd) || started.elapsed() > DRAIN_LIMIT {
                    break;
                }
            }
        }
        handle.update(cx, |_, window, _| window.remove_window()).ok();
    })
    .detach();
}

#[cfg(windows)]
const DRAIN_EVERY: std::time::Duration = std::time::Duration::from_millis(10);
#[cfg(windows)]
const DRAIN_LIMIT: std::time::Duration = std::time::Duration::from_secs(1);

/// Hidden, with nothing posted to `hwnd` still queued. Sent messages are left alone: peeking for them
/// would dispatch them here, inside this task.
#[cfg(windows)]
fn drained(hwnd: windows::Win32::Foundation::HWND) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        IsWindowVisible, MSG, PM_NOREMOVE, PM_QS_INPUT, PM_QS_PAINT, PM_QS_POSTMESSAGE, PeekMessageW,
    };
    let mut msg = MSG::default();
    unsafe {
        !IsWindowVisible(hwnd).as_bool()
            && !PeekMessageW(&mut msg, Some(hwnd), 0, 0, PM_NOREMOVE | PM_QS_POSTMESSAGE | PM_QS_INPUT | PM_QS_PAINT).as_bool()
    }
}

#[cfg(not(windows))]
pub fn hide(window: &Window, _cx: &mut App) {
    window.minimize_window();
}

#[cfg(not(windows))]
pub fn show(window: &Window, _cx: &mut App) {
    window.activate_window();
}

#[cfg(not(windows))]
pub fn is_shown(_window: &Window) -> bool {
    true
}
