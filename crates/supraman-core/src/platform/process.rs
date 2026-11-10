use std::process::{Child, Command};
use std::time::Duration;

#[cfg(windows)]
pub(crate) const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// A child of the app gets no console window of its own: Supraman has no console to share when started
/// from Explorer, and every `claude`, `git` and `icacls` would flash one. Elsewhere a child has no window to hide.
pub(crate) fn no_window(command: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

#[cfg(windows)]
pub(crate) fn suspended(command: &mut Command) -> &mut Command {
    use std::os::windows::process::CommandExt as _;
    command.creation_flags(CREATE_NO_WINDOW | windows::Win32::System::Threading::CREATE_SUSPENDED.0)
}

/// Started as asked: [`resume`] has nothing to resume here.
#[cfg(not(windows))]
pub(crate) fn suspended(command: &mut Command) -> &mut Command {
    command
}

/// Resumes a child started [`suspended`]. `NtResumeProcess` resumes it by the process handle `Child` holds:
/// std keeps no handle to its main thread, and finding that thread through Toolhelp32 snapshots every
/// thread on the machine.
#[cfg(windows)]
pub(crate) fn resume(child: &Child) -> std::io::Result<()> {
    use std::os::windows::io::AsRawHandle as _;
    #[link(name = "ntdll", kind = "raw-dylib")]
    unsafe extern "system" {
        fn NtResumeProcess(process: *mut std::ffi::c_void) -> i32;
    }
    // SAFETY: the child's process handle, open while `child` lives.
    let status = unsafe { NtResumeProcess(child.as_raw_handle()) };
    if status >= 0 { Ok(()) } else { Err(std::io::Error::other(format!("NtResumeProcess failed: {status:#x}"))) }
}

#[cfg(not(windows))]
pub(crate) fn resume(_child: &Child) -> std::io::Result<()> {
    Ok(())
}

/// Waits up to `wait` for the child to exit; `None` while it still runs.
#[cfg(windows)]
pub(crate) fn wait_exit(child: &mut Child, wait: Duration) -> std::io::Result<Option<std::process::ExitStatus>> {
    use std::os::windows::io::AsRawHandle as _;
    use windows::Win32::Foundation::{HANDLE, WAIT_FAILED};
    let ms = u32::try_from(wait.as_nanos().div_ceil(1_000_000)).unwrap_or(u32::MAX - 1);
    // SAFETY: the child's process handle, open while `child` lives.
    if unsafe { windows::Win32::System::Threading::WaitForSingleObject(HANDLE(child.as_raw_handle()), ms) } == WAIT_FAILED {
        return Err(std::io::Error::last_os_error());
    }
    child.try_wait()
}

#[cfg(not(windows))]
pub(crate) fn wait_exit(child: &mut Child, wait: Duration) -> std::io::Result<Option<std::process::ExitStatus>> {
    let until = std::time::Instant::now() + wait;
    loop {
        match child.try_wait()? {
            None if std::time::Instant::now() < until => std::thread::sleep(Duration::from_millis(5)),
            status => return Ok(status),
        }
    }
}

/// A job object holding one child's tree. It kills the tree when its last handle closes, so a tree whose owner
/// dies goes with it; [`Job::release`] lifts that once the child exited, and its descendants keep running.
#[cfg(windows)]
pub(crate) struct Job(windows::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl Job {
    /// `None` when the job cannot be made or joined, said on stderr; the caller then kills the child alone.
    pub(crate) fn adopt(child: &Child) -> Option<Self> {
        use std::os::windows::io::AsRawHandle as _;
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::System::JobObjects::{AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE};
        // SAFETY: the job handle is owned by the returned value; the child's handle outlives the call.
        let made = unsafe {
            CreateJobObjectW(None, windows::core::PCWSTR::null()).map(Job).and_then(|job| {
                job.limit(JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE)?;
                AssignProcessToJobObject(job.0, HANDLE(child.as_raw_handle()))?;
                Ok(job)
            })
        };
        made.inspect_err(|e| eprintln!("process {} runs outside a job, a timeout kills it alone: {e}", child.id())).ok()
    }

    pub(crate) fn release(&self) {
        let _ = self.limit(windows::Win32::System::JobObjects::JOB_OBJECT_LIMIT(0));
    }

    fn limit(&self, flags: windows::Win32::System::JobObjects::JOB_OBJECT_LIMIT) -> windows::core::Result<()> {
        use windows::Win32::System::JobObjects::{JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject};
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = flags;
        // SAFETY: a job handle this value owns, and a struct of the size passed.
        unsafe {
            SetInformationJobObject(
                self.0,
                JobObjectExtendedLimitInformation,
                std::ptr::from_ref(&info).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        }
    }

    pub(crate) fn terminate(&self) -> bool {
        // SAFETY: a job handle this value owns.
        unsafe { windows::Win32::System::JobObjects::TerminateJobObject(self.0, 1).is_ok() }
    }
}

// SAFETY: a job handle may be used and closed from any thread.
#[cfg(windows)]
unsafe impl Send for Job {}

#[cfg(windows)]
impl Drop for Job {
    fn drop(&mut self) {
        // SAFETY: closed once, here.
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

#[cfg(not(windows))]
pub(crate) struct Job;

#[cfg(not(windows))]
impl Job {
    pub(crate) fn adopt(child: &Child) -> Option<Self> {
        eprintln!("process {} runs outside a job, a timeout kills it alone: {}", child.id(), super::refused("a job object"));
        None
    }

    pub(crate) fn release(&self) {}

    pub(crate) fn terminate(&self) -> bool {
        false
    }
}

/// The start time `pid` runs with; `None` when it does not run or cannot be opened.
#[cfg(windows)]
pub fn started(pid: u32) -> Option<u64> {
    use windows::Win32::Foundation::{CloseHandle, FILETIME, STILL_ACTIVE};
    use windows::Win32::System::Threading::{GetExitCodeProcess, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    // SAFETY: plain Win32 calls on a handle this function opens and closes.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut code = 0u32;
        let (mut created, mut exited, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        let running = GetExitCodeProcess(handle, &mut code).is_ok() && code == STILL_ACTIVE.0 as u32;
        let times = GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user).is_ok();
        let _ = CloseHandle(handle);
        (running && times).then(|| (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
    }
}

/// No start time off Windows: a running pid reads as 0.
#[cfg(not(windows))]
pub fn started(pid: u32) -> Option<u64> {
    std::path::Path::new("/proc").join(pid.to_string()).exists().then_some(0)
}

/// Ends `pid` while it runs with the start time `start` ([`started`]), never a process that took its pid since.
#[cfg(windows)]
pub fn terminate_started(pid: u32, start: u64) -> bool {
    use windows::Win32::Foundation::{CloseHandle, FILETIME};
    use windows::Win32::System::Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE, TerminateProcess};
    // SAFETY: plain Win32 calls on a handle this function opens and closes.
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return false;
        };
        let (mut created, mut exited, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        let same = GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user).is_ok()
            && (u64::from(created.dwHighDateTime) << 32 | u64::from(created.dwLowDateTime)) == start;
        let ended = same && TerminateProcess(handle, 1).is_ok();
        let _ = CloseHandle(handle);
        ended
    }
}

#[cfg(not(windows))]
pub fn terminate_started(pid: u32, start: u64) -> bool {
    started(pid) == Some(start) && crate::process::command("kill").args(["-9", &pid.to_string()]).status().is_ok_and(|s| s.success())
}

/// Blocks until one of `pids` exits or `timeout` passes; `true` when one exited. A pid that cannot be
/// opened is not waited on, and with none left it returns `false` at once.
#[cfg(windows)]
pub fn wait_any_exit(pids: &[u32], timeout: Duration) -> bool {
    use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForMultipleObjects};
    const MAXIMUM_WAIT_OBJECTS: usize = 64;
    // SAFETY: plain Win32 calls on handles this function opens and closes.
    unsafe {
        let handles: Vec<_> = pids.iter().filter_map(|&pid| OpenProcess(PROCESS_SYNCHRONIZE, false, pid).ok()).take(MAXIMUM_WAIT_OBJECTS).collect();
        if handles.is_empty() {
            return false;
        }
        let ms = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX - 1);
        let woke = WaitForMultipleObjects(&handles, false, ms);
        for handle in &handles {
            let _ = CloseHandle(*handle);
        }
        (WAIT_OBJECT_0.0..WAIT_OBJECT_0.0 + handles.len() as u32).contains(&woke.0)
    }
}

#[cfg(not(windows))]
pub fn wait_any_exit(pids: &[u32], timeout: Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    let watched: Vec<u32> = pids.iter().copied().filter(|&pid| started(pid).is_some()).collect();
    while !watched.is_empty() && std::time::Instant::now() < deadline {
        if watched.iter().any(|&pid| started(pid).is_none()) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

/// A process as a [`snapshot`] lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proc {
    pub pid: u32,
    pub parent: u32,
    /// The executable's file name, e.g. `claude.exe`.
    pub exe: String,
}

/// Every process now, from one Toolhelp32 snapshot; empty when the snapshot fails.
#[cfg(windows)]
pub fn snapshot() -> Vec<Proc> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS};
    let mut out = Vec::new();
    // SAFETY: a snapshot handle this function opens, walks and closes.
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return out;
        };
        let mut entry = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut more = Process32FirstW(snap, &mut entry).is_ok();
        while more {
            let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            out.push(Proc {
                pid: entry.th32ProcessID,
                parent: entry.th32ParentProcessID,
                exe: String::from_utf16_lossy(&entry.szExeFile[..len]),
            });
            more = Process32NextW(snap, &mut entry).is_ok();
        }
        let _ = CloseHandle(snap);
    }
    out
}

/// Empty, as a failed snapshot is, and said on stderr.
#[cfg(not(windows))]
pub fn snapshot() -> Vec<Proc> {
    eprintln!("{}", super::refused("a process snapshot"));
    Vec::new()
}

/// (private bytes, working set) of this process; (0, 0) when they cannot be read.
#[cfg(windows)]
pub fn memory() -> (usize, usize) {
    use windows::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows::Win32::System::Threading::GetCurrentProcess;
    let mut counters = PROCESS_MEMORY_COUNTERS_EX {
        cb: size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        ..Default::default()
    };
    let ok = unsafe {
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut counters as *mut PROCESS_MEMORY_COUNTERS_EX as *mut PROCESS_MEMORY_COUNTERS,
            counters.cb,
        )
    };
    if ok.is_err() {
        return (0, 0);
    }
    (counters.PrivateUsage, counters.WorkingSetSize)
}

/// (0, 0), as a failed read is, and said on stderr.
#[cfg(not(windows))]
pub fn memory() -> (usize, usize) {
    eprintln!("{}", super::refused("this process's memory"));
    (0, 0)
}
