//! A selftest's PNG of one of this process's windows, drawn by the window itself (`PrintWindow` with
//! `PW_RENDERFULLCONTENT`): no other window ends up in it, and it works on a hidden desktop.

use std::path::Path;

use anyhow::{anyhow, bail};

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
