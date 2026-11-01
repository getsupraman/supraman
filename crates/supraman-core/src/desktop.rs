//! A private desktop in this process's window station that is never switched to, and a child started on it
//! with its stdout and stderr piped back. Windows a child opens there neither take the user's focus nor see
//! the user's keyboard. Nothing here calls `SwitchDesktop`.

use std::ffi::OsString;
use std::io::{self, PipeReader};
use std::path::Path;

/// A desktop created by this process; closing the last handle (this value's and every child's) destroys it.
pub struct HiddenDesktop {
    #[cfg(windows)]
    handle: windows::Win32::System::StationsAndDesktops::HDESK,
    name: String,
}

/// A child started by [`spawn_on`].
pub struct DesktopChild {
    pub pid: u32,
    pub stdout: Option<PipeReader>,
    pub stderr: Option<PipeReader>,
    #[cfg(windows)]
    process: std::os::windows::io::OwnedHandle,
}

impl HiddenDesktop {
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Windows command-line quoting of one argument, as `CommandLineToArgvW` reads it back.
pub fn quote_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '\n', '\x0b', '"']) {
        return arg.to_string();
    }
    let mut out = String::from("\"");
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            c => {
                out.extend(std::iter::repeat_n('\\', backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    out.extend(std::iter::repeat_n('\\', backslashes * 2));
    out.push('"');
    out
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::os::windows::ffi::OsStrExt as _;
    use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};
    use windows::Win32::Foundation::{CloseHandle, HANDLE, HANDLE_FLAG_INHERIT, SetHandleInformation, WAIT_OBJECT_0};
    use windows::Win32::System::StationsAndDesktops::{
        CloseDesktop, CreateDesktopW, DESKTOP_CONTROL_FLAGS, DESKTOP_READOBJECTS, GetThreadDesktop, GetUserObjectInformationW, HDESK,
        OpenInputDesktop, UOI_NAME,
    };
    use windows::Win32::System::Threading::{
        CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetCurrentThreadId,
        GetExitCodeProcess, INFINITE, InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
        PROCESS_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOEXW, UpdateProcThreadAttribute, WaitForSingleObject,
    };
    use windows::core::{HSTRING, PCWSTR, PWSTR};

    // Every DESKTOP_* right below DESKTOP_SWITCHDESKTOP (0x100), and the standard rights.
    const ACCESS: u32 = 0x00FF | 0x000F_0000;

    impl HiddenDesktop {
        /// A new desktop named `<prefix>-<pid>-<hex6>`.
        pub fn create(prefix: &str) -> io::Result<Self> {
            let name = format!("{prefix}-{}-{}", std::process::id(), crate::time::hex6());
            // SAFETY: plain FFI; the handle is owned by the returned value.
            let handle = unsafe { CreateDesktopW(&HSTRING::from(&name), PCWSTR::null(), None, DESKTOP_CONTROL_FLAGS(0), ACCESS, None) }
                .map_err(io::Error::from)?;
            Ok(HiddenDesktop { handle, name })
        }
    }

    impl Drop for HiddenDesktop {
        fn drop(&mut self) {
            // SAFETY: closed once, here.
            unsafe {
                let _ = CloseDesktop(self.handle);
            }
        }
    }

    fn object_name(desk: HDESK) -> Option<String> {
        let mut buf = [0u16; 256];
        // SAFETY: the buffer outlives the call and its size in bytes is passed.
        unsafe { GetUserObjectInformationW(HANDLE(desk.0), UOI_NAME, Some(buf.as_mut_ptr().cast()), (buf.len() * 2) as u32, None) }.ok()?;
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..len]))
    }

    /// The desktop this thread's windows go to.
    pub fn thread_desktop_name() -> Option<String> {
        // SAFETY: a thread desktop handle is not closed by its caller.
        object_name(unsafe { GetThreadDesktop(GetCurrentThreadId()) }.ok()?)
    }

    /// The desktop the user's keyboard and screen are on; `None` while it cannot be opened (the lock screen).
    pub fn input_desktop_name() -> Option<String> {
        // SAFETY: the handle is closed below.
        let desk = unsafe { OpenInputDesktop(DESKTOP_CONTROL_FLAGS(0), false, DESKTOP_READOBJECTS) }.ok()?;
        let name = object_name(desk);
        unsafe {
            let _ = CloseDesktop(desk);
        }
        name
    }

    fn inheritable(handle: HANDLE) -> io::Result<()> {
        // SAFETY: a handle the caller owns.
        unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT.0, HANDLE_FLAG_INHERIT) }.map_err(io::Error::from)
    }

    fn env_block(extra: &[(&str, &str)]) -> Vec<u16> {
        let mut vars: Vec<(OsString, OsString)> =
            std::env::vars_os().filter(|(k, _)| !extra.iter().any(|(e, _)| k.eq_ignore_ascii_case(e))).collect();
        vars.extend(extra.iter().map(|(k, v)| (OsString::from(k), OsString::from(v))));
        vars.sort_by_key(|(k, _)| k.to_ascii_uppercase());
        let mut block = Vec::new();
        for (k, v) in vars {
            block.extend(k.encode_wide());
            block.push(u16::from(b'='));
            block.extend(v.encode_wide());
            block.push(0);
        }
        block.push(0);
        block
    }

    /// Starts `program args` on `desktop` with no console, no stdin, both pipes captured, and this process's
    /// environment plus `env`. It inherits the two pipe ends and no other handle.
    pub fn spawn_on(desktop: &HiddenDesktop, program: &Path, args: &[OsString], env: &[(&str, &str)]) -> io::Result<DesktopChild> {
        let (out_r, out_w) = io::pipe()?;
        let (err_r, err_w) = io::pipe()?;
        let handles = [HANDLE(out_w.as_raw_handle()), HANDLE(err_w.as_raw_handle())];
        for h in handles {
            inheritable(h)?;
        }
        let mut line = quote_arg(&program.to_string_lossy());
        for arg in args {
            line.push(' ');
            line.push_str(&quote_arg(&arg.to_string_lossy()));
        }
        let mut line: Vec<u16> = line.encode_utf16().chain([0]).collect();
        let mut desk: Vec<u16> = desktop.name.encode_utf16().chain([0]).collect();
        let env = env_block(env);

        let mut size = 0usize;
        // SAFETY: the first call only reports the size; the buffer below is that size and outlives the spawn.
        let _ = unsafe { InitializeProcThreadAttributeList(None, 1, None, &mut size) };
        let mut attrs = vec![0u8; size];
        let list = LPPROC_THREAD_ATTRIBUTE_LIST(attrs.as_mut_ptr().cast());
        unsafe { InitializeProcThreadAttributeList(Some(list), 1, None, &mut size) }.map_err(io::Error::from)?;
        let spawned = (|| {
            unsafe {
                UpdateProcThreadAttribute(
                    list,
                    0,
                    PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                    Some(handles.as_ptr().cast()),
                    std::mem::size_of_val(&handles),
                    None,
                    None,
                )
            }
            .map_err(io::Error::from)?;
            let mut si = STARTUPINFOEXW::default();
            si.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
            si.StartupInfo.lpDesktop = PWSTR(desk.as_mut_ptr());
            si.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
            si.StartupInfo.hStdOutput = handles[0];
            si.StartupInfo.hStdError = handles[1];
            si.lpAttributeList = list;
            let mut pi = PROCESS_INFORMATION::default();
            // SAFETY: every pointer refers to a buffer that outlives the call.
            unsafe {
                windows::Win32::System::Threading::CreateProcessW(
                    PCWSTR::null(),
                    Some(PWSTR(line.as_mut_ptr())),
                    None,
                    None,
                    true,
                    CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT,
                    Some(env.as_ptr().cast()),
                    PCWSTR::null(),
                    &si.StartupInfo,
                    &mut pi,
                )
            }
            .map_err(io::Error::from)?;
            unsafe {
                let _ = CloseHandle(pi.hThread);
            }
            Ok::<_, io::Error>(pi)
        })();
        unsafe { DeleteProcThreadAttributeList(list) };
        let pi = spawned?;
        drop((out_w, err_w));
        Ok(DesktopChild {
            pid: pi.dwProcessId,
            stdout: Some(out_r),
            stderr: Some(err_r),
            // SAFETY: the process handle is ours to own.
            process: unsafe { OwnedHandle::from_raw_handle(pi.hProcess.0) },
        })
    }

    impl DesktopChild {
        /// Waits for the child to exit; its exit code.
        pub fn wait(&self) -> io::Result<i32> {
            let process = HANDLE(self.process.as_raw_handle());
            // SAFETY: a process handle this value owns.
            unsafe {
                if WaitForSingleObject(process, INFINITE) != WAIT_OBJECT_0 {
                    return Err(io::Error::last_os_error());
                }
                let mut code = 0u32;
                GetExitCodeProcess(process, &mut code).map_err(io::Error::from)?;
                Ok(code as i32)
            }
        }
    }
}

