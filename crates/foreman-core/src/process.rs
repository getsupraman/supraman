//! Spawning a child: no console window, both pipes drained on threads, a deadline, and on timeout a
//! kill of the child or of its whole tree.

use std::ffi::OsStr;
use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub const GIT_TIMEOUT: Duration = Duration::from_secs(30);

/// What a finished child left behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error("{0}")]
    Spawn(#[source] std::io::Error),
    #[error("did not finish within {}s and was killed", .0.as_secs())]
    Timeout(Duration),
}

/// What a timeout kills.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kill {
    /// The child and every process it started (a job object on Windows).
    Tree,
    /// The child alone: for `claude`, whose descendants include the background service every session runs in.
    Child,
}

/// A child of the app gets no console window of its own: Foreman has no console to share when started
/// from Explorer (m5/22), and every `claude`, `git` and `icacls` would flash one.
pub fn no_window(command: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// `Command::new(program)` with [`no_window`].
pub fn command(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    no_window(&mut command);
    command
}

/// Runs `cmd` with stdin closed and both pipes captured, killing it (per `kill`) once `timeout` passes.
pub fn run(cmd: &mut Command, timeout: Duration, kill: Kill) -> Result<Output, RunError> {
    no_window(cmd).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(RunError::Spawn)?;
    let tree = (kill == Kill::Tree).then(|| Job::adopt(&child)).flatten();
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                terminate(&mut child, tree.as_ref());
                return Err(RunError::Timeout(timeout));
            }
            Err(e) => {
                terminate(&mut child, tree.as_ref());
                return Err(RunError::Spawn(e));
            }
        }
    };
    Ok(Output {
        code: status.code(),
        stdout: stdout.join().unwrap_or_default(),
        stderr: stderr.join().unwrap_or_default(),
    })
}

/// `git -C <dir> <args>` within [`GIT_TIMEOUT`].
pub fn git(dir: &Path, args: &[&str]) -> Result<Output, RunError> {
    run(command("git").arg("-C").arg(dir).args(args), GIT_TIMEOUT, Kill::Tree)
}

fn terminate(child: &mut Child, tree: Option<&Job>) {
    if !tree.is_some_and(Job::terminate) {
        let _ = child.kill();
    }
    let _ = child.wait();
}

fn drain(pipe: Option<impl Read + Send + 'static>) -> thread::JoinHandle<String> {
    thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut buf);
        }
        String::from_utf8_lossy(&buf).into_owned()
    })
}

