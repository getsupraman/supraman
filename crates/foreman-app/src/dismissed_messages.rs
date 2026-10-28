//! Messages dismissed with their × (m5/36): hidden here, the outbox files untouched. Keyed by outbox file
//! name per workspace, so a newer message (another file) shows. Persisted as
//! `<FOREMAN_HOME>/messages-dismissed.json`, `{"workspaces": {"<workspace>": ["<outbox file>", …]}}`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use foreman_core::OutboxMessage;
use serde::{Deserialize, Serialize};

pub const FILE: &str = "messages-dismissed.json";

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct File {
    workspaces: BTreeMap<String, BTreeSet<String>>,
}

#[derive(Debug, Default, Clone)]
pub struct DismissedMessages {
    path: Option<PathBuf>,
    files: BTreeMap<String, BTreeSet<String>>,
}

impl DismissedMessages {
    pub fn path(home: &Path) -> PathBuf {
        home.join(FILE)
    }

    /// An unreadable file starts empty, with the reason.
    pub fn load(home: Option<&Path>) -> (Self, Option<String>) {
        let Some(path) = home.map(Self::path) else {
            return (Self::default(), None);
        };
        let (files, notice) = match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<File>(&text) {
                Ok(f) => (f.workspaces, None),
                Err(e) => (BTreeMap::new(), Some(format!("Cannot read {} ({e}): every message shows", path.display()))),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (BTreeMap::new(), None),
            Err(e) => (BTreeMap::new(), Some(format!("Cannot read {} ({e}): every message shows", path.display()))),
        };
        (Self { path: Some(path), files }, notice)
    }

    pub fn hides(&self, workspace: &str, file: &str) -> bool {
        self.files.get(workspace).is_some_and(|f| f.contains(file))
    }

    /// Hides `file` of `workspace` and writes the file; `Err` says why it could not be written.
    pub fn dismiss(&mut self, workspace: &str, file: &str) -> Result<(), String> {
        if self.files.entry(workspace.to_string()).or_default().insert(file.to_string()) {
            self.save()?;
        }
        Ok(())
    }

    /// After a poll: forgets the dismissals of each listed workspace whose message is no longer among
    /// its listed ones (gone from the outbox, or older than what is listed). A workspace not in `messages`
    /// (an empty outbox, or one that could not be read) keeps its dismissals. Whether anything changed.
    pub fn prune(&mut self, messages: &BTreeMap<String, Vec<OutboxMessage>>) -> Result<bool, String> {
        let mut changed = false;
        for (workspace, listed) in messages {
            let Some(files) = self.files.get_mut(workspace) else { continue };
            let before = files.len();
            files.retain(|f| listed.iter().any(|m| &m.file == f));
            changed |= files.len() != before;
            if files.is_empty() {
                self.files.remove(workspace);
            }
        }
        if changed {
            self.save()?;
        }
        Ok(changed)
    }

    fn save(&self) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let text = serde_json::to_string_pretty(&File { workspaces: self.files.clone() }).map_err(|e| e.to_string())?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("cannot write {}: {e}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(file: &str) -> OutboxMessage {
        OutboxMessage {
            file: file.into(),
            from: "uds:x".into(),
            from_name: "fm-w-workspace".into(),
            body: "ready".into(),
            at: 0,
            error: None,
        }
    }

    fn listed(w: &str, files: &[&str]) -> BTreeMap<String, Vec<OutboxMessage>> {
        [(w.to_string(), files.iter().map(|f| message(f)).collect())].into()
    }

    #[test]
    fn a_dismissal_survives_a_reload_and_leaves_the_outbox_alone() {
        let home = tempfile::tempdir().unwrap();
        let outbox = home.path().join("relay").join("w").join("outbox");
        std::fs::create_dir_all(&outbox).unwrap();
        std::fs::write(outbox.join("0001-tell.json"), "{}").unwrap();
        let (mut d, notice) = DismissedMessages::load(Some(home.path()));
        assert!(notice.is_none() && !d.hides("w", "0001-tell.json"));
        d.dismiss("w", "0001-tell.json").unwrap();
        assert!(d.hides("w", "0001-tell.json"));
        assert!(!d.hides("other", "0001-tell.json"), "per workspace");
        let (again, _) = DismissedMessages::load(Some(home.path()));
        assert!(again.hides("w", "0001-tell.json"), "read back from {FILE}");
        assert!(outbox.join("0001-tell.json").exists(), "the outbox file is kept");
        assert!(!home.path().join("messages-dismissed.json.tmp").exists());
    }

    #[test]
    fn a_newer_message_from_the_same_sender_shows() {
        let mut d = DismissedMessages::default();
        d.dismiss("w", "0001-tell.json").unwrap();
        assert!(!d.hides("w", "0002-tell.json"));
    }

    #[test]
    fn prune_forgets_messages_no_longer_listed_and_keeps_unlisted_workspaces() {
        let home = tempfile::tempdir().unwrap();
        let (mut d, _) = DismissedMessages::load(Some(home.path()));
        d.dismiss("w", "a.json").unwrap();
        d.dismiss("w", "b.json").unwrap();
        d.dismiss("v", "c.json").unwrap();
        assert_eq!(d.prune(&listed("w", &["a.json", "b.json", "d.json"])), Ok(false));
        assert_eq!(d.prune(&listed("w", &["b.json", "d.json"])), Ok(true));
        assert!(!d.hides("w", "a.json") && d.hides("w", "b.json"));
        assert!(d.hides("v", "c.json"), "a workspace the poll did not list keeps its dismissals");
        let (again, _) = DismissedMessages::load(Some(home.path()));
        assert!(!again.hides("w", "a.json") && again.hides("w", "b.json") && again.hides("v", "c.json"), "the prune was written");
        assert_eq!(d.prune(&listed("w", &["d.json"])), Ok(true));
        assert!(!d.files.contains_key("w"), "an emptied workspace leaves nothing behind");
    }

    #[test]
    fn an_unreadable_file_starts_empty_and_says_so() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(DismissedMessages::path(home.path()), "not json").unwrap();
        let (d, notice) = DismissedMessages::load(Some(home.path()));
        assert!(!d.hides("w", "a.json"));
        assert!(notice.unwrap().contains(FILE));
        let (none, notice) = DismissedMessages::load(None);
        assert!(notice.is_none() && !none.hides("w", "a.json"));
    }
}
