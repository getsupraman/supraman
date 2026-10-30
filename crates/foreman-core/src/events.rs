//! `<FOREMAN_HOME>/events/<session_id>.jsonl`, appended by `fm hook`; payload facts in LEARNINGS m0/3, m2/2.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{ErrorKind, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::fs::rotated;

pub const ROTATE_AT: u64 = 256 * 1024;

/// The fixed subset of a hook payload Foreman keeps. Never the prompt or the assistant's text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    /// Unix epoch milliseconds, when `fm hook` ran.
    pub ts: i64,
    pub event: String,
    pub session_id: String,
    /// `session_title`, the `--name` the session was launched with.
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notification_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<String>,
    #[serde(default)]
    pub cwd: String,
    /// The session's transcript (`.jsonl`); where its replies can be read back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript_path: Option<String>,
}

impl Event {
    /// `None` when the payload is not a JSON object or has no usable `session_id`.
    pub fn from_payload(event: &str, payload: &str, ts: i64) -> Option<Self> {
        let v: serde_json::Value = serde_json::from_str(payload).ok()?;
        let text = |k: &str| v.get(k).and_then(serde_json::Value::as_str).map(str::to_string);
        let session_id = text("session_id").filter(|s| is_safe_id(s))?;
        Some(Self {
            ts,
            event: event.to_string(),
            session_id,
            name: text("session_title").unwrap_or_default(),
            source: text("source"),
            notification_type: text("notification_type"),
            trigger: text("trigger"),
            cwd: text("cwd").unwrap_or_default(),
            transcript_path: text("transcript_path"),
        })
    }
}

/// Session ids become file names: UUIDs in practice, so letters, digits, `-` and `_` only.
pub fn is_safe_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

pub fn events_dir(home: &Path) -> PathBuf {
    home.join("events")
}

pub fn log_path(home: &Path, session_id: &str) -> PathBuf {
    events_dir(home).join(format!("{session_id}.jsonl"))
}

pub fn append(home: &Path, event: &Event, rotate_at: u64) -> std::io::Result<PathBuf> {
    let path = log_path(home, &event.session_id);
    fs::create_dir_all(events_dir(home))?;
    let mut line = serde_json::to_vec(event).map_err(std::io::Error::other)?;
    line.push(b'\n');
    crate::fs::append_capped(&path, &line, rotate_at)?;
    Ok(path)
}

/// What a SessionStart hook prints: a tag line `FOREMAN-ROLE <role> v<sha8>` and the role text from
/// `<home>/roles/<role>.md`, with `{workspace}` replaced by `workspace` when given (one role file serves
/// every workspace). The tag changes only with the text, so a repeated injection (every `--resume`
/// and compaction fires SessionStart) is recognisable. `None` when the role name is unsafe or the
/// file is missing or empty.
pub fn role_injection(home: &Path, role: &str, workspace: Option<&str>) -> Option<String> {
    role_injection_for(home, role, workspace, None)
}

/// [`role_injection`] for a repo session: `{repo}` too, from `FOREMAN_REPO`.
pub fn role_injection_for(home: &Path, role: &str, workspace: Option<&str>, repo: Option<&str>) -> Option<String> {
    if !is_safe_id(role) {
        return None;
    }
    let mut text = fs::read_to_string(home.join("roles").join(format!("{role}.md"))).ok()?;
    if let Some(repo) = repo {
        text = text.replace("{repo}", repo);
    }
    if let Some(workspace) = workspace {
        text = text.replace("{workspace}", workspace);
    }
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    Some(format!("{}\n{text}\n", role_tag(role, text)))
}

pub fn role_tag(role: &str, text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    let sha8: String = digest.iter().take(4).map(|b| format!("{b:02x}")).collect();
    format!("FOREMAN-ROLE {role} v{sha8}")
}

/// Where the reader is in one session's log. `head` is the log's first line as last seen: a
/// different first line at the same path means the log was rotated under us.
#[derive(Debug, Clone, Default)]
struct Cursor {
    offset: u64,
    head: Vec<u8>,
}

