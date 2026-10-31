use std::path::Path;

/// FNV-1a, stable across builds unlike `DefaultHasher`, so an old and a new build on one home see each other.
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

const NAME: &str = "Supraman";
/// An older build's name for the same lock: it never sees a `Supraman-` one.
const LEGACY_NAME: &str = "Foreman";

pub enum Acquired {
    First(Instance),
    Running,
    /// An older build runs on this home, or on the home this one was copied from; it was asked to show its window.
    Legacy(std::path::PathBuf),
}

#[cfg(windows)]
pub use imp::{Instance, acquire};

#[cfg(windows)]
mod imp {
    use std::path::Path;

    use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{
        CreateEventW, CreateMutexW, EVENT_MODIFY_STATE, INFINITE, OpenEventW, OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE, SetEvent,
        WaitForSingleObject,
    };
    use windows::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};
    use windows::core::HSTRING;

    use super::{Acquired, LEGACY_NAME, NAME, key};

    pub struct Instance {
        mutex: HANDLE,
        open: HANDLE,
        open_name: HSTRING,
    }

    // Kernel handles; usable from any thread.
    unsafe impl Send for Instance {}

    /// `copied_from`: the home this one was copied from, whose older build would supervise the same sessions.
    pub fn acquire(home: &Path, copied_from: Option<&Path>) -> anyhow::Result<Acquired> {
        for legacy in std::iter::once(home).chain(copied_from) {
            let key = key(legacy);
            let name = HSTRING::from(format!("Local\\{LEGACY_NAME}-{key}"));
            if let Ok(held) = unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, &name) } {
                unsafe { CloseHandle(held) }.ok();
                signal(LEGACY_NAME, &key);
                return Ok(Acquired::Legacy(legacy.to_path_buf()));
            }
        }
        let key = key(home);
        let mutex = unsafe { CreateMutexW(None, true, &HSTRING::from(format!("Local\\{NAME}-{key}"))) }?;
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            unsafe { CloseHandle(mutex) }.ok();
            signal(NAME, &key);
            return Ok(Acquired::Running);
        }
        let open_name = HSTRING::from(format!("Local\\{NAME}-{key}-open"));
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
    fn signal(prefix: &str, key: &str) {
        let name = HSTRING::from(format!("Local\\{prefix}-{key}-open"));
        unsafe {
            let _ = AllowSetForegroundWindow(ASFW_ANY);
            if let Ok(open) = OpenEventW(EVENT_MODIFY_STATE, false, &name) {
                let _ = SetEvent(open);
                let _ = CloseHandle(open);
            }
        }
    }

    impl Instance {
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
pub fn acquire(_home: &Path, _copied_from: Option<&Path>) -> anyhow::Result<Acquired> {
    Ok(Acquired::First(Instance))
}

#[cfg(all(test, windows))]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{CreateEventW, CreateMutexW, OpenEventW, SYNCHRONIZATION_SYNCHRONIZE, WaitForSingleObject};
    use windows::core::HSTRING;

    use super::{Acquired, acquire, key};

    #[test]
    fn a_second_acquire_on_the_same_home_finds_the_first_and_asks_it_to_open() {
        let home = tempfile::tempdir().unwrap();
        let Acquired::First(first) = acquire(home.path(), None).unwrap() else {
            panic!("the first acquire must win");
        };
        let (tx, rx) = mpsc::channel();
        first.on_open(move || { tx.send(()).ok(); }).unwrap();
        assert!(matches!(acquire(home.path(), None).unwrap(), Acquired::Running));
        rx.recv_timeout(Duration::from_secs(5)).expect("the first instance was not asked to open");
        let upper = std::path::PathBuf::from(home.path().to_string_lossy().to_uppercase());
        assert!(matches!(acquire(&upper, None).unwrap(), Acquired::Running));
        drop(first);
        assert!(matches!(acquire(home.path(), None).unwrap(), Acquired::First(_)));
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
        let first = acquire(a.path(), None).unwrap();
        assert!(matches!(first, Acquired::First(_)));
        assert!(matches!(acquire(b.path(), None).unwrap(), Acquired::First(_)));
    }

    /// What a Foreman build holds for `home`, by the names its `instance.rs` gives them.
    fn old_build_on(home: &std::path::Path) -> impl Drop {
        struct Held([windows::Win32::Foundation::HANDLE; 2]);
        impl Drop for Held {
            fn drop(&mut self) {
                for h in self.0 {
                    unsafe { CloseHandle(h) }.ok();
                }
            }
        }
        let key = key(home);
        let mutex = unsafe { CreateMutexW(None, true, &HSTRING::from(format!(r"Local\Foreman-{key}"))) }.unwrap();
        let open = unsafe { CreateEventW(None, false, false, &HSTRING::from(format!(r"Local\Foreman-{key}-open"))) }.unwrap();
        Held([mutex, open])
    }

    #[test]
    fn an_old_foreman_on_this_home_or_the_one_it_was_copied_from_keeps_the_new_build_out() {
        let parent = tempfile::tempdir().unwrap();
        let (old_home, home) = (parent.path().join("Foreman"), parent.path().join("Supraman"));
        let old = old_build_on(&old_home);
        assert!(matches!(acquire(&home, None).unwrap(), Acquired::First(_)), "another home's old build is not this one's");
        let Acquired::Legacy(running) = acquire(&home, Some(&old_home)).unwrap() else {
            panic!("an old build on the home this one was copied from must keep it out");
        };
        assert_eq!(running, old_home);
        let open_name = HSTRING::from(format!(r"Local\Foreman-{}-open", key(&old_home)));
        let event = unsafe { OpenEventW(SYNCHRONIZATION_SYNCHRONIZE, false, &open_name) }.unwrap();
        assert_eq!(unsafe { WaitForSingleObject(event, 0) }, WAIT_OBJECT_0, "the old build was asked to show its window");
        unsafe { CloseHandle(event) }.ok();
        drop(old);

        let same = old_build_on(&home);
        assert!(matches!(acquire(&home, None).unwrap(), Acquired::Legacy(p) if p == home), "an old build with FOREMAN_HOME at this home");
        drop(same);
        assert!(matches!(acquire(&home, Some(&old_home)).unwrap(), Acquired::First(_)));
    }
}
