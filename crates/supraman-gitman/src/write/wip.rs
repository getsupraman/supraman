//! The WIP panel's writes (docs/research/gitkraken-12.6.1.md, "Staging" and "Committing"): stage, unstage, the
//! discard of the Unstaged list, discard all, commit, a new message for HEAD, and a merge's or rebase's next step.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use super::journal::{self, Action, Kind};
use super::conflicts::{self, Sequence};
use super::rebase::{self, Autostash, Rebased};
use super::{Call, Session, WriteError};
use crate::changes::{ChangeKind, FileChange};
use crate::parse::{nul_records, text};

/// Each file's path and a rename's old path, once each, in order.
pub(super) fn with_old_paths(files: &[FileChange]) -> Vec<String> {
    let mut seen = HashSet::new();
    files.iter().flat_map(|f| [Some(&f.path), f.old_path.as_ref()]).flatten().filter(|p| seen.insert(p.as_str())).cloned().collect()
}

fn indexed(s: &Session) -> Result<HashSet<String>, WriteError> {
    Ok(nul_records(&s.output(&Call::new(&["ls-files", "-z"]))?).map(text).collect())
}

pub(super) fn nul_joined(paths: &[String]) -> Vec<u8> {
    paths.iter().flat_map(|p| p.bytes().chain([0])).collect()
}

fn config_true(s: &Session, key: &str) -> Result<bool, WriteError> {
    let out = s.run(&Call::new(&["config", "--type=bool", "--get", key]))?;
    Ok(out.success() && text(&out.stdout).trim() == "true")
}

fn has_head(s: &Session) -> Result<bool, WriteError> {
    Ok(s.run(&Call::new(&["rev-parse", "-q", "--verify", "HEAD"]))?.success())
}

/// Stages `files` (and a rename's old path) whatever their change, untracked and conflicted ones too; a conflicted
/// file's `.orig` leftover goes.
pub fn stage(s: &Session, files: &[FileChange]) -> Result<(), WriteError> {
    let top = s.writer().dirs().top.clone();
    let index = indexed(s)?;
    let paths: Vec<String> = with_old_paths(files).into_iter().filter(|p| index.contains(p) || top.join(p).symlink_metadata().is_ok()).collect();
    if paths.is_empty() {
        return Ok(());
    }
    let now_folders: Vec<String> = paths.iter().filter(|p| index.contains(*p) && top.join(p).is_dir()).cloned().collect();
    if !now_folders.is_empty() {
        s.output(&Call::new(&["update-index", "--remove", "-z", "--stdin"]).input(nul_joined(&now_folders)))?;
    }
    let mut args = vec!["add"];
    if config_true(s, "core.sparseCheckout")? {
        args.push("--sparse");
    }
    s.output(&Call::new(&args).paths(paths))?;
    for f in files.iter().filter(|f| f.kind == ChangeKind::Unmerged) {
        let orig = format!("{}.orig", f.path);
        if !index.contains(&orig) {
            remove_file(&top, &orig)?;
        }
    }
    Ok(())
}

/// The index flags `unstage` keeps: skip-worktree, assume-unchanged.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Flags {
    skip_worktree: bool,
    assume_unchanged: bool,
}

/// `ls-files -v`'s tag letter of each path that has a flag.
fn flags(s: &Session) -> Result<HashMap<String, Flags>, WriteError> {
    let out = s.output(&Call::new(&["ls-files", "-v", "-z"]))?;
    Ok(nul_records(&out)
        .map(text)
        .filter_map(|r| {
            let (tag, path) = r.split_once(' ')?;
            let tag = tag.chars().next()?;
            let flags = Flags { skip_worktree: tag.eq_ignore_ascii_case(&'S'), assume_unchanged: tag.is_ascii_lowercase() };
            (flags != Flags::default()).then(|| (path.to_string(), flags))
        })
        .collect())
}

/// Whether one of `paths` holds another, which `restore --staged` mishandles.
fn nested(paths: &[String]) -> bool {
    paths.iter().any(|p| paths.iter().any(|q| q.len() > p.len() && q.starts_with(p.as_str()) && q.as_bytes()[p.len()] == b'/'))
}

/// The command that puts `paths`' index entries back to HEAD's (`head` false: before the first commit).
fn unstage_args(paths: &[String], head: bool) -> &'static [&'static str] {
    match (head, nested(paths)) {
        (false, _) => &["rm", "--cached", "-r", "-q"],
        (true, true) => &["reset", "-q"],
        (true, false) => &["restore", "--staged"],
    }
}