/// A job object holding one child's tree; closing it leaves the tree running.
#[cfg(windows)]
struct Job(windows::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl Job {
    /// `None` when the job cannot be made or joined; the caller then kills the child alone.
    fn adopt(child: &Child) -> Option<Self> {
        use std::os::windows::io::AsRawHandle as _;
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::System::JobObjects::{AssignProcessToJobObject, CreateJobObjectW};
        // SAFETY: the job handle is owned by the returned value; the child's handle outlives the call.
        unsafe {
            let job = Job(CreateJobObjectW(None, windows::core::PCWSTR::null()).ok()?);
            AssignProcessToJobObject(job.0, HANDLE(child.as_raw_handle())).ok()?;
            Some(job)
        }
    }

    fn terminate(&self) -> bool {
        // SAFETY: a job handle this value owns.
        unsafe { windows::Win32::System::JobObjects::TerminateJobObject(self.0, 1).is_ok() }
    }
}

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
struct Job;

#[cfg(not(windows))]
impl Job {
    fn adopt(_child: &Child) -> Option<Self> {
        None
    }

    fn terminate(&self) -> bool {
        false
    }
}

/// `ShellExecuteW(verb, target, args, cwd)`; `Err` carries its return code (`<= 32`; 5 is a declined UAC prompt).
#[cfg(windows)]
pub fn shell_execute(verb: &str, target: &OsStr, args: Option<&OsStr>, cwd: Option<&Path>) -> Result<(), isize> {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::{HSTRING, PCWSTR};
    let (verb, target) = (HSTRING::from(verb), HSTRING::from(target));
    let args = args.map(HSTRING::from);
    let cwd = cwd.map(|c| HSTRING::from(c.as_os_str()));
    let ptr = |h: &Option<HSTRING>| h.as_ref().map_or(PCWSTR::null(), |h| PCWSTR(h.as_ptr()));
    // SAFETY: every string is an HSTRING that outlives the call.
    let code = unsafe { ShellExecuteW(None, &verb, &target, ptr(&args), ptr(&cwd), SW_SHOWNORMAL) }.0 as isize;
    if code > 32 { Ok(()) } else { Err(code) }
}

#[cfg(not(windows))]
pub fn shell_execute(_verb: &str, _target: &OsStr, _args: Option<&OsStr>, _cwd: Option<&Path>) -> Result<(), isize> {
    Err(0)
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
    Path::new("/proc").join(pid.to_string()).exists().then_some(0)
}

pub fn alive(pid: u32) -> bool {
    started(pid).is_some()
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
    let deadline = Instant::now() + timeout;
    let watched: Vec<u32> = pids.iter().copied().filter(|&pid| alive(pid)).collect();
    while !watched.is_empty() && Instant::now() < deadline {
        if watched.iter().any(|&pid| !alive(pid)) {
            return true;
        }
        thread::sleep(Duration::from_millis(100));
    }
    false
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    /// Windows PowerShell, a real exe: `pwsh` can be a Store alias whose process is no descendant of the caller.
    fn powershell(script: &str) -> Command {
        let mut cmd = command("powershell");
        cmd.args(["-NoProfile", "-NonInteractive", "-Command", script]);
        cmd
    }

    #[test]
    fn output_and_exit_code_come_back() {
        let out = run(&mut powershell("[Console]::Out.Write('out'); [Console]::Error.Write('err'); exit 3"), Duration::from_secs(60), Kill::Tree).unwrap();
        assert_eq!(out, Output { code: Some(3), stdout: "out".into(), stderr: "err".into() });
        assert!(!out.success());
    }

    #[test]
    fn a_missing_program_is_a_spawn_error_with_its_kind() {
        match run(&mut command("foreman-no-such-program"), Duration::from_secs(5), Kill::Tree) {
            Err(RunError::Spawn(e)) => assert_eq!(e.kind(), std::io::ErrorKind::NotFound),
            other => panic!("{other:?}"),
        }
    }

    /// The child starts a grandchild that would outlive it; a tree timeout kills both, a child one only the child.
    #[test]
    fn a_timeout_kills_the_tree_or_only_the_child() {
        for (kill, grandchild_left) in [(Kill::Tree, false), (Kill::Child, true)] {
            let dir = tempfile::tempdir().unwrap();
            let pid_file = dir.path().join("pid");
            let script = format!(
                "$p = [System.Diagnostics.Process]::Start('ping', '-n 60 127.0.0.1'); Set-Content -LiteralPath '{}' $p.Id; Start-Sleep 60",
                pid_file.display()
            );
            let started_at = Instant::now();
            let err = run(&mut powershell(&script), Duration::from_secs(8), kill).unwrap_err();
            assert!(matches!(err, RunError::Timeout(d) if d == Duration::from_secs(8)), "{err:?}");
            assert!(started_at.elapsed() < Duration::from_secs(30), "{:?}", started_at.elapsed());
            let pid: u32 = std::fs::read_to_string(&pid_file).expect("the grandchild started").trim().parse().unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            while alive(pid) && !grandchild_left && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(100));
            }
            assert_eq!(alive(pid), grandchild_left, "{kill:?}");
            if grandchild_left {
                let _ = Command::new("taskkill").args(["/PID", &pid.to_string(), "/F"]).stdout(Stdio::null()).stderr(Stdio::null()).status();
            }
        }
    }

    #[test]
    fn git_runs_in_the_given_dir() {
        let dir = tempfile::tempdir().unwrap();
        let out = git(dir.path(), &["rev-parse", "--is-inside-work-tree"]).unwrap();
        assert!(!out.success(), "a fresh temp dir is no repo: {out:?}");
    }

    #[test]
    fn this_process_is_alive_and_a_spent_pid_is_not() {
        assert!(alive(std::process::id()));
        let mut child = command("cmd").args(["/C", "exit 0"]).spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        assert!(!alive(pid));
    }

    #[test]
    fn a_wait_returns_when_one_pid_exits_and_times_out_on_live_ones() {
        let me = std::process::id();
        let started = Instant::now();
        assert!(!wait_any_exit(&[me], Duration::from_millis(150)));
        assert!(started.elapsed() >= Duration::from_millis(140));
        assert!(!wait_any_exit(&[], Duration::from_secs(10)), "nothing to wait on");

        let mut child = powershell("Start-Sleep -Milliseconds 300").spawn().unwrap();
        let started = Instant::now();
        assert!(wait_any_exit(&[me, child.id()], Duration::from_secs(30)));
        assert!(started.elapsed() < Duration::from_secs(20), "{:?}", started.elapsed());
        child.wait().unwrap();
    }
}
