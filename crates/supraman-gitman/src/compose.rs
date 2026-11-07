//! What the commit area reads besides the status: whether a commit runs hooks, HEAD's message for an amend, and
//! whether a shown commit's message can be amended or reworded.

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

/// Whether a commit's message can be amended in place, or reworded by a rebase of the commits above it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Amendable {
    /// Not HEAD nor below it on a reword's terms, a merge commit, or an operation is under way.
    No,
    /// HEAD, on no branch.
    Detached,
    /// HEAD of `branch`; `pushed` while a remote-tracking branch holds it.
    Branch { name: String, pushed: bool },
    /// Below HEAD of `branch` on [`rewordable`]'s terms: a reword rebases the `above` commits over it; `pushed`
    /// while a remote-tracking branch holds it.
    Older { name: String, above: usize, pushed: bool },
    /// Below a HEAD on no branch, on [`rewordable`]'s terms.
    OlderDetached,
}

/// Whether `sha`'s message can be amended: it is HEAD, or an older commit [`rewordable`], with nothing under
/// way; blocking.
pub fn amendable(repo: &Repo, sha: &str) -> Result<Amendable, GitError> {
    let head = repo.run(&["rev-parse", "-q", "--verify", "HEAD^{commit}"], None)?;
    if !head.success() || operation_state(&dirs(repo)?.git_dir) != OperationState::None {
        return Ok(Amendable::No);
    }
    let above = if text(&head.stdout).trim() == sha {
        if repo.run(&["rev-parse", "-q", "--verify", &format!("{sha}^2")], None)?.success() {
            return Ok(Amendable::No);
        }
        None
    } else {
        match rewordable(repo, sha)? {
            Some(above) => Some(above),
            None => return Ok(Amendable::No),
        }
    };
    let branch = repo.run(&["symbolic-ref", "-q", "--short", "HEAD"], None)?;
    if !branch.success() {
        return Ok(if above.is_some() { Amendable::OlderDetached } else { Amendable::Detached });
    }
    let name = text(&branch.stdout).trim().to_string();
    let remotes = repo.output(&["for-each-ref", "--count=1", "--format=%(refname)", "--contains", sha, "refs/remotes"], None)?;
    let pushed = !text(&remotes).trim().is_empty();
    Ok(match above {
        Some(above) => Amendable::Older { name, above, pushed },
        None => Amendable::Branch { name, pushed },
    })
}

/// How many commits a reword of `sha` rebases: it is below HEAD on HEAD's first-parent chain, and it and every
/// commit from it up to HEAD has exactly one parent; `None` for HEAD, a merge, the root, anything below a merge or
/// off the chain; blocking.
pub fn rewordable(repo: &Repo, sha: &str) -> Result<Option<usize>, GitError> {
    let parents = repo.run(&["rev-parse", &format!("{sha}^@")], None)?;
    let parents = text(&parents.stdout);
    let [parent] = parents.lines().collect::<Vec<_>>()[..] else { return Ok(None) };
    if !repo.run(&["merge-base", "--is-ancestor", sha, "HEAD"], None)?.success() {
        return Ok(None);
    }
    let chain = repo.output(&["rev-list", "--first-parent", "--parents", "HEAD", &format!("^{parent}")], None)?;
    let chain = text(&chain);
    let lines: Vec<Vec<&str>> = chain.lines().map(|l| l.split(' ').collect()).collect();
    let linked = lines.iter().all(|l| l.len() == 2) && lines.windows(2).all(|w| w[0][1] == w[1][0]);
    let ends_at = lines.last().is_some_and(|l| l[0] == sha);
    Ok((linked && ends_at && lines.len() > 1).then(|| lines.len() - 1))
}
