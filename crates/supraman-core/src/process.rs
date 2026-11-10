//! Spawning a child: no console window, both pipes drained on threads, a deadline, and on timeout a
//! kill of the child or of its whole tree.

use std::ffi::OsStr;
use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
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

/// A child of the app gets no console window of its own: Supraman has no console to share when started
/// from Explorer, and every `claude`, `git` and `icacls` would flash one.
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

/// Starts `cmd` with no pipes and does not wait for it: a child that outlives the call (Run now's `smw`).
/// Its exit is never reaped by the caller; it returns the pid.
pub fn spawn_detached(cmd: &mut Command) -> std::io::Result<u32> {
    let child = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
    Ok(child.id())
}

/// How [`run_watched`] ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
    Exited,
    TimedOut,
    /// Its `stop` returned true.
    Stopped,
}

/// Runs `cmd` with stdin closed and both pipes captured, killing it (per `kill`) once `timeout` passes.
pub fn run(cmd: &mut Command, timeout: Duration, kill: Kill) -> Result<Output, RunError> {
    match run_watched(cmd, timeout, kill, &mut || false).map_err(RunError::Spawn)? {
        (Ended::Exited, out) => Ok(out),
        _ => Err(RunError::Timeout(timeout)),
    }
}

/// [`run`] that also kills the child once `stop` returns true (asked every 20 ms). A killed child's output is
/// what it wrote until then, and `code` is `None`.
pub fn run_watched(cmd: &mut Command, timeout: Duration, kill: Kill, stop: &mut dyn FnMut() -> bool) -> std::io::Result<(Ended, Output)> {
    let (ended, raw) = run_raw(cmd, None, timeout, kill, Some(stop))?;
    let text = |bytes: Vec<u8>| String::from_utf8_lossy(&bytes).into_owned();
    Ok((ended, Output { code: raw.code, stdout: text(raw.stdout), stderr: text(raw.stderr) }))
}

/// What a finished child left behind, as the bytes it wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawOutput {
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl RawOutput {
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

/// [`run`] that writes `input` to the child's stdin and closes it, and keeps both pipes as bytes.
pub fn run_bytes(cmd: &mut Command, input: Option<Vec<u8>>, timeout: Duration, kill: Kill) -> Result<RawOutput, RunError> {
    match run_raw(cmd, input, timeout, kill, None).map_err(RunError::Spawn)? {
        (Ended::Exited, out) => Ok(out),
        _ => Err(RunError::Timeout(timeout)),
    }
}

/// How often a [`run_watched`] `stop` is asked.
const STOP_TICK: Duration = Duration::from_millis(20);

fn run_raw(cmd: &mut Command, input: Option<Vec<u8>>, timeout: Duration, kill: Kill, mut stop: Option<&mut dyn FnMut() -> bool>) -> std::io::Result<(Ended, RawOutput)> {
    let stdin = if input.is_some() { Stdio::piped() } else { Stdio::null() };
    cmd.stdin(stdin).stdout(Stdio::piped()).stderr(Stdio::piped());
    let (mut child, tree) = spawn(cmd, kill)?;
    feed(child.stdin.take(), input);
    let (done, settled) = mpsc::channel();
    let stdout = drain(child.stdout.take(), done.clone());
    let stderr = drain(child.stderr.take(), done);
    let deadline = Instant::now() + timeout;
    let (ended, code) = loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let wait = if stop.is_some() { left.min(STOP_TICK) } else { left };
        match wait_exit(&mut child, wait) {
            Ok(Some(status)) => break (Ended::Exited, status.code()),
            Ok(None) if Instant::now() >= deadline => break (Ended::TimedOut, None),
            Ok(None) if stop.as_mut().is_some_and(|stop| stop()) => break (Ended::Stopped, None),
            Ok(None) => {}
            Err(e) => {
                terminate(&mut child, tree.as_ref());
                return Err(e);
            }
        }
    };
    if ended == Ended::Exited {
        if let Some(job) = &tree {
            job.release();
        }
    } else {
        terminate(&mut child, tree.as_ref());
    }
    // A descendant that outlives the child (claude's background service, a `Kill::Child` survivor) keeps the pipes
    // open: take what was read by then.
    // Disconnected once both readers ended, as nothing is ever sent.
    let _ = settled.recv_timeout(PIPES_SETTLE);
    Ok((ended, RawOutput { code, stdout: stdout.finish(), stderr: stderr.finish() }))
}

