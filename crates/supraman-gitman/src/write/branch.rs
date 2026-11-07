//! Checkout, branches and tags, fast-forward and merge, and the worktrees' writes (docs/research/gitkraken-12.6.1.md
//! §4 part A). A checkout is kept for Undo, and so is a branch delete; the work in progress a write stashes is
//! always put back after it, or kept in a named stash the outcome names.

use std::path::{Path, PathBuf};

use super::journal::{self, Action};
use super::rebase::{Autostash, Autostashed, autostash, pop};
use super::{Call, Session, WriteError};
use crate::operation::{OperationState, operation_state};
use crate::parse::text;
use crate::{GitError, Head, Refs, Repo};

pub const BAD_NAME: &str = "git does not accept that name";
pub const BRANCH_EXISTS: &str = "A branch of that name exists";
pub const TAG_EXISTS: &str = "A tag of that name exists";
pub const DELETE_CURRENT: &str = "The checked-out branch cannot be deleted: check out another one first";
pub const NOT_FAST_FORWARD: &str = "The branch has commits the other does not: it cannot be fast-forwarded";
pub const IN_WORKTREE: &str = "The branch is checked out in another worktree: change it there";
pub const UNDER_WAY: &str = "Finish or abort the operation under way first";
pub const DETACHED: &str = "HEAD is detached: check out a branch first";

/// Why `name` cannot name a branch or a tag, after git's `check-ref-format`; `None` when it can.
pub fn name_problem(name: &str) -> Option<&'static str> {
    if name.is_empty() {
        return Some("Type a name");
    }
    if name == "HEAD" || name == "@" {
        return Some("HEAD and @ are git's own names");
    }
    if name.starts_with('-') {
        return Some("A name cannot start with -");
    }
    if name.starts_with('/') || name.ends_with('/') || name.contains("//") {
        return Some("A name cannot start or end with / or hold //");
    }
    if name.ends_with('.') {
        return Some("A name cannot end with .");
    }
    if name.contains("..") || name.contains("@{") {
        return Some("A name cannot hold .. or @{");
    }
    if let Some(c) = name.chars().find(|c| c.is_control() || matches!(c, ' ' | '~' | '^' | ':' | '\\' | '?' | '[' | '*')) {
        return Some(match c {
            ' ' => "A name cannot hold a space",
            c if c.is_control() => "A name cannot hold a control character",
            _ => "A name cannot hold ~ ^ : \\ ? [ or *",
        });
    }
    if name.split('/').any(|part| part.starts_with('.') || part.ends_with(".lock")) {
        return Some("No part of a name can start with . or end with .lock");
    }
    None
}

fn checked_name(s: &Session, name: &str) -> Result<(), WriteError> {
    if let Some(problem) = name_problem(name) {
        return Err(WriteError::Refused(problem));
    }
    if !s.run(&Call::new(&["check-ref-format", "--branch", name]))?.success() {
        return Err(WriteError::Refused(BAD_NAME));
    }
    Ok(())
}

fn exists(s: &Session, full: &str) -> Result<bool, WriteError> {
    Ok(s.run(&Call::new(&["rev-parse", "-q", "--verify", full]))?.success())
}

fn rev(s: &Session, name: &str) -> Result<String, WriteError> {
    Ok(text(&s.output(&Call::new(&["rev-parse", "--verify", &format!("{name}^{{commit}}")]))?).trim().to_string())
}

/// HEAD's branch, `None` when detached.
fn current(s: &Session) -> Result<Option<String>, WriteError> {
    let name = text(&s.run(&Call::new(&["symbolic-ref", "-q", "--short", "HEAD"]))?.stdout).trim().to_string();
    Ok((!name.is_empty()).then_some(name))
}

fn idle(s: &Session) -> Result<(), WriteError> {
    match operation_state(&s.writer().dirs().git_dir) {
        OperationState::None => Ok(()),
        _ => Err(WriteError::Refused(UNDER_WAY)),
    }
}

/// What a checkout of a ref does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Branch(String),
    /// `origin/main`.
    Remote(String),
    /// A commit or a tag's commit, checked out detached.
    Commit(String),
}

