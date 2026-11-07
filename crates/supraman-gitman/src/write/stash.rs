//! The stash's writes (docs/research/gitkraken-12.6.1.md, "Stashing"): stash everything or some files, apply or
//! pop a stash with everything back unstaged, apply some of its files, delete it, give it a new message. None is
//! undoable: none writes a journal entry.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use super::journal::drop_noop_reset;
use super::snapshot::{TempIndex, commit_tree_call};
use super::wip::{nul_joined, remove_file, with_old_paths};
use super::{Call, Session, WriteError};
use crate::changes::{ChangeKind, FileChange};
use crate::parse::{nul_records, text};
use crate::{EntryKind, Stash};

pub const NO_CHANGES: &str = "There are no local changes to stash";
pub const NOTHING_STAGED: &str = "Nothing is staged to stash";
pub const PARTLY_STAGED: &str = "A file with both staged and unstaged changes cannot be stashed staged only";
pub const CONFLICTED: &str = "Resolve the conflicted files before stashing them";
pub const STAGED: &str = "Commit or unstage the staged changes before applying a stash";
pub const OVERWRITE: &str = "The stash changes files you have changed too: commit, stash or discard those changes first";
pub const NO_COMMIT: &str = "Make a first commit before stashing files";
pub const GONE: &str = "The stash is gone: the list was out of date";

/// How an apply or a pop ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applied {
    Clean,
    /// Conflicts are left in the work tree; the stash stays, popped or not.
    Conflicted,
}

/// HEAD's branch, or `(no branch)` on a detached HEAD, as git's stash messages name it.
fn branch_name(s: &Session) -> Result<String, WriteError> {
    let branch = text(&s.run(&Call::new(&["symbolic-ref", "-q", "--short", "HEAD"]))?.stdout).trim().to_string();
    Ok(if branch.is_empty() { "(no branch)".into() } else { branch })
}

/// `WIP on <branch>`: a stash's message when none is given.
pub fn default_message(s: &Session) -> Result<String, WriteError> {
    Ok(format!("WIP on {}", branch_name(s)?))
}

fn message_or_default(s: &Session, message: &str) -> Result<String, WriteError> {
    if message.trim().is_empty() { default_message(s) } else { Ok(message.trim().to_string()) }
}

fn has_head(s: &Session) -> Result<bool, WriteError> {
    Ok(s.run(&Call::new(&["rev-parse", "-q", "--verify", "HEAD"]))?.success())
}

/// `stash push`, then HEAD's reflog line of the reset it ran taken out, so Undo's walk goes past it.
fn push(s: &Session, call: Call) -> Result<(), WriteError> {
    s.output(&call)?;
    drop_noop_reset(s)
}

/// Stashes every change, untracked files too (all staged first), or with `staged_only` only what is staged;
/// `message` empty for [`default_message`].
pub fn stash_all(s: &Session, message: &str, staged_only: bool) -> Result<(), WriteError> {
    let status = crate::status(s.writer().repo())?;
    if status.entries.iter().any(|e| e.kind == EntryKind::Unmerged) {
        return Err(WriteError::Refused(CONFLICTED));
    }
    if status.entries.is_empty() {
        return Err(WriteError::Refused(NO_CHANGES));
    }
    if staged_only && !status.entries.iter().any(|e| e.staged.is_some()) {
        return Err(WriteError::Refused(NOTHING_STAGED));
    }
    if staged_only && status.entries.iter().any(|e| e.staged.is_some() && e.unstaged.is_some()) {
        return Err(WriteError::Refused(PARTLY_STAGED));
    }
    let message = format!("--message={}", message_or_default(s, message)?);
    if staged_only {
        return push(s, Call::new(&["stash", "push", "-q", "--staged", &message]));
    }
    s.output(&Call::new(&["add", "-A"]))?;
    push(s, Call::new(&["stash", "push", "-q", &message]))
}

