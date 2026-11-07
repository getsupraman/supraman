//! A commit's writes from the graph's menus (docs/research/gitkraken-12.6.1.md §B B1): cherry-pick and revert
//! (`--mainline 1` for a merge, the work in progress stashed before and popped after; neither undoable), reset
//! soft, mixed or hard (a hard one snapshots first; all recorded for Undo), and a commit's patch.

use std::path::{Path, PathBuf};

use super::journal::{self, Action};
use super::rebase::{Autostash, Autostashed, autostash, pop};
use super::{Call, Session, WriteError};
use crate::operation::{OperationState, operation_state};
use crate::parse::text;
use crate::{GitError, Repo};

pub const UNDER_WAY: &str = "Finish or abort the operation under way first";

/// What a reset keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetMode {
    /// Every change, staged.
    Soft,
    /// The working files; the index reset.
    Mixed,
    /// Nothing: every change discarded.
    Hard,
}

impl ResetMode {
    fn flag(self) -> &'static str {
        match self {
            ResetMode::Soft => "--soft",
            ResetMode::Mixed => "--mixed",
            ResetMode::Hard => "--hard",
        }
    }
}

/// How a cherry-pick or a revert ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Picked {
    Done(Autostash),
    /// Stopped on a conflict, the operation under way; the stash of the work in progress kept, when one was made.
    Conflicted(Option<Autostashed>),
}

/// How many parents `sha` has; blocking.
pub fn parent_count(repo: &Repo, sha: &str) -> Result<usize, GitError> {
    let out = repo.output(&["rev-list", "--parents", "-n", "1", sha], None)?;
    Ok(text(&out).split_whitespace().count().saturating_sub(1))
}

/// Whether `sha` has more than one parent; blocking.
pub fn is_merge(repo: &Repo, sha: &str) -> Result<bool, GitError> {
    Ok(parent_count(repo, sha)? > 1)
}

fn idle(s: &Session) -> Result<(), WriteError> {
    match operation_state(&s.writer().dirs().git_dir) {
        OperationState::None => Ok(()),
        _ => Err(WriteError::Refused(UNDER_WAY)),
    }
}

/// `git cherry-pick <sha>`; with `commit` false its changes are left staged (`--no-commit`) on top of the work in
/// progress, which is then not stashed.
pub fn cherry_pick(s: &Session, sha: &str, commit: bool) -> Result<Picked, WriteError> {
    apply_commit(s, "cherry-pick", sha, commit)
}

/// `git revert <sha>`, as [`cherry_pick`].
pub fn revert(s: &Session, sha: &str, commit: bool) -> Result<Picked, WriteError> {
    apply_commit(s, "revert", sha, commit)
}

fn apply_commit(s: &Session, verb: &str, sha: &str, commit: bool) -> Result<Picked, WriteError> {
    idle(s)?;
    let mut args = vec![verb];
    args.push(if commit { "--no-edit" } else { "--no-commit" });
    if is_merge(s.writer().repo(), sha)? {
        args.extend(["--mainline", "1"]);
    }
    args.push(sha);
    let stashed = if commit { autostash(s, &format!("GitMan: before the {verb} of {}", crate::history::short_sha(sha)))? } else { None };
    let out = s.run(&Call::new(&args).editor("true"))?;
    if out.success() {
        return Ok(Picked::Done(pop(s, stashed.as_ref())));
    }
    let under_way = operation_state(&s.writer().dirs().git_dir) != OperationState::None;
    if under_way && conflicted(s)? {
        return Ok(Picked::Conflicted(stashed));
    }
    if under_way {
        s.run(&Call::new(&[verb, "--abort"]))?;
    }
    let said = [text(&out.stderr), text(&out.stdout)].map(|t| t.trim().to_string());
    let kept = match pop(s, stashed.as_ref()) {
        Autostash::Kept { name, why } => Some(format!("Your changes stay in the stash \"{name}\": {why}")),
        _ => None,
    };
    let stderr = said.into_iter().chain(kept).filter(|t| !t.is_empty()).collect::<Vec<_>>().join("\n");
    Err(GitError::Failed { args: verb.to_string(), code: out.code, stderr }.into())
}

fn conflicted(s: &Session) -> Result<bool, WriteError> {
    Ok(!s.output(&Call::new(&["diff", "--name-only", "--diff-filter=U"]))?.is_empty())
}

/// HEAD (and its branch) reset to `sha`; a hard reset snapshots the work in progress first. Recorded for Undo.
pub fn reset(s: &Session, sha: &str, mode: ResetMode) -> Result<(), WriteError> {
    idle(s)?;
    let snapshot = match mode {
        ResetMode::Hard => {
            let status = crate::status(s.writer().repo())?;
            let paths: Vec<&str> = status.entries.iter().flat_map(|e| [Some(e.path.as_str()), e.orig_path.as_deref()]).flatten().collect();
            Some(s.snapshot(&paths, "Before reset")?)
        }
        _ => None,
    };
    s.output(&Call::new(&["reset", "-q", mode.flag(), sha]))?;
    journal::record(s, Action::Reset { hard: mode == ResetMode::Hard }, snapshot.as_deref())
}

/// `sha`'s patch (`git format-patch -1`) written into `folder`; its file. Blocking; the repository is not written.
pub fn format_patch(repo: &Repo, sha: &str, folder: &Path) -> Result<PathBuf, GitError> {
    let folder_text = folder.to_string_lossy();
    let out = repo.run(&["format-patch", "-1", "--no-signature", "-o", &folder_text, sha], None)?;
    if !out.success() {
        return Err(GitError::Failed { args: "format-patch".into(), code: out.code, stderr: text(&out.stderr).trim().to_string() });
    }
    let name = text(&out.stdout).lines().last().unwrap_or_default().trim().to_string();
    Ok(PathBuf::from(name))
}