/// A checkout decided ([`plan`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// It is checked out already.
    Head,
    /// Another worktree has the branch checked out: that folder.
    Worktree(PathBuf),
    Branch(String),
    /// A new local branch `name` tracking `remote`.
    Track { remote: String, name: String },
    /// The local branch `name`, behind `remote` which it tracks, moved to it (`sha`) and checked out.
    Forward { name: String, remote: String, sha: String },
    /// The local branch `name` and `remote` went apart: the user picks a new name or a reset to `sha`.
    Clash { name: String, remote: String, sha: String },
    Detached(String),
}

/// `remote`'s branch name below its remote: `main` for `origin/main`.
pub fn local_name<'a>(refs: &Refs, remote: &'a str) -> &'a str {
    let remotes = refs.remotes.iter().filter(|r| r.name == remote).count();
    let cut = remote.split_once('/').map_or(remote, |(_, rest)| rest);
    if remotes > 0 { cut } else { remote }
}

fn is_ancestor(repo: &Repo, a: &str, b: &str) -> Result<bool, GitError> {
    Ok(repo.run(&["merge-base", "--is-ancestor", a, b], None)?.success())
}

/// How `a` stands to `b`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relation {
    Same,
    /// `a` is an ancestor of `b`: `a` can fast-forward to `b`.
    Behind,
    /// `b` is an ancestor of `a`.
    Ahead,
    Diverged,
}

/// How the commit `a` stands to `b`; blocking.
pub fn relation(repo: &Repo, a: &str, b: &str) -> Result<Relation, GitError> {
    if a == b {
        return Ok(Relation::Same);
    }
    if is_ancestor(repo, a, b)? {
        return Ok(Relation::Behind);
    }
    if is_ancestor(repo, b, a)? {
        return Ok(Relation::Ahead);
    }
    Ok(Relation::Diverged)
}

/// The worktree other than `top` that has `branch` checked out.
pub fn held_elsewhere(refs: &Refs, top: &Path, branch: &str) -> Option<PathBuf> {
    let here = supraman_core::paths::path_key(top);
    refs.worktrees.iter().find(|w| w.branch.as_deref() == Some(branch) && supraman_core::paths::path_key(&w.path) != here).map(|w| w.path.clone())
}

/// What a checkout of `target` in the checkout at `top` does; blocking.
pub fn plan(repo: &Repo, refs: &Refs, top: &Path, target: &Target) -> Result<Plan, GitError> {
    let (head_branch, head_sha) = match &refs.head {
        Head::Branch { name, sha } => (Some(name.as_str()), sha.as_deref()),
        Head::Detached { sha } => (None, Some(sha.as_str())),
    };
    let branch = |name: &str| refs.branches.iter().find(|b| b.name == name);
    let local = |name: &str| -> Plan {
        if head_branch == Some(name) {
            return Plan::Head;
        }
        match held_elsewhere(refs, top, name) {
            Some(path) => Plan::Worktree(path),
            None => Plan::Branch(name.to_string()),
        }
    };
    Ok(match target {
        Target::Branch(name) => local(name),
        Target::Commit(sha) if head_branch.is_none() && head_sha == Some(sha.as_str()) => Plan::Head,
        Target::Commit(sha) => Plan::Detached(sha.clone()),
        Target::Remote(remote) => {
            let Some(sha) = refs.remotes.iter().find(|r| &r.name == remote).map(|r| r.sha.clone()) else {
                return Err(GitError::Parse { args: "checkout".into(), what: format!("no remote branch {remote}") });
            };
            let name = local_name(refs, remote).to_string();
            let Some(own) = branch(&name) else { return Ok(Plan::Track { remote: remote.clone(), name }) };
            if let Some(path) = held_elsewhere(refs, top, &name) {
                return Ok(Plan::Worktree(path));
            }
            if own.sha == sha {
                return Ok(local(&name));
            }
            let tracks = own.upstream.as_ref().is_some_and(|u| &u.name == remote);
            if tracks && is_ancestor(repo, &own.sha, &sha)? {
                Plan::Forward { name, remote: remote.clone(), sha }
            } else {
                Plan::Clash { name, remote: remote.clone(), sha }
            }
        }
    })
}

/// How a checkout ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckedOut {
    Done(Autostash),
    /// git refused: the checkout would overwrite local changes in `files` files. Nothing changed.
    WouldOverwrite { files: usize },
}