/// Incremental reader over `<home>/events/*.jsonl`. Each [`EventReader::poll`] returns the
/// complete lines appended since the last one; a partial last line waits for its newline. A
/// rotation between polls is followed: the rest of `.1` first, then the new log. Two rotations
/// between polls lose the middle file. Lines that do not parse are skipped.
#[derive(Debug, Clone)]
pub struct EventReader {
    dir: PathBuf,
    cursors: HashMap<String, Cursor>,
}

impl EventReader {
    pub fn new(home: &Path) -> Self {
        Self {
            dir: events_dir(home),
            cursors: HashMap::new(),
        }
    }

    /// New events across all sessions, in file order per session; sessions in name order.
    pub fn poll(&mut self) -> Vec<Event> {
        let mut ids: Vec<String> = match fs::read_dir(&self.dir) {
            Ok(entries) => entries
                .flatten()
                .filter_map(|e| e.file_name().to_str()?.strip_suffix(".jsonl").map(str::to_string))
                .filter(|id| is_safe_id(id))
                .collect(),
            Err(_) => Vec::new(),
        };
        ids.sort();
        self.cursors.retain(|id, _| ids.binary_search(id).is_ok());
        ids.into_iter().flat_map(|id| self.poll_session(&id)).collect()
    }

    pub fn poll_session(&mut self, session_id: &str) -> Vec<Event> {
        let path = self.dir.join(format!("{session_id}.jsonl"));
        let mut cursor = self.cursors.remove(session_id).unwrap_or_default();
        let mut out = Vec::new();
        let Ok(mut file) = File::open(&path) else {
            self.cursors.insert(session_id.to_string(), cursor);
            return out;
        };
        let len = file.metadata().map(|m| m.len()).unwrap_or(0);
        let head = first_line(&mut file);
        if cursor.offset > 0 && (len < cursor.offset || head != cursor.head) {
            if let Ok(mut old) = File::open(rotated(&path))
                && first_line(&mut old) == cursor.head
            {
                read_lines(&mut old, &mut cursor.offset, &mut out);
            }
            cursor.offset = 0;
        }
        read_lines(&mut file, &mut cursor.offset, &mut out);
        cursor.head = head;
        self.cursors.insert(session_id.to_string(), cursor);
        out
    }
}

/// The first line without its newline, up to 512 bytes; empty if there is no complete one yet.
fn first_line(file: &mut File) -> Vec<u8> {
    let mut buf = vec![0u8; 512];
    let n = file.seek(SeekFrom::Start(0)).and_then(|_| read_up_to(file, &mut buf)).unwrap_or(0);
    buf.truncate(n);
    match buf.iter().position(|&b| b == b'\n') {
        Some(end) => buf[..end].to_vec(),
        None if n == 512 => buf,
        None => Vec::new(),
    }
}

fn read_up_to(file: &mut File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match file.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(n)
}

/// Reads complete lines from `offset`, advancing it past the last newline only.
fn read_lines(file: &mut File, offset: &mut u64, out: &mut Vec<Event>) {
    let mut buf = Vec::new();
    if file.seek(SeekFrom::Start(*offset)).is_err() || file.read_to_end(&mut buf).is_err() {
        return;
    }
    let Some(end) = buf.iter().rposition(|&b| b == b'\n') else {
        return;
    };
    for line in buf[..end].split(|&b| b == b'\n') {
        if let Ok(event) = serde_json::from_slice::<Event>(line.strip_suffix(b"\r").unwrap_or(line)) {
            out.push(event);
        }
    }
    *offset += end as u64 + 1;
}

#[cfg(test)]
mod tests {
    use std::fs::OpenOptions;
    use std::io::Write;

    use super::*;

    /// m0/3's SessionStart(compact) payload shape, values made up.
    const COMPACT: &str = r#"{"cwd":"C:\\w","hook_event_name":"SessionStart","model":"m","prompt_id":"p","session_id":"0a1b2c3d-0000-4000-8000-000000000001","session_title":"aw-x","source":"compact","transcript_path":"t"}"#;
    const STOP: &str = r#"{"session_id":"0a1b2c3d-0000-4000-8000-000000000001","hook_event_name":"Stop","cwd":"C:\\w","last_assistant_message":"SECRET-REPLY","prompt":"SECRET-PROMPT","stop_hook_active":false}"#;
    const SID: &str = "0a1b2c3d-0000-4000-8000-000000000001";

