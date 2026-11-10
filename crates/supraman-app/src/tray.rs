//! The menu and click handlers run inside the tray's own window procedure: they only queue a [`Command`].

use std::sync::atomic::{AtomicBool, Ordering};

use futures::channel::mpsc::UnboundedSender;
use gpui::{App, Entity, PromptLevel, Window};

use crate::view::FleetView;
#[cfg(windows)]
use supraman_core::platform::desktop;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Open,
    StopAll,
    Quit,
}

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
        "{} started ({}): nothing restarts or delivers for them until Supraman runs again. Their sessions keep running.",
        if started.len() == 1 { "1 workspace is".to_string() } else { format!("{} workspaces are", started.len()) },
        started.join(", ")
    );
    let answer = window.prompt(PromptLevel::Warning, "Quit Supraman and stop supervising?", Some(&detail), &["Quit", "Cancel"], cx);
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

/// gpui reports no change of the taskbar's mode alone, so the icon looks this often.
pub const FOLLOW_TASKBAR_EVERY: std::time::Duration = std::time::Duration::from_secs(5);

/// The tray icon's resource in `assets/supraman.rc`: the kit's on-dark or on-light tray mark.
#[cfg(windows)]
fn icon_resource(light_taskbar: bool) -> u16 {
    if light_taskbar { 3 } else { 2 }
}

#[cfg(windows)]
fn load_icon(light_taskbar: bool) -> anyhow::Result<tray_icon::Icon> {
    let size = desktop::small_icon_size();
    Ok(tray_icon::Icon::from_resource(icon_resource(light_taskbar), Some((size, size)))?)
}

#[cfg(windows)]
pub struct Tray(tray_icon::TrayIcon, std::cell::RefCell<String>, std::cell::Cell<bool>);

#[cfg(windows)]
impl Tray {
    pub fn follow_taskbar(&self) {
        let light = desktop::light_taskbar();
        if self.2.get() != light {
            match load_icon(light) {
                Ok(icon) => match self.0.set_icon(Some(icon)) {
                    Ok(()) => self.2.set(light),
                    Err(e) => log::warn!("cannot set the tray icon: {e}"),
                },
                Err(e) => log::warn!("cannot load the tray icon: {e:#}"),
            }
        }
    }

    pub fn set_tooltip(&self, text: &str) {
        if *self.1.borrow() != text {
            if let Err(e) = self.0.set_tooltip(Some(text)) {
                log::warn!("cannot set the tray tooltip: {e}");
            }
            *self.1.borrow_mut() = text.to_string();
        }
    }
}

#[cfg(windows)]
pub fn install(commands: UnboundedSender<Command>) -> anyhow::Result<Tray> {
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let menu = Menu::new();
    menu.append_items(&[
        &MenuItem::with_id("open", "Open Supraman", true, None),
        &PredefinedMenuItem::separator(),
        &MenuItem::with_id("stop-all", "Stop all workspaces", true, None),
        &PredefinedMenuItem::separator(),
        &MenuItem::with_id("quit", "Quit Supraman", true, None),
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
    let light = desktop::light_taskbar();
    let tray = TrayIconBuilder::new()
        .with_tooltip("Supraman")
        .with_icon(load_icon(light)?)
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .build()?;
    Ok(Tray(tray, std::cell::RefCell::new("Supraman".into()), std::cell::Cell::new(light)))
}

#[cfg(not(windows))]
pub struct Tray;

#[cfg(not(windows))]
impl Tray {
    pub fn set_tooltip(&self, _text: &str) {}
    pub fn follow_taskbar(&self) {}
}

#[cfg(not(windows))]
pub fn install(_commands: UnboundedSender<Command>) -> anyhow::Result<Tray> {
    Ok(Tray)
}

#[cfg(windows)]
pub(crate) fn hwnd(window: &Window) -> Option<isize> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
        _ => None,
    }
}

// `ShowWindow` from inside a gpui update finds "window not found": these run as a task of their own.

#[cfg(windows)]
pub fn hide(window: &Window, cx: &mut App) {
    if let Some(hwnd) = hwnd(window) {
        cx.spawn(async move |_| {
            let _ = desktop::hide_window(hwnd);
        })
        .detach();
    }
}

#[cfg(windows)]
pub fn show(window: &Window, cx: &mut App) {
    if let Some(hwnd) = hwnd(window) {
        cx.spawn(async move |_| {
            let _ = desktop::show_window(hwnd);
        })
        .detach();
    }
    window.activate_window();
}

#[cfg(windows)]
pub fn is_shown(window: &Window) -> bool {
    hwnd(window).is_some_and(desktop::is_window_visible)
}

/// Every close goes this way, a caption ✕ included, so Windows never destroys the HWND under gpui.
pub fn close(window: &Window, cx: &mut App) {
    let handle = window.window_handle();
    #[cfg(windows)]
    let hwnd = hwnd(window);
    cx.spawn(async move |cx| {
        #[cfg(windows)]
        if let Some(hwnd) = hwnd {
            let _ = desktop::hide_window(hwnd);
            desktop::hand_activation_away(hwnd);
            let started = std::time::Instant::now();
            loop {
                cx.background_executor().timer(DRAIN_EVERY).await;
                if desktop::drained(hwnd) || started.elapsed() > DRAIN_LIMIT {
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
