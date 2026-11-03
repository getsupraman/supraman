//! Bounded retention for what sessions leave in the Supraman home: event logs and relay messages.

use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use crate::events::events_dir;
use crate::relay::{ANSWERED_FILE, READY_FILE, REJECTED_DIR, SENT_DIR, answered_lock};
use crate::time::modified_ms;

pub const KEEP_MS: i64 = 30 * 24 * 60 * 60 * 1000;

/// Removes the files not modified for `keep_ms` before `now`: `events/*.jsonl` and their `.1`, and per
/// workspace `outbox/*.json` (never `ready.json`), `inbox/sent` and `inbox/rejected`, then the
/// `answered.json` entries of outbox files that are gone. A drop still waiting in the inbox stays. Then the
/// schedules': `runs/*.jsonl.1`, the state of removed schedules, and spent run worktrees.
/// Returns what could not be removed.
pub fn prune(home: &Path, now: i64, keep_ms: i64) -> Vec<String> {
    let cutoff = now - keep_ms;
    let mut errors = Vec::new();
    remove_older(&events_dir(home), cutoff, |n| n.ends_with(".jsonl") || n.ends_with(".jsonl.1"), &mut errors);
    crate::schedule::prune_schedules(home, cutoff, &mut errors);
    let Ok(workspaces) = fs::read_dir(home.join("relay")) else { return errors };
    for dir in workspaces.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).map(|e| e.path()) {
        let outbox = dir.join("outbox");
        remove_older(&outbox, cutoff, |n| n.ends_with(".json") && n != READY_FILE, &mut errors);
        for sub in [SENT_DIR, REJECTED_DIR] {
            remove_older(&dir.join("inbox").join(sub), cutoff, |_| true, &mut errors);
        }
        if let Err(e) = forget_answers(&dir, &outbox) {
            errors.push(format!("Cannot prune {}: {e}", dir.join(ANSWERED_FILE).display()));
        }
    }
    errors
}

pub(crate) fn remove_older(dir: &Path, cutoff: i64, matches: impl Fn(&str) -> bool, errors: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() || !matches(&name) || modified_ms(&meta) >= cutoff {
            continue;
        }
        match fs::remove_file(entry.path()) {
            Err(e) if e.kind() != ErrorKind::NotFound => errors.push(format!("Cannot remove {}: {e}", entry.path().display())),
            _ => {}
        }
    }
}

fn forget_answers(dir: &Path, outbox: &Path) -> std::io::Result<()> {
    let path = dir.join(ANSWERED_FILE);
    if !path.exists() {
        return Ok(());
    }
    let _lock = answered_lock(dir)?;
    let mut answered: std::collections::BTreeMap<String, String> =
        serde_json::from_str(&fs::read_to_string(&path)?).map_err(|e| std::io::Error::new(ErrorKind::InvalidData, e))?;
    let before = answered.len();
    answered.retain(|file, _| outbox.join(file).exists());
    if answered.len() == before {
        return Ok(());
    }
    crate::fs::write_atomic(&path, serde_json::to_string_pretty(&answered).map_err(std::io::Error::other)?)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use super::*;
    use crate::{inbox_dir, mark_answered, outbox_dir, read_answered, send, tell};

    fn age(path: &Path, days: u64) {
        let file = fs::File::options().write(true).open(path).unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(days * 24 * 3600)).unwrap();
    }

    #[test]
    fn prune_removes_old_logs_and_messages_and_keeps_new_ones_and_waiting_drops() {
        let home = tempfile::tempdir().unwrap();
        let events = events_dir(home.path());
        fs::create_dir_all(&events).unwrap();
        let (old_log, new_log, rotated) = (events.join("old.jsonl"), events.join("new.jsonl"), events.join("old.jsonl.1"));
        for f in [&old_log, &new_log, &rotated] {
            fs::write(f, "{}\n").unwrap();
        }
        age(&old_log, 40);
        age(&rotated, 40);

        let waiting = send(home.path(), "demo", "fm-demo-workspace", "still waiting").unwrap();
        age(&waiting, 40);
        let inbox = inbox_dir(home.path(), "demo").unwrap();
        fs::create_dir_all(inbox.join(SENT_DIR)).unwrap();
        let sent = inbox.join(SENT_DIR).join("1-1-0001.json");
        fs::write(&sent, "{}").unwrap();
        age(&sent, 40);

        let old_msg = tell(home.path(), "demo", "fm-demo-workspace", "APPROVE? old").unwrap();
        let new_msg = tell(home.path(), "demo", "fm-demo-workspace", "APPROVE? new").unwrap();
        age(&old_msg, 40);
        let ready = outbox_dir(home.path(), "demo").unwrap().join(READY_FILE);
        fs::write(&ready, "{}").unwrap();
        age(&ready, 40);
        let name = |p: &Path| p.file_name().unwrap().to_str().unwrap().to_string();
        mark_answered(home.path(), "demo", &name(&old_msg), "yes").unwrap();
        mark_answered(home.path(), "demo", &name(&new_msg), "no").unwrap();

        let now = crate::time::now_ms();
        assert_eq!(prune(home.path(), now, KEEP_MS), Vec::<String>::new());
        assert!(!old_log.exists() && !rotated.exists() && new_log.exists());
        assert!(waiting.exists(), "a drop not yet sent stays");
        assert!(!sent.exists());
        assert!(!old_msg.exists() && new_msg.exists() && ready.exists());
        assert_eq!(read_answered(home.path(), "demo").unwrap().into_keys().collect::<Vec<_>>(), [name(&new_msg)]);
    }
}