/// Stashes `files` (a rename with its old path, an untracked file in the stash's untracked commit) and nothing
/// else, staged and unstaged changes alike, then puts those paths back to HEAD's; a conflicted one refuses. Built
/// with plumbing: `stash push -- <paths>` keeps every other staged change in the stash's tree.
pub fn stash_files(s: &Session, files: &[FileChange], message: &str) -> Result<(), WriteError> {
    if files.iter().any(|f| f.kind == ChangeKind::Unmerged) {
        return Err(WriteError::Refused(CONFLICTED));
    }
    if !has_head(s)? {
        return Err(WriteError::Refused(NO_COMMIT));
    }
    let top = s.writer().dirs().top.clone();
    let exists = |p: &String| top.join(p).symlink_metadata().is_ok();
    let index = staged_entries(s)?;
    let in_head = listed(s, &["ls-tree", "-r", "-z", "--full-tree", "--name-only", "HEAD"])?;
    let (tracked, untracked): (Vec<String>, Vec<String>) = with_old_paths(files).into_iter().partition(|p| index.contains_key(p) || in_head.contains(p));
    let untracked: Vec<String> = untracked.into_iter().filter(exists).collect();
    if tracked.is_empty() && untracked.is_empty() {
        return Err(WriteError::Refused(NO_CHANGES));
    }
    let (branch, short) = (branch_name(s)?, text(&s.output(&Call::new(&["rev-parse", "--short", "HEAD"]))?).trim().to_string());
    let head = text(&s.output(&Call::new(&["rev-parse", "HEAD"]))?).trim().to_string();
    let temp = s.temp_index("stash");
    s.output(&temp.on(Call::new(&["read-tree", "HEAD"])))?;
    let removed: Vec<String> = tracked.iter().filter(|p| !index.contains_key(*p)).cloned().collect();
    if !removed.is_empty() {
        s.output(&temp.on(Call::new(&["update-index", "--force-remove", "-z", "--stdin"])).input(nul_joined(&removed)))?;
    }
    let info: Vec<u8> = tracked.iter().filter_map(|p| index.get(p)).flat_map(|r| r.bytes().chain([0])).collect();
    if !info.is_empty() {
        s.output(&temp.on(Call::new(&["update-index", "-z", "--index-info"])).input(info))?;
    }
    let index_commit = commit(s, &write_tree(s, &temp)?, &[&head], &format!("index on {branch}: {short}"))?;
    let present: Vec<String> = tracked.iter().filter(|p| index.contains_key(*p) || exists(p)).cloned().collect();
    if !present.is_empty() {
        s.output(&temp.on(Call::new(&["add", "-A"]).paths(present)))?;
    }
    let work_tree = write_tree(s, &temp)?;
    let mut parents = vec![head.clone(), index_commit];
    if !untracked.is_empty() {
        let loose = s.temp_index("stash-untracked");
        s.output(&loose.on(Call::new(&["add", "-f"]).paths(untracked.clone())))?;
        parents.push(commit(s, &write_tree(s, &loose)?, &[], &format!("untracked files on {branch}: {short}"))?);
    }
    let subject = format!("On {branch}: {}", message_or_default(s, message)?);
    let parents: Vec<&str> = parents.iter().map(String::as_str).collect();
    let stash = commit(s, &work_tree, &parents, &subject)?;
    s.output(&Call::new(&["stash", "store", "-q", "-m", &subject, &stash]))?;
    put_back(s, &top, &tracked, &in_head, &untracked)
}

/// `tracked` back to HEAD's in the index and the work tree (deleted where HEAD lacks them), `untracked` deleted.
fn put_back(s: &Session, top: &Path, tracked: &[String], in_head: &HashSet<String>, untracked: &[String]) -> Result<(), WriteError> {
    let (old, new): (Vec<String>, Vec<String>) = tracked.iter().cloned().partition(|p| in_head.contains(p));
    if !old.is_empty() {
        s.output(&Call::new(&["checkout", "-q", "HEAD"]).paths(old))?;
    }
    if !new.is_empty() {
        s.output(&Call::new(&["rm", "--cached", "-q", "--ignore-unmatch"]).paths(new.clone()))?;
    }
    for path in new.iter().chain(untracked) {
        remove_file(top, path)?;
    }
    Ok(())
}

/// Each stage-0 index entry's `ls-files -s` record by its path.
fn staged_entries(s: &Session) -> Result<HashMap<String, String>, WriteError> {
    let out = s.output(&Call::new(&["ls-files", "-s", "-z"]))?;
    Ok(nul_records(&out).map(text).filter_map(|r| Some((r.split_once('\t')?.1.to_string(), r.clone()))).collect())
}

