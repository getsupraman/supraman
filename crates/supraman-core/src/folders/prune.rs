use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::Result;
use crate::paths::path_key;

use super::{Entries, Shown, read_subfolders};

/// The excluded paths whose parent was read and lacks them among its sub folders: from `shown` for a folder
/// showing them, else read now; a parent that cannot be read gives none. Blocking.
pub fn gone_excluded(entries: &Entries, shown: &Shown) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for f in entries.folders().filter(|f| !f.excluded.is_empty()) {
        let read = match f.show_subfolders {
            true => shown.get(&path_key(&f.path)).cloned(),
            false => read_subfolders(&f.path),
        };
        let Some(children) = read else { continue };
        let present: HashSet<String> = children.iter().map(|c| path_key(c)).collect();
        for p in &f.excluded {
            if !present.contains(&path_key(p)) && !out.iter().any(|o| path_key(o) == path_key(p)) {
                out.push(p.clone());
            }
        }
    }
    out
}

/// `gone` dropped from every folder's `excluded` where its parent, read again under the lock, still lacks it;
/// nothing is written when none is dropped. The count dropped.
pub fn prune_excluded(home: &Path, gone: &[PathBuf]) -> Result<usize> {
    if gone.is_empty() {
        return Ok(0);
    }
    let gone: HashSet<String> = gone.iter().map(|p| path_key(p)).collect();
    let (_, dropped) = super::edit_when(home, |entries| {
        let mut reads: HashMap<String, Option<HashSet<String>>> = HashMap::new();
        let mut dropped = 0;
        for f in entries.list.iter_mut().flat_map(super::Entry::folders_mut) {
            let parent = path_key(&f.path);
            let before = f.excluded.len();
            f.excluded.retain(|p| {
                let key = path_key(p);
                if !gone.contains(&key) {
                    return true;
                }
                let children = reads.entry(parent.clone()).or_insert_with(|| read_subfolders(&f.path).map(|c| c.iter().map(|c| path_key(c)).collect()));
                children.as_ref().is_none_or(|c| c.contains(&key))
            });
            dropped += before - f.excluded.len();
        }
        Ok((dropped, dropped > 0))
    })?;
    Ok(dropped)
}
