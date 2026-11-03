//! Whether this process can reach claude's background service: one an elevated `claude --bg` started
//! refuses every unelevated client, whose own `--bg` then gives up after 45 s. Facts: LEARNINGS m5/25.

use std::time::Duration;

use supraman_core::process::{self, Kill};

/// What `claude` prints when its `--bg` cannot connect to the service (2.1.311).
pub const UNREACHABLE: &str = "Couldn't reach the background service";

const STATUS_TIMEOUT: Duration = Duration::from_secs(10);

/// What `claude daemon status` says about the service, as seen from this process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DaemonReach {
    Reachable,
    /// A service runs (its pid is known) but its control pipe refuses this process.
    Unreachable { pid: u32 },
    /// No service runs: the next `claude --bg` starts one at this process's level, which everyone reaches.
    NotRunning,
    Unknown,
}

/// Reads `claude daemon status` output: `pid:     58604` and `control.sock: reachable` /
/// `control.sock: unreachable (connect ECONNREFUSED …)` (2.1.311).
pub fn parse_daemon_status(text: &str) -> DaemonReach {
    let field = |name: &str| {
        text.lines().find_map(|l| l.trim().strip_prefix(name).map(|rest| rest.trim().to_string()))
    };
    let pid = field("pid:").and_then(|p| p.parse::<u32>().ok());
    match (pid, field("control.sock:")) {
        (Some(_), Some(sock)) if sock.starts_with("reachable") => DaemonReach::Reachable,
        (Some(pid), Some(sock)) if sock.starts_with("unreachable") => DaemonReach::Unreachable { pid },
        (None, _) if text.to_lowercase().contains("not running") => DaemonReach::NotRunning,
        _ => DaemonReach::Unknown,
    }
}

/// The Inbox row's text when an unelevated Supraman faces a running service it cannot reach.
pub fn elevation_mismatch(elevated: bool, reach: &DaemonReach) -> Option<String> {
    match reach {
        DaemonReach::Unreachable { pid } if !elevated => Some(format!(
            "The claude background service (pid {pid}) runs as administrator and Supraman does not, so Windows keeps Supraman's claude sessions from reaching it. Turn on Settings → Run Supraman as administrator, or start Supraman from an elevated shell, or let the service end (close the elevated claude --bg sessions) so the next one starts unelevated; then Retry."
        )),
        _ => None,
    }
}

/// [`elevation_mismatch`] for this process. An elevated process reaches every service, so it runs
/// nothing; otherwise one `claude daemon status`.
pub fn daemon_mismatch() -> Option<String> {
    let elevated = is_elevated();
    if elevated {
        return None;
    }
    elevation_mismatch(elevated, &daemon_status())
}

/// One `claude daemon status`, read-only: it never starts or stops the service.
pub fn daemon_status() -> DaemonReach {
    match process::run(process::command("claude").args(["daemon", "status"]), STATUS_TIMEOUT, Kill::Child) {
        Ok(out) => parse_daemon_status(&format!("{}\n{}", out.stdout, out.stderr)),
        Err(_) => DaemonReach::Unknown,
    }
}

/// Whether this process runs elevated (UAC's full administrator token).
#[cfg(windows)]
pub fn is_elevated() -> bool {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    let mut token = HANDLE::default();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }.is_err() {
        return false;
    }
    let mut elevation = TOKEN_ELEVATION::default();
    let mut len = 0u32;
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut _),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        )
    };
    unsafe {
        let _ = CloseHandle(token);
    }
    ok.is_ok() && elevation.TokenIsElevated != 0
}

#[cfg(not(windows))]
pub fn is_elevated() -> bool {
    true
}

/// The service's pid from `<claude_dir>/daemon/roster.json` (`supervisorPid`), when it runs. Read from the
/// file because `claude daemon status` reads `NotRunning` from an unelevated process facing an elevated
/// service (docs/research/scheduler.md, probe (a)).
pub fn service_pid() -> Option<u32> {
    let roster = supraman_core::paths::claude_dir()?.join("daemon").join("roster.json");
    service_pid_in(&std::fs::read_to_string(roster).ok()?).filter(|pid| process::alive(*pid))
}

fn service_pid_in(roster: &str) -> Option<u32> {
    let v: serde_json::Value = serde_json::from_str(roster).ok()?;
    v.get("supervisorPid")?.as_u64().and_then(|p| u32::try_from(p).ok())
}

/// Whether the running service has an elevated token; `None` when none runs or its token cannot be read.
pub fn service_elevated() -> Option<bool> {
    pid_elevated(service_pid()?)
}

/// A scheduled task's run level: `HighestAvailable` while the service is elevated (Decision 5 of
/// docs/research/scheduler.md); with no service the task's own `claude --bg` starts one at its level.
pub fn task_run_level() -> supraman_core::schedule::RunLevel {
    supraman_core::schedule::RunLevel::for_service(service_elevated().unwrap_or(false))
}

