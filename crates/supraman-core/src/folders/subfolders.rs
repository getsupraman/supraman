use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::paths::path_key;

use super::Entries;

/// The direct child folders shown under each listed folder that shows them, by the parent's [`path_key`].
pub type Shown = BTreeMap<String, Vec<PathBuf>>;

/// Build output and tool folders never worth a row, matched case-insensitively.
pub const SKIPPED: &[&str] = &["node_modules", "target", "bin", "obj", "__pycache__", "venv"];

/// `path`'s direct child folders, sorted by name: no file, no dot or hidden folder, none of [`SKIPPED`];
/// nothing when it cannot be read. Blocking.
pub fn subfolders(path: &Path) -> Vec<PathBuf> {
    let Ok(read) = fs::read_dir(path) else { return Vec::new() };
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
    out.into_iter().map(|(_, p)| p).collect()
}

/// Each folder that shows its sub folders, read; blocking.
pub fn shown(entries: &Entries) -> Shown {
    entries.folders().filter(|f| f.show_subfolders).map(|f| (path_key(&f.path), subfolders(&f.path))).collect()
}

impl Entries {
    /// Every folder placement weighs: the listed ones, then the shown children of each that shows them.
    pub fn placed<'a>(&'a self, shown: &'a Shown) -> impl Iterator<Item = &'a Path> {
        let children = self.folders().filter(|f| f.show_subfolders).flat_map(move |f| shown.get(&path_key(&f.path)).into_iter().flatten().map(PathBuf::as_path));
        self.folders().map(|f| f.path.as_path()).chain(children)
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