#[cfg(windows)]
pub use imp::{input_desktop_name, spawn_on, thread_desktop_name};

#[cfg(not(windows))]
mod imp {
    use super::*;

    fn unsupported() -> io::Error {
        io::Error::new(io::ErrorKind::Unsupported, "hidden desktops are Windows desktops")
    }

    impl HiddenDesktop {
        pub fn create(_prefix: &str) -> io::Result<Self> {
            Err(unsupported())
        }
    }

    impl DesktopChild {
        pub fn wait(&self) -> io::Result<i32> {
            Err(unsupported())
        }
    }

    pub fn spawn_on(_desktop: &HiddenDesktop, _program: &Path, _args: &[OsString], _env: &[(&str, &str)]) -> io::Result<DesktopChild> {
        Err(unsupported())
    }

    pub fn thread_desktop_name() -> Option<String> {
        None
    }

    pub fn input_desktop_name() -> Option<String> {
        None
    }
}

#[cfg(not(windows))]
pub use imp::{input_desktop_name, spawn_on, thread_desktop_name};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_arg_round_trips_through_the_windows_rules() {
        assert_eq!(quote_arg("--selftest"), "--selftest");
        assert_eq!(quote_arg(""), "\"\"");
        assert_eq!(quote_arg(r"C:\Program Files\x"), r#""C:\Program Files\x""#);
        assert_eq!(quote_arg(r"a\b c\"), r#""a\b c\\""#);
        assert_eq!(quote_arg(r#"say "hi""#), r#""say \"hi\"""#);
    }

    #[cfg(windows)]
    #[test]
    fn a_child_runs_on_the_hidden_desktop_and_its_output_comes_back() {
        use std::io::Read as _;
        let desktop = HiddenDesktop::create("supraman-test").expect("a desktop");
        let script = r#"Add-Type 'using System; using System.Runtime.InteropServices; public static class D {
            [DllImport("user32.dll")] public static extern IntPtr GetThreadDesktop(uint t);
            [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
            [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern bool GetUserObjectInformationW(IntPtr o, int i, System.Text.StringBuilder b, int n, out int need);
            public static string Name() { var b = new System.Text.StringBuilder(256); int n; GetUserObjectInformationW(GetThreadDesktop(GetCurrentThreadId()), 2, b, 512, out n); return b.ToString(); } }';
            [Console]::Out.Write($env:SUPRAMAN_DESKTOP_TEST + ' on ' + [D]::Name()); [Console]::Error.Write('e'); exit 7"#;
        let pwsh = std::env::var_os("SystemRoot").map(std::path::PathBuf::from).unwrap_or_default().join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
        let args: Vec<OsString> = ["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", script].iter().map(OsString::from).collect();
        let mut child = spawn_on(&desktop, &pwsh, &args, &[("SUPRAMAN_DESKTOP_TEST", "here")]).expect("spawned");
        let (mut out, mut err) = (String::new(), String::new());
        child.stdout.take().unwrap().read_to_string(&mut out).unwrap();
        child.stderr.take().unwrap().read_to_string(&mut err).unwrap();
        assert_eq!(child.wait().unwrap(), 7);
        assert_eq!(out, format!("here on {}", desktop.name()));
        assert_eq!(err, "e");
        assert_ne!(thread_desktop_name().as_deref(), Some(desktop.name()));
    }
}
