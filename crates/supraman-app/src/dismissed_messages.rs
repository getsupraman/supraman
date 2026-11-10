use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use supraman_core::OutboxMessage;
use supraman_core::store::{JsonStore, Save};
use serde::{Deserialize, Serialize};

pub const FILE: &str = "messages-dismissed.json";

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct File {
    workspaces: BTreeMap<String, BTreeSet<String>>,
}

#[derive(Debug, Default)]
pub struct DismissedMessages(JsonStore<File>);

impl DismissedMessages {
    pub fn path(home: &Path) -> PathBuf {
        home.join(FILE)
    }

    pub fn load(home: Option<&Path>) -> (Self, Option<String>) {
        Self::load_at(home.map(Self::path), "every message shows")
    }

    /// The same per-workspace keys in another file; `lost` says what a lost file means.
    pub fn load_at(path: Option<PathBuf>, lost: &str) -> (Self, Option<String>) {
        let (store, notice) = JsonStore::load(path, lost);
        (Self(store), notice)
    }

    pub fn hides(&self, workspace: &str, file: &str) -> bool {
        self.0.get().workspaces.get(workspace).is_some_and(|f| f.contains(file))
    }

    pub fn dismiss(&mut self, workspace: &str, file: &str) -> Option<Save> {
        self.0.edit(|f| f.workspaces.entry(workspace.to_string()).or_default().insert(file.to_string()))
    }

    /// The dismissals made before this store was read join the file's.
    pub fn absorb(&mut self, early: &DismissedMessages) -> Option<Save> {
        self.0.edit(|f| {
            let mut changed = false;
            for (workspace, files) in &early.0.get().workspaces {
                let mine = f.workspaces.entry(workspace.clone()).or_default();
                for file in files {
                    changed |= mine.insert(file.clone());
                }
            }
            changed
        })
    }

    /// A workspace not in `messages` (an empty or unreadable outbox) keeps its dismissals.
    pub fn prune(&mut self, messages: &BTreeMap<String, Vec<OutboxMessage>>) -> (bool, Option<Save>) {
        let listed: BTreeMap<String, Vec<String>> = messages.iter().map(|(w, msgs)| (w.clone(), msgs.iter().map(|m| m.file.clone()).collect())).collect();
        self.prune_keys(&listed)
    }

    /// [`Self::prune`] for any keys: a workspace not in `current` keeps its dismissals.
    pub fn prune_keys(&mut self, current: &BTreeMap<String, Vec<String>>) -> (bool, Option<Save>) {
        let mut changed = false;
        let save = self.0.edit(|f| {
            for (workspace, listed) in current {
                let Some(files) = f.workspaces.get_mut(workspace) else { continue };
                let before = files.len();
                files.retain(|f| listed.contains(f));
                changed |= files.len() != before;
                if files.is_empty() {
                    f.workspaces.remove(workspace);
                }
            }
            changed
        });
        (changed, save)
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

    fn dismissed(d: &mut DismissedMessages, workspace: &str, file: &str) {
        if let Some(save) = d.dismiss(workspace, file) {
            save.write().unwrap();
        }
    }

    fn pruned(d: &mut DismissedMessages, messages: &BTreeMap<String, Vec<OutboxMessage>>) -> bool {
        let (changed, save) = d.prune(messages);
        if let Some(save) = save {
            save.write().unwrap();
        }
        changed
    }

    #[test]
    fn a_dismissal_survives_a_reload_and_leaves_the_outbox_alone() {
        let home = tempfile::tempdir().unwrap();
        let outbox = home.path().join("relay").join("w").join("outbox");
        std::fs::create_dir_all(&outbox).unwrap();
        std::fs::write(outbox.join("0001-tell.json"), "{}").unwrap();
        let (mut d, notice) = DismissedMessages::load(Some(home.path()));
        assert!(notice.is_none() && !d.hides("w", "0001-tell.json"));
        dismissed(&mut d, "w", "0001-tell.json");
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
        assert!(d.dismiss("w", "0001-tell.json").is_none(), "no file, nothing to save");
        assert!(!d.hides("w", "0002-tell.json"));
    }

    #[test]
    fn prune_forgets_messages_no_longer_listed_and_keeps_unlisted_workspaces() {
        let home = tempfile::tempdir().unwrap();
        let (mut d, _) = DismissedMessages::load(Some(home.path()));
        dismissed(&mut d, "w", "a.json");
        dismissed(&mut d, "w", "b.json");
        dismissed(&mut d, "v", "c.json");
        assert!(!pruned(&mut d, &listed("w", &["a.json", "b.json", "d.json"])));
        assert!(pruned(&mut d, &listed("w", &["b.json", "d.json"])));
        assert!(!d.hides("w", "a.json") && d.hides("w", "b.json"));
        assert!(d.hides("v", "c.json"), "a workspace the poll did not list keeps its dismissals");
        let (again, _) = DismissedMessages::load(Some(home.path()));
        assert!(!again.hides("w", "a.json") && again.hides("w", "b.json") && again.hides("v", "c.json"), "the prune was written");
        assert!(pruned(&mut d, &listed("w", &["d.json"])));
        assert!(!d.0.get().workspaces.contains_key("w"), "an emptied workspace leaves nothing behind");
    }

    #[test]
    fn a_dismissal_made_before_the_file_is_read_is_kept_with_the_files() {
        let home = tempfile::tempdir().unwrap();
        let (mut d, _) = DismissedMessages::load(Some(home.path()));
        dismissed(&mut d, "w", "a.json");
        let mut early = DismissedMessages::default();
        assert!(early.dismiss("v", "b.json").is_none(), "no file yet, nothing written");
        let (mut read, _) = DismissedMessages::load(Some(home.path()));
        read.absorb(&early).expect("a change").write().unwrap();
        assert!(read.absorb(&early).is_none(), "nothing new the second time");
        let (again, _) = DismissedMessages::load(Some(home.path()));
        assert!(again.hides("w", "a.json") && again.hides("v", "b.json"));
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
