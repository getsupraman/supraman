//! Writing into a session's own message pipe, as another session's `SendMessage` does
//! (`docs/peer-messaging.md`). Read-only on `~/.claude/sessions`: Supraman never registers a pipe, a
//! `.json` or a `.key` of its own, so a session cannot answer it there (it answers with `sm tell`).
//!
//! Registry: `<pid>.json` (`name`, `messagingSocketPath`, `peerProtocol`, `procStart` = the process
//! creation FILETIME) and `<pid>.<sha256(canonical pipe)>.key` (`peerToken`). A `.json` outlives its
//! process, so a row counts only while its pid runs with the same start time. One connection per
//! message: an `auth` line, then a `user` line with no `from` and no `from-mode` (claiming a mode is
//! spoofing; with none, a prompting or auto receiver accepts and a bypass one holds).

use std::fmt;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

/// The registry version this module speaks; anything else is [`PeerError::UnknownProtocol`].
pub const PEER_PROTOCOL: i64 = 1;
/// The sender side's own bound (SendMessage uses the same).
pub const DELIVER_TIMEOUT: Duration = Duration::from_secs(5);
/// What the receiver shows as the sender's name; its wrapper is only honoured when it re-renders
/// byte-identical, which `from-name` alone does (measured, m5/4).
pub const FROM_NAME: &str = "supraman";
const CLOSE_TAG: &str = "</cross-session-message>";
/// Under `cfg(test)`, [`deliver`] writes to no other pipe: a test can never reach a live session.
#[cfg(test)]
const TEST_PIPE_PREFIX: &str = r"\\.\pipe\local\cc-msg-supramantest-";

#[derive(Debug, thiserror::Error)]
pub enum PeerError {
    #[error("no session named {name} in {}", .dir.display())]
    NotFound { name: String, dir: PathBuf },
    #[error("{name} is not running (its registry entries are stale: pid {pids:?})")]
    Dead { name: String, pids: Vec<u32> },
    #[error("{name} speaks peer protocol {found:?}, not {PEER_PROTOCOL}")]
    UnknownProtocol { name: String, found: Option<i64> },
    #[error("{name} has no message pipe")]
    NoInbox { name: String },
    #[error("{name} (pid {pid}) published no key file")]
    NoKey { name: String, pid: u32 },
    /// Never carries the file's text or a parser message: either could echo the token.
    #[error("the key file of {name} (pid {pid}) is unreadable")]
    BadKey { name: String, pid: u32 },
    #[error("cannot read {}: {source}", .dir.display())]
    Registry { dir: PathBuf, source: io::Error },
    #[error("cannot write to {pipe}: {source}")]
    Pipe { pipe: String, source: io::Error },
    #[error("{pipe} did not take the message within {}s", DELIVER_TIMEOUT.as_secs())]
    Timeout { pipe: String },
}

/// A live session's inbox. Holds no token: [`deliver`] reads it from `key_file` and drops it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerTarget {
    pub name: String,
    pub pid: u32,
    /// The session's working directory, as it registered it.
    pub cwd: Option<PathBuf>,
    pub pipe: String,
    pub key_file: PathBuf,
}

/// claude's session registry: `<claude_dir>/sessions`.
pub fn sessions_dir() -> Option<PathBuf> {
    supraman_core::paths::claude_dir().map(|d| d.join("sessions"))
}

