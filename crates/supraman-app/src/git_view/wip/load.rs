//! What the WIP panel reads, blocking, for `cx.background_spawn`: the status, split into the unstaged and
//! the staged files, each as a list and a tree.

use supraman_gitman::{ChangeKind, EntryKind, FileChange, OperationState, Repo, Status, short_sha};
use supraman_ui::tree::{Order, Tree};

use crate::git_view::details::{Counts, sort_changes};
use crate::git_view::diff::{Of, Target};

pub type FileTree = Tree<Option<ChangeKind>, Counts>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Branch {
    Named(String),
    /// HEAD's commit, cut to 7, while detached; `None` before the first commit.
    Detached(Option<String>),
}

/// Which of the two lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Unstaged,
    Staged,
}

impl Section {
    /// The list's id in its hit keys.
    pub fn list(self) -> &'static str {
        match self {
            Section::Unstaged => "wip-unstaged",
            Section::Staged => "wip-staged",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Section::Unstaged => "Unstaged Files",
            Section::Staged => "Staged Files",
        }
    }

    /// What clicking `file` in this list shows.
    pub fn target(self, file: &FileChange) -> Target {
        let of = match (self, file.kind) {
            (Section::Staged, _) => Of::Staged,
            (Section::Unstaged, ChangeKind::Untracked) => Of::Untracked,
            (Section::Unstaged, ChangeKind::Unmerged) => Of::Conflicted,
            (Section::Unstaged, _) => Of::Unstaged,
        };
        Target { of, path: file.path.clone(), change: Some(file.clone()) }
    }
}

pub struct Loaded {
    pub branch: Branch,
    /// Changed paths, each once whether staged, unstaged or both.
    pub total: usize,
    pub unstaged: Vec<FileChange>,
    pub staged: Vec<FileChange>,
    pub unstaged_tree: FileTree,
    pub staged_tree: FileTree,
    pub state: OperationState,
}

/// `1 file change`, `3 file changes`.
pub fn file_changes(n: usize) -> String {
    if n == 1 { "1 file change".into() } else { format!("{n} file changes") }
}

/// Each entry's unstaged side and staged side: a partly staged file is in both, a conflicted or untracked one
/// only among the unstaged.
pub fn sections(status: &Status) -> (Vec<FileChange>, Vec<FileChange>) {
    let (mut unstaged, mut staged) = (Vec::new(), Vec::new());
    for e in &status.entries {
        let file = |kind: ChangeKind, old_path: Option<String>| FileChange { kind, path: e.path.clone(), old_path, similarity: None };
        match e.kind {
            EntryKind::Unmerged => unstaged.push(file(ChangeKind::Unmerged, None)),
            EntryKind::Untracked => unstaged.push(file(ChangeKind::Untracked, None)),
            EntryKind::Ordinary | EntryKind::Renamed { .. } | EntryKind::Copied { .. } => {
                if let Some(kind) = e.staged {
                    staged.push(file(kind, e.orig_path.clone()));
                }
                if let Some(kind) = e.unstaged {
                    unstaged.push(file(kind, None));
                }
            }
        }
    }
    (unstaged, staged)
}

fn tree(files: &[FileChange], order: Order) -> FileTree {
    Tree::build(files.iter().map(|c| (c.path.clone(), Some(c.kind))), Counts::add, order)
}

pub fn load(repo: &Repo, order: Order) -> Result<Loaded, String> {
    let status = supraman_gitman::status(repo).map_err(|e| e.to_string())?;
    Ok(of_status(&status, order))
}

/// The lists of a status read already.
pub fn of_status(status: &Status, order: Order) -> Loaded {
    let branch = match &status.branch.head {
        Some(name) => Branch::Named(name.clone()),
        None => Branch::Detached(status.branch.oid.as_ref().map(|oid| short_sha(oid).to_string())),
    };
    let (mut unstaged, mut staged) = sections(status);
    sort_changes(&mut unstaged, order);
    sort_changes(&mut staged, order);
    let (unstaged_tree, staged_tree) = (tree(&unstaged, order), tree(&staged, order));
    Loaded { branch, total: status.entries.len(), unstaged, staged, unstaged_tree, staged_tree, state: status.state }
}
