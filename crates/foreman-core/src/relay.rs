//! The relay's files: `<FOREMAN_HOME>/relay/<workspace>/{inbox,outbox}`. Foreman drops `{to, body}`
//! into the inbox ([`send`]); the workspace's relay session watches it with `fm inbox` under its
//! `Monitor` tool ([`InboxWatcher`]) and sends each drop with native `SendMessage`; every message the
//! relay receives comes back as an outbox file `{from, from_name, body}` ([`read_outbox`]), as does a
//! session's own `fm tell` ([`tell`]). It is the fallback after `foreman_claude::human`'s pipe write, and
//! runs only on demand ([`relay_demand`]). Facts behind the shape: LEARNINGS m0/4, m3/1 and m5/32.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use serde::{Deserialize, Serialize};

use crate::fs::write_atomic;
use crate::time::{modified_ms, now_ms};
use crate::{CoreError, Result, validate_name};

/// Written by the relay each time its watcher is armed (first arm and every re-arm).
pub const READY_FILE: &str = "ready.json";
/// Where the watcher moves a drop once it printed it, and where a malformed one is set aside.
pub const SENT_DIR: &str = "sent";
pub const REJECTED_DIR: &str = "rejected";
/// An outbox file that does not parse yet may still be being written; after this it is shown as is.
pub const UNREADABLE_AFTER_MS: i64 = 5_000;
/// `relay/<w>/last-drop`: when [`send`] last dropped a message, Unix epoch milliseconds as text.
pub const LAST_DROP_FILE: &str = "last-drop";
/// How long the relay stays wanted after the last drop (`settings.toml` `relay_idle_secs` overrides it).
pub const DEFAULT_RELAY_IDLE_MS: i64 = 10 * 60 * 1000;

pub fn relay_dir(home: &Path, workspace: &str) -> Result<PathBuf> {
    validate_name(workspace)?;
    Ok(home.join("relay").join(workspace))
}

pub fn inbox_dir(home: &Path, workspace: &str) -> Result<PathBuf> {
    Ok(relay_dir(home, workspace)?.join("inbox"))
}

pub fn outbox_dir(home: &Path, workspace: &str) -> Result<PathBuf> {
    Ok(relay_dir(home, workspace)?.join("outbox"))
}

/// Creates `inbox/` and `outbox/`; the relay's `--add-dir` must exist before it launches.
pub fn ensure_relay_dirs(home: &Path, workspace: &str) -> Result<PathBuf> {
    let dir = relay_dir(home, workspace)?;
    for sub in ["inbox", "outbox"] {
        let path = dir.join(sub);
        fs::create_dir_all(&path).map_err(|e| CoreError::io(&path, e))?;
    }
    Ok(dir)
}

/// One message for the relay to send: `SendMessage(to, body)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayDrop {
    pub to: String,
    pub body: String,
}

static SEQ: AtomicU32 = AtomicU32::new(0);

/// Drops `{to, body}` into the workspace's inbox, written atomically so the watcher never sees half a file. Names sort in drop order (`<ms>-<pid>-<seq>.json`).
pub fn send(home: &Path, workspace: &str, to: &str, body: &str) -> Result<PathBuf> {
    if to.trim().is_empty() {
        return Err(CoreError::io(
            inbox_dir(home, workspace)?,
            io::Error::new(ErrorKind::InvalidInput, "a message needs a recipient"),
        ));
    }
    let dir = ensure_relay_dirs(home, workspace)?.join("inbox");
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let name = format!("{:013}-{}-{seq:04}.json", now_ms(), std::process::id());
    let path = dir.join(&name);
    let text = serde_json::to_string(&RelayDrop {
        to: to.to_string(),
        body: body.to_string(),
    })
    .map_err(|e| CoreError::io(&path, io::Error::other(e)))?;
    write_atomic(&path, text).map_err(|e| CoreError::io(&path, e))?;
    let stamp = dir.with_file_name(LAST_DROP_FILE);
    write_atomic(&stamp, now_ms().to_string()).map_err(|e| CoreError::io(&stamp, e))?;
    Ok(path)
}

