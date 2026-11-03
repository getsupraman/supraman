use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::fs::{load_json_or_notice, write_atomic};

/// A small JSON state file held in memory (stars, dismissals): read once with [`load_json_or_notice`], each
/// change saved whole through [`Save::write`] off the UI thread.
#[derive(Debug, Default)]
pub struct JsonStore<T> {
    path: Option<PathBuf>,
    value: T,
    pending: Arc<Pending>,
}

#[derive(Debug, Default)]
struct Pending {
    newest: Mutex<Option<Result<String, String>>>,
    writing: Mutex<()>,
}

/// The newest value of a [`JsonStore`] on its way to disk.
#[must_use]
#[derive(Debug)]
pub struct Save {
    path: PathBuf,
    pending: Arc<Pending>,
}

impl<T: Serialize + DeserializeOwned + Default> JsonStore<T> {
    /// `lost` says what a lost or unreadable file means; without a path the store lives in memory only.
    pub fn load(path: Option<PathBuf>, lost: &str) -> (Self, Option<String>) {
        let Some(path) = path else { return (Self::default(), None) };
        let (value, notice) = load_json_or_notice(&path, lost);
        (Self { path: Some(path), value, pending: Arc::default() }, notice)
    }

    /// A store of `value` in memory only.
    pub fn in_memory(value: T) -> Self {
        Self { path: None, value, pending: Arc::default() }
    }

    pub fn get(&self) -> &T {
        &self.value
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Applies `edit`, which says whether it changed anything; a change of a store with a file is a [`Save`].
    pub fn edit(&mut self, edit: impl FnOnce(&mut T) -> bool) -> Option<Save> {
        if !edit(&mut self.value) {
            return None;
        }
        let path = self.path.clone()?;
        let text = serde_json::to_string_pretty(&self.value).map_err(|e| e.to_string());
        *self.pending.newest.lock().unwrap_or_else(|e| e.into_inner()) = Some(text);
        Some(Save { path, pending: self.pending.clone() })
    }
}

impl Save {
    /// Writes the store's newest value, unless a later save already wrote it; blocks on the disk.
    pub fn write(self) -> Result<(), String> {
        let _writing = self.pending.writing.lock().unwrap_or_else(|e| e.into_inner());
        let Some(text) = self.pending.newest.lock().unwrap_or_else(|e| e.into_inner()).take() else { return Ok(()) };
        let fail = |e: &dyn std::fmt::Display| format!("cannot write {}: {e}", self.path.display());
        write_atomic(&self.path, text.map_err(|e| fail(&e))?).map_err(|e| fail(&e))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    fn set(path: &Path) -> (JsonStore<BTreeSet<String>>, Option<String>) {
        JsonStore::load(Some(path.to_path_buf()), "nothing is kept")
    }

    #[test]
    fn a_change_is_written_by_its_save_and_read_back() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("keys.json");
        let (mut store, notice) = set(&path);
        assert!(notice.is_none() && store.get().is_empty());
        assert!(store.edit(|s| s.insert("a".into())).is_some_and(|save| save.write().is_ok()));
        assert!(store.edit(|s| s.insert("a".into())).is_none(), "no change, no save");
        assert_eq!(set(&path).0.get(), &BTreeSet::from(["a".to_string()]));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[\n  \"a\"\n]");
    }

    #[test]
    fn the_last_write_is_the_newest_value_whatever_order_the_saves_run_in() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("keys.json");
        let (mut store, _) = set(&path);
        let first = store.edit(|s| s.insert("a".into())).unwrap();
        let second = store.edit(|s| s.insert("b".into())).unwrap();
        second.write().unwrap();
        first.write().unwrap();
        assert_eq!(set(&path).0.get().len(), 2);
        assert_eq!(store.path(), Some(path.as_path()));
    }

    #[test]
    fn an_unreadable_file_starts_empty_and_says_so_and_no_path_keeps_it_in_memory() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("keys.json");
        std::fs::write(&path, "not json").unwrap();
        let (store, notice) = set(&path);
        assert!(store.get().is_empty() && notice.is_some_and(|n| n.contains("keys.json") && n.contains("nothing is kept")));
        let (mut memory, notice) = JsonStore::<BTreeSet<String>>::load(None, "x");
        assert!(notice.is_none() && memory.edit(|s| s.insert("a".into())).is_none() && memory.get().len() == 1);
        assert_eq!(JsonStore::in_memory(BTreeSet::from([1])).get().len(), 1);
    }
}
