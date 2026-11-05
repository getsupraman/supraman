//! What the commit panel reads for one commit, blocking, for `cx.background_spawn`: its details, its changed
//! files and the whole tree at it, with both file trees built.

use std::collections::{HashMap, HashSet};

use supraman_gitman::{ChangeKind, Details, FileChange, Repo};
use supraman_ui::tree::{Order, Tree};

/// A file's change in this commit; `None` on a file the commit left as it was.
pub type Mark = Option<ChangeKind>;

/// The changes a folder holds over its whole subtree, or a commit holds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub modified: usize,
    pub added: usize,
    pub deleted: usize,
    pub renamed: usize,
}

impl Counts {
    pub fn add(&mut self, mark: &Mark) {
        match mark {
            Some(ChangeKind::Added | ChangeKind::Copied | ChangeKind::Untracked) => self.added += 1,
            Some(ChangeKind::Deleted) => self.deleted += 1,
            Some(ChangeKind::Renamed) => self.renamed += 1,
            Some(ChangeKind::Modified | ChangeKind::TypeChanged | ChangeKind::Unmerged) => self.modified += 1,
            None => {}
        }
    }

    /// The non-zero counts, modified, added, deleted then renamed.
    pub fn parts(&self) -> Vec<(ChangeKind, usize)> {
        [(ChangeKind::Modified, self.modified), (ChangeKind::Added, self.added), (ChangeKind::Deleted, self.deleted), (ChangeKind::Renamed, self.renamed)]
            .into_iter()
            .filter(|(_, n)| *n > 0)
            .collect()
    }
}

pub type FileTree = Tree<Mark, Counts>;

pub struct Loaded {
    pub sha: String,
    pub details: Details,
    pub changes: Vec<FileChange>,
    pub counts: Counts,
    /// The changed files as a tree.
    pub changed: FileTree,
    /// Every file at the commit, the changed ones marked, and the deleted ones it no longer holds.
    pub all: FileTree,
    pub files: HashSet<String>,
}

pub fn load(repo: &Repo, sha: &str, order: Order) -> Result<Loaded, String> {
    let details = supraman_gitman::details(repo, sha).map_err(|e| e.to_string())?;
    let changes = supraman_gitman::changed_files(repo, sha).map_err(|e| e.to_string())?;
    let tree = supraman_gitman::tree(repo, sha).map_err(|e| e.to_string())?;
    let mut counts = Counts::default();
    changes.iter().for_each(|c| counts.add(&Some(c.kind)));
    let all = all_files(tree.into_iter().map(|f| f.path), &changes);
    let files = all.iter().map(|(p, _)| p.clone()).collect();
    let changed = Tree::build(changes.iter().map(|c| (c.path.clone(), Some(c.kind))), Counts::add, order);
    let all = Tree::build(all, Counts::add, order);
    Ok(Loaded { sha: sha.to_string(), details, changes, counts, changed, all, files })
}

/// The tree's files, each marked with its change, then every changed path the tree does not list.
pub fn all_files(tree: impl Iterator<Item = String>, changes: &[FileChange]) -> Vec<(String, Mark)> {
    let mut marks: HashMap<&str, ChangeKind> = changes.iter().map(|c| (c.path.as_str(), c.kind)).collect();
    let mut files: Vec<(String, Mark)> = tree
        .map(|path| {
            let mark = marks.remove(path.as_str());
            (path, mark)
        })
        .collect();
    files.extend(changes.iter().filter(|c| marks.contains_key(c.path.as_str())).map(|c| (c.path.clone(), Some(c.kind))));
    files
}