/// Drops still waiting in the inbox (`*.json`; the watcher moves each one it printed out of it).
pub fn pending_drops(home: &Path, workspace: &str) -> Result<usize> {
    let dir = inbox_dir(home, workspace)?;
    match fs::read_dir(&dir) {
        Ok(entries) => Ok(entries
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_file()) && e.file_name().to_string_lossy().ends_with(".json"))
            .count()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(0),
        Err(e) => Err(CoreError::io(&dir, e)),
    }
}

/// When [`send`] last dropped a message, if ever.
pub fn last_drop(home: &Path, workspace: &str) -> Option<i64> {
    fs::read_to_string(relay_dir(home, workspace).ok()?.join(LAST_DROP_FILE)).ok()?.trim().parse().ok()
}

/// Whether the workspace needs its relay now: a drop is waiting, or the last one is younger than `idle_ms`.
pub fn relay_demand(home: &Path, workspace: &str, now: i64, idle_ms: i64) -> Result<bool> {
    Ok(pending_drops(home, workspace)? > 0 || last_drop(home, workspace).is_some_and(|t| now - t < idle_ms))
}

/// `fm tell`: a session's message to the human, written into the outbox as the relay writes what it
/// receives (`{from, from_name, body}`, `from` = the session's name: it has no pipe address here).
/// Named `<ms>-tell-<pid>-<seq>.json`, never `msg-NNNN.json`, the relay's own numbering.
pub fn tell(home: &Path, workspace: &str, from_name: &str, body: &str) -> Result<PathBuf> {
    let dir = ensure_relay_dirs(home, workspace)?.join("outbox");
    if from_name.trim().is_empty() {
        return Err(CoreError::io(&dir, io::Error::new(ErrorKind::InvalidInput, "a message needs a sender")));
    }
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let name = format!("{:013}-tell-{}-{seq:04}.json", now_ms(), std::process::id());
    let path = dir.join(&name);
    let text = serde_json::json!({ "from": from_name, "from_name": from_name, "body": body }).to_string();
    write_atomic(&path, text).map_err(|e| CoreError::io(&path, e))?;
    Ok(path)
}

/// One message the relay received, as it wrote it to the outbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxMessage {
    pub file: String,
    /// The sender's address (`uds:…` pipe, opaque), or `relay-notice` for a delivery notice or a
    /// failed `SendMessage`.
    pub from: String,
    /// The sender's session name: what Foreman routes by (m0/4).
    pub from_name: String,
    pub body: String,
    /// Arrival: the file's modification time, Unix epoch milliseconds.
    pub at: i64,
    /// Set when the file is not the expected JSON; `body` is then its raw text.
    pub error: Option<String>,
}

impl OutboxMessage {
    pub fn is_notice(&self) -> bool {
        self.from == "relay-notice"
    }
}

#[derive(Deserialize)]
struct OutboxFile {
    #[serde(default)]
    from: String,
    #[serde(default)]
    from_name: String,
    body: String,
}

/// Every message in the workspace's outbox, oldest first (by arrival, then file name). `ready.json`
/// is not a message. A file that does not parse is skipped while it is younger than
/// [`UNREADABLE_AFTER_MS`] (the relay may still be writing it), then listed with `error` set.
pub fn read_outbox(home: &Path, workspace: &str) -> Result<Vec<OutboxMessage>> {
    let dir = outbox_dir(home, workspace)?;
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(CoreError::io(&dir, e)),
    };
    let now = now_ms();
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let file = entry.file_name().to_string_lossy().into_owned();
        if !file.ends_with(".json") || file == READY_FILE {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let at = modified_ms(&meta);
        let Ok(text) = fs::read_to_string(entry.path()) else { continue };
        match serde_json::from_str::<OutboxFile>(&text) {
            Ok(m) => out.push(OutboxMessage {
                from_name: if m.from_name.is_empty() { m.from.clone() } else { m.from_name },
                from: m.from,
                body: m.body,
                file,
                at,
                error: None,
            }),
            Err(_) if now - at < UNREADABLE_AFTER_MS => {}
            Err(e) => out.push(OutboxMessage {
                file,
                from: String::new(),
                from_name: String::new(),
                body: text,
                at,
                error: Some(e.to_string()),
            }),
        }
    }
    out.sort_by(|a, b| a.at.cmp(&b.at).then_with(|| a.file.cmp(&b.file)));
    Ok(out)
}

