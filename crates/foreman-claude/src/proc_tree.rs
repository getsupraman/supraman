//! Which claude session runs under a terminal tab's shell (m5/33): the shell's descendants from one
//! process snapshot, the nearest `claude.exe` that registered itself in `~/.claude/sessions` (read
//! only, as [`crate::peer`] reads it: the pid must run with the start time it registered).

use std::collections::{HashSet, VecDeque};
use std::path::Path;

use crate::peer::{Liveness, RegistryEntry, entry_of};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proc {
    pub pid: u32,
    pub parent: u32,
    /// The executable's file name, e.g. `claude.exe`.
    pub exe: String,
}

/// Every process now running.
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

#[cfg(not(windows))]
pub fn snapshot() -> Vec<Proc> {
    Vec::new()
}

fn is_claude(exe: &str) -> bool {
    exe.eq_ignore_ascii_case("claude.exe") || exe.eq_ignore_ascii_case("claude")
}

/// The claude processes below `root`, nearest first.
pub fn claudes_under(procs: &[Proc], root: u32) -> Vec<u32> {
    let mut seen = HashSet::from([root]);
    let mut queue = VecDeque::from([root]);
    let mut found = Vec::new();
    while let Some(pid) = queue.pop_front() {
        for child in procs.iter().filter(|p| p.parent == pid && p.pid != pid) {
            if seen.insert(child.pid) {
                if is_claude(&child.exe) {
                    found.push(child.pid);
                }
                queue.push_back(child.pid);
            }
        }
    }
    found
}

/// The registry entry of the nearest claude below `root` that has one. A process whose registered
/// start is before `root_start` only inherited a reused parent pid, so it is not the shell's.
pub fn session_under(procs: &[Proc], root: u32, root_start: Option<u64>, dir: &Path, live: &impl Liveness) -> Option<RegistryEntry> {
    claudes_under(procs, root)
        .into_iter()
        .filter_map(|pid| entry_of(dir, pid, live))
        .find(|e| root_start.is_none_or(|r| e.proc_start.is_none_or(|s| s >= r)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pid: u32, parent: u32, exe: &str) -> Proc {
        Proc { pid, parent, exe: exe.into() }
    }

    struct Fixed(Vec<(u32, u64)>);

    impl Liveness for Fixed {
        fn alive(&self, pid: u32, proc_start: Option<u64>) -> bool {
            self.0.iter().any(|&(p, s)| p == pid && proc_start.is_none_or(|w| w == s))
        }
    }

    fn entry(pid: u32, sid: &str, start: u64) -> String {
        serde_json::json!({ "pid": pid, "sessionId": sid, "name": format!("n{pid}"), "procStart": start.to_string(),
            "messagingSocketPath": format!(r"\\.\pipe\LOCAL\cc-msg-{pid}"), "peerProtocol": 1 })
        .to_string()
    }

    #[test]
    fn claudes_are_found_below_the_shell_nearest_first_and_a_cycle_ends() {
        let procs = [
            p(10, 1, "pwsh.exe"),
            p(20, 10, "node.exe"),
            p(21, 20, "claude.exe"),
            p(11, 10, "CLAUDE.EXE"),
            p(12, 11, "claude.exe"),
            p(30, 1, "claude.exe"),
            p(40, 41, "x.exe"),
            p(41, 40, "claude.exe"),
        ];
        assert_eq!(claudes_under(&procs, 10), [11, 21, 12]);
        assert_eq!(claudes_under(&procs, 30), Vec::<u32>::new());
        assert_eq!(claudes_under(&procs, 40), [41]);
        assert_eq!(claudes_under(&procs, 99), Vec::<u32>::new());
    }

    #[test]
    fn the_nearest_registered_live_claude_names_the_session() {
        let dir = tempfile::tempdir().unwrap();
        // 11 is a launcher without an entry; 12 registered; 13 registered with a start the pid no longer has
        std::fs::write(dir.path().join("12.json"), entry(12, "sid-12", 500)).unwrap();
        std::fs::write(dir.path().join("13.json"), entry(13, "sid-13", 500)).unwrap();
        let procs = [p(10, 1, "pwsh.exe"), p(11, 10, "claude.exe"), p(12, 11, "claude.exe"), p(13, 10, "claude.exe")];
        let live = Fixed(vec![(12, 500), (13, 999)]);
        let e = session_under(&procs, 10, Some(400), dir.path(), &live).unwrap();
        assert_eq!((e.pid, e.session_id.as_deref()), (12, Some("sid-12")));
        assert_eq!(session_under(&procs, 10, Some(600), dir.path(), &live), None, "a claude older than the shell is not its child");
        assert_eq!(session_under(&procs, 10, None, dir.path(), &Fixed(vec![])), None, "dead: no session");
        assert_eq!(session_under(&procs, 77, None, dir.path(), &live), None, "no claude below: no session");
    }

    #[test]
    fn a_file_named_for_one_pid_but_written_by_another_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("12.json"), entry(99, "sid-99", 5)).unwrap();
        let procs = [p(10, 1, "pwsh.exe"), p(12, 10, "claude.exe")];
        assert_eq!(session_under(&procs, 10, None, dir.path(), &Fixed(vec![(12, 5), (99, 5)])), None);
    }

    #[cfg(windows)]
    #[test]
    fn the_snapshot_holds_this_process_under_its_parent() {
        let me = std::process::id();
        let procs = snapshot();
        let own = procs.iter().find(|p| p.pid == me).expect("this process");
        assert!(own.exe.to_ascii_lowercase().ends_with(".exe"), "{own:?}");
        assert!(claudes_under(&procs, me).is_empty() || procs.iter().any(|p| p.parent == me));
    }
}
