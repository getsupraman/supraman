//! What the commit panel reads for one commit or stash, blocking, for `cx.background_spawn`: its details, its
//! changed files and the whole tree at it, with both file trees built.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use supraman_gitman::{Amendable, ChangeKind, Details, FileChange, Repo, Stash};
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
    pub stash: Option<Arc<Stash>>,
    pub changes: Vec<FileChange>,
    pub counts: Counts,
    /// The changed files as a tree.
    pub changed: FileTree,
    /// Every file at the commit, the changed ones marked, and the deleted ones it no longer holds; read only
    /// with View all files on.
    pub all: Option<Whole>,
    /// The changed files' paths.
    pub files: HashSet<String>,
    /// Whether the message box may amend the commit's message; [`Amendable::No`] for a stash.
    pub amendable: Amendable,
}

/// The whole tree at a commit ([`whole`]) and every path in it.
pub struct Whole {
    pub tree: FileTree,
    pub files: HashSet<String>,
}

/// A commit's details and changed files, without the whole tree.
#[cfg(test)]
pub fn load(repo: &Repo, sha: &str, order: Order) -> Result<Loaded, String> {
    load_with(repo, sha, order, false)
}

/// [`load`], with the whole tree when `all_files`.
pub fn load_with(repo: &Repo, sha: &str, order: Order, all_files: bool) -> Result<Loaded, String> {
    let details = supraman_gitman::details(repo, sha).map_err(|e| e.to_string())?;
    let changes = supraman_gitman::changed_files(repo, sha).map_err(|e| e.to_string())?;
    let amendable = supraman_gitman::amendable(repo, sha).unwrap_or_else(|e| {
        log::warn!("git tab: cannot tell whether {sha} can be amended: {e}");
        Amendable::No
    });
    Ok(Loaded { amendable, ..build(repo, sha, details, changes, None, order, all_files)? })
}

/// A stash's commit and its work-tree changes against its base.
pub fn load_stash(repo: &Repo, stash: Arc<Stash>, order: Order, all_files: bool) -> Result<Loaded, String> {
    let details = supraman_gitman::details(repo, &stash.sha).map_err(|e| e.to_string())?;
    let changes = supraman_gitman::changed_between(repo, &stash.base, &stash.sha).map_err(|e| e.to_string())?;
    build(repo, &stash.sha, details, changes, Some(stash.clone()), order, all_files)
}

fn build(repo: &Repo, sha: &str, details: Details, changes: Vec<FileChange>, stash: Option<Arc<Stash>>, order: Order, all_files: bool) -> Result<Loaded, String> {
    let all = if all_files { Some(whole(repo, sha, &changes, order)?) } else { None };
    let mut counts = Counts::default();
    changes.iter().for_each(|c| counts.add(&Some(c.kind)));
    let files = changes.iter().map(|c| c.path.clone()).collect();
    let changed = Tree::build(changes.iter().map(|c| (c.path.clone(), Some(c.kind))), Counts::add, order);
    Ok(Loaded { sha: sha.to_string(), details, stash, changes, counts, changed, all, files, amendable: Amendable::No })
}

/// Every file at `sha` (`ls-tree -r`) with `changes` marked, as a tree.
pub fn whole(repo: &Repo, sha: &str, changes: &[FileChange], order: Order) -> Result<Whole, String> {
    let tree = supraman_gitman::tree(repo, sha).map_err(|e| e.to_string())?;
    let all = all_files(tree.into_iter().map(|f| f.path), changes);
    let files = all.iter().map(|(p, _)| p.clone()).collect();
    Ok(Whole { tree: Tree::build(all, Counts::add, order), files })
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
