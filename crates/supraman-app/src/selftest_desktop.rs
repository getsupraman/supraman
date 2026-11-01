//! A selftest runs on a hidden desktop of its own unless `--visible` (or `SUPRAMAN_SELFTEST_VISIBLE=1`) asks for
//! the user's: its windows then never take the user's focus, and the user's held keys never reach its posted ones.

use std::ffi::OsString;
use std::io::{Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use supraman_core::desktop::{self, HiddenDesktop};

/// Set for the selftest on its hidden desktop (and inherited by its children): the desktop's name.
pub const DESKTOP_VAR: &str = "SUPRAMAN_SELFTEST_DESKTOP";
pub const VISIBLE_VAR: &str = "SUPRAMAN_SELFTEST_VISIBLE";
pub const VISIBLE_FLAG: &str = "--visible";

/// Whether this process is the one to start the selftest on a hidden desktop.
pub fn relaunch_wanted(visible_flag: bool, get: impl Fn(&str) -> Option<String>) -> bool {
    !visible_flag && get(DESKTOP_VAR).is_none() && !get(VISIBLE_VAR).is_some_and(|v| v == "1")
}

/// Where this selftest runs, as its first output line says.
pub fn placement() -> String {
    let here = desktop::thread_desktop_name().unwrap_or_else(|| "?".into());
    let input = desktop::input_desktop_name().unwrap_or_else(|| "not readable".into());
    match std::env::var(DESKTOP_VAR) {
        Ok(_) => format!("selftest on hidden desktop {here} (input desktop: {input}); {VISIBLE_FLAG} runs it on yours"),
        Err(_) => format!("selftest on desktop {here} (input desktop: {input}): its windows can take the focus and read held keys"),
    }
}

/// Runs this exe with `args` on a new hidden desktop, its stdout and stderr copied to ours; its exit code.
pub fn run_hidden(args: &[OsString]) -> i32 {
    let started = (|| {
        let exe = std::env::current_exe()?;
        let desktop = HiddenDesktop::create("supraman-selftest")?;
        let child = desktop::spawn_on(&desktop, &exe, args, &[(DESKTOP_VAR, desktop.name())])?;
        Ok::<_, std::io::Error>((desktop, child))
    })();
    let (_desktop, mut child) = match started {
        Ok(started) => started,
        Err(error) => {
            println!("FAIL setup: cannot start the selftest on a hidden desktop: {error}; {VISIBLE_FLAG} runs it on yours");
            return 1;
        }
    };
    let out = child.stdout.take().map(|pipe| std::thread::spawn(move || copy(pipe, std::io::stdout())));
    let err = child.stderr.take().map(|pipe| std::thread::spawn(move || copy(pipe, std::io::stderr())));
    let done = Arc::new(AtomicBool::new(false));
    let watcher = {
        let done = done.clone();
        std::thread::spawn(move || watch_focus(&done))
    };
    let code = child.wait();
    done.store(true, Ordering::SeqCst);
    for copier in [out, err].into_iter().flatten() {
        let _ = copier.join();
    }
    let focus = watcher.join().unwrap_or_default();
    let code = match code {
        Ok(code) => code,
        Err(error) => {
            println!("FAIL setup: lost the selftest on its hidden desktop (pid {}): {error}", child.pid);
            1
        }
    };
    match focus {
        Focus { stolen: Some(pid), .. } => {
            println!("FAIL isolation: a window of this selftest build (pid {pid}) had the focus on the input desktop");
            1
        }
        Focus { samples, .. } => {
            println!("isolation: no window of this selftest build had the focus on the input desktop ({samples} samples)");
            code
        }
    }
}

#[derive(Default)]
struct Focus {
    samples: u32,
    stolen: Option<u32>,
}

/// Samples the input desktop's foreground window until `done`: whether a process of this exe other than this one owned it.
fn watch_focus(done: &AtomicBool) -> Focus {
    let mut focus = Focus::default();
    let Ok(exe) = std::env::current_exe() else { return focus };
    let exe = supraman_core::paths::path_key(&exe);
    while !done.load(Ordering::SeqCst) {
        if let Some(pid) = foreground_pid().filter(|&pid| pid != std::process::id())
            && image_of(pid).is_some_and(|image| supraman_core::paths::path_key(&image) == exe)
        {
            focus.stolen.get_or_insert(pid);
        }
        focus.samples += 1;
        std::thread::sleep(Duration::from_millis(100));
    }
    focus
}

#[cfg(windows)]
fn foreground_pid() -> Option<u32> {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_invalid() {
        return None;
    }
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    (pid != 0).then_some(pid)
}

#[cfg(windows)]
fn image_of(pid: u32) -> Option<std::path::PathBuf> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};
    use windows::core::PWSTR;
    // SAFETY: the handle is closed below; the buffer outlives the call.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let named = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
        let _ = CloseHandle(process);
        named.ok()?;
        Some(String::from_utf16_lossy(&buf[..len as usize]).into())
    }
}

#[cfg(not(windows))]
fn foreground_pid() -> Option<u32> {
    None
}

#[cfg(not(windows))]
fn image_of(_pid: u32) -> Option<std::path::PathBuf> {
    None
}

fn copy(mut from: impl Read, mut to: impl Write) {
    let mut chunk = [0u8; 8192];
    while let Ok(n @ 1..) = from.read(&mut chunk) {
        if to.write_all(&chunk[..n]).and_then(|()| to.flush()).is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wanted(visible: bool, vars: &[(&str, &str)]) -> bool {
        relaunch_wanted(visible, |name| vars.iter().find(|(k, _)| *k == name).map(|(_, v)| v.to_string()))
    }

    #[test]
    fn a_selftest_relaunches_hidden_unless_asked_for_the_visible_desktop() {
        assert!(wanted(false, &[]));
        assert!(!wanted(true, &[]));
        assert!(!wanted(false, &[(VISIBLE_VAR, "1")]));
        assert!(wanted(false, &[(VISIBLE_VAR, "0")]));
        assert!(!wanted(false, &[(DESKTOP_VAR, "supraman-selftest-1-abcdef")]));
    }
}