/// `\\.\pipe\` + the rest of the name lowercased: what the key file's name hashes. Unix socket
/// paths are hashed as they are.
pub fn canonical_pipe(path: &str) -> String {
    let lower = path.to_ascii_lowercase();
    for prefix in [r"\\.\pipe\", r"\\?\pipe\", "//./pipe/", "//?/pipe/"] {
        if lower.starts_with(prefix) {
            return format!(r"\\.\pipe\{}", lower[prefix.len()..].replace('/', r"\"));
        }
    }
    path.to_string()
}

pub fn key_file_name(pid: u32, pipe: &str) -> String {
    let hash = Sha256::digest(canonical_pipe(pipe).as_bytes());
    format!("{pid}.{}.key", supraman_core::hex::encode(&hash))
}

/// One `<pid>.json`, as far as delivery needs it.
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

pub use supraman_core::process::started;

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

/// ListAgents' ref before shortening: sha256 hex of `session:<messagingSocketPath>`, the path as
/// registered (measured on 2.1.312 for 7 live sessions; with the `tengu_session_stable_address` flag on,
/// claude hashes `session:sid:<sessionId>` instead).
pub fn ref_hash(pipe: &str) -> String {
    supraman_core::hex::encode(&Sha256::digest(format!("session:{pipe}").as_bytes()))
}

/// What SendMessage takes for the session `session_id`: its registered name, or `name [ref]` when
/// another session in `live` has the same name. `None` without a live entry, a name or a pipe.
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

/// The live session called `name` in `dir`; the newest when a name was reused.
pub fn find_session(dir: &Path, name: &str, live: &impl Liveness) -> Result<PeerTarget, PeerError> {
    std::fs::read_dir(dir).map_err(|source| PeerError::Registry { dir: dir.to_path_buf(), source })?;
    let named: Vec<RegistryEntry> = read_entries(dir).into_iter().filter(|e| e.name == name).collect();
    if named.is_empty() {
        return Err(PeerError::NotFound { name: name.into(), dir: dir.to_path_buf() });
    }
    let Some(entry) = named
        .iter()
        .filter(|e| live.alive(e.pid, e.proc_start))
        .max_by_key(|e| e.proc_start.unwrap_or(0))
    else {
        let mut pids: Vec<u32> = named.iter().map(|e| e.pid).collect();
        pids.sort_unstable();
        return Err(PeerError::Dead { name: name.into(), pids });
    };
    if entry.protocol != Some(PEER_PROTOCOL) {
        return Err(PeerError::UnknownProtocol { name: name.into(), found: entry.protocol });
    }
    let pipe = entry.pipe.clone().ok_or_else(|| PeerError::NoInbox { name: name.into() })?;
    let key_file = dir.join(key_file_name(entry.pid, &pipe));
    if cfg!(windows) && !key_file.is_file() {
        return Err(PeerError::NoKey { name: name.into(), pid: entry.pid });
    }
    Ok(PeerTarget { name: name.into(), pid: entry.pid, cwd: entry.cwd.clone(), pipe, key_file })
}

/// The receiver's `peerToken`. `Debug` and `Display` never show it.
struct Token(String);

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(<redacted>)")
    }
}

fn read_token(target: &PeerTarget) -> Result<Option<Token>, PeerError> {
    let bad = || PeerError::BadKey { name: target.name.clone(), pid: target.pid };
    let text = match std::fs::read_to_string(&target.key_file) {
        Ok(text) => text,
        // auth is optional off Windows (the receiver checks the peer's credentials instead)
        Err(e) if e.kind() == io::ErrorKind::NotFound && !cfg!(windows) => return Ok(None),
        Err(_) => return Err(bad()),
    };
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|_| bad())?;
    let token = v["peerToken"].as_str().filter(|t| !t.is_empty()).ok_or_else(bad)?;
    Ok(Some(Token(token.to_string())))
}

/// `content` as the receiver shows a message from [`FROM_NAME`]: the wrapper on its own lines, as
/// SendMessage writes it. A body that contains the closing tag goes unwrapped (it could not re-render).
pub fn wrap(body: &str) -> String {
    if body.contains(CLOSE_TAG) || body.contains("<cross-session-message") {
        return body.to_string();
    }
    format!("<cross-session-message from-name=\"{FROM_NAME}\">\n{body}\n{CLOSE_TAG}")
}

/// The `user` line: no `from` (Supraman has no inbox to name), no mode.
pub fn user_line(content: &str, msg_id: &str) -> String {
    serde_json::json!({
        "type": "user",
        "message": { "role": "user", "content": content },
        "msg_id": msg_id,
        "priority": "next",
    })
    .to_string()
}

fn auth_line(token: &Token) -> String {
    serde_json::json!({ "type": "auth", "token": token.0 }).to_string()
}