impl Plan {
    /// The words a toast and a stash name it by.
    pub fn target(&self) -> String {
        match self {
            Plan::Head | Plan::Worktree(_) => String::new(),
            Plan::Branch(name) | Plan::Track { name, .. } | Plan::Forward { name, .. } | Plan::Clash { name, .. } => name.clone(),
            Plan::Detached(sha) => crate::history::short_sha(sha).to_string(),
        }
    }

    fn call(&self) -> Option<Call> {
        Some(match self {
            Plan::Branch(name) => Call::new(&["checkout", "-q", name, "--"]),
            Plan::Track { remote, name } => Call::new(&["checkout", "-q", "-b", name, "--track", remote, "--"]),
            Plan::Forward { name, sha, .. } => Call::new(&["checkout", "-q", "-B", name, sha, "--"]),
            Plan::Detached(sha) => Call::new(&["checkout", "-q", "--detach", sha, "--"]),
            Plan::Head | Plan::Worktree(_) | Plan::Clash { .. } => return None,
        })
    }

    /// A reset of a branch or a detached checkout stashes the work in progress first, and puts it back after.
    fn always_stashes(&self) -> bool {
        matches!(self, Plan::Forward { .. } | Plan::Detached(_))
    }
}

/// The checkout `plan` says, recorded for Undo. A branch's checkout that git refuses over local changes is
/// [`CheckedOut::WouldOverwrite`] unless `stash`; with `stash` (and always for a reset or a detached checkout) the
/// work in progress is stashed first and put back after, a conflict keeping the stash.
pub fn checkout(s: &Session, plan: &Plan, stash: bool) -> Result<CheckedOut, WriteError> {
    idle(s)?;
    let Some(call) = plan.call() else { return Err(WriteError::Refused("Nothing to check out")) };
    let stashed = if stash || plan.always_stashes() { autostash(s, &format!("GitMan: before checking out {}", plan.target()))? } else { None };
    let out = s.run(&call)?;
    if !out.success() {
        let stderr = text(&out.stderr);
        let kept = put_back(s, stashed.as_ref());
        if stashed.is_none() && stderr.contains("would be overwritten by checkout") {
            return Ok(CheckedOut::WouldOverwrite { files: stderr.lines().filter(|l| l.starts_with('\t')).count() });
        }
        return Err(GitError::Failed { args: "checkout".into(), code: out.code, stderr: [stderr.trim().to_string()].into_iter().chain(kept).collect::<Vec<_>>().join("\n") }.into());
    }
    journal::record(s, Action::Checkout, None)?;
    Ok(CheckedOut::Done(pop(s, stashed.as_ref())))
}

/// `stashed` popped back after a write that did not happen; the words to add to its error when it stays.
fn put_back(s: &Session, stashed: Option<&Autostashed>) -> Option<String> {
    match pop(s, stashed) {
        Autostash::Kept { name, why } => Some(format!("Your changes stay in the stash \"{name}\": {why}")),
        _ => None,
    }
}

/// A new branch `name` at `at` (a commit or a ref); not checked out.
pub fn create_branch(s: &Session, name: &str, at: &str) -> Result<(), WriteError> {
    checked_name(s, name)?;
    if exists(s, &format!("refs/heads/{name}"))? {
        return Err(WriteError::Refused(BRANCH_EXISTS));
    }
    let at = rev(s, at)?;
    s.output(&Call::new(&["branch", "--no-track", name, &at])).map(drop)
}

/// The local branch `from` renamed `to`, its upstream and reflog kept.
pub fn rename_branch(s: &Session, from: &str, to: &str) -> Result<(), WriteError> {
    checked_name(s, to)?;
    if exists(s, &format!("refs/heads/{to}"))? {
        return Err(WriteError::Refused(BRANCH_EXISTS));
    }
    s.output(&Call::new(&["branch", "-m", from, to])).map(drop)
}

/// Deletes the local branch `name`, kept for Undo; refused for HEAD's branch.
pub fn delete_branch(s: &Session, name: &str) -> Result<(), WriteError> {
    if current(s)?.as_deref() == Some(name) {
        return Err(WriteError::Refused(DELETE_CURRENT));
    }
    journal::delete_branch(s, name)
}

