//! What the WIP panel reads, blocking: the status split into conflicted, unstaged and staged files, each a list and a tree.

use supraman_core::text::count;
use std::collections::HashMap;

use supraman_gitman::{ChangeKind, Conflict, EntryKind, FileChange, OperationState, Repo, Status, UnderWay, short_sha};
use supraman_ui::tree::{Order, Tree};

use crate::git_view::details::{Counts, sort_changes};
use crate::git_view::diff::{Of, Target};

pub(crate) type FileTree = Tree<Option<ChangeKind>, Counts>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Branch {
    Named(String),
    /// HEAD's commit, cut to 7, while detached; `None` before the first commit.
    Detached(Option<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Section {
    Conflicted,
    Unstaged,
    Staged,
}

impl Section {
    /// The list's id in its hit keys.
    pub(crate) fn list(self) -> &'static str {
        match self {
            Section::Conflicted => "wip-conflicted",
            Section::Unstaged => "wip-unstaged",
            Section::Staged => "wip-staged",
        }
    }

    /// The list's title; the staged files are the resolved ones while an operation is under way.
    pub(crate) fn title(self, under_way: bool) -> &'static str {
        match self {
            Section::Conflicted => "Conflicted Files",
            Section::Unstaged => "Unstaged Files",
            Section::Staged if under_way => "Resolved Files",
            Section::Staged => "Staged Files",
        }
    }

    /// What clicking `file` in this list shows.
    pub(crate) fn target(self, file: &FileChange) -> Target {
        let of = match (self, file.kind) {
            (Section::Staged, _) => Of::Staged,
            (Section::Conflicted, _) => Of::Conflicted,
            (Section::Unstaged, ChangeKind::Untracked) => Of::Untracked,
            (Section::Unstaged, ChangeKind::Unmerged) => Of::Conflicted,
            (Section::Unstaged, _) => Of::Unstaged,
        };
        Target { of, path: file.path.clone(), change: Some(file.clone()) }
    }
}

pub(crate) struct Loaded {
    pub(crate) branch: Branch,
    /// Changed paths, each once whether staged, unstaged or both.
    pub(crate) total: usize,
    pub(crate) conflicted: Vec<FileChange>,
    pub(crate) unstaged: Vec<FileChange>,
    pub(crate) staged: Vec<FileChange>,
    pub(crate) conflicted_tree: FileTree,
    pub(crate) unstaged_tree: FileTree,
    pub(crate) staged_tree: FileTree,
    pub(crate) state: OperationState,
    pub(crate) under_way: Option<UnderWay>,
    pub(crate) conflicts: HashMap<String, Conflict>,
}

/// `1 file change`, `3 file changes`.
pub(crate) fn file_changes(n: usize) -> String {
    count(n, "file change", "file changes")
}

/// Each entry's unstaged and staged side: a partly staged file is in both, a conflicted or untracked one only unstaged.
pub(crate) fn sections(status: &Status) -> (Vec<FileChange>, Vec<FileChange>) {
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

pub(crate) fn load(repo: &Repo, order: Order) -> Result<Loaded, String> {
    let status = supraman_gitman::status(repo).map_err(|e| e.to_string())?;
    Ok(of_status(&status, order))
}

/// The lists of a status read already.
pub(crate) fn of_status(status: &Status, order: Order) -> Loaded {
    let branch = match &status.branch.head {
        Some(name) => Branch::Named(name.clone()),
        None => Branch::Detached(status.branch.oid.as_ref().map(|oid| short_sha(oid).to_string())),
    };
    let (unstaged, mut staged) = sections(status);
    let (mut conflicted, mut unstaged): (Vec<FileChange>, Vec<FileChange>) = unstaged.into_iter().partition(|f| f.kind == ChangeKind::Unmerged);
    for files in [&mut conflicted, &mut unstaged, &mut staged] {
        sort_changes(files, order);
    }
    let (conflicted_tree, unstaged_tree, staged_tree) = (tree(&conflicted, order), tree(&unstaged, order), tree(&staged, order));
    let conflicts = status.entries.iter().filter_map(|e| Some((e.path.clone(), e.conflict()?))).collect();
    Loaded {
        branch,
        total: status.entries.len(),
        conflicted,
        unstaged,
        staged,
        conflicted_tree,
        unstaged_tree,
        staged_tree,
        state: status.state,
        under_way: status.under_way.clone(),
        conflicts,
    }
}