/// Writes `content` into `target`'s pipe: fire and forget, nothing comes back on the connection. Bounded
/// by [`DELIVER_TIMEOUT`]; a write that hangs is left to its thread.
pub fn deliver(target: &PeerTarget, content: &str) -> Result<(), PeerError> {
    #[cfg(test)]
    if !canonical_pipe(&target.pipe).contains(TEST_PIPE_PREFIX) {
        let refused = io::Error::new(io::ErrorKind::PermissionDenied, format!("a test writes only to {TEST_PIPE_PREFIX}* pipes"));
        return Err(PeerError::Pipe { pipe: target.pipe.clone(), source: refused });
    }
    let token = read_token(target)?;
    let mut wire = String::new();
    if let Some(token) = &token {
        wire.push_str(&auth_line(token));
        wire.push('\n');
    }
    drop(token);
    wire.push_str(&user_line(content, &uuid::Uuid::new_v4().to_string()));
    wire.push('\n');
    let pipe = target.pipe.clone();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(write_pipe(&pipe, wire.as_bytes()));
    });
    match rx.recv_timeout(DELIVER_TIMEOUT) {
        Ok(result) => result.map_err(|source| PeerError::Pipe { pipe: target.pipe.clone(), source }),
        Err(_) => Err(PeerError::Timeout { pipe: target.pipe.clone() }),
    }
}

#[cfg(windows)]
fn write_pipe(pipe: &str, bytes: &[u8]) -> io::Result<()> {
    const ERROR_PIPE_BUSY: i32 = 231;
    let deadline = Instant::now() + DELIVER_TIMEOUT;
    let mut file = loop {
        match std::fs::OpenOptions::new().write(true).open(pipe) {
            Ok(file) => break file,
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return Err(e),
        }
    };
    file.write_all(bytes)?;
    file.flush()
}

