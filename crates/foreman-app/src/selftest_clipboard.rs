//! gpui turns a CF_DIB into a PNG-only entry that claude's Alt+V does not see as an image, so steps that borrow the clipboard use this.

pub(crate) use win::{Held, Snapshot, put_test_image, sequence};

use std::time::{Duration, Instant};

use gpui::{AsyncWindowContext, ClipboardItem};

const HELD_WITHIN: Duration = Duration::from_secs(60);
const MUTEX: &str = r"Local\Foreman-selftest-clipboard";

/// Every `--selftest` on the machine borrows it through the one named mutex; released on drop, on the UI thread that owns it.
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

fn test_dib() -> Vec<u8> {
    let mut dib = Vec::new();
    for field in [40u32, 2, 2] {
        dib.extend_from_slice(&field.to_le_bytes());
    }
    dib.extend_from_slice(&1u16.to_le_bytes());
    dib.extend_from_slice(&32u16.to_le_bytes());
    dib.extend_from_slice(&[0; 24]);
    dib.extend_from_slice(&[0x40, 0x80, 0xC0, 0xFF].repeat(4));
    dib
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
                Ole::CF_DIB,
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

    pub(crate) struct Held {
        mutex: HANDLE,
        owned: std::cell::Cell<bool>,
    }

    impl Held {
        pub(crate) fn open(name: &str) -> windows::core::Result<Self> {
            let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
            let mutex = unsafe { CreateMutexW(None, false, PCWSTR(wide.as_ptr())) }?;
            Ok(Held { mutex, owned: std::cell::Cell::new(false) })
        }

        pub(crate) fn try_take(&self) -> windows::core::Result<bool> {
            match unsafe { WaitForSingleObject(self.mutex, 0) } {
                WAIT_OBJECT_0 | WAIT_ABANDONED => {
                    self.owned.set(true);
                    Ok(true)
                }
                WAIT_TIMEOUT => Ok(false),
                _ => Err(windows::core::Error::from_thread()),
            }
        }
    }

    impl Drop for Held {
        fn drop(&mut self) {
            unsafe {
                if self.owned.get() {
                    ReleaseMutex(self.mutex).ok();
                }
                CloseHandle(self.mutex).ok();
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

        pub(crate) fn formats(&self) -> usize {
            self.0.len()
        }

        pub(crate) fn restore(&self) -> windows::core::Result<()> {
            set(self.0.iter().map(|(f, b)| (*f, b.as_slice())))
        }
    }

    pub(crate) fn sequence() -> u32 {
        unsafe { GetClipboardSequenceNumber() }
    }

    /// Windows offers it as CF_BITMAP too, which `Clipboard.ContainsImage` asks for.
    pub(crate) fn put_test_image() -> windows::core::Result<()> {
        set([(CF_DIB.0 as u32, super::test_dib().as_slice())].into_iter())
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
        pub(crate) fn open(_name: &str) -> anyhow::Result<Self> {
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
        let name = format!(r"Local\Foreman-selftest-clipboard-test-{}", crate::selftest::hex6());
        let take = |name: String| std::thread::spawn(move || super::Held::open(&name).unwrap().try_take().unwrap()).join().unwrap();
        let first = super::Held::open(&name).unwrap();
        assert!(first.try_take().unwrap());
        let second = take(name.clone());
        assert!(!second, "a second holder took the clipboard while the first held it");
        drop(first);
        let third = take(name);
        assert!(third, "the clipboard stayed held after its holder dropped");
    }

    #[test]
    fn the_test_image_is_a_2x2_32_bit_dib() {
        let dib = super::test_dib();
        let u32_at = |at: usize| u32::from_le_bytes(dib[at..at + 4].try_into().unwrap());
        let u16_at = |at: usize| u16::from_le_bytes(dib[at..at + 2].try_into().unwrap());
        assert_eq!((u32_at(0), u32_at(4), u32_at(8), u16_at(12), u16_at(14)), (40, 2, 2, 1, 32));
        assert_eq!(dib.len(), 40 + 2 * 2 * 4);
    }
}
