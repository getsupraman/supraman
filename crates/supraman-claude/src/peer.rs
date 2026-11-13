//! claude's live session registry (`~/.claude/sessions`), read only: who runs, and the name SendMessage takes for each.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// claude's session registry: `<claude_dir>/sessions`.
pub fn sessions_dir() -> Option<PathBuf> {
    supraman_core::paths::claude_dir().map(|d| d.join("sessions"))
}

/// One `<pid>.json`, as far as Supraman reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryEntry {
    pub pid: u32,
    /// `sessionId`: the transcript's id, which `/clear` changes.
    pub session_id: Option<String>,
    pub name: String,
    pub cwd: Option<PathBuf>,
    pub pipe: Option<String>,
    pub protocol: Option<i64>,
    /// The process creation time (Windows FILETIME), as `procStart` writes it.
    pub proc_start: Option<u64>,
}

pub fn parse_entry(text: &str) -> Option<RegistryEntry> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    let proc_start = match &v["procStart"] {
        serde_json::Value::String(s) => s.parse().ok(),
        n => n.as_u64(),
    };
    Some(RegistryEntry {
        pid: u32::try_from(v["pid"].as_u64()?).ok()?,
        session_id: v["sessionId"].as_str().filter(|s| !s.is_empty()).map(String::from),
        name: v["name"].as_str()?.to_string(),
        cwd: v["cwd"].as_str().map(PathBuf::from),
        pipe: v["messagingSocketPath"].as_str().filter(|s| !s.is_empty()).map(String::from),
        protocol: v["peerProtocol"].as_i64(),
        proc_start,
    })
}

/// Whether `pid` runs and was created at `proc_start`.
pub trait Liveness {
    fn alive(&self, pid: u32, proc_start: Option<u64>) -> bool;
}

pub struct System;

impl Liveness for System {
    fn alive(&self, pid: u32, proc_start: Option<u64>) -> bool {
        started(pid).is_some_and(|start| proc_start.is_none_or(|want| want == start))
    }
}

pub use supraman_core::platform::process::started;

fn read_entries(dir: &Path) -> Vec<RegistryEntry> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".json"))
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .filter_map(|text| parse_entry(&text))
        .collect()
}

/// Every entry in `dir` whose pid runs with the start time it registered.
pub fn live_entries(dir: &Path, live: &impl Liveness) -> Vec<RegistryEntry> {
    read_entries(dir).into_iter().filter(|e| live.alive(e.pid, e.proc_start)).collect()
}

/// `<pid>.json`, while `pid` is still the process that wrote it.
pub fn entry_of(dir: &Path, pid: u32, live: &impl Liveness) -> Option<RegistryEntry> {
    let text = std::fs::read_to_string(dir.join(format!("{pid}.json"))).ok()?;
    parse_entry(&text).filter(|e| e.pid == pid && live.alive(pid, e.proc_start))
}

/// The shortest `[ref]` ListAgents prints: 6 hex digits, longer only where two refs share them.
const REF_MIN: usize = 6;

/// ListAgents' ref before shortening: sha256 hex of `session:<messagingSocketPath>` as registered (measured on 2.1.312).
pub fn ref_hash(pipe: &str) -> String {
    supraman_core::hex::encode(&Sha256::digest(format!("session:{pipe}").as_bytes()))
}