#[cfg(unix)]
fn write_pipe(pipe: &str, bytes: &[u8]) -> io::Result<()> {
    let mut stream = std::os::unix::net::UnixStream::connect(pipe)?;
    stream.set_write_timeout(Some(DELIVER_TIMEOUT))?;
    stream.write_all(bytes)?;
    stream.flush()
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
            fs::write(dir.path().join(key_file_name(*pid, PIPE)), r#"{"peerToken":"0123456789abcdef0123456789abcdef","procStartFt":"1","pidDomain":"win32:test"}"#).unwrap();
        }
        dir
    }

    #[test]
    fn the_key_file_name_hashes_the_lowercased_canonical_pipe() {
        assert_eq!(canonical_pipe(PIPE), r"\\.\pipe\local\cc-msg-supramantest-nobody-serves-this");
        assert_eq!(canonical_pipe("//./pipe/LOCAL/cc-msg-AB"), r"\\.\pipe\local\cc-msg-ab");
        assert_eq!(canonical_pipe("/tmp/cc-socks-1000/42.sock"), "/tmp/cc-socks-1000/42.sock");
        let name = key_file_name(4242, PIPE);
        assert_eq!(name.len(), "4242.".len() + 64 + ".key".len());
        assert_eq!(name, key_file_name(4242, &PIPE.to_uppercase().replace("PIPE", "pipe")));
        assert!(name.starts_with("4242.") && name.ends_with(".key"));
    }

    #[test]
    fn the_key_file_name_matches_a_fixed_fixture() {
        // computed independently (Python hashlib) for this exact pipe path
        assert_eq!(
            key_file_name(7, r"\\.\pipe\LOCAL\cc-msg-00000000000000000000000000000000"),
            "7.a5d2f7d7ce03015750f90cfd8ca377c4575300b5798e4d4f4c239a36527cad76.key"
        );
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

    #[test]
    fn find_skips_stale_files_and_a_reused_pid_and_takes_the_newest_live_one() {
        let dir = registry(&[
            (10, entry(10, "fm-a-workspace", Some(1), 100)),
            (11, entry(11, "fm-a-workspace", Some(1), 300)),
            (12, entry(12, "fm-a-workspace", Some(1), 200)),
            (13, entry(13, "other", Some(1), 50)),
        ]);
        fs::write(dir.path().join("14.json"), "{ half").unwrap();
        // 10 dead; 11 is a different process now (start time differs); 12 is the live one
        let live = Fixed(vec![(11, 999), (12, 200), (13, 50)]);
        let t = find_session(dir.path(), "fm-a-workspace", &live).unwrap();
        assert_eq!((t.pid, t.pipe.as_str()), (12, PIPE));
        assert_eq!(t.key_file, dir.path().join(key_file_name(12, PIPE)));
        let both = Fixed(vec![(11, 300), (12, 200)]);
        assert_eq!(find_session(dir.path(), "fm-a-workspace", &both).unwrap().pid, 11, "newest start wins");
    }

    #[test]
    fn find_names_why_it_found_nothing() {
        let dir = registry(&[(10, entry(10, "fm-a-workspace", Some(1), 100)), (20, entry(20, "future", Some(2), 5)), (30, entry(30, "old", None, 5))]);
        let live = Fixed(vec![(20, 5), (30, 5)]);
        assert!(matches!(find_session(dir.path(), "nobody", &live), Err(PeerError::NotFound { .. })));
        match find_session(dir.path(), "fm-a-workspace", &live) {
            Err(PeerError::Dead { pids, .. }) => assert_eq!(pids, [10]),
            other => panic!("{other:?}"),
        }
        assert!(matches!(find_session(dir.path(), "future", &live), Err(PeerError::UnknownProtocol { found: Some(2), .. })));
        assert!(matches!(find_session(dir.path(), "old", &live), Err(PeerError::UnknownProtocol { found: None, .. })));
        let missing = dir.path().join("nope");
        assert!(matches!(find_session(&missing, "x", &live), Err(PeerError::Registry { .. })));
    }

    #[cfg(windows)]
    #[test]
    fn a_missing_or_broken_key_file_is_typed_and_never_echoes_the_file() {
        let dir = registry(&[(10, entry(10, "a", Some(1), 1))]);
        let live = Fixed(vec![(10, 1)]);
        let key = dir.path().join(key_file_name(10, PIPE));
        fs::write(&key, r#"{"peerToken": 12345678901234567890, "secret": "s3cr3t-value"}"#).unwrap();
        let t = find_session(dir.path(), "a", &live).unwrap();
        let err = deliver(&t, "hi").unwrap_err();
        assert!(matches!(err, PeerError::BadKey { .. }), "{err:?}");
        let shown = format!("{err} {err:?}");
        assert!(!shown.contains("s3cr3t") && !shown.contains("1234567890"), "{shown}");
        fs::remove_file(&key).unwrap();
        assert!(matches!(find_session(dir.path(), "a", &live), Err(PeerError::NoKey { pid: 10, .. })));
    }

    #[test]
    fn under_test_a_real_looking_pipe_is_refused_before_anything_is_read_or_written() {
        let dir = tempfile::tempdir().unwrap();
        let real = r"\\.\pipe\LOCAL\cc-msg-0123456789abcdef0123456789abcdef";
        let t = PeerTarget { name: "x".into(), pid: 1, cwd: None, pipe: real.into(), key_file: dir.path().join("missing.key") };
        match deliver(&t, "hi") {
            Err(PeerError::Pipe { source, .. }) => assert_eq!(source.kind(), io::ErrorKind::PermissionDenied, "{source}"),
            other => panic!("{other:?}"),
        }
        let unix = PeerTarget { pipe: "/tmp/cc-socks-1000/42.sock".into(), ..t };
        assert!(matches!(deliver(&unix, "hi"), Err(PeerError::Pipe { .. })));
    }

    /// Serves one connection on a fresh `cc-msg-supramantest-…` pipe and returns what it read.
    #[cfg(windows)]
    fn serve_once(name: &str) -> std::thread::JoinHandle<Vec<u8>> {
        use windows::Win32::Foundation::{CloseHandle, HANDLE};
        use windows::Win32::Storage::FileSystem::{PIPE_ACCESS_INBOUND, ReadFile};
        use windows::Win32::System::Pipes::{ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_WAIT};
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: a pipe this test creates, reads and closes.
        let handle = unsafe {
            CreateNamedPipeW(windows::core::PCWSTR(wide.as_ptr()), PIPE_ACCESS_INBOUND, PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT, 1, 0, 65536, 0, None)
        };
        assert!(!handle.is_invalid(), "CreateNamedPipeW {name}");
        let raw = handle.0 as usize;
        std::thread::spawn(move || {
            let handle = HANDLE(raw as *mut _);
            let mut out = Vec::new();
            // SAFETY: as above.
            unsafe {
                let _ = ConnectNamedPipe(handle, None);
                let mut buf = [0u8; 4096];
                loop {
                    let mut n = 0u32;
                    if ReadFile(handle, Some(&mut buf), Some(&mut n), None).is_err() || n == 0 {
                        break;
                    }
                    out.extend_from_slice(&buf[..n as usize]);
                }
                let _ = CloseHandle(handle);
            }
            out
        })
    }

    #[cfg(windows)]
    #[test]
    fn deliver_writes_the_auth_line_then_the_user_line_into_a_pipe_the_test_serves() {
        let pipe = format!(r"\\.\pipe\LOCAL\cc-msg-supramantest-{}-wire", std::process::id());
        let server = serve_once(&pipe);
        let dir = tempfile::tempdir().unwrap();
        let key_file = dir.path().join(key_file_name(10, &pipe));
        fs::write(&key_file, r#"{"peerToken":"fixturetoken0001","procStartFt":"1"}"#).unwrap();
        let t = PeerTarget { name: "a".into(), pid: 10, cwd: None, pipe: pipe.clone(), key_file };
        deliver(&t, &wrap("From the human (via Supraman, workspace w):\nhi")).unwrap();
        let text = String::from_utf8(server.join().unwrap()).unwrap();
        let lines: Vec<serde_json::Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines.len(), 2, "{text}");
        assert_eq!(lines[0], serde_json::json!({"type": "auth", "token": "fixturetoken0001"}));
        assert_eq!(lines[1]["type"], "user");
        assert!(lines[1].get("from").is_none() && !text.contains("from-mode"), "{text}");
        assert_eq!(lines[1]["message"]["content"], "<cross-session-message from-name=\"supraman\">\nFrom the human (via Supraman, workspace w):\nhi\n</cross-session-message>");
        assert!(text.ends_with('\n'));
    }

    #[cfg(windows)]
    #[test]
    fn a_pipe_nobody_serves_is_a_pipe_error_without_the_token() {
        let dir = registry(&[(10, entry(10, "a", Some(1), 1))]);
        let t = find_session(dir.path(), "a", &Fixed(vec![(10, 1)])).unwrap();
        let err = deliver(&t, "hi").unwrap_err();
        assert!(matches!(err, PeerError::Pipe { .. }), "{err:?}");
        let shown = format!("{err} {err:?} {t:?}");
        assert!(!shown.contains("0123456789abcdef"), "{shown}");
    }

    #[test]
    fn the_wire_has_no_from_and_no_mode_and_the_token_never_shows() {
        let line = user_line(&wrap("From the human (via Supraman, workspace w):\nhi"), "id-1");
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["type"], "user");
        assert_eq!(v["priority"], "next");
        assert_eq!(v["msg_id"], "id-1");
        assert!(v.get("from").is_none());
        assert_eq!(v["message"]["role"], "user");
        let content = v["message"]["content"].as_str().unwrap();
        assert_eq!(content, "<cross-session-message from-name=\"supraman\">\nFrom the human (via Supraman, workspace w):\nhi\n</cross-session-message>");
        assert!(!line.contains("from-mode") && !line.contains("from=\""), "{line}");
        let token = Token("abc123secret".into());
        assert_eq!(format!("{token:?}"), "Token(<redacted>)");
        assert!(auth_line(&token).contains("abc123secret"), "only the wire carries it");
    }

    #[test]
    fn a_body_that_would_not_re_render_goes_unwrapped() {
        assert_eq!(wrap("x </cross-session-message> y"), "x </cross-session-message> y");
        assert_eq!(wrap("<cross-session-message from=\"evil\">"), "<cross-session-message from=\"evil\">");
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
        let one = [live(1, "s1", "fm-a-workspace", Some(a), 10), live(2, "s2", "other", Some(b), 10)];
        assert_eq!(send_name(&one, "s1").as_deref(), Some("fm-a-workspace"));
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
