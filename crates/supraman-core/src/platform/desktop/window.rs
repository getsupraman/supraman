//! A top-level window by its HWND, as `raw_window_handle` gives it.

/// Windows' "Choose your default Windows mode", which colours the taskbar: true when it is Light.
#[cfg(windows)]
pub fn light_taskbar() -> bool {
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
    use windows::core::w;
    let mut value = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    let read = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    read.is_ok() && value == 1
}

/// No taskbar mode to read here: dark.
#[cfg(not(windows))]
pub fn light_taskbar() -> bool {
    false
}

/// A small icon's width in pixels, at least 16.
#[cfg(windows)]
pub fn small_icon_size() -> u32 {
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSMICON};
    unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16) as u32
}

#[cfg(not(windows))]
pub fn small_icon_size() -> u32 {
    16
}

#[cfg(windows)]
fn handle(hwnd: isize) -> windows::Win32::Foundation::HWND {
    windows::Win32::Foundation::HWND(hwnd as *mut _)
}

#[cfg(windows)]
pub fn hide_window(hwnd: isize) -> Result<(), String> {
    use windows::Win32::UI::WindowsAndMessaging::{SW_HIDE, ShowWindow};
    unsafe {
        let _ = ShowWindow(handle(hwnd), SW_HIDE);
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn hide_window(_hwnd: isize) -> Result<(), String> {
    Err(crate::platform::refused("hiding a window by its HWND"))
}

/// Shown and brought to the foreground.
#[cfg(windows)]
pub fn show_window(hwnd: isize) -> Result<(), String> {
    use windows::Win32::UI::WindowsAndMessaging::{SW_SHOW, SetForegroundWindow, ShowWindow};
    unsafe {
        let _ = ShowWindow(handle(hwnd), SW_SHOW);
        let _ = SetForegroundWindow(handle(hwnd));
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn show_window(_hwnd: isize) -> Result<(), String> {
    Err(crate::platform::refused("showing a window by its HWND"))
}

#[cfg(windows)]
pub fn is_window_visible(hwnd: isize) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::IsWindowVisible;
    unsafe { IsWindowVisible(handle(hwnd)) }.as_bool()
}

#[cfg(not(windows))]
pub fn is_window_visible(_hwnd: isize) -> bool {
    false
}

/// A process that is not in the foreground keeps a hidden window active, and DestroyWindow would then deactivate it after gpui dropped it.
#[cfg(windows)]
pub fn hand_activation_away(hwnd: isize) {
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetActiveWindow, SetActiveWindow};
    use windows::Win32::UI::WindowsAndMessaging::{EnumThreadWindows, IsWindowVisible};
    unsafe extern "system" fn first_visible(other: HWND, found: LPARAM) -> windows::core::BOOL {
        unsafe {
            let found = &mut *(found.0 as *mut (HWND, Option<HWND>));
            if other != found.0 && IsWindowVisible(other).as_bool() {
                found.1 = Some(other);
                return false.into();
            }
        }
        true.into()
    }
    let hwnd = handle(hwnd);
    if unsafe { GetActiveWindow() } != hwnd {
        return;
    }
    let mut found: (HWND, Option<HWND>) = (hwnd, None);
    unsafe {
        let _ = EnumThreadWindows(GetCurrentThreadId(), Some(first_visible), LPARAM(&mut found as *mut _ as isize));
        if let Some(other) = found.1 {
            let _ = SetActiveWindow(other);
        }
    }
}

#[cfg(not(windows))]
pub fn hand_activation_away(_hwnd: isize) {}

/// Hidden, with no posted, input or paint message left; sent ones are not peeked, which would dispatch them here.
#[cfg(windows)]
pub fn drained(hwnd: isize) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        IsWindowVisible, MSG, PM_NOREMOVE, PM_QS_INPUT, PM_QS_PAINT, PM_QS_POSTMESSAGE, PeekMessageW,
    };
    let hwnd = handle(hwnd);
    let mut msg = MSG::default();
    unsafe {
        !IsWindowVisible(hwnd).as_bool()
            && !PeekMessageW(&mut msg, Some(hwnd), 0, 0, PM_NOREMOVE | PM_QS_POSTMESSAGE | PM_QS_INPUT | PM_QS_PAINT).as_bool()
    }
}

#[cfg(not(windows))]
pub fn drained(_hwnd: isize) -> bool {
    true
}
