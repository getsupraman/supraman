//! The desktop's Windows code.

mod hidden;
mod instance;
mod window;

pub use hidden::{DesktopChild, HiddenDesktop, input_desktop_name, quote_arg, spawn_on, thread_desktop_name};
pub use instance::{Claimed, Instance, claim};
pub use window::{drained, hand_activation_away, hide_window, is_window_visible, light_taskbar, show_window, small_icon_size};

/// Frees a console Windows allocated despite the manifest's detached `consoleAllocationPolicy`.
#[cfg(windows)]
pub fn detach_console() {
    use windows::Win32::System::Console::{FreeConsole, GetConsoleProcessList};
    let mut attached = [0u32; 2];
    if unsafe { GetConsoleProcessList(&mut attached) } == 1 {
        unsafe {
            let _ = FreeConsole();
        }
    }
}

/// No console is allocated here.
#[cfg(not(windows))]
pub fn detach_console() {}

/// Windows' build number (22000 and up is Windows 11).
#[cfg(windows)]
pub fn windows_build() -> Result<u32, String> {
    use windows::Wdk::System::SystemServices::RtlGetVersion;
    let mut version = unsafe { std::mem::zeroed() };
    unsafe { RtlGetVersion(&mut version) }.ok().map_err(|e| e.to_string())?;
    Ok(version.dwBuildNumber)
}

#[cfg(not(windows))]
pub fn windows_build() -> Result<u32, String> {
    Err(super::refused("Windows' build number"))
}
