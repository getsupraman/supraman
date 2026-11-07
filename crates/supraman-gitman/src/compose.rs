//! What the commit area reads besides the status: whether a commit runs hooks, HEAD's message for an amend, and
//! whether a shown commit's message can be amended.

use std::path::PathBuf;

use crate::parse::text;
use crate::{GitError, OperationState, Repo, dirs, operation_state};

/// The hooks a `git commit` runs.
pub const COMMIT_HOOKS: [&str; 4] = ["pre-commit", "prepare-commit-msg", "commit-msg", "post-commit"];

/// Whether the hooks folder git uses (`core.hooksPath` or the repo's own) holds one of [`COMMIT_HOOKS`]; blocking.
pub fn commit_hooks(repo: &Repo) -> Result<bool, GitError> {
    let hooks = repo.read(&["rev-parse", "--path-format=absolute", "--git-path", "hooks"], None, |out| Ok(PathBuf::from(text(out).trim())))?;
    Ok(COMMIT_HOOKS.iter().any(|h| hooks.join(h).is_file()))
}

/// HEAD's whole message, `None` before the first commit; blocking.
pub fn head_message(repo: &Repo) -> Result<Option<String>, GitError> {
    let out = repo.run(&["log", "-1", "--format=%B", "HEAD", "--"], None)?;
    Ok(out.success().then(|| text(&out.stdout).trim_end().to_string()))
}

/// Whether a commit's message can be amended in place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Amendable {
    /// Not HEAD, a merge commit, or an operation is under way.
    No,
    /// HEAD, on no branch.
    Detached,
    /// HEAD of `branch`; `pushed` while a remote-tracking branch holds it.
    Branch { name: String, pushed: bool },
}

/// Whether `sha`'s message can be amended: it is HEAD, not a merge, with nothing under way; blocking.
pub fn amendable(repo: &Repo, sha: &str) -> Result<Amendable, GitError> {
    let head = repo.run(&["rev-parse", "-q", "--verify", "HEAD^{commit}"], None)?;
    if !head.success() || text(&head.stdout).trim() != sha {
        return Ok(Amendable::No);
    }
    if operation_state(&dirs(repo)?.git_dir) != OperationState::None || repo.run(&["rev-parse", "-q", "--verify", &format!("{sha}^2")], None)?.success() {
        return Ok(Amendable::No);
    }
    let branch = repo.run(&["symbolic-ref", "-q", "--short", "HEAD"], None)?;
    if !branch.success() {
        return Ok(Amendable::Detached);
    }
    let remotes = repo.output(&["for-each-ref", "--count=1", "--format=%(refname)", "--contains", sha, "refs/remotes"], None)?;
    Ok(Amendable::Branch { name: text(&branch.stdout).trim().to_string(), pushed: !text(&remotes).trim().is_empty() })
}
