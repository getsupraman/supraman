use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::paths::path_key;

use super::{Entries, FolderEntry};

/// The direct child folders shown under each listed folder that shows them, by the parent's [`path_key`].
pub type Shown = BTreeMap<String, Vec<PathBuf>>;

/// Build output and tool folders never worth a row, matched case-insensitively.
pub const SKIPPED: &[&str] = &["node_modules", "target", "bin", "obj", "__pycache__", "venv"];

/// `path`'s direct child folders, sorted by name: no file, no dot or hidden folder, none of [`SKIPPED`];
/// nothing when it cannot be read. Blocking.
pub fn subfolders(path: &Path) -> Vec<PathBuf> {
    read_subfolders(path).unwrap_or_default()
}

/// [`subfolders`], `None` when `path` cannot be read. Blocking.
pub fn read_subfolders(path: &Path) -> Option<Vec<PathBuf>> {
    let read = fs::read_dir(path).ok()?;
    let mut out: Vec<(String, PathBuf)> = read
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()) || e.path().is_dir())
        .filter(|e| !crate::platform::fs::hidden(e))
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let skipped = name.starts_with('.') || SKIPPED.iter().any(|s| s.eq_ignore_ascii_case(&name));
            (!skipped).then(|| (name.to_lowercase(), e.path()))
        })
        .collect();
    out.sort();
    Some(out.into_iter().map(|(_, p)| p).collect())
}

/// Each folder that shows its sub folders and could be read; blocking.
pub fn shown(entries: &Entries) -> Shown {
    entries.folders().filter(|f| f.show_subfolders).filter_map(|f| Some((path_key(&f.path), read_subfolders(&f.path)?))).collect()
}

impl FolderEntry {
    /// Whether this folder leaves its child folder `child` out.
    pub fn excludes(&self, child: &Path) -> bool {
        let key = path_key(child);
        self.excluded.iter().any(|p| path_key(p) == key)
    }

    /// Its shown children, excluded ones too; none while it does not show them.
    pub fn read_children<'a>(&self, shown: &'a Shown) -> &'a [PathBuf] {
        match self.show_subfolders {
            true => shown.get(&path_key(&self.path)).map_or(&[], Vec::as_slice),
            false => &[],
        }
    }

    /// Its shown children but the excluded: the ones that hold sessions.
    pub fn children<'a>(&'a self, shown: &'a Shown) -> impl Iterator<Item = &'a Path> {
        self.read_children(shown).iter().map(PathBuf::as_path).filter(|c| !self.excludes(c))
    }
}

impl Entries {
    /// Every folder placement weighs: the listed ones, then the shown, not excluded, children of each.
    pub fn placed<'a>(&'a self, shown: &'a Shown) -> impl Iterator<Item = &'a Path> {
        self.folders().map(|f| f.path.as_path()).chain(self.folders().flat_map(|f| f.children(shown)))
    }

    /// The folders that show their sub folders, each once.
    pub fn showing(&self) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = Vec::new();
        for f in self.folders().filter(|f| f.show_subfolders) {
            if !out.iter().any(|p| path_key(p) == path_key(&f.path)) {
                out.push(f.path.clone());
            }
        }
        out
    }
}
