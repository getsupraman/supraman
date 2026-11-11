use std::path::{Path, PathBuf};

use supraman_core::paths::path_key;

use crate::changes::ChangeKind;
use crate::{Entry, EntryKind, GitError, Repo, Status, Worktree, status, worktrees};

/// How many worktrees [`wips`] reads at once.
pub(crate) const WIP_READERS: usize = 4;

/// Changed files by kind, each path once by [`WipCounts::kind`]: a worktree's, a commit's, a folder's.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WipCounts {
    pub added: u32,
    pub modified: u32,
    pub deleted: u32,
    pub renamed: u32,
    pub conflicted: u32,
}

/// The order every count is listed in.
const ORDER: [ChangeKind; 5] = [ChangeKind::Added, ChangeKind::Modified, ChangeKind::Deleted, ChangeKind::Renamed, ChangeKind::Unmerged];

impl WipCounts {
    pub fn of(entries: &[Entry]) -> WipCounts {
        let mut counts = WipCounts::default();
        for entry in entries {
            counts.add(&Some(WipCounts::kind(entry)));
        }
        counts
    }

    /// The one kind a changed path counts as: any conflict conflicted, a rename renamed, a copy or an untracked file added, else added or deleted on either side, else modified.
    pub fn kind(entry: &Entry) -> ChangeKind {
        let either = |kind| entry.staged == Some(kind) || entry.unstaged == Some(kind);
        match entry.kind {
            EntryKind::Unmerged => ChangeKind::Unmerged,
            EntryKind::Renamed { .. } => ChangeKind::Renamed,
            EntryKind::Untracked | EntryKind::Copied { .. } => ChangeKind::Added,
            EntryKind::Ordinary if either(ChangeKind::Added) => ChangeKind::Added,
            EntryKind::Ordinary if either(ChangeKind::Deleted) => ChangeKind::Deleted,
            EntryKind::Ordinary => ChangeKind::Modified,
        }
    }

    /// One file of `kind` more; `None` is a file left as it was.
    pub fn add(&mut self, kind: &Option<ChangeKind>) {
        let slot = match kind {
            Some(ChangeKind::Added | ChangeKind::Copied | ChangeKind::Untracked) => &mut self.added,
            Some(ChangeKind::Modified | ChangeKind::TypeChanged) => &mut self.modified,
            Some(ChangeKind::Deleted) => &mut self.deleted,
            Some(ChangeKind::Renamed) => &mut self.renamed,
            Some(ChangeKind::Unmerged) => &mut self.conflicted,
            None => return,
        };
        *slot += 1;
    }

    pub fn total(&self) -> u32 {
        self.added + self.modified + self.deleted + self.renamed + self.conflicted
    }

    fn of_kind(&self, kind: ChangeKind) -> u32 {
        match kind {
            ChangeKind::Added => self.added,
            ChangeKind::Modified => self.modified,
            ChangeKind::Deleted => self.deleted,
            ChangeKind::Renamed => self.renamed,
            _ => self.conflicted,
        }
    }

    /// The non-zero counts: added, modified, deleted, renamed, then conflicted.
    pub fn parts(&self) -> Vec<(ChangeKind, u32)> {
        ORDER.into_iter().map(|kind| (kind, self.of_kind(kind))).filter(|(_, n)| *n > 0).collect()
    }

    /// `+1 ✎2 −1 →1 !1`: [`WipCounts::parts`], each after its [`count_mark`].
    pub fn words(&self) -> String {
        self.parts().into_iter().map(|(kind, n)| format!("{}{n}", count_mark(kind))).collect::<Vec<_>>().join(" ")
    }
}

/// A kind's mark before its count in [`WipCounts::words`].
pub fn count_mark(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Added | ChangeKind::Copied | ChangeKind::Untracked => "+",
        ChangeKind::Modified | ChangeKind::TypeChanged => "✎",
        ChangeKind::Deleted => "−",
        ChangeKind::Renamed => "→",
        ChangeKind::Unmerged => "!",
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

/// The work in progress of every worktree of `repo` but the bare one and those at `shown`, [`WIP_READERS`] at a time.
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
    fn every_entry_kind_counts_once_by_the_one_rule() {
        use ChangeKind::*;
        let copied = Entry { orig_path: Some("o".into()), ..entry(EntryKind::Copied { score: 100 }, Some(Copied), None) };
        let cases = [
            (entry(EntryKind::Ordinary, Some(Added), Some(Modified)), Added, "added on either side"),
            (entry(EntryKind::Ordinary, None, Some(Added)), Added, "intent to add"),
            (entry(EntryKind::Untracked, None, Some(Untracked)), Added, "untracked"),
            (copied, Added, "a copy"),
            (entry(EntryKind::Ordinary, None, Some(Deleted)), Deleted, "deleted"),
            (entry(EntryKind::Ordinary, Some(Modified), Some(Deleted)), Deleted, "deleted after a staged change"),
            (entry(EntryKind::Ordinary, Some(Modified), Some(Modified)), Modified, "modified"),
            (entry(EntryKind::Ordinary, Some(TypeChanged), None), Modified, "a type change"),
            (entry(EntryKind::Renamed { score: 100 }, Some(Renamed), Some(Modified)), Renamed, "renamed"),
            (entry(EntryKind::Unmerged, Some(Unmerged), Some(Unmerged)), Unmerged, "both modified"),
            (entry(EntryKind::Unmerged, Some(Added), Some(Added)), Unmerged, "both added"),
            (entry(EntryKind::Unmerged, Some(Deleted), Some(Deleted)), Unmerged, "both deleted"),
            (entry(EntryKind::Unmerged, Some(Added), None), Unmerged, "added by us"),
            (entry(EntryKind::Unmerged, None, Some(Deleted)), Unmerged, "deleted by them"),
        ];
        for (e, kind, what) in &cases {
            assert_eq!(WipCounts::kind(e), *kind, "{what}");
        }
        let entries: Vec<Entry> = cases.into_iter().map(|(e, _, _)| e).collect();
        let counts = WipCounts::of(&entries);
        assert_eq!(counts, WipCounts { added: 4, modified: 2, deleted: 2, renamed: 1, conflicted: 5 });
        assert_eq!((counts.total(), counts.words().as_str()), (14, "+4 ✎2 −2 →1 !5"));
    }

    #[test]
    fn the_words_leave_out_the_zero_counts() {
        assert_eq!(WipCounts { added: 1, deleted: 1, ..WipCounts::default() }.words(), "+1 −1");
        assert_eq!(WipCounts::default().words(), "");
    }
}