/// Whether the local branch `name` has no upstream and commits no other branch, remote branch or tag reaches:
/// a delete loses them; blocking.
pub fn only_here(repo: &Repo, name: &str) -> Result<bool, GitError> {
    if repo.run(&["config", "--get", &format!("branch.{name}.remote")], None)?.success() {
        return Ok(false);
    }
    let own = format!("refs/heads/{name}");
    let out = repo.run(&["rev-list", "--count", &own, "--not", &format!("--exclude={name}"), "--branches", "--remotes", "--tags"], None)?;
    Ok(text(&out.stdout).trim().parse::<u64>().unwrap_or(0) > 0)
}

/// A tag `name` at `at`: annotated with `message`, else lightweight; `force` replaces one of that name.
pub fn create_tag(s: &Session, name: &str, at: &str, message: Option<&str>, force: bool) -> Result<(), WriteError> {
    checked_name(s, name)?;
    if !force && exists(s, &format!("refs/tags/{name}"))? {
        return Err(WriteError::Refused(TAG_EXISTS));
    }
    let at = rev(s, at)?;
    let mut args = match message {
        Some(_) => vec!["tag", "--create-reflog", "--annotate", "-F", "-"],
        None => vec!["-c", "tag.gpgSign=false", "tag", "--create-reflog"],
    };
    if force {
        args.push("--force");
    }
    args.extend([name, at.as_str()]);
    let call = Call::new(&args).editor("true");
    let call = match message {
        Some(message) => call.input(message.as_bytes().to_vec()),
        None => call,
    };
    s.output(&call).map(drop)
}

/// Deletes the local tag `name`; not undoable.
pub fn delete_tag(s: &Session, name: &str) -> Result<(), WriteError> {
    s.output(&Call::new(&["tag", "-d", name])).map(drop)
}

/// Moves the local branch `branch` forward to `to`: HEAD's by `merge --ff-only`, another's ref alone, with no
/// checkout. Refused when `to` is not ahead of it, or another worktree has it checked out.
pub fn fast_forward(s: &Session, branch: &str, to: &str) -> Result<(), WriteError> {
    idle(s)?;
    let to = rev(s, to)?;
    if current(s)?.as_deref() == Some(branch) {
        return s.output(&Call::new(&["merge", "--ff-only", "-q", &to])).map(drop);
    }
    let refs = crate::refs(s.writer().repo())?;
    if held_elsewhere(&refs, &s.writer().dirs().top, branch).is_some() {
        return Err(WriteError::Refused(IN_WORKTREE));
    }
    let full = format!("refs/heads/{branch}");
    let old = rev(s, &full)?;
    if !is_ancestor(s.writer().repo(), &old, &to)? {
        return Err(WriteError::Refused(NOT_FAST_FORWARD));
    }
    s.output(&Call::new(&["update-ref", "-m", &format!("fast-forward: {branch} to {}", crate::history::short_sha(&to)), &full, &to, &old])).map(drop)
}

/// Where a merge into a branch other than HEAD's left the checkout while its conflicts wait: what was checked
/// out before, and the work in progress stashed then.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Away {
    pub back: String,
    pub autostash: Option<Autostashed>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Merged {
    UpToDate,
    Done(Autostash),
    /// Stopped on a conflict, the merge under way; `away` when it was checked out for it.
    Conflicted { away: Option<Away> },
}

fn merge_call(source: &str) -> Call {
    Call::new(&["merge", "--no-ff", "--no-edit", "-q", source]).editor("true")
}

fn conflicted(s: &Session) -> bool {
    operation_state(&s.writer().dirs().git_dir) == OperationState::Merge
}