/// When the relay last armed its watcher (`outbox/ready.json`'s modification time), if ever.
pub fn relay_ready(home: &Path, workspace: &str) -> Option<i64> {
    let path = outbox_dir(home, workspace).ok()?.join(READY_FILE);
    fs::metadata(path).ok().filter(fs::Metadata::is_file).map(|m| modified_ms(&m))
}

/// Forgets the last arm: a relay that is not running has no watcher, whatever `ready.json` says.
pub fn clear_ready(home: &Path, workspace: &str) -> Result<()> {
    let path = outbox_dir(home, workspace)?.join(READY_FILE);
    match fs::remove_file(&path) {
        Err(e) if e.kind() != ErrorKind::NotFound => Err(CoreError::io(&path, e)),
        _ => Ok(()),
    }
}

/// What a workspace session's message asks of the human, by its first word (the skills' formats:
/// `plugins/foreman/skills/workspace-controller/SKILL.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ask {
    /// `APPROVE? …`: a plan waiting for the human's approval.
    Approve,
    /// `DECIDE: …`: a question only the human answers.
    Decide,
}

pub fn ask_kind(body: &str) -> Option<Ask> {
    let body = body.trim_start();
    if body.starts_with("APPROVE?") {
        Some(Ask::Approve)
    } else if body.starts_with("DECIDE:") {
        Some(Ask::Decide)
    } else {
        None
    }
}

/// Where a question stands for the Messages list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AskState {
    Open(Ask),
    /// The first line of the answer sent.
    Answered(String),
    /// An unanswered `APPROVE?` with a later `APPROVE?` from the same sender: a re-plan replaces the
    /// plan, so only the newest one is approved. `DECIDE:` questions are separate and never superseded.
    Superseded,
}

/// The state of every question `from_name` sent in `msgs` (oldest first, as [`read_outbox`] lists
/// them), keyed by file name; `answered` is file name → first line of the answer.
pub fn ask_states(msgs: &[OutboxMessage], from_name: &str, answered: &BTreeMap<String, String>) -> BTreeMap<String, AskState> {
    let asks: Vec<(&OutboxMessage, Ask)> = msgs
        .iter()
        .filter(|m| m.error.is_none() && m.from_name == from_name)
        .filter_map(|m| ask_kind(&m.body).map(|a| (m, a)))
        .collect();
    let newest_approve = asks.iter().rev().find(|(_, a)| *a == Ask::Approve).map(|(m, _)| m.file.as_str());
    asks.iter()
        .map(|(m, ask)| {
            let state = match answered.get(&m.file) {
                Some(line) => AskState::Answered(line.clone()),
                None if *ask == Ask::Approve && newest_approve != Some(m.file.as_str()) => AskState::Superseded,
                None => AskState::Open(*ask),
            };
            (m.file.clone(), state)
        })
        .collect()
}

/// The questions the human answered from Foreman: `relay/<w>/answered.json`, outbox file name → the
/// answer's first line, so a restart does not offer Approve twice.
pub const ANSWERED_FILE: &str = "answered.json";

/// Missing file: nothing answered yet.
pub fn read_answered(home: &Path, workspace: &str) -> Result<BTreeMap<String, String>> {
    let path = relay_dir(home, workspace)?.join(ANSWERED_FILE);
    match fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| CoreError::io(&path, io::Error::new(ErrorKind::InvalidData, e))),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(e) => Err(CoreError::io(&path, e)),
    }
}

pub fn mark_answered(home: &Path, workspace: &str, file: &str, answer: &str) -> Result<()> {
    let mut answered = read_answered(home, workspace)?;
    answered.insert(file.to_string(), answer.to_string());
    let dir = ensure_relay_dirs(home, workspace)?;
    let path = dir.join(ANSWERED_FILE);
    let text = serde_json::to_string_pretty(&answered).map_err(|e| CoreError::io(&path, io::Error::other(e)))?;
    write_atomic(&path, text).map_err(|e| CoreError::io(&path, e))
}

/// `fm inbox --dir <inbox>`'s state: each [`InboxWatcher::scan`] turns every new `*.json` drop into
/// one JSON line and moves the file to `sent/` (a malformed one to `rejected/`), so a re-armed
/// watcher neither loses nor replays a drop. A file it could not move is remembered, not re-printed.
#[derive(Debug, Default)]
pub struct InboxWatcher {
    dir: PathBuf,
    printed: HashSet<String>,
}

