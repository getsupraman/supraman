//! The selftests' Win32: posted window messages, window captures, the clipboard, CPU times and the foreground window.

use std::path::Path;

use anyhow::{anyhow, bail};
#[cfg(windows)]
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

/// Queued as the OS queues a real click or key: through gpui's window procedure (and `TranslateMessage`), not straight to `dispatch_event`.
#[cfg(windows)]
pub(crate) fn post(hwnd: HWND, messages: &[(u32, usize, isize)]) -> anyhow::Result<()> {
    for &(message, wparam, lparam) in messages {
        unsafe { PostMessageW(Some(hwnd), message, WPARAM(wparam), LPARAM(lparam)) }.map_err(|e| anyhow!("PostMessage: {e}"))?;
    }
    Ok(())
}

#[cfg(windows)]
pub(crate) fn post_close(hwnd: HWND) -> anyhow::Result<()> {
    use windows::Win32::UI::WindowsAndMessaging::WM_CLOSE;
    unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) }.map_err(|e| anyhow!("WM_CLOSE: {e}"))
}

/// `window`'s HWND as [`destroyed`] takes it.
#[cfg(windows)]
pub(crate) fn hwnd_of(window: &gpui::Window) -> Option<isize> {
    crate::tray::hwnd(window).map(|hwnd| hwnd.0 as isize)
}

#[cfg(not(windows))]
pub(crate) fn hwnd_of(_window: &gpui::Window) -> Option<isize> {
    None
}

/// `hwnd` is no window any more: gpui destroys a closed window's HWND on a later turn of its executor.
#[cfg(windows)]
pub(crate) fn destroyed(hwnd: Option<isize>) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::IsWindow;
    hwnd.is_none_or(|hwnd| !unsafe { IsWindow(Some(HWND(hwnd as *mut _))) }.as_bool())
}

#[cfg(not(windows))]
pub(crate) fn destroyed(_hwnd: Option<isize>) -> bool {
    true
}

/// A left click at `(x, y)`, device pixels in the client area.
#[cfg(windows)]
pub(crate) fn click_messages(x: isize, y: isize) -> [(u32, usize, isize); 3] {
    use windows::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE};
    let at = (y << 16) | (x & 0xffff);
    [(WM_MOUSEMOVE, 0, at), (WM_LBUTTONDOWN, 1, at), (WM_LBUTTONUP, 0, at)]
}

#[cfg(windows)]
const MODIFIERS: [(windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY, &str); 5] = {
    use windows::Win32::UI::Input::KeyboardAndMouse::{VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT};
    [(VK_CONTROL, "Ctrl"), (VK_MENU, "Alt"), (VK_SHIFT, "Shift"), (VK_LWIN, "Win"), (VK_RWIN, "Win")]
};

/// The modifiers down in this thread's key state, which gpui reads a key's modifiers from and a posted message does not set.
#[cfg(windows)]
pub(crate) fn held_modifiers() -> Vec<&'static str> {
    use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyState;
    MODIFIERS.into_iter().filter(|(vk, _)| unsafe { GetKeyState(vk.0 as i32) } < 0).map(|(_, name)| name).collect()
}

/// Ctrl, Alt and Shift as held on the keyboard now.
#[cfg(windows)]
pub(crate) fn pressed_modifiers() -> Vec<&'static str> {
    use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
    MODIFIERS[..3].iter().filter(|(vk, _)| unsafe { GetAsyncKeyState(vk.0 as i32) } as u16 & 0x8000 != 0).map(|(_, name)| *name).collect()
}

/// Every modifier released in this thread's key state.
#[cfg(windows)]
pub(crate) fn clear_held_modifiers() -> anyhow::Result<()> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyboardState, SetKeyboardState, VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU, VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_RWIN,
        VK_SHIFT,
    };
    let mut state = [0u8; 256];
    unsafe { GetKeyboardState(&mut state) }.map_err(|e| anyhow!("GetKeyboardState: {e}"))?;
    for vk in [VK_CONTROL, VK_LCONTROL, VK_RCONTROL, VK_MENU, VK_LMENU, VK_RMENU, VK_SHIFT, VK_LSHIFT, VK_RSHIFT, VK_LWIN, VK_RWIN] {
        state[vk.0 as usize] &= !0x80;
    }
    unsafe { SetKeyboardState(&state) }.map_err(|e| anyhow!("SetKeyboardState: {e}"))
}