/// What SendMessage takes for `session_id`: its registered name, or `name [ref]` when another live session shares it.
pub fn send_name(live: &[RegistryEntry], session_id: &str) -> Option<String> {
    let listed: Vec<&RegistryEntry> = live.iter().filter(|e| e.pipe.is_some() && !e.name.trim().is_empty()).collect();
    let own = listed.iter().filter(|e| e.session_id.as_deref() == Some(session_id)).max_by_key(|e| e.proc_start.unwrap_or(0))?;
    let name = own.name.trim();
    let key = name.to_lowercase();
    if listed.iter().filter(|e| e.name.trim().to_lowercase() == key).count() < 2 {
        return Some(name.to_string());
    }
    let mut refs: Vec<String> = listed.iter().filter_map(|e| e.pipe.as_deref()).map(ref_hash).collect();
    refs.sort_unstable();
    refs.dedup();
    let mine = ref_hash(own.pipe.as_deref()?);
    let at = refs.binary_search(&mine).ok()?;
    let common = |a: &str, b: &str| a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count();
    let shared = [at.checked_sub(1), Some(at + 1)].into_iter().flatten().filter_map(|i| refs.get(i)).map(|r| common(r, &mine)).max().unwrap_or(0);
    Some(format!("{name} [{}]", &mine[..mine.len().min(REF_MIN.max(shared + 1))]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Live: the pids in the set, each with the start time it maps to.
    struct Fixed(Vec<(u32, u64)>);

    impl Liveness for Fixed {
        fn alive(&self, pid: u32, proc_start: Option<u64>) -> bool {
            self.0.iter().any(|&(p, s)| p == pid && proc_start.is_none_or(|w| w == s))
        }
    }

    const PIPE: &str = r"\\.\pipe\LOCAL\cc-msg-supramantest-nobody-serves-this";

    fn entry(pid: u32, name: &str, protocol: Option<i64>, start: u64) -> String {
        let mut v = serde_json::json!({
            "pid": pid, "sessionId": "x", "name": name, "kind": "bg", "messagingSocketPath": PIPE,
            "procStart": start.to_string(), "pidDomain": "win32:test", "version": "2.1.310",
        });
        if let Some(p) = protocol {
            v["peerProtocol"] = p.into();
        }
        v.to_string()
    }

    fn registry(entries: &[(u32, String)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (pid, text) in entries {
            fs::write(dir.path().join(format!("{pid}.json")), text).unwrap();
        }
        dir
    }

    #[test]
    fn a_registry_entry_reads_procstart_as_string_or_number() {
        let e = parse_entry(&entry(4242, "eval", Some(1), 134369700000000000)).unwrap();
        assert_eq!(e.proc_start, Some(134369700000000000));
        assert_eq!(e.protocol, Some(1));
        assert_eq!(e.pipe.as_deref(), Some(PIPE));
        let n = parse_entry(r#"{"pid": 5, "name": "a", "procStart": 9}"#).unwrap();
        assert_eq!((n.proc_start, n.protocol, n.pipe), (Some(9), None, None));
        assert_eq!(parse_entry(r#"{"pid": "5", "name": "a"}"#), None);
        assert_eq!(parse_entry("not json"), None);
    }

    fn live(pid: u32, sid: &str, name: &str, pipe: Option<&str>, start: u64) -> RegistryEntry {
        RegistryEntry {
            pid,
            session_id: Some(sid.into()),
            name: name.into(),
            cwd: None,
            pipe: pipe.map(String::from),
            protocol: Some(1),
            proc_start: Some(start),
        }
    }

    #[test]
    fn the_ref_hashes_the_registered_pipe_as_listagents_does() {
        // computed independently (Python hashlib, sha256 of "session:" + the path)
        assert_eq!(ref_hash(r"\\.\pipe\LOCAL\cc-msg-00000000000000000000000000000000"), "ef3ec927eeae638bc2d16c82cf894cdc17c6c267bd3726b4b5ad90bf46a826eb");
    }

    #[test]
    fn the_send_name_is_the_registered_name_and_takes_a_ref_only_when_another_live_session_shares_it() {
        let (a, b) = (r"\\.\pipe\LOCAL\cc-msg-00000000000000000000000000000000", r"\\.\pipe\LOCAL\cc-msg-11111111111111111111111111111111");
        let one = [live(1, "s1", "fm-a-tab", Some(a), 10), live(2, "s2", "other", Some(b), 10)];
        assert_eq!(send_name(&one, "s1").as_deref(), Some("fm-a-tab"));
        assert_eq!(send_name(&one, "nobody"), None);
        let twins = [live(1, "s1", " Twin ", Some(a), 10), live(2, "s2", "twin", Some(b), 10)];
        assert_eq!(send_name(&twins, "s1").as_deref(), Some("Twin [ef3ec9]"));
        assert_eq!(send_name(&twins, "s2").as_deref(), Some("twin [d923d7]"));
        // a twin without a pipe is not listed, so it makes no ambiguity; an own entry without one has no name
        let unlisted = [live(1, "s1", "twin", Some(a), 10), live(2, "s2", "twin", None, 10)];
        assert_eq!(send_name(&unlisted, "s1").as_deref(), Some("twin"));
        assert_eq!(send_name(&unlisted, "s2"), None);
    }

    #[test]
    fn a_ref_grows_past_six_digits_where_two_refs_share_them() {
        // both hash to 92bdcb…: 92bdcb95… and 92bdcb2f…
        let (a, b) = (r"\\.\pipe\LOCAL\cc-msg-supramantest-000455", r"\\.\pipe\LOCAL\cc-msg-supramantest-006484");
        assert_eq!(&ref_hash(a)[..7], "92bdcb9");
        let rows = [live(1, "s1", "x", Some(a), 10), live(2, "s2", "x", Some(b), 10)];
        assert_eq!(send_name(&rows, "s1").as_deref(), Some("x [92bdcb9]"));
        assert_eq!(send_name(&rows, "s2").as_deref(), Some("x [92bdcb2]"));
    }

    #[test]
    fn live_entries_and_entry_of_check_the_pid_and_its_start() {
        let dir = registry(&[(10, entry(10, "a", Some(1), 100)), (11, entry(11, "b", Some(1), 200))]);
        let live = Fixed(vec![(10, 100), (11, 999)]);
        let found: Vec<u32> = live_entries(dir.path(), &live).iter().map(|e| e.pid).collect();
        assert_eq!(found, [10]);
        assert_eq!(entry_of(dir.path(), 10, &live).map(|e| e.session_id), Some(Some("x".into())));
        assert_eq!(entry_of(dir.path(), 11, &live), None, "pid reused");
        assert_eq!(entry_of(dir.path(), 12, &live), None);
        assert!(live_entries(&dir.path().join("nope"), &live).is_empty());
    }

    #[test]
    fn the_live_system_knows_this_process_and_not_a_wrong_start_time() {
        let pid = std::process::id();
        assert!(System.alive(pid, None));
        if cfg!(windows) {
            let start = started(pid).unwrap();
            assert!(System.alive(pid, Some(start)));
            assert!(!System.alive(pid, Some(start + 1)), "same pid, other process");
        }
        assert!(!System.alive(u32::MAX - 1, None));
    }
}