#[cfg(windows)]
pub fn pid_elevated(pid: u32) -> Option<bool> {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
    use windows::Win32::System::Threading::{OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION};
    // SAFETY: plain Win32 calls on handles this function opens and closes.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut token = HANDLE::default();
        let opened = OpenProcessToken(process, TOKEN_QUERY, &mut token);
        let _ = CloseHandle(process);
        opened.ok()?;
        let mut elevation = TOKEN_ELEVATION::default();
        let mut len = 0u32;
        let read = GetTokenInformation(token, TokenElevation, Some(&mut elevation as *mut _ as *mut _), size_of::<TOKEN_ELEVATION>() as u32, &mut len);
        let _ = CloseHandle(token);
        read.ok()?;
        Some(elevation.TokenIsElevated != 0)
    }
}

#[cfg(not(windows))]
pub fn pid_elevated(_pid: u32) -> Option<bool> {
    None
}

#[cfg(test)]
mod tests {
    use std::process::Stdio;
    use std::thread;

    use super::*;

    /// 2.1.311 from an elevated shell, verbatim but for the paths.
    const REACHABLE: &str = "pid:     58604\nversion: 2.1.311\nuptime:  43723s\n\nbg sessions:\n  sock dir:     \\\\.\\pipe\\cc-daemon-*\n  control.sock: reachable\n  bg workers:   5 running (control.sock), 5 in roster.json\n";
    /// The same service from a Medium-IL process (`runas /trustlevel:0x20000`).
    const REFUSED: &str = "pid:     58604\nversion: 2.1.311\nuptime:  43784s\n\nbg sessions:\n  sock dir:     \\\\.\\pipe\\cc-daemon-*\n  control.sock: unreachable (connect ECONNREFUSED \\\\.\\pipe\\cc-daemon-*-control)\n  bg workers:   4 in roster.json (control unreachable)\n";

    #[test]
    fn daemon_status_is_read_from_the_cli_output() {
        assert_eq!(parse_daemon_status(REACHABLE), DaemonReach::Reachable);
        assert_eq!(parse_daemon_status(REFUSED), DaemonReach::Unreachable { pid: 58604 });
        assert_eq!(parse_daemon_status("daemon: not running\n"), DaemonReach::NotRunning);
        assert_eq!(parse_daemon_status("something else"), DaemonReach::Unknown);
    }

    #[test]
    fn the_service_pid_is_the_rosters_supervisor_pid() {
        assert_eq!(service_pid_in(r#"{"proto":1,"supervisorPid":49204,"workers":{}}"#), Some(49204));
        assert_eq!(service_pid_in(r#"{"proto":1,"workers":{}}"#), None);
        assert_eq!(service_pid_in("not json"), None);
    }

    #[cfg(windows)]
    #[test]
    fn this_process_reads_as_elevated_exactly_when_is_elevated_says_so() {
        assert_eq!(pid_elevated(std::process::id()), Some(is_elevated()));
        assert_eq!(pid_elevated(u32::MAX - 1), None);
    }

    #[test]
    fn only_an_unelevated_process_facing_a_refusing_service_is_a_mismatch() {
        let refused = parse_daemon_status(REFUSED);
        let why = elevation_mismatch(false, &refused).expect("unelevated + refused");
        assert!(why.contains("pid 58604") && why.contains("administrator") && why.contains("Retry"), "{why}");
        assert_eq!(elevation_mismatch(true, &refused), None);
        for reach in [DaemonReach::Reachable, DaemonReach::NotRunning, DaemonReach::Unknown] {
            assert_eq!(elevation_mismatch(false, &reach), None, "{reach:?}");
        }
    }

    /// The real thing: from an elevated test process that reaches a running service, the same
    /// `claude daemon status` run at Medium IL (`runas /trustlevel:0x20000`, as Explorer would start
    /// Supraman) is refused, and [`elevation_mismatch`] names it. Skipped unless elevated with a
    /// reachable service that was itself started elevated; read-only, it starts and stops nothing.
    #[cfg(windows)]
    #[test]
    #[ignore = "runas opens a console window of its own; run elevated with --ignored"]
    fn a_medium_il_process_cannot_reach_an_elevated_service() {
        if !is_elevated() || daemon_status() != DaemonReach::Reachable {
            eprintln!("skipped: needs an elevated test process and a running service it reaches");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("status.txt");
        let script = dir.path().join("status.cmd");
        std::fs::write(&script, format!("@claude daemon status > \"{}\" 2>&1\r\n", out.display())).unwrap();
        let ran = supraman_core::process::command("runas")
            .arg("/trustlevel:0x20000")
            .arg(format!("cmd /c \"{}\"", script.display()))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(ran.success(), "runas: {ran}");
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        let text = loop {
            let text = std::fs::read_to_string(&out).unwrap_or_default();
            if text.contains("control.sock") || std::time::Instant::now() > deadline {
                break text;
            }
            thread::sleep(Duration::from_millis(200));
        };
        let reach = parse_daemon_status(&text);
        if reach == DaemonReach::Reachable {
            eprintln!("skipped: the running service is unelevated, so Medium reaches it too");
            return;
        }
        assert!(matches!(reach, DaemonReach::Unreachable { .. }), "{text}");
        assert!(elevation_mismatch(false, &reach).is_some());
    }
}