fn write_tree(s: &Session, index: &TempIndex) -> Result<String, WriteError> {
    Ok(text(&s.output(&index.on(Call::new(&["write-tree"])))?).trim().to_string())
}

fn commit(s: &Session, tree: &str, parents: &[&str], message: &str) -> Result<String, WriteError> {
    Ok(text(&s.output(&commit_tree_call(tree, parents, message))?).trim().to_string())
}

fn listed(s: &Session, args: &[&str]) -> Result<HashSet<String>, WriteError> {
    Ok(nul_records(&s.output(&Call::new(args))?).map(text).collect())
}

/// The stash whose commit is `sha`, with its place in the stash list.
fn find(s: &Session, sha: &str) -> Result<(usize, Stash), WriteError> {
    let all = crate::stashes(s.writer().repo())?;
    all.into_iter().enumerate().find(|(_, st)| st.sha == sha).ok_or(WriteError::Refused(GONE))
}

fn reflog_name(n: usize) -> String {
    format!("stash@{{{n}}}")
}

/// Applies the stash `sha` (its staged changes not kept apart) with everything back unstaged, and with `pop`
/// drops it, unless the apply left conflicts. Refused while anything is staged.
pub fn apply(s: &Session, sha: &str, pop: bool) -> Result<Applied, WriteError> {
    let head = has_head(s)?;
    if head && !s.run(&Call::new(&["diff", "--cached", "--quiet"]))?.success() {
        return Err(WriteError::Refused(STAGED));
    }
    let (n, _) = find(s, sha)?;
    let name = reflog_name(n);
    let out = s.run(&Call::new(&["stash", "apply", &name]))?;
    let said = format!("{}\n{}", text(&out.stdout), text(&out.stderr));
    if said.contains("CONFLICT") {
        return Ok(Applied::Conflicted);
    }
    if !out.success() {
        if said.contains("would be overwritten") || said.contains("already exists, no checkout") {
            return Err(WriteError::Refused(OVERWRITE));
        }
        return Err(crate::GitError::Failed { args: format!("stash apply {name}"), code: out.code, stderr: text(&out.stderr).trim().to_string() }.into());
    }
    if head {
        s.output(&Call::new(&["reset", "-q", "HEAD"]))?;
        drop_noop_reset(s)?;
    }
    if pop {
        s.output(&Call::new(&["stash", "drop", "-q", &name]))?;
    }
    Ok(Applied::Clean)
}

/// Puts `files` (and a rename's old path) in the work tree as the stash `sha` has them; the index is untouched.
pub fn apply_files(s: &Session, sha: &str, files: &[FileChange]) -> Result<(), WriteError> {
    find(s, sha)?;
    let in_stash = listed(s, &["ls-tree", "-r", "-z", "--full-tree", "--name-only", sha])?;
    let indexed = listed(s, &["ls-files", "-z"])?;
    let paths: Vec<String> = with_old_paths(files).into_iter().filter(|p| in_stash.contains(p) || indexed.contains(p)).collect();
    if paths.is_empty() {
        return Ok(());
    }
    let source = format!("--source={sha}");
    s.output(&Call::new(&["restore", "-q", "--worktree", &source]).paths(paths.clone()))?;
    if has_head(s)? {
        s.output(&Call::new(&["reset", "-q", "HEAD"]).paths(paths))?;
    }
    Ok(())
}

/// Deletes the stash `sha`.
pub fn delete(s: &Session, sha: &str) -> Result<(), WriteError> {
    let (n, _) = find(s, sha)?;
    s.output(&Call::new(&["stash", "drop", "-q", &reflog_name(n)])).map(drop)
}

/// Gives the stash `sha` a new message: the same commit stored again, now the newest stash.
pub fn set_message(s: &Session, sha: &str, message: &str) -> Result<(), WriteError> {
    let message = message.trim();
    if message.is_empty() {
        return Err(WriteError::Refused("A stash needs a message"));
    }
    let (n, stash) = find(s, sha)?;
    s.output(&Call::new(&["stash", "drop", "-q", &reflog_name(n)]))?;
    let subject = format!("On {}: {message}", stash.branch);
    s.output(&Call::new(&["stash", "store", "-q", "-m", &subject, sha])).map(drop)
}
