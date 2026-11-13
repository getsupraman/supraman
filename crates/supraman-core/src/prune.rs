//! Bounded retention for what sessions leave in the Supraman home: event logs, schedule leftovers and prompt files.

use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use crate::events::events_dir;
use crate::time::modified_ms;

pub const KEEP_MS: i64 = 30 * 24 * 60 * 60 * 1000;

/// Removes event logs, schedule leftovers and prompt files idle for `keep_ms`; what it could not.
pub fn prune(home: &Path, now: i64, keep_ms: i64) -> Vec<String> {
    let cutoff = now - keep_ms;
    let mut errors = Vec::new();
    remove_older(&events_dir(home), cutoff, |n| n.ends_with(".jsonl") || n.ends_with(".jsonl.1"), &mut errors);
    crate::schedule::prune_schedules(home, cutoff, &mut errors);
    remove_older(&crate::home::prompts_dir(home), cutoff, |n| n.ends_with(".txt"), &mut errors);
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

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use super::*;

    fn age(path: &Path, days: u64) {
        let file = fs::File::options().write(true).open(path).unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(days * 24 * 3600)).unwrap();
    }

    #[test]
    fn prune_removes_old_logs_and_keeps_new_ones() {
        let home = tempfile::tempdir().unwrap();
        let events = events_dir(home.path());
        fs::create_dir_all(&events).unwrap();
        let (old_log, new_log, rotated) = (events.join("old.jsonl"), events.join("new.jsonl"), events.join("old.jsonl.1"));
        for f in [&old_log, &new_log, &rotated] {
            fs::write(f, "{}\n").unwrap();
        }
        age(&old_log, 40);
        age(&rotated, 40);

        let now = crate::time::now_ms();
        assert_eq!(prune(home.path(), now, KEEP_MS), Vec::<String>::new());
        assert!(!old_log.exists() && !rotated.exists() && new_log.exists());
    }

    #[test]
    fn prune_removes_old_prompt_files_left_by_a_send() {
        let home = tempfile::tempdir().unwrap();
        let prompts = crate::home::prompts_dir(home.path());
        fs::create_dir_all(&prompts).unwrap();
        let (old, new) = (prompts.join("1-abcdef.txt"), prompts.join("2-abcdef.txt"));
        fs::write(&old, "the user's text").unwrap();
        fs::write(&new, "the user's text").unwrap();
        age(&old, 40);
        assert_eq!(prune(home.path(), crate::time::now_ms(), KEEP_MS), Vec::<String>::new());
        assert!(!old.exists() && new.exists());
    }
}