impl InboxWatcher {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            printed: HashSet::new(),
        }
    }

    /// The lines for this pass, in file-name order: `{"to","body","file"}` per drop, or
    /// `{"error","file"}` for a drop that is not `{to, body}` with a non-empty `to`.
    pub fn scan(&mut self) -> io::Result<Vec<String>> {
        let mut names: Vec<String> = fs::read_dir(&self.dir)?
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".json") && !self.printed.contains(n))
            .collect();
        names.sort();
        let mut lines = Vec::new();
        for name in names {
            let path = self.dir.join(&name);
            let parsed = fs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|text| serde_json::from_str::<RelayDrop>(text.trim_start_matches('\u{feff}')).map_err(|e| e.to_string()))
                .and_then(|d| if d.to.trim().is_empty() { Err("\"to\" is empty".into()) } else { Ok(d) });
            let (line, to_dir) = match parsed {
                Ok(d) => (serde_json::json!({ "to": d.to, "body": d.body, "file": name }), SENT_DIR),
                Err(e) => (serde_json::json!({ "error": format!("not a {{to, body}} drop: {e}"), "file": name }), REJECTED_DIR),
            };
            lines.push(line.to_string());
            self.printed.insert(name.clone());
            let target = self.dir.join(to_dir);
            let _ = fs::create_dir_all(&target).and_then(|()| fs::rename(&path, target.join(&name)));
        }
        Ok(lines)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drop_is_one_json_line_with_its_multiline_body_escaped_and_is_moved_to_sent() {
        let home = tempfile::tempdir().unwrap();
        let path = send(home.path(), "demo", "fm-demo-workspace", "line one\nline \"two\"").unwrap();
        let inbox = inbox_dir(home.path(), "demo").unwrap();
        assert_eq!(path.parent(), Some(inbox.as_path()));
        let mut w = InboxWatcher::new(&inbox);
        let lines = w.scan().unwrap();
        assert_eq!(lines.len(), 1);
        assert!(!lines[0].contains('\n'), "{}", lines[0]);
        let v: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
        let file = path.file_name().unwrap().to_str().unwrap();
        assert_eq!(v, serde_json::json!({"to": "fm-demo-workspace", "body": "line one\nline \"two\"", "file": file}));
        assert!(!path.exists());
        assert!(inbox.join(SENT_DIR).join(file).is_file());
        assert_eq!(w.scan().unwrap(), Vec::<String>::new());
    }

    #[test]
    fn the_relay_is_in_demand_while_a_drop_waits_and_until_the_idle_timeout() {
        let home = tempfile::tempdir().unwrap();
        let idle = DEFAULT_RELAY_IDLE_MS;
        let now = now_ms();
        assert!(!relay_demand(home.path(), "demo", now, idle).unwrap(), "no relay dir: no demand");
        send(home.path(), "demo", "fm-demo-workspace", "x").unwrap();
        let at = last_drop(home.path(), "demo").unwrap();
        assert!((at - now).abs() < 60_000, "{at} vs {now}");
        assert_eq!(pending_drops(home.path(), "demo").unwrap(), 1);
        assert!(relay_demand(home.path(), "demo", at + 2 * idle, idle).unwrap(), "a waiting drop keeps it wanted");
        InboxWatcher::new(inbox_dir(home.path(), "demo").unwrap()).scan().unwrap();
        assert_eq!(pending_drops(home.path(), "demo").unwrap(), 0);
        assert!(relay_demand(home.path(), "demo", at + idle - 1, idle).unwrap());
        assert!(!relay_demand(home.path(), "demo", at + idle, idle).unwrap(), "idle for the timeout: retired");
    }

    #[test]
    fn a_rearmed_watcher_neither_loses_nor_replays() {
        let home = tempfile::tempdir().unwrap();
        let inbox = inbox_dir(home.path(), "demo").unwrap();
        send(home.path(), "demo", "a", "1").unwrap();
        assert_eq!(InboxWatcher::new(&inbox).scan().unwrap().len(), 1);
        // dropped while no watcher ran
        send(home.path(), "demo", "b", "2").unwrap();
        send(home.path(), "demo", "c", "3").unwrap();
        let lines = InboxWatcher::new(&inbox).scan().unwrap();
        let to: Vec<String> = lines
            .iter()
            .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap()["to"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(to, ["b", "c"], "in drop order, none replayed");
    }

    #[test]
    fn malformed_drops_print_an_error_line_and_are_set_aside() {
        let home = tempfile::tempdir().unwrap();
        let inbox = ensure_relay_dirs(home.path(), "demo").unwrap().join("inbox");
        fs::write(inbox.join("1.json"), "not json").unwrap();
        fs::write(inbox.join("2.json"), r#"{"to": " ", "body": "x"}"#).unwrap();
        fs::write(inbox.join("3.json"), r#"{"body": "x"}"#).unwrap();
        fs::write(inbox.join("4.json.tmp"), r#"{"to": "a", "body": "half"#).unwrap();
        fs::write(inbox.join("5.json"), "\u{feff}{\"to\": \"a\", \"body\": \"bom\"}").unwrap();
        let mut w = InboxWatcher::new(&inbox);
        let lines: Vec<serde_json::Value> = w.scan().unwrap().iter().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines.len(), 4, "{lines:?}");
        for (line, file) in lines.iter().take(3).zip(["1.json", "2.json", "3.json"]) {
            assert_eq!(line["file"], file);
            assert!(line["error"].as_str().unwrap().starts_with("not a {to, body} drop"), "{line}");
            assert!(inbox.join(REJECTED_DIR).join(file).is_file());
        }
        assert_eq!(lines[3]["body"], "bom");
        assert!(inbox.join("4.json.tmp").is_file(), "a .tmp is never read");
        assert_eq!(w.scan().unwrap(), Vec::<String>::new());
    }

    #[test]
    fn tell_writes_an_outbox_message_the_list_reads_like_the_relays() {
        let home = tempfile::tempdir().unwrap();
        let path = tell(home.path(), "demo", "fm-demo-workspace", "DONE: x\nmore").unwrap();
        let file = path.file_name().unwrap().to_str().unwrap().to_string();
        assert!(file.contains("-tell-") && !file.starts_with("msg-"), "{file}");
        let msgs = read_outbox(home.path(), "demo").unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!((msgs[0].file.as_str(), msgs[0].from.as_str(), msgs[0].from_name.as_str()), (file.as_str(), "fm-demo-workspace", "fm-demo-workspace"));
        assert_eq!(msgs[0].body, "DONE: x\nmore");
        assert!(!msgs[0].is_notice());
        assert!(tell(home.path(), "demo", " ", "x").is_err());
        assert!(tell(home.path(), "../x", "a", "x").is_err());
    }

    #[test]
    fn send_refuses_an_empty_recipient_and_a_bad_workspace() {
        let home = tempfile::tempdir().unwrap();
        assert!(send(home.path(), "demo", " ", "x").is_err());
        assert!(send(home.path(), "../x", "a", "x").is_err());
    }

    #[test]
    fn outbox_lists_messages_oldest_first_and_ready_separately() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(read_outbox(home.path(), "demo").unwrap(), []);
        assert_eq!(relay_ready(home.path(), "demo"), None);
        let outbox = ensure_relay_dirs(home.path(), "demo").unwrap().join("outbox");
        fs::write(outbox.join(READY_FILE), r#"{"ready": true}"#).unwrap();
        assert!(relay_ready(home.path(), "demo").is_some());
        fs::write(outbox.join("msg-0002.json"), r#"{"from": "relay-notice", "body": "held"}"#).unwrap();
        fs::write(
            outbox.join("msg-0001.json"),
            r#"{"from": "uds:\\\\.\\pipe\\x", "from_name": "fm-demo-workspace", "body": "hi\nthere"}"#,
        )
        .unwrap();
        fs::write(outbox.join("msg-0003.json"), "{\"from\": ").unwrap();
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(60);
        for (name, age) in [("msg-0001.json", 2), ("msg-0002.json", 1)] {
            let f = fs::File::options().write(true).open(outbox.join(name)).unwrap();
            f.set_modified(old + std::time::Duration::from_secs(age)).unwrap();
        }
        let msgs = read_outbox(home.path(), "demo").unwrap();
        assert_eq!(msgs.len(), 2, "the half-written file is not listed yet: {msgs:?}");
        assert_eq!(msgs[0].file, "msg-0002.json");
        assert!(msgs[0].is_notice());
        assert_eq!(msgs[0].from_name, "relay-notice");
        assert_eq!(msgs[1].from_name, "fm-demo-workspace");
        assert_eq!(msgs[1].body, "hi\nthere");
        assert!(msgs[0].at < msgs[1].at);

        let f = fs::File::options().write(true).open(outbox.join("msg-0003.json")).unwrap();
        f.set_modified(old).unwrap();
        let msgs = read_outbox(home.path(), "demo").unwrap();
        assert_eq!(msgs[0].file, "msg-0003.json");
        assert!(msgs[0].error.is_some());
        assert_eq!(msgs[0].body, "{\"from\": ");

        clear_ready(home.path(), "demo").unwrap();
        clear_ready(home.path(), "demo").unwrap();
        assert_eq!(relay_ready(home.path(), "demo"), None);
    }

    fn msg(file: &str, from_name: &str, body: &str) -> OutboxMessage {
        OutboxMessage { file: file.into(), from: "uds:x".into(), from_name: from_name.into(), body: body.into(), at: 0, error: None }
    }

    #[test]
    fn a_question_is_known_by_its_first_word() {
        assert_eq!(ask_kind("APPROVE? Add dark mode — 2 repos"), Some(Ask::Approve));
        assert_eq!(ask_kind("
  DECIDE: which db (1.2, api)
more"), Some(Ask::Decide));
        assert_eq!(ask_kind("DONE: shipped"), None);
        assert_eq!(ask_kind("approve? lower case is not the format"), None);
        assert_eq!(ask_kind("I will send APPROVE? later"), None);
        assert_eq!(ask_kind("DECIDE without colon"), None);
    }

    #[test]
    fn a_later_approve_supersedes_earlier_unanswered_ones_and_decides_stay_open() {
        let ctrl = "fm-demo-workspace";
        let msgs = [
            msg("1.json", ctrl, "APPROVE? plan v1"),
            msg("2.json", ctrl, "DECIDE: a or b"),
            msg("3.json", ctrl, "APPROVE? plan v2"),
            msg("4.json", "fm-demo-relay", "APPROVE? not from the workspace session"),
            msg("5.json", ctrl, "DECIDE: c or d"),
            msg("6.json", ctrl, "DONE: something"),
        ];
        let mut answered = BTreeMap::new();
        let states = ask_states(&msgs, ctrl, &answered);
        assert_eq!(states.len(), 4, "{states:?}");
        assert_eq!(states["1.json"], AskState::Superseded);
        assert_eq!(states["2.json"], AskState::Open(Ask::Decide));
        assert_eq!(states["3.json"], AskState::Open(Ask::Approve));
        assert_eq!(states["5.json"], AskState::Open(Ask::Decide));
        answered.insert("1.json".to_string(), "approve".to_string());
        answered.insert("5.json".to_string(), "c".to_string());
        let states = ask_states(&msgs, ctrl, &answered);
        assert_eq!(states["1.json"], AskState::Answered("approve".into()), "answered before it was superseded");
        assert_eq!(states["5.json"], AskState::Answered("c".into()));
        assert_eq!(states["3.json"], AskState::Open(Ask::Approve));
    }

    #[test]
    fn answered_questions_persist_per_workspace() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(read_answered(home.path(), "demo").unwrap(), BTreeMap::new());
        mark_answered(home.path(), "demo", "1.json", "approve").unwrap();
        mark_answered(home.path(), "demo", "2.json", "use postgres").unwrap();
        mark_answered(home.path(), "demo", "1.json", "approve").unwrap();
        let answered = read_answered(home.path(), "demo").unwrap();
        assert_eq!(answered.len(), 2);
        assert_eq!(answered["2.json"], "use postgres");
        assert_eq!(read_answered(home.path(), "other").unwrap(), BTreeMap::new());
        let dir = relay_dir(home.path(), "demo").unwrap();
        assert!(dir.join(ANSWERED_FILE).is_file());
        assert!(!dir.join(format!("{ANSWERED_FILE}.tmp")).exists());
        fs::write(dir.join(ANSWERED_FILE), "not json").unwrap();
        assert!(read_answered(home.path(), "demo").is_err(), "a broken file is an error, not an empty set");
        assert!(mark_answered(home.path(), "../x", "1.json", "a").is_err());
    }
}
