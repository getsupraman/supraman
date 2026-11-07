//! What the commit area reads besides the status: whether a commit runs hooks, and HEAD's message for an amend.

use std::path::PathBuf;

use crate::parse::text;
use crate::{GitError, Repo};

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