    fn ev(n: i64) -> Event {
        Event::from_payload("Stop", STOP, n).unwrap()
    }

    #[test]
    fn keeps_the_fixed_subset_only() {
        let e = Event::from_payload("SessionStart", COMPACT, 7).unwrap();
        assert_eq!(e.session_id, SID);
        assert_eq!(e.name, "aw-x");
        assert_eq!(e.source.as_deref(), Some("compact"));
        assert_eq!(e.cwd, r"C:\w");
        assert_eq!(e.transcript_path.as_deref(), Some("t"));
        let line = serde_json::to_string(&ev(1)).unwrap();
        assert!(!line.contains("SECRET"), "{line}");
        assert!(!line.contains("source") && !line.contains("trigger"), "{line}");
        let n = Event::from_payload(
            "Notification",
            r#"{"session_id":"s1","notification_type":"permission_prompt","message":"m"}"#,
            1,
        )
        .unwrap();
        assert_eq!(n.notification_type.as_deref(), Some("permission_prompt"));
        let p = Event::from_payload("PreCompact", r#"{"session_id":"s1","trigger":"auto"}"#, 1).unwrap();
        assert_eq!(p.trigger.as_deref(), Some("auto"));
    }

    #[test]
    fn rejects_bad_payloads_and_unsafe_ids() {
        assert!(Event::from_payload("Stop", "", 1).is_none());
        assert!(Event::from_payload("Stop", "not json", 1).is_none());
        assert!(Event::from_payload("Stop", "[1]", 1).is_none());
        assert!(Event::from_payload("Stop", r#"{"cwd":"x"}"#, 1).is_none());
        assert!(Event::from_payload("Stop", r#"{"session_id":"..\\..\\x"}"#, 1).is_none());
        assert!(Event::from_payload("Stop", r#"{"session_id":""}"#, 1).is_none());
    }

    #[test]
    fn appends_one_line_per_event_and_rotates_at_the_cap() {
        let home = tempfile::tempdir().unwrap();
        let path = append(home.path(), &ev(1), 1 << 20).unwrap();
        append(home.path(), &ev(2), 1 << 20).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap().lines().count(), 2);
        let size = fs::metadata(&path).unwrap().len();
        append(home.path(), &ev(3), size).unwrap();
        assert_eq!(fs::read_to_string(rotated(&path)).unwrap().lines().count(), 2);
        assert_eq!(fs::read_to_string(&path).unwrap().lines().count(), 1);
    }

    #[test]
    fn reader_is_incremental_and_waits_for_a_partial_line() {
        let home = tempfile::tempdir().unwrap();
        let mut reader = EventReader::new(home.path());
        assert!(reader.poll().is_empty());
        let path = append(home.path(), &ev(1), ROTATE_AT).unwrap();
        assert_eq!(reader.poll().iter().map(|e| e.ts).collect::<Vec<_>>(), [1]);
        assert!(reader.poll().is_empty());

        let line = serde_json::to_string(&ev(2)).unwrap();
        let (a, b) = line.split_at(10);
        OpenOptions::new().append(true).open(&path).unwrap().write_all(a.as_bytes()).unwrap();
        assert!(reader.poll().is_empty());
        OpenOptions::new().append(true).open(&path).unwrap().write_all(format!("{b}\n").as_bytes()).unwrap();
        assert_eq!(reader.poll().iter().map(|e| e.ts).collect::<Vec<_>>(), [2]);

        OpenOptions::new().append(true).open(&path).unwrap().write_all(b"garbage\n").unwrap();
        append(home.path(), &ev(3), ROTATE_AT).unwrap();
        assert_eq!(reader.poll().iter().map(|e| e.ts).collect::<Vec<_>>(), [3]);
    }

    #[test]
    fn reader_follows_a_rotation() {
        let home = tempfile::tempdir().unwrap();
        let mut reader = EventReader::new(home.path());
        let path = append(home.path(), &ev(1), ROTATE_AT).unwrap();
        assert_eq!(reader.poll().len(), 1);
        append(home.path(), &ev(2), ROTATE_AT).unwrap();
        let size = fs::metadata(&path).unwrap().len();
        // rotates before 3; the new log then grows past the old offset before the next poll
        append(home.path(), &ev(3), size).unwrap();
        for ts in 4..8 {
            append(home.path(), &ev(ts), ROTATE_AT).unwrap();
        }
        assert!(fs::metadata(&path).unwrap().len() > size);
        assert_eq!(reader.poll().iter().map(|e| e.ts).collect::<Vec<_>>(), [2, 3, 4, 5, 6, 7]);
        // a rotation that leaves the new log shorter than the old offset
        let size = fs::metadata(&path).unwrap().len();
        append(home.path(), &ev(8), size).unwrap();
        assert_eq!(reader.poll().iter().map(|e| e.ts).collect::<Vec<_>>(), [8]);
        assert!(reader.poll().is_empty());
    }

    #[test]
    fn reader_drops_the_cursor_of_a_removed_log() {
        let home = tempfile::tempdir().unwrap();
        let mut reader = EventReader::new(home.path());
        let path = append(home.path(), &ev(1), ROTATE_AT).unwrap();
        assert_eq!(reader.poll().len(), 1);
        assert_eq!(reader.cursors.len(), 1);
        fs::remove_file(&path).unwrap();
        assert!(reader.poll().is_empty());
        assert!(reader.cursors.is_empty());
    }

    #[test]
    fn reader_keeps_sessions_apart() {
        let home = tempfile::tempdir().unwrap();
        let mut reader = EventReader::new(home.path());
        append(home.path(), &ev(1), ROTATE_AT).unwrap();
        let other = Event::from_payload("Stop", r#"{"session_id":"ffff"}"#, 9).unwrap();
        append(home.path(), &other, ROTATE_AT).unwrap();
        let got = reader.poll();
        assert_eq!(got.len(), 2);
        assert_eq!(reader.poll_session("ffff").len(), 0);
        append(home.path(), &other, ROTATE_AT).unwrap();
        assert_eq!(reader.poll_session("ffff").len(), 1);
        assert_eq!(reader.poll_session(SID).len(), 0);
    }

    #[test]
    fn role_injection_is_tagged_by_content() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(role_injection(home.path(), "worker", None), None);
        fs::create_dir_all(home.path().join("roles")).unwrap();
        fs::write(home.path().join("roles").join("worker.md"), "You are the worker.\n").unwrap();
        let text = role_injection(home.path(), "worker", None).unwrap();
        let tag = role_tag("worker", "You are the worker.");
        assert_eq!(text, format!("{tag}\nYou are the worker.\n"));
        assert_ne!(role_tag("worker", "other text"), tag);
        // sha256("abc") = ba7816bf…
        assert_eq!(role_tag("r", "abc"), "FOREMAN-ROLE r vba7816bf");
        assert_eq!(role_injection(home.path(), "../worker", None), None);
        fs::write(home.path().join("roles").join("empty.md"), " \n").unwrap();
        assert_eq!(role_injection(home.path(), "empty", None), None);
    }

    #[test]
    fn role_injection_fills_in_the_workspace() {
        let home = tempfile::tempdir().unwrap();
        fs::create_dir_all(home.path().join("roles")).unwrap();
        fs::write(home.path().join("roles").join("workspace.md"), "Controller for {workspace}.\n").unwrap();
        let text = role_injection(home.path(), "workspace", Some("nomos")).unwrap();
        assert_eq!(text, format!("{}\nController for nomos.\n", role_tag("workspace", "Controller for nomos.")));
        assert!(role_injection(home.path(), "workspace", None).unwrap().contains("{workspace}"));
        fs::write(home.path().join("roles").join("repo.md"), "Repo {repo} of {workspace}.
").unwrap();
        let text = role_injection_for(home.path(), "repo", Some("nomos"), Some("Nomos.Pdf")).unwrap();
        assert!(text.ends_with("
Repo Nomos.Pdf of nomos.
"), "{text}");
    }
}
