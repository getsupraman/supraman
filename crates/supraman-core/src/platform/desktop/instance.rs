/// What [`claim`] found.
pub enum Claimed {
    First(Instance),
    Running,
}

#[cfg(windows)]
pub use imp::{Instance, claim};

#[cfg(windows)]
mod imp {
    use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{
        CreateEventW, CreateMutexW, EVENT_MODIFY_STATE, INFINITE, OpenEventW, SYNCHRONIZATION_SYNCHRONIZE, SetEvent,
        WaitForSingleObject,
    };
    use windows::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};
    use windows::core::HSTRING;

    use super::Claimed;

    /// The first instance's hold on `name`, released when dropped.
    pub struct Instance {
        mutex: HANDLE,
        open: HANDLE,
        open_name: HSTRING,
    }

    // Kernel handles; usable from any thread.
    unsafe impl Send for Instance {}

    /// The session-wide lock `name`: [`Claimed::First`] when taken, else [`Claimed::Running`] after asking the holder to open.
    pub fn claim(name: &str) -> Result<Claimed, String> {
        let mutex = unsafe { CreateMutexW(None, true, &HSTRING::from(format!("Local\\{name}"))) }.map_err(|e| e.to_string())?;
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            unsafe { CloseHandle(mutex) }.ok();
            signal(name);
            return Ok(Claimed::Running);
        }
        let open_name = HSTRING::from(format!("Local\\{name}-open"));
        let open = match unsafe { CreateEventW(None, false, false, &open_name) } {
            Ok(open) => open,
            Err(e) => {
                unsafe { CloseHandle(mutex) }.ok();
                return Err(e.to_string());
            }
        };
        Ok(Claimed::First(Instance { mutex, open, open_name }))
    }

    /// The running instance may take the foreground: this launch has it (the user just started it).
    fn signal(name: &str) {
        let name = HSTRING::from(format!("Local\\{name}-open"));
        unsafe {
            let _ = AllowSetForegroundWindow(ASFW_ANY);
            if let Ok(open) = OpenEventW(EVENT_MODIFY_STATE, false, &name) {
                let _ = SetEvent(open);
                let _ = CloseHandle(open);
            }
        }
    }

    impl Instance {
        /// Runs `opened` on a thread of its own each time a later [`claim`] of the same name asks this one to open.
        pub fn on_open(&self, opened: impl Fn() + Send + 'static) -> Result<(), String> {
            let open = unsafe { OpenEventW(SYNCHRONIZATION_SYNCHRONIZE, false, &self.open_name) }.map_err(|e| e.to_string())?;
            let open = open.0 as isize;
            std::thread::Builder::new()
                .name("supraman-instance".into())
                .spawn(move || {
                    let open = HANDLE(open as *mut _);
                    while unsafe { WaitForSingleObject(open, INFINITE) } == WAIT_OBJECT_0 {
                        opened();
                    }
                })
                .map_err(|e| e.to_string())?;
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

/// Never held: [`claim`] refuses here.
#[cfg(not(windows))]
pub struct Instance;

#[cfg(not(windows))]
impl Instance {
    pub fn on_open(&self, _opened: impl Fn() + Send + 'static) -> Result<(), String> {
        Err(crate::platform::refused("a second launch's call to open"))
    }
}

#[cfg(not(windows))]
pub fn claim(_name: &str) -> Result<Claimed, String> {
    Err(crate::platform::refused("one instance per home"))
}