/// Unstages `files` (and a rename's old path), keeping their skip-worktree and assume-unchanged flags.
pub fn unstage(s: &Session, files: &[FileChange]) -> Result<(), WriteError> {
    let paths = with_old_paths(files);
    if paths.is_empty() {
        return Ok(());
    }
    let before = flags(s)?;
    s.output(&Call::new(unstage_args(&paths, has_head(s)?)).paths(paths.clone()))?;
    let index = indexed(s)?;
    let kept: Vec<(&String, Flags)> = paths.iter().filter(|p| index.contains(*p)).filter_map(|p| before.get(p).map(|f| (p, *f))).collect();
    let kinds: [(&str, fn(Flags) -> bool); 2] = [("--skip-worktree", |f| f.skip_worktree), ("--assume-unchanged", |f| f.assume_unchanged)];
    for (flag, on) in kinds {
        let paths: Vec<String> = kept.iter().filter(|(_, f)| on(*f)).map(|(p, _)| p.to_string()).collect();
        if !paths.is_empty() {
            s.output(&Call::new(&["update-index", flag, "-z", "--stdin"]).input(nul_joined(&paths)))?;
        }
    }
    Ok(())
}

/// Discards the Unstaged list's `files`: a modified file goes back to the index's, a deleted or conflicted one to
/// HEAD's, an untracked one is deleted; a snapshot first and a journal entry after ([`journal::discard_from_index`]).
pub fn discard_unstaged(s: &Session, files: &[FileChange]) -> Result<(), WriteError> {
    let paths = with_old_paths(files);
    let from_index: Vec<String> = files.iter().filter(|f| !matches!(f.kind, ChangeKind::Deleted | ChangeKind::Unmerged | ChangeKind::Untracked)).map(|f| f.path.clone()).collect();
    journal::discard_from_index(s, &paths, &from_index)
}

/// Throws away every change: staged, unstaged and untracked files (ignored ones stay), kept first in a snapshot
/// and recorded; how many paths it discarded.
pub fn discard_all(s: &Session) -> Result<usize, WriteError> {
    let status = crate::status(s.writer().repo())?;
    let mut seen = HashSet::new();
    let paths: Vec<String> = status.entries.iter().flat_map(|e| [Some(&e.path), e.orig_path.as_ref()]).flatten().filter(|p| seen.insert(p.as_str())).cloned().collect();
    if paths.is_empty() {
        return Ok(0);
    }
    let untracked: Vec<&String> = status.entries.iter().filter(|e| e.kind == crate::EntryKind::Untracked).map(|e| &e.path).collect();
    let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
    let snapshot = s.snapshot(&refs, "Before discard all")?;
    let top = s.writer().dirs().top.clone();
    let mut args = vec!["reset", "-q", "--hard"];
    if top.join(".gitmodules").exists() {
        args.push("--recurse-submodules");
    }
    args.push("HEAD");
    s.output(&Call::new(&args))?;
    journal::drop_noop_reset(s)?;
    for path in untracked {
        remove_file(&top, path)?;
    }
    journal::record(s, Action::Discard { paths: paths.clone(), from_index: Vec::new(), left: Vec::new() }, Some(&snapshot))?;
    Ok(paths.len())
}

/// Deletes `path` under `top` when it is there, then each folder above it it leaves empty.
pub(super) fn remove_file(top: &Path, path: &str) -> Result<(), WriteError> {
    let file = top.join(path);
    match std::fs::remove_file(&file) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => return Err(WriteError::Io { what: format!("remove {}", file.display()), source }),
    }
    let mut folder = file.parent();
    while let Some(dir) = folder.filter(|d| *d != top && d.starts_with(top)) {
        if std::fs::remove_dir(dir).is_err() {
            break;
        }
        folder = dir.parent();
    }
    Ok(())
}

/// A commit's message: the summary, a blank line and the description when there is one, a newline at the end.
pub fn message(summary: &str, description: &str) -> String {
    let (summary, description) = (summary.trim_end(), description.trim_end());
    if description.trim().is_empty() { format!("{summary}\n") } else { format!("{summary}\n\n{description}\n") }
}

