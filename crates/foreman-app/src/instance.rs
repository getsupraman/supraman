//! One Foreman per `FOREMAN_HOME` in the user's session: a named mutex `Local\Foreman-<key>`, the key
//! hashed from the home's canonical path, so a selftest's scratch home never collides with the user's
//! running app. A second launch on the same home sets the named event `Local\Foreman-<key>-open`,
//! which the first one waits on to bring its window to the front, and exits.

use std::path::Path;

/// FNV-1a of the lowercased canonical path (Windows paths are case-insensitive): stable across builds,
/// unlike `DefaultHasher`, so an old and a new `foreman.exe` on one home still see each other. A home
/// that does not exist yet hashes as its absolute path, which is what its canonical path becomes once
/// it does, less the `\\?\` prefix.
pub fn key(home: &Path) -> String {
    let path = std::fs::canonicalize(home)
        .or_else(|_| std::path::absolute(home))
        .unwrap_or_else(|_| home.to_path_buf());
    let path = path.to_string_lossy();
    let path = match path.strip_prefix(r"\\?\") {
        Some(rest) => rest.strip_prefix(r"UNC\").map_or_else(|| rest.to_string(), |share| format!(r"\\{share}")),
        None => path.into_owned(),
    };
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in path.to_lowercase().bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

pub enum Acquired {
    /// This process is Foreman for the home; the lock is held until the value is dropped.
    First(Instance),
    /// Another process holds it; it was asked to show its window.
    Running,
}

#[cfg(windows)]
pub use imp::{Instance, acquire};

#[cfg(windows)]
mod imp {
    use std::path::Path;

    use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{
        CreateEventW, CreateMutexW, EVENT_MODIFY_STATE, INFINITE, OpenEventW, SYNCHRONIZATION_SYNCHRONIZE, SetEvent,
        WaitForSingleObject,
    };
    use windows::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};
    use windows::core::HSTRING;

    use super::{Acquired, key};

    pub struct Instance {
        mutex: HANDLE,
        open: HANDLE,
        open_name: HSTRING,
    }

    // Kernel handles; usable from any thread.
    unsafe impl Send for Instance {}

    pub fn acquire(home: &Path) -> anyhow::Result<Acquired> {
        let key = key(home);
        let mutex = unsafe { CreateMutexW(None, true, &HSTRING::from(format!("Local\\Foreman-{key}"))) }?;
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            unsafe { CloseHandle(mutex) }.ok();
            signal(&key);
            return Ok(Acquired::Running);
        }
        let open_name = HSTRING::from(format!("Local\\Foreman-{key}-open"));
        let open = match unsafe { CreateEventW(None, false, false, &open_name) } {
            Ok(open) => open,
            Err(e) => {
                unsafe { CloseHandle(mutex) }.ok();
                return Err(e.into());
            }
        };
        Ok(Acquired::First(Instance { mutex, open, open_name }))
    }

    /// The running instance may take the foreground: this launch has it (the user just started it).
    fn signal(key: &str) {
        let name = HSTRING::from(format!("Local\\Foreman-{key}-open"));
        unsafe {
            let _ = AllowSetForegroundWindow(ASFW_ANY);
            if let Ok(open) = OpenEventW(EVENT_MODIFY_STATE, false, &name) {
                let _ = SetEvent(open);
                let _ = CloseHandle(open);
            }
        }
    }

    impl Instance {
        /// Calls `opened` on a thread of its own each time a second launch asks for the window.
        pub fn on_open(&self, opened: impl Fn() + Send + 'static) -> anyhow::Result<()> {
            let open = unsafe { OpenEventW(SYNCHRONIZATION_SYNCHRONIZE, false, &self.open_name) }?;
            let open = open.0 as isize;
            std::thread::Builder::new().name("foreman-instance".into()).spawn(move || {
                let open = HANDLE(open as *mut _);
                while unsafe { WaitForSingleObject(open, INFINITE) } == WAIT_OBJECT_0 {
                    opened();
                }
            })?;
            Ok(())
        }
    }

    impl Drop for Instance {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.open);
                let _ = CloseHandle(self.mutex);
            }
        }
    }
}

#[cfg(not(windows))]
pub struct Instance;

#[cfg(not(windows))]
impl Instance {
    pub fn on_open(&self, _opened: impl Fn() + Send + 'static) -> anyhow::Result<()> {
        Ok(())
    }
}

#[cfg(not(windows))]
pub fn acquire(_home: &Path) -> anyhow::Result<Acquired> {
    Ok(Acquired::First(Instance))
}

#[cfg(all(test, windows))]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use super::{Acquired, acquire, key};

    #[test]
    fn a_second_acquire_on_the_same_home_finds_the_first_and_asks_it_to_open() {
        let home = tempfile::tempdir().unwrap();
        let Acquired::First(first) = acquire(home.path()).unwrap() else {
            panic!("the first acquire must win");
        };
        let (tx, rx) = mpsc::channel();
        first.on_open(move || { tx.send(()).ok(); }).unwrap();
        assert!(matches!(acquire(home.path()).unwrap(), Acquired::Running));
        rx.recv_timeout(Duration::from_secs(5)).expect("the first instance was not asked to open");
        // another spelling of the same directory is the same home
        let upper = std::path::PathBuf::from(home.path().to_string_lossy().to_uppercase());
        assert!(matches!(acquire(&upper).unwrap(), Acquired::Running));
        drop(first);
        assert!(matches!(acquire(home.path()).unwrap(), Acquired::First(_)));
    }

    #[test]
    fn a_home_hashes_the_same_before_and_after_it_exists() {
        let parent = tempfile::tempdir().unwrap();
        let home = parent.path().join("home");
        let before = key(&home);
        std::fs::create_dir(&home).unwrap();
        assert_eq!(before, key(&home));
    }

    #[test]
    fn another_home_takes_its_own_lock() {
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        assert_ne!(key(a.path()), key(b.path()));
        let first = acquire(a.path()).unwrap();
        assert!(matches!(first, Acquired::First(_)));
        assert!(matches!(acquire(b.path()).unwrap(), Acquired::First(_)));
    }
}
