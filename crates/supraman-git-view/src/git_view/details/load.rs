//! What the commit panel reads for one commit or stash, blocking: its details, changed files and whole tree.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use supraman_gitman::{Amendable, ChangeKind, Details, FileChange, Repo, Stash, WipCounts};
use supraman_ui::tree::{Order, Tree};

/// A file's change in this commit; `None` on a file the commit left as it was.
pub(crate) type Mark = Option<ChangeKind>;

pub(crate) type FileTree = Tree<Mark, WipCounts>;

pub(crate) struct Loaded {
    pub(crate) sha: String,
    pub(crate) details: Details,
    pub(crate) stash: Option<Arc<Stash>>,
    pub(crate) changes: Vec<FileChange>,
    pub(crate) counts: WipCounts,
    /// The changed files as a tree.
    pub(crate) changed: FileTree,
    /// Every file at the commit, the changed and deleted ones marked; read only with View all files on.
    pub(crate) all: Option<Whole>,
    /// The changed files' paths.
    pub(crate) files: HashSet<String>,
}

/// The whole tree at a commit ([`whole`]) and every path in it.
pub(crate) struct Whole {
    pub(crate) tree: FileTree,
    pub(crate) files: HashSet<String>,
}

/// A commit's details and changed files, without the whole tree.
#[cfg(test)]
pub(crate) fn load(repo: &Repo, sha: &str, order: Order) -> Result<Loaded, String> {
    load_with(repo, sha, order, false)
}

/// [`load`], with the whole tree when `all_files`.
pub(crate) fn load_with(repo: &Repo, sha: &str, order: Order, all_files: bool) -> Result<Loaded, String> {
    let details = supraman_gitman::details(repo, sha).map_err(|e| e.to_string())?;
    let changes = supraman_gitman::changed_files(repo, sha).map_err(|e| e.to_string())?;
    build(repo, sha, details, changes, None, order, all_files)
}

/// Whether the message box may amend or reword `sha`, read after its details are shown.
pub(crate) fn amendable(repo: &Repo, sha: &str) -> Amendable {
    supraman_gitman::amendable(repo, sha).unwrap_or_else(|e| {
        log::warn!("git tab: cannot tell whether {sha} can be amended: {e}");
        Amendable::No
    })
}

/// A stash's commit and its work-tree changes against its base.
pub(crate) fn load_stash(repo: &Repo, stash: Arc<Stash>, order: Order, all_files: bool) -> Result<Loaded, String> {
    let details = supraman_gitman::details(repo, &stash.sha).map_err(|e| e.to_string())?;
    let changes = supraman_gitman::changed_between(repo, &stash.base, &stash.sha).map_err(|e| e.to_string())?;
    build(repo, &stash.sha, details, changes, Some(stash.clone()), order, all_files)
}

fn build(repo: &Repo, sha: &str, details: Details, changes: Vec<FileChange>, stash: Option<Arc<Stash>>, order: Order, all_files: bool) -> Result<Loaded, String> {
    let all = if all_files { Some(whole(repo, sha, &changes, order)?) } else { None };
    let mut counts = WipCounts::default();
    changes.iter().for_each(|c| counts.add(&Some(c.kind)));
    let files = changes.iter().map(|c| c.path.clone()).collect();
    let changed = Tree::build(changes.iter().map(|c| (c.path.clone(), Some(c.kind))), WipCounts::add, order);
    Ok(Loaded { sha: sha.to_string(), details, stash, changes, counts, changed, all, files })
}

/// Every file at `sha` (`ls-tree -r`) with `changes` marked, as a tree.
pub(crate) fn whole(repo: &Repo, sha: &str, changes: &[FileChange], order: Order) -> Result<Whole, String> {
    let tree = supraman_gitman::tree(repo, sha).map_err(|e| e.to_string())?;
    let all = all_files(tree.into_iter().map(|f| f.path), changes);
    let files = all.iter().map(|(p, _)| p.clone()).collect();
    Ok(Whole { tree: Tree::build(all, WipCounts::add, order), files })
}

/// The tree's files, each marked with its change, then every changed path the tree does not list.
pub(crate) fn all_files(tree: impl Iterator<Item = String>, changes: &[FileChange]) -> Vec<(String, Mark)> {
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