/// Commits what is staged with `message`, or with `amend` replaces HEAD by it; `no_verify` skips the hooks. A
/// plain commit or amend is recorded for Undo; a merge's is not.
pub fn commit(s: &Session, message: &str, amend: bool, no_verify: bool) -> Result<(), WriteError> {
    let args: &[&str] = if amend { &["--amend"] } else { &[] };
    run_commit(s, message, args, no_verify)
}

/// Replaces HEAD's message by `message`, its tree, author and date kept and what is staged left staged; recorded
/// for Undo. HEAD's new sha.
pub fn reword_head(s: &Session, message: &str, no_verify: bool) -> Result<String, WriteError> {
    run_commit(s, message, &["--amend", "--only"], no_verify)?;
    Ok(text(&s.output(&Call::new(&["rev-parse", "HEAD"]))?).trim().to_string())
}

fn run_commit(s: &Session, message: &str, extra: &[&str], no_verify: bool) -> Result<(), WriteError> {
    if message.trim().is_empty() {
        return Err(WriteError::Refused("A commit needs a message"));
    }
    if conflicts::conflicted(s)? {
        return Err(WriteError::Refused(conflicts::RESOLVE_FIRST));
    }
    let mut args = vec!["commit", "--file=-", "--quiet"];
    args.extend_from_slice(extra);
    if no_verify {
        args.push("--no-verify");
    }
    let out = s.run(&Call::new(&args).input(message.as_bytes().to_vec()))?;
    if !out.success() {
        let said = [text(&out.stderr), text(&out.stdout)].map(|t| t.trim().to_string());
        let stderr = said.into_iter().filter(|t| !t.is_empty()).collect::<Vec<_>>().join("
");
        return Err(WriteError::CommitFailed { code: out.code.unwrap_or(-1), stderr });
    }
    let amend = extra.contains(&"--amend");
    let (kind, action) = if amend { (Kind::Amend, Action::Amend) } else { (Kind::Commit, Action::Commit) };
    if journal::newest_kind(s)? == Some(kind) {
        journal::record(s, action, None)?;
    }
    Ok(())
}

/// The next step of the operation under way, from the commit area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    ContinueRebase,
    SkipCommit,
    AbortRebase,
    AbortMerge,
    Continue(Sequence),
    Skip(Sequence),
    Abort(Sequence),
}

/// Runs `step`: a rebase's as [`rebase`](super::rebase) runs them, a merge's abort, a cherry-pick's, revert's or
/// am's with git's own; the operation's state after.
pub fn step(s: &Session, step: Step) -> Result<Rebased, WriteError> {
    match step {
        Step::ContinueRebase => rebase::continue_rebase(s),
        Step::SkipCommit => rebase::skip(s),
        Step::AbortRebase => rebase::abort(s),
        Step::AbortMerge => {
            s.output(&Call::new(&["merge", "--abort"]))?;
            let head = text(&s.output(&Call::new(&["rev-parse", "HEAD"]))?).trim().to_string();
            Ok(Rebased::Done { head, autostash: Autostash::None })
        }
        Step::Continue(sequence) => conflicts::sequence_step(s, sequence, "--continue"),
        Step::Skip(sequence) => conflicts::sequence_step(s, sequence, "--skip"),
        Step::Abort(sequence) => conflicts::sequence_step(s, sequence, "--abort"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(p: &[&str]) -> Vec<String> {
        p.iter().map(|p| p.to_string()).collect()
    }

    #[test]
    fn nested_paths_unstage_with_reset_and_an_unborn_head_with_rm_cached() {
        assert_eq!(unstage_args(&paths(&["d", "d/a.txt"]), true), ["reset", "-q"]);
        assert_eq!(unstage_args(&paths(&["d", "db/a.txt", "a.txt"]), true), ["restore", "--staged"]);
        assert_eq!(unstage_args(&paths(&["a.txt"]), false)[0], "rm");
    }

    #[test]
    fn a_message_is_the_summary_a_blank_line_and_the_description() {
        assert_eq!(message("Fix it  ", ""), "Fix it\n");
        assert_eq!(message("Fix it", "Why\nand how\n\n"), "Fix it\n\nWhy\nand how\n");
    }

    #[test]
    fn a_rename_brings_its_old_path_once() {
        let f = |path: &str, old: Option<&str>| FileChange { kind: ChangeKind::Renamed, path: path.into(), old_path: old.map(Into::into), similarity: None };
        assert_eq!(with_old_paths(&[f("b", Some("a")), f("a", None)]), paths(&["b", "a"]));
    }
}