/// WM_KEYDOWN and WM_KEYUP of `key`: a named key (end, left, right, pageup, pagedown, backspace, enter, escape) or a character.
#[cfg(windows)]
pub(crate) fn key_messages(key: &str) -> [(u32, usize, isize); 2] {
    use windows::Win32::UI::{
        Input::KeyboardAndMouse::{MAPVK_VK_TO_VSC, MapVirtualKeyW, VK_BACK, VK_END, VK_ESCAPE, VK_LEFT, VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT, VkKeyScanW},
        WindowsAndMessaging::{WM_KEYDOWN, WM_KEYUP},
    };
    let vk = match key {
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
    [(WM_KEYDOWN, vk as usize, 1 | (scan << 16)), (WM_KEYUP, vk as usize, 1 | (scan << 16) | (0b11 << 30))]
}

/// Every message queued for this thread dispatched.
#[cfg(windows)]
pub(crate) fn pump_messages() {
    use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage};
    let mut msg = MSG::default();
    // SAFETY: a plain message loop on this thread's own queue.
    unsafe {
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

#[cfg(windows)]
pub(crate) fn post_close_to_own_windows() -> usize {
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::WindowsAndMessaging::{EnumThreadWindows, IsWindowVisible, PostMessageW, WM_CLOSE};
    unsafe extern "system" fn post(hwnd: HWND, count: LPARAM) -> windows::core::BOOL {
        unsafe {
            if IsWindowVisible(hwnd).as_bool() && PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)).is_ok() {
                *(count.0 as *mut usize) += 1;
            }
        }
        true.into()
    }
    let mut count = 0usize;
    unsafe {
        let _ = EnumThreadWindows(GetCurrentThreadId(), Some(post), LPARAM(&mut count as *mut usize as isize));
    }
    count
}

#[cfg(not(windows))]
pub(crate) fn post_close_to_own_windows() -> usize {
    0
}


/// A build's rustc threads at normal priority otherwise preempt it mid-frame; the process class stays normal, so the sessions and shells it starts are not raised.
#[cfg(windows)]
pub(crate) fn raise_ui_thread() -> bool {
    use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_HIGHEST};
    unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST).is_ok() }
}

#[cfg(not(windows))]
pub(crate) fn raise_ui_thread() -> bool {
    false
}


/// The machine's idle and total CPU time and this process's own, in 100 ns units; kernel time includes idle.
#[cfg(windows)]
pub(crate) fn cpu_times() -> Option<(u64, u64, u64)> {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes, GetSystemTimes};
    let ticks = |t: FILETIME| (t.dwHighDateTime as u64) << 32 | t.dwLowDateTime as u64;
    let (mut idle, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
    let (mut created, mut exited, mut own_kernel, mut own_user) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
    unsafe {
        GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)).ok()?;
        GetProcessTimes(GetCurrentProcess(), &mut created, &mut exited, &mut own_kernel, &mut own_user).ok()?;
    }
    Some((ticks(idle), ticks(kernel) + ticks(user), ticks(own_kernel) + ticks(own_user)))
}

#[cfg(not(windows))]
pub(crate) fn cpu_times() -> Option<(u64, u64, u64)> {
    None
}

