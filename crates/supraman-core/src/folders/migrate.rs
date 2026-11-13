use std::collections::HashSet;

use crate::Registry;
use crate::paths::path_key;

use super::{Entries, Entry, FolderEntry};

/// Each workspace that loads, by name, as a group of its folders; a path already placed keeps its first place.
pub(super) fn from_workspaces(registry: &Registry) -> Entries {
    let workspaces = registry.load_all().map(|(loaded, _)| loaded).unwrap_or_default();
    let mut placed = HashSet::new();
    let list = workspaces
        .into_iter()
        .map(|w| Entry::Group {
            folders: w.folders.into_iter().filter(|f| placed.insert(path_key(&f.path))).map(|f| FolderEntry::new(f.path)).collect(),
            name: w.name,
        })
        .collect();
    Entries::new(list)
}
