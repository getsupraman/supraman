use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use crate::changes::ChangeKind;
use crate::operation::{OperationState, operation_state};
use crate::parse::{nul_records, text};
use crate::{GitError, Repo};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BranchStatus {
    /// `None` before the first commit.
    pub oid: Option<String>,
    /// `None` when detached.
    pub head: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Ordinary,
    Renamed { score: u8 },
    Copied { score: u8 },
    Unmerged,
    Untracked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub kind: EntryKind,
    pub path: String,
    /// The source of a rename or a copy.
    pub orig_path: Option<String>,
    /// Index against HEAD; for an unmerged entry, what "us" did.
    pub staged: Option<ChangeKind>,
    /// Worktree against the index; for an unmerged entry, what "them" did; `Untracked` for an untracked one.
    pub unstaged: Option<ChangeKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Merge,
    Rebase,
    CherryPick,
    Revert,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Status {
    pub branch: BranchStatus,
    pub entries: Vec<Entry>,
    pub operation: Option<Operation>,
    pub state: OperationState,
}

/// The work in progress: staged, unstaged and untracked files, and a merge or rebase under way.
pub fn status(repo: &Repo) -> Result<Status, GitError> {
    read_status(repo, &git_dir(repo)?, false)
}

/// [`status`], the first time per git dir in this process with git's optional locks, so git refreshes the
/// index's stat cache after a clone or a checkout; every later one as [`status`].
pub fn status_first(repo: &Repo) -> Result<Status, GitError> {
    static SEEN: Mutex<Option<HashSet<String>>> = Mutex::new(None);
    let git_dir = git_dir(repo)?;
    let first = SEEN.lock().unwrap_or_else(PoisonError::into_inner).get_or_insert_with(HashSet::new).insert(supraman_core::paths::path_key(&git_dir));
    read_status(repo, &git_dir, first)
}

fn git_dir(repo: &Repo) -> Result<PathBuf, GitError> {
    match repo.known_dirs() {
        Some(dirs) => Ok(dirs.git_dir.clone()),
        None => repo.read(&["rev-parse", "--absolute-git-dir"], None, |out| Ok(PathBuf::from(text(out).trim()))),
    }
}

fn read_status(repo: &Repo, git_dir: &std::path::Path, optional_locks: bool) -> Result<Status, GitError> {
    let args = ["status", "--porcelain=v2", "-z", "--untracked-files=all", "--branch", "--renames"];
    let mut status = if optional_locks { repo.read_locking(&args, parse_status)? } else { repo.read(&args, None, parse_status)? };
    status.state = operation_state(git_dir);
    status.operation = operation(status.state);
    Ok(status)
}

fn operation(state: OperationState) -> Option<Operation> {
    match state {
        OperationState::None => None,
        OperationState::Merge => Some(Operation::Merge),
        OperationState::Rebase { .. } | OperationState::Am => Some(Operation::Rebase),
        OperationState::Revert { .. } => Some(Operation::Revert),
        OperationState::CherryPick { .. } => Some(Operation::CherryPick),
    }
}

fn parse_status(out: &[u8]) -> Result<Status, String> {
    let mut status = Status::default();
    let mut records = nul_records(out).map(text);
    while let Some(record) = records.next() {
        let (kind, rest) = record.split_once(' ').ok_or_else(|| format!("status record {record:?}"))?;
        match kind {
            "#" => branch_line(&mut status.branch, rest),
            "1" | "u" => status.entries.push(changed(kind, rest)?),
            "2" => {
                let mut entry = changed(kind, rest)?;
                entry.orig_path = Some(records.next().ok_or("a rename without its source")?);
                status.entries.push(entry);
            }
            "?" => status.entries.push(Entry { kind: EntryKind::Untracked, path: rest.into(), orig_path: None, staged: None, unstaged: Some(ChangeKind::Untracked) }),
            _ => {}
        }
    }
    Ok(status)
}

fn branch_line(branch: &mut BranchStatus, line: &str) {
    let (key, value) = line.split_once(' ').unwrap_or((line, ""));
    match key {
        "branch.oid" => branch.oid = (value != "(initial)").then(|| value.into()),
        "branch.head" => branch.head = (value != "(detached)").then(|| value.into()),
        "branch.upstream" => branch.upstream = Some(value.into()),
        "branch.ab" => {
            let mut counts = value.split(' ').map(|n| n.trim_start_matches(['+', '-']).parse().unwrap_or(0));
            branch.ahead = counts.next().unwrap_or(0);
            branch.behind = counts.next().unwrap_or(0);
        }
        _ => {}
    }
}

/// `1 XY sub mH mI mW hH hI path`, `2 XY sub mH mI mW hH hI Xscore path`, `u XY sub m1 m2 m3 mW h1 h2 h3 path`.
fn changed(kind: &str, rest: &str) -> Result<Entry, String> {
    let fields = match kind {
        "1" => 7,
        "2" => 8,
        _ => 9,
    };
    let parts: Vec<&str> = rest.splitn(fields + 1, ' ').collect();
    let (Some(xy), Some(path)) = (parts.first(), parts.get(fields)) else {
        return Err(format!("status entry {kind} {rest:?}"));
    };
    let &[x, y] = xy.as_bytes() else { return Err(format!("status XY {xy:?}")) };
    let score = || parts[7].get(1..).and_then(|s| s.parse().ok()).unwrap_or(0);
    let kind = match kind {
        "1" => EntryKind::Ordinary,
        "2" if parts[7].starts_with('C') => EntryKind::Copied { score: score() },
        "2" => EntryKind::Renamed { score: score() },
        _ => EntryKind::Unmerged,
    };
    Ok(Entry { kind, path: path.to_string(), orig_path: None, staged: ChangeKind::from_letter(x), unstaged: ChangeKind::from_letter(y) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_of_entry() {
        let out = b"# branch.oid abc\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +1 -2\0\
1 MM N... 100644 100644 100644 h1 h2 a b.txt\0\
2 R. N... 100644 100644 100644 h1 h2 R100 new name\0old name\0\
u UU N... 100644 100644 100644 100644 h1 h2 h3 c.txt\0? u.txt\0";
        let status = parse_status(out).unwrap();
        assert_eq!(status.branch, BranchStatus { oid: Some("abc".into()), head: Some("main".into()), upstream: Some("origin/main".into()), ahead: 1, behind: 2 });
        let e = &status.entries;
        assert_eq!((e[0].path.as_str(), e[0].staged, e[0].unstaged), ("a b.txt", Some(ChangeKind::Modified), Some(ChangeKind::Modified)));
        assert_eq!((e[1].kind, e[1].path.as_str(), e[1].orig_path.as_deref()), (EntryKind::Renamed { score: 100 }, "new name", Some("old name")));
        assert_eq!((e[2].kind, e[2].staged), (EntryKind::Unmerged, Some(ChangeKind::Unmerged)));
        assert_eq!((e[3].kind, e[3].unstaged), (EntryKind::Untracked, Some(ChangeKind::Untracked)));
    }
}