#[cfg(windows)]
pub(crate) fn foreground_pid() -> Option<u32> {
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
pub(crate) fn image_of(pid: u32) -> Option<std::path::PathBuf> {
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
pub(crate) fn foreground_pid() -> Option<u32> {
    None
}

#[cfg(not(windows))]
pub(crate) fn image_of(_pid: u32) -> Option<std::path::PathBuf> {
    None
}

/// This process's visible top-level window titled `title`, or its largest one.
#[cfg(windows)]
pub fn window(title: Option<&str>) -> anyhow::Result<isize> {
    use windows::Win32::Foundation::{HWND, LPARAM, RECT};
    use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindow, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, GW_OWNER, IsWindowVisible};
    use windows::core::BOOL;
    struct Found {
        pid: u32,
        windows: Vec<(isize, String, i64)>,
    }
    unsafe extern "system" fn each(hwnd: HWND, found: LPARAM) -> BOOL {
        // SAFETY: `found` is the `Found` passed to EnumWindows below, alive for the call.
        let found = unsafe { &mut *(found.0 as *mut Found) };
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        let owned = unsafe { GetWindow(hwnd, GW_OWNER) }.is_ok_and(|o| !o.is_invalid());
        if pid == found.pid && unsafe { IsWindowVisible(hwnd) }.as_bool() && !owned {
            let mut text = [0u16; 256];
            let n = unsafe { GetWindowTextW(hwnd, &mut text) } as usize;
            let mut r = RECT::default();
            let _ = unsafe { GetWindowRect(hwnd, &mut r) };
            let area = i64::from(r.right - r.left) * i64::from(r.bottom - r.top);
            found.windows.push((hwnd.0 as isize, String::from_utf16_lossy(&text[..n]), area));
        }
        true.into()
    }
    let mut found = Found { pid: std::process::id(), windows: Vec::new() };
    // SAFETY: the callback only touches `found`, which outlives the call.
    unsafe { EnumWindows(Some(each), LPARAM(&mut found as *mut Found as isize)) }.map_err(|e| anyhow!("EnumWindows: {e}"))?;
    let chosen = match title {
        Some(title) => found.windows.iter().find(|(_, t, _)| t == title),
        None => found.windows.iter().max_by_key(|(_, _, area)| *area),
    };
    chosen.map(|(hwnd, _, _)| *hwnd).ok_or_else(|| {
        let titles: Vec<&str> = found.windows.iter().map(|(_, t, _)| t.as_str()).collect();
        anyhow!("this process has no visible window titled {title:?}; its windows: {titles:?}")
    })
}

/// Writes the window `hwnd` as it paints itself to `out`; off the UI thread, which must keep pumping messages.
#[cfg(windows)]
pub fn print_window(hwnd: isize, out: &Path) -> anyhow::Result<()> {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleBitmap, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits,
        HGDIOBJ, ReleaseDC, SelectObject,
    };
    use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};
    use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;
    const PW_RENDERFULLCONTENT: PRINT_WINDOW_FLAGS = PRINT_WINDOW_FLAGS(2);
    let hwnd = HWND(hwnd as *mut _);
    let mut r = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut r) }.map_err(|e| anyhow!("GetWindowRect: {e}"))?;
    let (w, h) = (r.right - r.left, r.bottom - r.top);
    if w <= 0 || h <= 0 {
        bail!("the window is {w}×{h}");
    }
    let mut pixels = vec![0u8; (w * h * 4) as usize];
    // SAFETY: every GDI object made here is released before returning; the buffers outlive the calls.
    let (printed, lines) = unsafe {
        let screen = GetDC(None);
        let dc = CreateCompatibleDC(Some(screen));
        let bitmap = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(dc, HGDIOBJ(bitmap.0));
        let printed = PrintWindow(hwnd, dc, PW_RENDERFULLCONTENT).as_bool();
        SelectObject(dc, old);
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let lines = GetDIBits(dc, bitmap, 0, h as u32, Some(pixels.as_mut_ptr().cast()), &mut info, DIB_RGB_COLORS);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen);
        (printed, lines)
    };
    if !printed {
        bail!("PrintWindow failed");
    }
    if lines != h {
        bail!("GetDIBits read {lines} of {h} lines");
    }
    for px in pixels.chunks_exact_mut(4) {
        px.swap(0, 2);
        px[3] = 255;
    }
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir)?;
    }
    image::RgbaImage::from_raw(w as u32, h as u32, pixels).ok_or_else(|| anyhow!("a {w}×{h} frame"))?.save(out)?;
    Ok(())
}

#[cfg(not(windows))]
pub fn window(_title: Option<&str>) -> anyhow::Result<isize> {
    bail!("window captures are Windows captures")
}

#[cfg(not(windows))]
pub fn print_window(_hwnd: isize, _out: &Path) -> anyhow::Result<()> {
    bail!("window captures are Windows captures")
}

#[cfg(windows)]
pub(crate) mod clipboard {
    use std::time::{Duration, Instant};

