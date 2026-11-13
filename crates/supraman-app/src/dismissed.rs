use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use supraman_core::store::{JsonStore, Save};
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct File {
    /// Its name before folders goes with folders::migrate in 0.1.19 (backlog/end-controllers-removal).
    #[serde(alias = "workspaces")]
    folders: BTreeMap<String, BTreeSet<String>>,
}

/// Inbox entries dismissed, per folder key, by their key (a failed run's, a waiting schedule's).
#[derive(Debug, Default)]
pub struct Dismissed(JsonStore<File>);

impl Dismissed {
    /// `lost` says what a lost file means.
    pub fn load_at(path: Option<PathBuf>, lost: &str) -> (Self, Option<String>) {
        let (store, notice) = JsonStore::load(path, lost);
        (Self(store), notice)
    }

    pub fn hides(&self, folder: &str, key: &str) -> bool {
        self.0.get().folders.get(folder).is_some_and(|f| f.contains(key))
    }

    pub fn dismiss(&mut self, folder: &str, key: &str) -> Option<Save> {
        self.0.edit(|f| f.folders.entry(folder.to_string()).or_default().insert(key.to_string()))
    }

    /// The dismissals made before this store was read join the file's.
    pub fn absorb(&mut self, early: &Dismissed) -> Option<Save> {
        self.0.edit(|f| {
            let mut changed = false;
            for (folder, keys) in &early.0.get().folders {
                let mine = f.folders.entry(folder.clone()).or_default();
                for key in keys {
                    changed |= mine.insert(key.clone());
                }
            }
            changed
        })
    }

    /// Forgets the keys no longer in `current`; a folder not in `current` keeps its dismissals.
    pub fn prune_keys(&mut self, current: &BTreeMap<String, Vec<String>>) -> (bool, Option<Save>) {
        let mut changed = false;
        let save = self.0.edit(|f| {
            for (folder, listed) in current {
                let Some(keys) = f.folders.get_mut(folder) else { continue };
                let before = keys.len();
                keys.retain(|k| listed.contains(k));
                changed |= keys.len() != before;
                if keys.is_empty() {
                    f.folders.remove(folder);
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

    const LOST: &str = "every entry shows";

    fn listed(w: &str, keys: &[&str]) -> BTreeMap<String, Vec<String>> {
        [(w.to_string(), keys.iter().map(|k| k.to_string()).collect())].into()
    }

    fn dismissed(d: &mut Dismissed, folder: &str, key: &str) {
        if let Some(save) = d.dismiss(folder, key) {
            save.write().unwrap();
        }
    }

    fn pruned(d: &mut Dismissed, current: &BTreeMap<String, Vec<String>>) -> bool {
        let (changed, save) = d.prune_keys(current);
        if let Some(save) = save {
            save.write().unwrap();
        }
        changed
    }

    #[test]
    fn a_dismissal_survives_a_reload() {
        let home = tempfile::tempdir().unwrap();
        let file = home.path().join("dismissed.json");
        let (mut d, notice) = Dismissed::load_at(Some(file.clone()), LOST);
        assert!(notice.is_none() && !d.hides("w", "a"));
        dismissed(&mut d, "w", "a");
        assert!(d.hides("w", "a"));
        assert!(!d.hides("other", "a"), "per folder");
        let (again, _) = Dismissed::load_at(Some(file), LOST);
        assert!(again.hides("w", "a"), "read back");
        assert!(!home.path().join("dismissed.json.tmp").exists());
    }

    #[test]
    fn a_file_written_before_folders_is_read_by_its_folder_keys() {
        let home = tempfile::tempdir().unwrap();
        let file = home.path().join("dismissed.json");
        std::fs::write(&file, r#"{"workspaces":{"gaia-1a2b3c4d":["run-1"]}}"#).unwrap();
        let (d, notice) = Dismissed::load_at(Some(file), LOST);
        assert!(notice.is_none() && d.hides("gaia-1a2b3c4d", "run-1"));
    }

    #[test]
    fn without_a_file_nothing_is_saved() {
        let mut d = Dismissed::default();
        assert!(d.dismiss("w", "a").is_none(), "no file, nothing to save");
        assert!(!d.hides("w", "b"));
    }

    #[test]
    fn prune_forgets_keys_no_longer_listed_and_keeps_unlisted_folders() {
        let home = tempfile::tempdir().unwrap();
        let file = home.path().join("dismissed.json");
        let (mut d, _) = Dismissed::load_at(Some(file.clone()), LOST);
        dismissed(&mut d, "w", "a");
        dismissed(&mut d, "w", "b");
        dismissed(&mut d, "v", "c");
        assert!(!pruned(&mut d, &listed("w", &["a", "b", "d"])));
        assert!(pruned(&mut d, &listed("w", &["b", "d"])));
        assert!(!d.hides("w", "a") && d.hides("w", "b"));
        assert!(d.hides("v", "c"), "a folder the poll did not list keeps its dismissals");
        let (again, _) = Dismissed::load_at(Some(file), LOST);
        assert!(!again.hides("w", "a") && again.hides("w", "b") && again.hides("v", "c"), "the prune was written");
        assert!(pruned(&mut d, &listed("w", &["d"])));
        assert!(!d.0.get().folders.contains_key("w"), "an emptied folder leaves nothing behind");
    }

    #[test]
    fn a_dismissal_made_before_the_file_is_read_is_kept_with_the_files() {
        let home = tempfile::tempdir().unwrap();
        let file = home.path().join("dismissed.json");
        let (mut d, _) = Dismissed::load_at(Some(file.clone()), LOST);
        dismissed(&mut d, "w", "a");
        let mut early = Dismissed::default();
        assert!(early.dismiss("v", "b").is_none(), "no file yet, nothing written");
        let (mut read, _) = Dismissed::load_at(Some(file.clone()), LOST);
        read.absorb(&early).expect("a change").write().unwrap();
        assert!(read.absorb(&early).is_none(), "nothing new the second time");
        let (again, _) = Dismissed::load_at(Some(file), LOST);
        assert!(again.hides("w", "a") && again.hides("v", "b"));
    }

    #[test]
    fn an_unreadable_file_starts_empty_and_says_so() {
        let home = tempfile::tempdir().unwrap();
        let file = home.path().join("dismissed.json");
        std::fs::write(&file, "not json").unwrap();
        let (d, notice) = Dismissed::load_at(Some(file), LOST);
        assert!(!d.hides("w", "a"));
        assert!(notice.unwrap().contains("dismissed.json"));
        let (none, notice) = Dismissed::load_at(None, LOST);
        assert!(notice.is_none() && !none.hides("w", "a"));
    }
}
