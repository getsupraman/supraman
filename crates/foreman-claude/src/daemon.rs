//! Whether this process can reach claude's background service (m5/25). The service listens on named
//! pipes (`\\.\pipe\cc-daemon-<key>-control`); one started by an elevated `claude --bg` refuses every
//! unelevated client (`ECONNREFUSED`, measured with `runas /trustlevel:0x20000`). A
//! Foreman started from Explorer is unelevated, and so is every `claude` it or its tabs run; each
//! then starts a service of its own, which loses the lock race to the running one, and gives up after
//! 45 s with "Couldn't reach the background service". Nothing a child's flags or console change fixes.

use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

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

/// The Inbox row's text when an unelevated Foreman faces a running service it cannot reach.
pub fn elevation_mismatch(elevated: bool, reach: &DaemonReach) -> Option<String> {
    match reach {
        DaemonReach::Unreachable { pid } if !elevated => Some(format!(
            "The claude background service (pid {pid}) runs as administrator and Foreman does not, so Windows keeps Foreman's claude sessions from reaching it. Turn on Settings → Run Foreman as administrator, or start Foreman from an elevated shell, or let the service end (close the elevated claude --bg sessions) so the next one starts unelevated; then Retry."
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
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let out = crate::no_window(&mut Command::new("claude"))
            .args(["daemon", "status"])
            .stdin(Stdio::null())
            .output();
        let _ = tx.send(out);
    });
    match rx.recv_timeout(STATUS_TIMEOUT) {
        Ok(Ok(out)) => parse_daemon_status(&format!(
            "{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )),
        _ => DaemonReach::Unknown,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 2.1.311 from an elevated shell, verbatim but for the paths.
    const REACHABLE: &str = "pid:     58604\nversion: 2.1.311\nuptime:  43723s\n\nbg sessions:\n  sock dir:     \\\\.\\pipe\\cc-daemon-*\n  control.sock: reachable\n  bg workers:   5 running (control.sock), 5 in roster.json\n";
    /// The same service from a Medium-IL process (`runas /trustlevel:0x20000`), m5/25.
    const REFUSED: &str = "pid:     58604\nversion: 2.1.311\nuptime:  43784s\n\nbg sessions:\n  sock dir:     \\\\.\\pipe\\cc-daemon-*\n  control.sock: unreachable (connect ECONNREFUSED \\\\.\\pipe\\cc-daemon-*-control)\n  bg workers:   4 in roster.json (control unreachable)\n";

    #[test]
    fn daemon_status_is_read_from_the_cli_output() {
        assert_eq!(parse_daemon_status(REACHABLE), DaemonReach::Reachable);
        assert_eq!(parse_daemon_status(REFUSED), DaemonReach::Unreachable { pid: 58604 });
        assert_eq!(parse_daemon_status("daemon: not running\n"), DaemonReach::NotRunning);
        assert_eq!(parse_daemon_status("something else"), DaemonReach::Unknown);
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

    /// The real thing (m5/25): from an elevated test process that reaches a running service, the same
    /// `claude daemon status` run at Medium IL (`runas /trustlevel:0x20000`, as Explorer would start
    /// Foreman) is refused, and [`elevation_mismatch`] names it. Skipped unless elevated with a
    /// reachable service that was itself started elevated; read-only, it starts and stops nothing.
    #[cfg(windows)]
    #[test]
    fn a_medium_il_process_cannot_reach_an_elevated_service() {
        if !is_elevated() || daemon_status() != DaemonReach::Reachable {
            eprintln!("skipped: needs an elevated test process and a running service it reaches");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("status.txt");
        let script = dir.path().join("status.cmd");
        std::fs::write(&script, format!("@claude daemon status > \"{}\" 2>&1\r\n", out.display())).unwrap();
        let ran = Command::new("runas")
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
