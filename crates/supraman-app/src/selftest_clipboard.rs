//! gpui turns a CF_DIB into a PNG-only entry that claude's Alt+V does not see as an image, so steps that borrow the clipboard use this.

pub(crate) use win::{Held, Snapshot, sequence};

use std::time::{Duration, Instant};

use gpui::{AsyncWindowContext, ClipboardItem};

const HELD_WITHIN: Duration = Duration::from_secs(60);
const MUTEX: &[&str] = &[r"Local\Supraman-selftest-clipboard"];
const MACHINE: &[&str] = &[r"Local\Supraman-selftest"];
const MACHINE_WITHIN: Duration = Duration::from_secs(20 * 60);

/// Held for a whole `--selftest`, `--selftest-supervise` or `--selftest-detach-shutdown`: two runs at once break each other's windows.
pub(crate) fn hold_machine() -> anyhow::Result<Held> {
    let pid_file = std::env::temp_dir().join("Supraman-selftest.pid");
    let tell = |pid: Option<u32>, waited: Duration| {
        let pid = pid.map_or_else(|| "unknown".to_string(), |p| p.to_string());
        println!("setup: waiting for another selftest (pid {pid}), {} s so far", waited.as_secs());
    };
    match wait_for(MACHINE, &pid_file, MACHINE_WITHIN, Duration::from_secs(30), tell)? {
        Some(held) => Ok(held),
        None => anyhow::bail!("another selftest held the machine for {} min", MACHINE_WITHIN.as_secs() / 60),
    }
}

/// `None` once `within` ran out; `tell` gets the last holder's pid every `every`.
fn wait_for(
    names: &[&str],
    pid_file: &std::path::Path,
    within: Duration,
    every: Duration,
    mut tell: impl FnMut(Option<u32>, Duration),
) -> anyhow::Result<Option<Held>> {
    let held = Held::open(names)?;
    let started = Instant::now();
    let mut told: Option<Instant> = None;
    loop {
        if held.try_take()? {
            supraman_core::fs::write_atomic(pid_file, std::process::id().to_string()).ok();
            return Ok(Some(held));
        }
        let waited = started.elapsed();
        if waited >= within {
            return Ok(None);
        }
        if told.is_none_or(|t| t.elapsed() >= every) {
            tell(std::fs::read_to_string(pid_file).ok().and_then(|p| p.trim().parse().ok()), waited);
            told = Some(Instant::now());
        }
        std::thread::sleep(Duration::from_millis(250).min(within - waited));
    }
}

/// Every `--selftest` on the machine borrows it through the named mutexes; released on drop, on the UI thread that owns it.
pub(crate) async fn borrow(step: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<Held> {
    let held = Held::open(MUTEX)?;
    let started = Instant::now();
    let mut told = false;
    while !held.try_take()? {
        if !told {
            println!("step {step}: waiting up to {} s for the clipboard, which another selftest holds", HELD_WITHIN.as_secs());
            told = true;
        }
        if started.elapsed() > HELD_WITHIN {
            anyhow::bail!("another selftest held the clipboard for {} s", HELD_WITHIN.as_secs());
        }
        crate::selftest::sleep(cx, Duration::from_millis(100)).await;
    }
    if told {
        println!("step {step}: the clipboard is this run's after {} ms", started.elapsed().as_millis());
    }
    Ok(held)
}

pub(crate) fn text(cx: &mut AsyncWindowContext) -> anyhow::Result<Option<String>> {
    cx.update(|_, cx| cx.read_from_clipboard().and_then(|c| c.text()))
}

pub(crate) fn put_text(text: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    cx.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string(text.into())))
}

#[cfg(windows)]
mod win {
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
mod win {
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

    pub(crate) fn put_test_image() -> anyhow::Result<()> {
        anyhow::bail!("windows only")
    }

    pub(crate) fn sequence() -> u32 {
        0
    }
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    #[test]
    fn the_clipboard_is_one_holders_until_dropped() {
        let name = format!(r"Local\Supraman-selftest-clipboard-test-{}", crate::selftest::hex6());
        let take = |name: String| std::thread::spawn(move || super::Held::open(&[&name]).unwrap().try_take().unwrap()).join().unwrap();
        let first = super::Held::open(&[&name]).unwrap();
        assert!(first.try_take().unwrap());
        let second = take(name.clone());
        assert!(!second, "a second holder took the clipboard while the first held it");
        drop(first);
        let third = take(name);
        assert!(third, "the clipboard stayed held after its holder dropped");
    }

    #[cfg(windows)]
    #[test]
    fn a_selftest_waits_for_the_machine_and_names_the_holder() {
        use std::time::Duration;
        let hex = crate::selftest::hex6();
        let name = format!(r"Local\Supraman-selftest-machine-test-{hex}");
        let pid_file = std::env::temp_dir().join(format!("Supraman-selftest-test-{hex}.pid"));
        let (taken_tx, taken) = std::sync::mpsc::channel();
        let (release_tx, release) = std::sync::mpsc::channel::<()>();
        let (n, f) = (name.clone(), pid_file.clone());
        let holder = std::thread::spawn(move || {
            let held = super::wait_for(&[&n], &f, Duration::from_secs(5), Duration::from_secs(5), |_, _| {}).unwrap();
            assert!(held.is_some());
            taken_tx.send(()).unwrap();
            release.recv().unwrap();
        });
        taken.recv().unwrap();
        let mut told = Vec::new();
        let waited = super::wait_for(&[&name], &pid_file, Duration::from_millis(600), Duration::from_millis(200), |pid, _| told.push(pid)).unwrap();
        assert!(waited.is_none(), "took the machine while another run held it");
        assert!(told.len() >= 2 && told.iter().all(|p| *p == Some(std::process::id())), "{told:?}");
        release_tx.send(()).unwrap();
        holder.join().unwrap();
        let after = super::wait_for(&[&name], &pid_file, Duration::from_secs(2), Duration::from_secs(5), |_, _| {}).unwrap();
        assert!(after.is_some(), "the machine stayed held after its holder ended");
        std::fs::remove_file(&pid_file).ok();
    }

}