thread_local! {
    static SPAWNED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many children this thread has started through this module, for a test that counts them.
pub fn spawned_here() -> usize {
    SPAWNED.get()
}

/// Starts `cmd` with no window; for [`Kill::Tree`] suspended, inside its job, and only then resumed, so nothing
/// it starts escapes the job.
fn spawn(cmd: &mut Command, kill: Kill) -> std::io::Result<(Child, Option<Job>)> {
    SPAWNED.set(SPAWNED.get() + 1);
    if kill == Kill::Child {
        return Ok((no_window(cmd).spawn()?, None));
    }
    let spawned = suspended(cmd).spawn();
    no_window(cmd);
    let mut child = spawned?;
    let job = Job::adopt(&child);
    if let Err(e) = resume(&child) {
        terminate(&mut child, job.as_ref());
        return Err(e);
    }
    Ok((child, job))
}

#[cfg(windows)]
fn suspended(command: &mut Command) -> &mut Command {
    use std::os::windows::process::CommandExt as _;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW | windows::Win32::System::Threading::CREATE_SUSPENDED.0)
}

#[cfg(not(windows))]
fn suspended(command: &mut Command) -> &mut Command {
    command
}

/// Resumes a child started [`suspended`]. `NtResumeProcess` resumes it by the process handle `Child` holds:
/// std keeps no handle to its main thread, and finding that thread through Toolhelp32 snapshots every
/// thread on the machine.
#[cfg(windows)]
fn resume(child: &Child) -> std::io::Result<()> {
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
fn resume(_child: &Child) -> std::io::Result<()> {
    Ok(())
}

/// Waits up to `wait` for the child to exit; `None` while it still runs.
#[cfg(windows)]
fn wait_exit(child: &mut Child, wait: Duration) -> std::io::Result<Option<std::process::ExitStatus>> {
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
fn wait_exit(child: &mut Child, wait: Duration) -> std::io::Result<Option<std::process::ExitStatus>> {
    let until = Instant::now() + wait;
    loop {
        match child.try_wait()? {
            None if Instant::now() < until => thread::sleep(Duration::from_millis(5)),
            status => return Ok(status),
        }
    }
}

/// Writes `input` on its own thread, so a child that writes while it reads never blocks on a full pipe.
fn feed(pipe: Option<std::process::ChildStdin>, input: Option<Vec<u8>>) {
    if let (Some(mut pipe), Some(input)) = (pipe, input) {
        thread::spawn(move || {
            use std::io::Write as _;
            let _ = pipe.write_all(&input);
        });
    }
}

/// How long the pipes may stay open after the child ended before its output is taken as it stands.
const PIPES_SETTLE: Duration = Duration::from_secs(1);

/// `git -C <dir> <args>` within [`GIT_TIMEOUT`].
pub fn git(dir: &Path, args: &[&str]) -> Result<Output, RunError> {
    run(command("git").arg("-C").arg(dir).args(args), GIT_TIMEOUT, Kill::Tree)
}

/// `cmd`, a git, with no system or user config: only the repository's own (a test's repo, C151).
pub fn without_user_config(cmd: &mut Command) -> &mut Command {
    cmd.env("GIT_CONFIG_NOSYSTEM", "1").env("GIT_CONFIG_GLOBAL", "/dev/null")
}

/// How many chunks of stdout a [`Streaming`] child's reader runs ahead of its caller; past them the child
/// blocks on its pipe until more are asked for.
const AHEAD: usize = 2;
const CHUNK: usize = 64 * 1024;

/// A child whose stdout (or stderr, [`stream_pipe`]) is read as it writes, chunk by chunk: no window, `input` on
/// stdin, the other pipe kept, its tree in a job. Dropped before [`Streaming::finish`], its tree is killed without
/// waiting.
pub struct Streaming {
    child: Child,
    tree: Option<Job>,
    chunks: mpsc::Receiver<Vec<u8>>,
    kept: Option<(Drain, mpsc::Receiver<()>)>,
    streamed: Pipe,
    finished: bool,
}

/// Which of a [`Streaming`] child's pipes is read as it writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pipe {
    Stdout,
    /// git's `--progress`.
    Stderr,
}

/// What [`Streaming::wait`] saw.
#[derive(Debug, PartialEq, Eq)]
pub enum Waited {
    Chunk(Vec<u8>),
    /// Nothing within the wait; the child runs on.
    Idle,
    /// The streamed pipe closed.
    End,
}

/// Starts `cmd` for [`Streaming`] reads of its stdout.
pub fn stream(cmd: &mut Command, input: Option<Vec<u8>>) -> std::io::Result<Streaming> {
    stream_pipe(cmd, input, Pipe::Stdout)
}

/// Starts `cmd` for [`Streaming`] reads of `pipe`.
pub fn stream_pipe(cmd: &mut Command, input: Option<Vec<u8>>, pipe: Pipe) -> std::io::Result<Streaming> {
    let stdin = if input.is_some() { Stdio::piped() } else { Stdio::null() };
    cmd.stdin(stdin).stdout(Stdio::piped()).stderr(Stdio::piped());
    let (mut child, tree) = spawn(cmd, Kill::Tree)?;
    feed(child.stdin.take(), input);
    let (to, chunks) = mpsc::sync_channel(AHEAD);
    let (streamed, kept): (Option<Box<dyn Read + Send>>, Option<Box<dyn Read + Send>>) = match pipe {
        Pipe::Stdout => (child.stdout.take().map(|p| Box::new(p) as _), child.stderr.take().map(|p| Box::new(p) as _)),
        Pipe::Stderr => (child.stderr.take().map(|p| Box::new(p) as _), child.stdout.take().map(|p| Box::new(p) as _)),
    };
    if let Some(mut read) = streamed {
        thread::spawn(move || loop {
            let mut chunk = vec![0u8; CHUNK];
            match read.read(&mut chunk) {
                Ok(n @ 1..) => {
                    chunk.truncate(n);
                    if to.send(chunk).is_err() {
                        return;
                    }
                }
                _ => return,
            }
        });
    }
    let (done, settled) = mpsc::channel();
    let kept = drain(kept, done);
    Ok(Streaming { child, tree, chunks, kept: Some((kept, settled)), streamed: pipe, finished: false })
}

impl Streaming {
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// The next bytes the child wrote, `None` once its stdout closed; none within `timeout` kills its tree.
    pub fn next(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>, RunError> {
        match self.chunks.recv_timeout(timeout) {
            Ok(chunk) => Ok(Some(chunk)),
            Err(mpsc::RecvTimeoutError::Disconnected) => Ok(None),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.finished = true;
                terminate(&mut self.child, self.tree.as_ref());
                Err(RunError::Timeout(timeout))
            }
        }
    }

    /// The next bytes within `wait`, [`Waited::Idle`] when none came; unlike [`Streaming::next`] an idle wait
    /// kills nothing, so a caller can stop between waits by dropping it.
    pub fn wait(&mut self, wait: Duration) -> Waited {
        match self.chunks.recv_timeout(wait) {
            Ok(chunk) => Waited::Chunk(chunk),
            Err(mpsc::RecvTimeoutError::Timeout) => Waited::Idle,
            Err(mpsc::RecvTimeoutError::Disconnected) => Waited::End,
        }
    }

    /// After [`Streaming::next`] gave `None`: the exit code and the other pipe, the tree killed when it has not exited
    /// within `timeout`.
    pub fn finish(mut self, timeout: Duration) -> Result<RawOutput, RunError> {
        self.finished = true;
        let code = match wait_exit(&mut self.child, timeout).map_err(RunError::Spawn)? {
            Some(status) => status.code(),
            None => {
                terminate(&mut self.child, self.tree.as_ref());
                return Err(RunError::Timeout(timeout));
            }
        };
        if let Some(job) = &self.tree {
            job.release();
        }
        let kept = self.kept.take().map(|(drain, settled)| {
            let _ = settled.recv_timeout(PIPES_SETTLE);
            drain.finish()
        });
        let kept = kept.unwrap_or_default();
        Ok(match self.streamed {
            Pipe::Stdout => RawOutput { code, stdout: Vec::new(), stderr: kept },
            Pipe::Stderr => RawOutput { code, stdout: kept, stderr: Vec::new() },
        })
    }
}

impl Drop for Streaming {
    fn drop(&mut self) {
        if !self.finished && !self.tree.as_ref().is_some_and(Job::terminate) {
            let _ = self.child.kill();
        }
    }
}

fn terminate(child: &mut Child, tree: Option<&Job>) {
    if !tree.is_some_and(Job::terminate) {
        let _ = child.kill();
    }
    let _ = child.wait();
}

/// A pipe read on its own thread into a buffer that can be taken before the pipe closes.
struct Drain(thread::JoinHandle<()>, Arc<Mutex<Vec<u8>>>);

impl Drain {
    /// Everything once the pipe closed; else what was read so far, and the reader runs on until it closes.
    fn finish(self) -> Vec<u8> {
        let Drain(handle, buf) = self;
        if handle.is_finished() {
            let _ = handle.join();
        }
        buf.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

/// Reads `pipe` until it closes; `done` drops after the last byte is in the buffer.
fn drain(pipe: Option<impl Read + Send + 'static>, done: mpsc::Sender<()>) -> Drain {
    let buf = Arc::new(Mutex::new(Vec::new()));
    let into = Arc::clone(&buf);
    let handle = thread::spawn(move || {
        let _done = done;
        let Some(mut pipe) = pipe else { return };
        let mut chunk = [0u8; 8192];
        while let Ok(n @ 1..) = pipe.read(&mut chunk) {
            into.lock().unwrap_or_else(|e| e.into_inner()).extend_from_slice(&chunk[..n]);
        }
    });
    Drain(handle, buf)
}

/// A job object holding one child's tree. It kills the tree when its last handle closes, so a tree whose owner
/// dies goes with it; [`Job::release`] lifts that once the child exited, and its descendants keep running.
#[cfg(windows)]
struct Job(windows::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl Job {
    /// `None` when the job cannot be made or joined, said on stderr; the caller then kills the child alone.
    fn adopt(child: &Child) -> Option<Self> {
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

    fn release(&self) {
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

    fn terminate(&self) -> bool {
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
struct Job;

#[cfg(not(windows))]
impl Job {
    fn adopt(_child: &Child) -> Option<Self> {
        None
    }

    fn release(&self) {}

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
    started(pid) == Some(start) && command("kill").args(["-9", &pid.to_string()]).status().is_ok_and(|s| s.success())
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
    use crate::testing::{eventually, polled, wait_until};

    /// Every child of a crate or its tests starts through [`command`]: a console program started bare gets
    /// a visible window when the parent has no console.
    #[test]
    fn no_crate_starts_a_process_without_command() {
        let needle = concat!("Command", "::new(");
        let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let own = Path::new("supraman-core").join("src").join("process.rs");
        let mut found = Vec::new();
        let mut dirs = vec![crates.to_path_buf()];
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if path.file_name().is_some_and(|n| n != "target") {
                        dirs.push(path);
                    }
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let at = path.strip_prefix(crates).unwrap().to_path_buf();
                    let text = std::fs::read_to_string(&path).unwrap();
                    for (i, line) in text.lines().enumerate() {
                        if line.contains(needle) && !line.trim_start().starts_with("//") {
                            found.push((at.clone(), i + 1));
                        }
                    }
                }
            }
        }
        let (own_uses, bare): (Vec<_>, Vec<_>) = found.into_iter().partition(|(at, _)| *at == own);
        assert!(bare.is_empty(), "start these through supraman_core::process::command: {bare:?}");
        assert_eq!(own_uses.len(), 1, "only `command` itself: {own_uses:?}");
    }

    /// Windows PowerShell, a real exe: `pwsh` can be a Store alias whose process is no descendant of the caller.
    fn powershell(script: &str) -> Command {
        let mut cmd = command("powershell");
        cmd.args(["-NoProfile", "-NonInteractive", "-Command", script]);
        cmd
    }

    #[test]
    fn terminate_started_ends_only_the_process_started_at_that_time() {
        let mut child = powershell("Start-Sleep 60").spawn().unwrap();
        let pid = child.id();
        let start = started(pid).expect("the child runs");
        assert!(!terminate_started(pid, start + 1), "another start time");
        assert!(alive(pid));
        assert!(terminate_started(pid, start));
        child.wait().unwrap();
        assert!(!alive(pid));
    }

    #[test]
    fn output_and_exit_code_come_back() {
        let out = run(&mut powershell("[Console]::Out.Write('out'); [Console]::Error.Write('err'); exit 3"), Duration::from_secs(60), Kill::Tree).unwrap();
        assert_eq!(out, Output { code: Some(3), stdout: "out".into(), stderr: "err".into() });
        assert!(!out.success());
    }

    /// Input larger than a pipe's buffer while the child echoes it: neither side blocks, and bytes come back unchanged.
    #[test]
    fn input_goes_in_and_bytes_come_back() {
        let line: Vec<u8> = (0..60u8).map(|i| 0x80 | i).chain(*b"\r\n").collect();
        let input = line.repeat(5_000);
        let mut cmd = command("findstr");
        cmd.arg("^");
        let out = run_bytes(&mut cmd, Some(input.clone()), Duration::from_secs(60), Kill::Tree).unwrap();
        assert!(out.success(), "{}", String::from_utf8_lossy(&out.stderr));
        assert_eq!(out.stdout, input);
    }

    #[test]
    fn a_detached_child_runs_on_after_the_call() {
        let dir = tempfile::tempdir().unwrap();
        let done = dir.path().join("done");
        let lives = PIPES_SETTLE * 3;
        let script = format!("Start-Sleep -Milliseconds {}; Set-Content -LiteralPath '{}' x", lives.as_millis(), done.display());
        let started = Instant::now();
        let pid = spawn_detached(&mut powershell(&script)).unwrap();
        assert!(pid > 0 && started.elapsed() < lives && !done.exists(), "returns before the child ends");
        wait_until(Duration::from_secs(30), polled(Duration::from_millis(50)), || done.exists());
        assert!(done.exists(), "the child finished on its own");
        assert!(spawn_detached(&mut command("supraman-no-such-program")).is_err());
    }

    #[test]
    fn a_missing_program_is_a_spawn_error_with_its_kind() {
        match run(&mut command("supraman-no-such-program"), Duration::from_secs(5), Kill::Tree) {
            Err(RunError::Spawn(e)) => assert_eq!(e.kind(), std::io::ErrorKind::NotFound),
            other => panic!("{other:?}"),
        }
    }

    /// The child starts a grandchild that would outlive it; a tree timeout kills both, a child one only the child.
    #[test]
    fn a_timeout_kills_the_tree_or_only_the_child() {
        let cases = [(Kill::Tree, false), (Kill::Child, true)].map(|(kill, grandchild_left)| thread::spawn(move || {
            let dir = tempfile::tempdir().unwrap();
            let pid_file = dir.path().join("pid");
            let script = format!(
                "$p = Start-Process ping -ArgumentList '-n','60','127.0.0.1' -NoNewWindow -PassThru -RedirectStandardOutput '{out}' -RedirectStandardError '{out}.err'; Set-Content -LiteralPath '{pid}' $p.Id; Start-Sleep 60",
                out = dir.path().join("ping.out").display(),
                pid = pid_file.display()
            );
            let started_at = Instant::now();
            let err = run(&mut powershell(&script), Duration::from_secs(8), kill).unwrap_err();
            assert!(matches!(err, RunError::Timeout(d) if d == Duration::from_secs(8)), "{err:?}");
            assert!(started_at.elapsed() < Duration::from_secs(30), "{:?}", started_at.elapsed());
            let pid: u32 = std::fs::read_to_string(&pid_file).expect("the grandchild started").trim().parse().unwrap();
            wait_until(Duration::from_secs(5), polled(Duration::from_millis(100)), || !(alive(pid) && !grandchild_left));
            assert_eq!(alive(pid), grandchild_left, "{kill:?}");
            if grandchild_left {
                let _ = command("taskkill").args(["/PID", &pid.to_string(), "/F"]).stdout(Stdio::null()).stderr(Stdio::null()).status();
            }
        }));
        for case in cases {
            case.join().unwrap();
        }
    }

    /// A child that starts `ping -n 60` with its own pipes (or none) and exits; the pid of ping is in `pid_file`.
    fn leaves_a_grandchild(pid_file: &Path, own_pipes: bool) -> Command {
        let redirect = if own_pipes {
            format!(" -RedirectStandardOutput '{0}.out' -RedirectStandardError '{0}.err'", pid_file.display())
        } else {
            String::new()
        };
        powershell(&format!(
            "$p = Start-Process ping -ArgumentList '-n','60','127.0.0.1' -NoNewWindow -PassThru{redirect}; Set-Content -LiteralPath '{}' $p.Id; Write-Output done",
            pid_file.display()
        ))
    }

    fn grandchild(pid_file: &Path) -> u32 {
        std::fs::read_to_string(pid_file).expect("the grandchild started").trim().parse().unwrap()
    }

    fn end(pid: u32) {
        let _ = command("taskkill").args(["/PID", &pid.to_string(), "/F"]).stdout(Stdio::null()).stderr(Stdio::null()).status();
    }

    #[test]
    fn a_child_that_exits_returns_its_output_while_a_descendant_holds_the_pipes() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("pid");
        let started_at = Instant::now();
        let out = run(&mut leaves_a_grandchild(&pid_file, false), Duration::from_secs(30), Kill::Child).unwrap();
        let took = started_at.elapsed();
        end(grandchild(&pid_file));
        assert_eq!(out.code, Some(0));
        assert!(out.stdout.lines().any(|l| l.trim() == "done"), "the child's own line, beside what the descendant wrote: {:?}", out.stdout);
        assert!(took < Duration::from_secs(20), "the call waited for the descendant: {took:?}");
    }

    #[test]
    fn a_tree_outlives_a_child_that_exited_and_dies_with_an_owner_that_did_not_release_it() {
        let dir = tempfile::tempdir().unwrap();
        let kept = dir.path().join("kept");
        let out = run(&mut leaves_a_grandchild(&kept, true), Duration::from_secs(30), Kill::Tree).unwrap();
        assert_eq!(out.code, Some(0), "{out:?}");
        let kept = grandchild(&kept);
        assert!(alive(kept), "a descendant of a child that exited keeps running");
        end(kept);

        let dropped = dir.path().join("dropped");
        let mut cmd = leaves_a_grandchild(&dropped, true);
        let mut child = cmd.stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
        let job = Job::adopt(&child).expect("a job");
        child.wait().unwrap();
        let pid = grandchild(&dropped);
        assert!(alive(pid));
        drop(job);
        wait_until(Duration::from_secs(5), polled(Duration::from_millis(50)), || !alive(pid));
        let left = alive(pid);
        end(pid);
        assert!(!left, "the job's last handle closing ends the tree, as when sm dies");
    }

    /// Every process now: pid, parent pid, exe name.
    fn processes() -> Vec<(u32, u32, String)> {
        use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS};
        let mut found = Vec::new();
        // SAFETY: a snapshot this function opens and closes, and an entry of the size it says.
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).unwrap();
            let mut entry = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
            let mut more = Process32FirstW(snapshot, &mut entry).is_ok();
            while more {
                let name = &entry.szExeFile[..entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len())];
                found.push((entry.th32ProcessID, entry.th32ParentProcessID, String::from_utf16_lossy(name)));
                more = Process32NextW(snapshot, &mut entry).is_ok();
            }
            let _ = windows::Win32::Foundation::CloseHandle(snapshot);
        }
        found
    }

    #[test]
    fn a_grandchild_started_at_once_is_in_the_tree_a_timeout_kills() {
        let me = std::process::id();
        let probe = thread::spawn(move || {
            eventually(Duration::from_secs(30), "no ping under our cmd", polled(Duration::from_millis(10)), || {
                let all = processes();
                let ours = |parent: u32| all.iter().any(|(pid, up, name)| *pid == parent && *up == me && name.eq_ignore_ascii_case("cmd.exe"));
                all.iter().find(|(_, parent, name)| name.eq_ignore_ascii_case("ping.exe") && ours(*parent)).map(|(pid, ..)| *pid)
            })
        });
        let err = run(command("cmd").args(["/C", "ping -n 60 127.0.0.1"]), Duration::from_secs(2), Kill::Tree).unwrap_err();
        assert!(matches!(err, RunError::Timeout(_)), "{err:?}");
        let pid = probe.join().unwrap();
        wait_until(Duration::from_secs(5), polled(Duration::from_millis(50)), || !alive(pid));
        let left = alive(pid);
        end(pid);
        assert!(!left, "the grandchild escaped the job");
    }

    #[test]
    fn a_watched_run_keeps_what_a_killed_child_wrote_and_stops_when_asked() {
        let script = "[Console]::Error.Write('partial'); [Console]::Error.Flush(); Start-Sleep 60";
        let (ended, out) = run_watched(&mut powershell(script), Duration::from_secs(8), Kill::Tree, &mut || false).unwrap();
        assert_eq!((ended, out.code, out.stderr.as_str()), (Ended::TimedOut, None, "partial"));

        let dir = tempfile::tempdir().unwrap();
        let flag = dir.path().join("stop");
        let script = format!("[Console]::Error.Write('partial'); [Console]::Error.Flush(); Set-Content -LiteralPath '{}' x; Start-Sleep 60", flag.display());
        let started_at = Instant::now();
        let (ended, out) = run_watched(&mut powershell(&script), Duration::from_secs(60), Kill::Tree, &mut || flag.exists()).unwrap();
        assert_eq!((ended, out.stderr.as_str()), (Ended::Stopped, "partial"));
        assert!(started_at.elapsed() < Duration::from_secs(30), "{:?}", started_at.elapsed());
    }

    #[test]
    fn a_streamed_child_is_read_as_it_writes_and_dies_with_its_tree_when_dropped() {
        let mut echo = stream(&mut powershell("[Console]::Out.Write([Console]::In.ReadToEnd()); [Console]::Error.Write('said')"), Some(b"in and out".to_vec())).unwrap();
        let mut out = Vec::new();
        while let Some(chunk) = echo.next(Duration::from_secs(30)).unwrap() {
            out.extend(chunk);
        }
        let end = echo.finish(Duration::from_secs(30)).unwrap();
        assert_eq!((out.as_slice(), end.code, end.stderr.as_slice()), (b"in and out".as_slice(), Some(0), b"said".as_slice()));

        let before = spawned_here();
        let mut quiet = stream(&mut powershell("[Console]::Out.Write('x'); [Console]::Out.Flush(); Start-Sleep 60"), None).unwrap();
        assert_eq!(spawned_here(), before + 1);
        assert_eq!(quiet.next(Duration::from_secs(30)).unwrap().unwrap(), b"x");
        let pid = quiet.pid();
        assert!(alive(pid), "the child sleeps, its pipe open");
        drop(quiet);
        assert!(wait_any_exit(&[pid], Duration::from_secs(10)), "dropped, its tree is killed");
    }

    #[test]
    fn a_child_streamed_by_its_stderr_waits_idle_without_dying_and_keeps_its_stdout() {
        let mut progress = stream_pipe(&mut powershell("[Console]::Error.Write('50%'); [Console]::Out.Write('out'); [Console]::Error.Write('100%')"), None, Pipe::Stderr).unwrap();
        let mut rest = Vec::new();
        loop {
            match progress.wait(Duration::from_secs(30)) {
                Waited::Chunk(c) => rest.extend(c),
                Waited::Idle => panic!("the child ends once its stdin closes"),
                Waited::End => break,
            }
        }
        let end = progress.finish(Duration::from_secs(30)).unwrap();
        assert_eq!((rest.as_slice(), end.stdout.as_slice(), end.code), (b"50%100%".as_slice(), b"out".as_slice(), Some(0)));

        let mut held = stream_pipe(&mut powershell("[Console]::Error.Write('x'); [Console]::Error.Flush(); Start-Sleep 60"), None, Pipe::Stderr).unwrap();
        assert_eq!(held.wait(Duration::from_secs(30)), Waited::Chunk(b"x".to_vec()));
        assert_eq!(held.wait(Duration::from_millis(50)), Waited::Idle);
        let pid = held.pid();
        assert!(alive(pid), "an idle wait kills nothing");
        drop(held);
        assert!(wait_any_exit(&[pid], Duration::from_secs(10)), "dropped, its tree is killed");
    }

    fn median_ms(runs: usize, mut once: impl FnMut()) -> f64 {
        let mut took: Vec<f64> = (0..runs)
            .map(|_| {
                let at = Instant::now();
                once();
                at.elapsed().as_secs_f64() * 1000.0
            })
            .collect();
        took.sort_by(f64::total_cmp);
        took[runs / 2]
    }

    #[test]
    fn a_child_that_exits_at_once_never_waits_out_the_pipes() {
        let ours = median_ms(20, || assert!(run_bytes(&mut exits_at_once(), None, Duration::from_secs(30), Kill::Tree).unwrap().success()));
        let settle = PIPES_SETTLE.as_secs_f64() * 1000.0;
        assert!(ours < settle / 2.0, "median {ours:.1} ms through run_bytes, PIPES_SETTLE {settle} ms");
    }

    fn exits_at_once() -> Command {
        let mut cmd = command("cmd");
        cmd.args(["/C", "exit 0"]);
        cmd
    }

    #[test]
    #[ignore = "a measurement: cargo test -p supraman-core --release -- --ignored --nocapture child_overhead"]
    fn child_overhead_measured() {
        let ours = median_ms(20, || assert!(run_bytes(&mut exits_at_once(), None, Duration::from_secs(30), Kill::Tree).unwrap().success()));
        let bare = median_ms(20, || assert!(exits_at_once().output().unwrap().status.success()));
        println!("cmd /C exit 0: run_bytes {ours:.1} ms, bare {bare:.1} ms (median of 20; within 10 ms when idle)");
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
        for args in [&["--version"][..], &["rev-parse", "HEAD"]] {
            let git = || {
                let mut cmd = command("git");
                cmd.args(args).current_dir(repo);
                cmd
            };
            let ours = median_ms(50, || assert!(run_bytes(&mut git(), None, GIT_TIMEOUT, Kill::Tree).unwrap().success()));
            let bare = median_ms(50, || assert!(git().output().unwrap().status.success()));
            println!("git {}: run_bytes {ours:.1} ms, bare {bare:.1} ms (median of 50)", args.join(" "));
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
