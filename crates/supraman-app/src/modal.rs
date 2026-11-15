//! A window modal over another: gpui's `WindowKind::Dialog` owns it to the active window and disables that one while it is open;
//! every close goes through `tray::close`, which enables the owner again before the window hides.

use gpui::{AnyWindowHandle, App, Bounds, Entity, Pixels, Render, Size, TitlebarOptions, Window, WindowBounds, WindowHandle, WindowKind, WindowOptions};

/// Opened centred on `owner`, which it owns and blocks until it closes.
pub fn open_over<V: Render + 'static>(
    owner: AnyWindowHandle,
    size: Size<Pixels>,
    titlebar: TitlebarOptions,
    cx: &mut App,
    build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
) -> anyhow::Result<WindowHandle<V>> {
    let placed = owner.update(cx, |_, window, cx| (window.bounds(), window.display(cx), hwnd(window)))?;
    let (owner_bounds, display, owner_hwnd) = placed;
    let room = display.as_ref().map_or(owner_bounds, |d| d.visible_bounds());
    let bounds = Bounds::centered_at(owner_bounds.center(), size.min(&room.size));
    make_active(owner_hwnd);
    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(titlebar),
            kind: WindowKind::Dialog,
            is_minimizable: false,
            display_id: display.map(|d| d.id()),
            ..Default::default()
        },
        |window, cx| {
            check_owner(window, owner_hwnd);
            build(window, cx)
        },
    )
}

// gpui's dialog takes this thread's active window as its owner.
#[cfg(windows)]
fn make_active(owner: Option<isize>) {
    if let Some(owner) = owner
        && let Err(e) = supraman_core::platform::desktop::set_active(owner)
    {
        log::warn!("cannot activate the modal window's owner: {e}");
    }
}

#[cfg(not(windows))]
fn make_active(_owner: Option<isize>) {}

#[cfg(windows)]
fn check_owner(window: &Window, owner: Option<isize>) {
    let owned = hwnd(window).and_then(supraman_core::platform::desktop::owner_of);
    if owned != owner {
        log::warn!("the modal window is owned by {owned:?}, not {owner:?}");
    }
}

#[cfg(not(windows))]
fn check_owner(_window: &Window, _owner: Option<isize>) {}

#[cfg(windows)]
fn hwnd(window: &Window) -> Option<isize> {
    crate::tray::hwnd(window)
}

#[cfg(not(windows))]
fn hwnd(_window: &Window) -> Option<isize> {
    None
}