    use windows::{
        Win32::{
            Foundation::{CloseHandle, GlobalFree, HANDLE, HGLOBAL, WAIT_ABANDONED, WAIT_OBJECT_0, WAIT_TIMEOUT},
            System::{
                DataExchange::{
                    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData, GetClipboardSequenceNumber,
                    OpenClipboard, SetClipboardData,
                },
                Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock},
                Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject},
            },
        },
        core::PCWSTR,
    };

    /// GDI-handle and owner-display formats, which are no HGLOBAL; Windows synthesizes CF_BITMAP again from CF_DIB.
    const NOT_GLOBAL: [u32; 8] = [2, 3, 9, 14, 0x80, 0x82, 0x83, 0x8E];

    struct Open;

    impl Open {
        fn new() -> windows::core::Result<Self> {
            let started = Instant::now();
            loop {
                match unsafe { OpenClipboard(None) } {
                    Ok(()) => return Ok(Open),
                    Err(e) if started.elapsed() > Duration::from_secs(1) => return Err(e),
                    Err(_) => std::thread::sleep(Duration::from_millis(20)),
                }
            }
        }
    }

    impl Drop for Open {
        fn drop(&mut self) {
            unsafe { CloseClipboard() }.ok();
        }
    }

    /// Named mutexes taken all or none.
    pub(crate) struct Held {
        mutexes: Vec<HANDLE>,
        owned: std::cell::Cell<bool>,
    }

    impl Held {
        pub(crate) fn open(names: &[&str]) -> windows::core::Result<Self> {
            let mut held = Held { mutexes: Vec::new(), owned: std::cell::Cell::new(false) };
            for name in names {
                let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
                held.mutexes.push(unsafe { CreateMutexW(None, false, PCWSTR(wide.as_ptr())) }?);
            }
            Ok(held)
        }

        pub(crate) fn try_take(&self) -> windows::core::Result<bool> {
            for (i, mutex) in self.mutexes.iter().enumerate() {
                let taken = match unsafe { WaitForSingleObject(*mutex, 0) } {
                    WAIT_OBJECT_0 | WAIT_ABANDONED => true,
                    WAIT_TIMEOUT => false,
                    _ => Err(windows::core::Error::from_thread())?,
                };
                if !taken {
                    for earlier in &self.mutexes[..i] {
                        unsafe { ReleaseMutex(*earlier) }.ok();
                    }
                    return Ok(false);
                }
            }
            self.owned.set(true);
            Ok(true)
        }
    }

    impl Drop for Held {
        fn drop(&mut self) {
            for mutex in &self.mutexes {
                unsafe {
                    if self.owned.get() {
                        ReleaseMutex(*mutex).ok();
                    }
                    CloseHandle(*mutex).ok();
                }
            }
        }
    }

    pub(crate) struct Snapshot(Vec<(u32, Vec<u8>)>);

    impl Snapshot {
        pub(crate) fn take() -> windows::core::Result<Self> {
            let _open = Open::new()?;
            let mut formats = Vec::new();
            let mut format = 0;
            loop {
                format = unsafe { EnumClipboardFormats(format) };
                if format == 0 {
                    break;
                }
                if NOT_GLOBAL.contains(&format) {
                    continue;
                }
                let Ok(handle) = (unsafe { GetClipboardData(format) }) else { continue };
                let global = HGLOBAL(handle.0);
                let size = unsafe { GlobalSize(global) };
                let ptr = unsafe { GlobalLock(global) };
                if ptr.is_null() {
                    continue;
                }
                let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, size) }.to_vec();
                unsafe { GlobalUnlock(global) }.ok();
                formats.push((format, bytes));
            }
            Ok(Snapshot(formats))
        }

        pub(crate) fn restore(&self) -> windows::core::Result<()> {
            set(self.0.iter().map(|(f, b)| (*f, b.as_slice())))
        }
    }

    pub(crate) fn sequence() -> u32 {
        unsafe { GetClipboardSequenceNumber() }
    }

    fn set<'a>(formats: impl Iterator<Item = (u32, &'a [u8])>) -> windows::core::Result<()> {
        let _open = Open::new()?;
        unsafe { EmptyClipboard() }?;
        for (format, bytes) in formats {
            let global = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1)) }?;
            let ptr = unsafe { GlobalLock(global) };
            if ptr.is_null() {
                unsafe { GlobalFree(Some(global)) }.ok();
                continue;
            }
            unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr as *mut u8, bytes.len()) };
            unsafe { GlobalUnlock(global) }.ok();
            // the clipboard owns the memory once this succeeds
            if unsafe { SetClipboardData(format, Some(HANDLE(global.0))) }.is_err() {
                unsafe { GlobalFree(Some(global)) }.ok();
            }
        }
        Ok(())
    }
}

#[cfg(not(windows))]
pub(crate) mod clipboard {
    pub(crate) struct Held;

    impl Held {
        pub(crate) fn open(_names: &[&str]) -> anyhow::Result<Self> {
            Ok(Held)
        }
        pub(crate) fn try_take(&self) -> anyhow::Result<bool> {
            Ok(true)
        }
    }

    pub(crate) struct Snapshot;

    impl Snapshot {
        pub(crate) fn take() -> anyhow::Result<Self> {
            anyhow::bail!("windows only")
        }
        pub(crate) fn formats(&self) -> usize {
            0
        }
        pub(crate) fn restore(&self) -> anyhow::Result<()> {
            Ok(())
        }
    }

    pub(crate) fn sequence() -> u32 {
        0
    }
}