/// `source` merged into `into` with a merge commit (`--no-ff`): HEAD's branch in place; another branch checked
/// out for it with the work in progress stashed, then the checkout before put back and the stash popped.
pub fn merge(s: &Session, source: &str, into: &str) -> Result<Merged, WriteError> {
    idle(s)?;
    let source_sha = rev(s, source)?;
    let into_sha = rev(s, &format!("refs/heads/{into}"))?;
    if is_ancestor(s.writer().repo(), &source_sha, &into_sha)? {
        return Ok(Merged::UpToDate);
    }
    let here = current(s)?;
    if here.as_deref() == Some(into) {
        return merge_here(s, source, None);
    }
    let refs = crate::refs(s.writer().repo())?;
    if held_elsewhere(&refs, &s.writer().dirs().top, into).is_some() {
        return Err(WriteError::Refused(IN_WORKTREE));
    }
    let back = match here {
        Some(branch) => branch,
        None => rev(s, "HEAD")?,
    };
    let stashed = autostash(s, &format!("GitMan: before merging into {into}"))?;
    let out = s.run(&Call::new(&["checkout", "-q", into, "--"]))?;
    if !out.success() {
        let kept = put_back(s, stashed.as_ref());
        return Err(GitError::Failed { args: "checkout".into(), code: out.code, stderr: [text(&out.stderr).trim().to_string()].into_iter().chain(kept).collect::<Vec<_>>().join("\n") }.into());
    }
    merge_here(s, source, Some(Away { back, autostash: stashed }))
}

fn merge_here(s: &Session, source: &str, away: Option<Away>) -> Result<Merged, WriteError> {
    let out = s.run(&merge_call(source))?;
    if !out.success() {
        if conflicted(s) {
            return Ok(Merged::Conflicted { away });
        }
        let failed: WriteError = GitError::Failed { args: "merge".into(), code: out.code, stderr: format!("{}\n{}", text(&out.stderr).trim(), text(&out.stdout).trim()).trim().to_string() }.into();
        if let Some(away) = away {
            go_back(s, &away)?;
        }
        return Err(failed);
    }
    match away {
        None => Ok(Merged::Done(Autostash::None)),
        Some(away) => go_back(s, &away).map(Merged::Done),
    }
}

/// The checkout before an [`Away`] put back and its stash popped.
fn go_back(s: &Session, away: &Away) -> Result<Autostash, WriteError> {
    let detached = !exists(s, &format!("refs/heads/{}", away.back))?;
    let args: Vec<&str> = if detached { vec!["checkout", "-q", "--detach", &away.back, "--"] } else { vec!["checkout", "-q", &away.back, "--"] };
    s.output(&Call::new(&args))?;
    Ok(pop(s, away.autostash.as_ref()))
}

/// A conflicted merge into another branch given up: `merge --abort`, the checkout before back, its stash popped.
pub fn abandon(s: &Session, away: &Away) -> Result<Autostash, WriteError> {
    s.output(&Call::new(&["merge", "--abort"]))?;
    go_back(s, away)
}

/// Removes the worktree at `path` with what it holds (`--force`), then deletes `branch` when given.
pub fn remove_worktree(s: &Session, path: &Path, branch: Option<&str>) -> Result<(), WriteError> {
    let path = path.to_string_lossy();
    s.output(&Call::new(&["worktree", "remove", "--force", &path]))?;
    match branch {
        Some(branch) => journal::delete_branch(s, branch),
        None => Ok(()),
    }
}

/// Locks or unlocks the worktree at `path`.
pub fn lock_worktree(s: &Session, path: &Path, lock: bool) -> Result<(), WriteError> {
    let path = path.to_string_lossy();
    s.output(&Call::new(&["worktree", if lock { "lock" } else { "unlock" }, &path])).map(drop)
}

/// Removes what git keeps of worktrees whose folders are gone.
pub fn prune_worktrees(s: &Session) -> Result<(), WriteError> {
    s.output(&Call::new(&["worktree", "prune"])).map(drop)
}

/// Merge, fast-forward and rebase need a branch checked out.
pub fn current_branch(refs: &Refs) -> Option<&str> {
    match &refs.head {
        Head::Branch { name, sha: Some(_) } => Some(name),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_check_ref_format() {
        for good in ["feature/x", "v1.2", "a-b_c", "über"] {
            assert_eq!(name_problem(good), None, "{good}");
        }
        for bad in ["", "HEAD", "@", "-x", "/x", "x/", "a//b", "x.", "a..b", "a@{b", "a b", "a~b", "a^b", "a:b", "a\\b", "a?b", "a[b", "a*b", ".x", "a/.b", "x.lock", "a/b.lock/c", "a\u{7f}b", "a\tb"] {
            assert!(name_problem(bad).is_some(), "{bad:?} is refused");
        }
    }
}
