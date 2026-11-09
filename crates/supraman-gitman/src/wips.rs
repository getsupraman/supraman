use std::path::{Path, PathBuf};

use supraman_core::paths::path_key;

use crate::changes::ChangeKind;
use crate::{Entry, EntryKind, GitError, Repo, Status, Worktree, status, worktrees};

/// How many worktrees [`wips`] reads at once.
pub const WIP_READERS: usize = 4;

/// A worktree's changed files by kind, a conflict counted as modified.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WipCounts {
    pub added: u32,
    pub deleted: u32,
    pub modified: u32,
    pub renamed: u32,
}

impl WipCounts {
    pub fn of(entries: &[Entry]) -> WipCounts {
        let mut counts = WipCounts::default();
        for entry in entries {
            let either = |kind| entry.staged == Some(kind) || entry.unstaged == Some(kind);
            let slot = match entry.kind {
                EntryKind::Unmerged => &mut counts.modified,
                EntryKind::Renamed { .. } => &mut counts.renamed,
                EntryKind::Untracked | EntryKind::Copied { .. } => &mut counts.added,
                EntryKind::Ordinary if either(ChangeKind::Added) => &mut counts.added,
                EntryKind::Ordinary if either(ChangeKind::Deleted) => &mut counts.deleted,
                EntryKind::Ordinary => &mut counts.modified,
            };
            *slot += 1;
        }
        counts
    }

    pub fn total(&self) -> u32 {
        self.added + self.deleted + self.modified + self.renamed
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WipRead {
    Read(Status),
    /// Its folder is gone, or git calls it prunable.
    Gone,
    Failed(String),
}

/// Another worktree's work in progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeWip {
    /// As `worktree list` gave it.
    pub worktree: Worktree,
    pub read: WipRead,
}

impl WorktreeWip {
    pub fn path(&self) -> &Path {
        &self.worktree.path
    }

    /// `None` before its first commit.
    pub fn head(&self) -> Option<&str> {
        self.status().and_then(|s| s.branch.oid.as_deref()).or(self.worktree.head.as_deref())
    }

    /// `None` when detached.
    pub fn branch(&self) -> Option<&str> {
        match self.status() {
            Some(s) => s.branch.head.as_deref(),
            None => self.worktree.branch.as_deref(),
        }
    }

    pub fn detached(&self) -> bool {
        self.branch().is_none()
    }

    pub fn counts(&self) -> WipCounts {
        WipCounts::of(self.files())
    }

    /// Its upstream, and how many commits it is ahead of and behind it.
    pub fn upstream(&self) -> Option<(&str, u32, u32)> {
        let branch = &self.status()?.branch;
        branch.upstream.as_deref().map(|name| (name, branch.ahead, branch.behind))
    }

    pub fn files(&self) -> &[Entry] {
        self.status().map_or(&[], |s| &s.entries)
    }

    pub fn status(&self) -> Option<&Status> {
        match &self.read {
            WipRead::Read(status) => Some(status),
            _ => None,
        }
    }
}

/// The work in progress of every worktree of `repo` but the bare one and those at `shown`, [`WIP_READERS`] at a
/// time, each read in its own folder; nothing while the repository has only one worktree. Blocking.
pub fn wips(repo: &Repo, shown: &[PathBuf]) -> Result<Vec<WorktreeWip>, GitError> {
    let all = worktrees(repo)?;
    if all.len() < 2 {
        return Ok(Vec::new());
    }
    let shown: Vec<String> = shown.iter().map(|p| path_key(p)).collect();
    let others: Vec<Worktree> = all.into_iter().filter(|w| !w.bare && !shown.contains(&path_key(&w.path))).collect();
    Ok(supraman_core::pool::map(others, WIP_READERS, |worktree| read(repo, worktree)))
}

fn read(repo: &Repo, worktree: Worktree) -> WorktreeWip {
    let read = if worktree.prunable || !worktree.path.is_dir() {
        WipRead::Gone
    } else {
        match status(&Repo::new(repo.git().clone(), &worktree.path)) {
            Ok(status) => WipRead::Read(status),
            Err(e) => WipRead::Failed(e.to_string()),
        }
    };
    WorktreeWip { worktree, read }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(kind: EntryKind, staged: Option<ChangeKind>, unstaged: Option<ChangeKind>) -> Entry {
        Entry { kind, path: "f".into(), orig_path: None, staged, unstaged }
    }

    #[test]
    fn each_entry_counts_once_and_a_conflict_as_modified() {
        use ChangeKind::*;
        let entries = [
            entry(EntryKind::Ordinary, Some(Added), Some(Modified)),
            entry(EntryKind::Untracked, None, Some(Untracked)),
            entry(EntryKind::Ordinary, None, Some(Deleted)),
            entry(EntryKind::Ordinary, Some(Modified), Some(Modified)),
            entry(EntryKind::Unmerged, Some(Unmerged), Some(Unmerged)),
            entry(EntryKind::Renamed { score: 100 }, Some(Renamed), Some(Modified)),
        ];
        assert_eq!(WipCounts::of(&entries), WipCounts { added: 2, deleted: 1, modified: 2, renamed: 1 });
    }
}
