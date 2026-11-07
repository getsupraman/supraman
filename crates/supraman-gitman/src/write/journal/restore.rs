//! Each kind's undo and redo, and the discard and the branch delete that redo runs again.

use std::collections::HashSet;

use super::walk::{Again, Item, Kind, UNDO, is_zero};
use super::{Action, Payload, Refused, log, payload, record, rewrite};
use crate::parse::{nul_records, text};
use crate::write::{Call, Session, WriteError};

/// Undoes `item`; whether work in progress went to a stash first.
pub(super) fn undo(s: &Session, item: &Item, kind: Kind, action: &str, stash: bool) -> Result<bool, Refused> {
    let line = item.head.as_ref();
    let undo_label = format!("{UNDO}{}", item.label);
    match kind {
        Kind::Commit | Kind::Amend => {
            let line = line.expect("a commit's HEAD line");
            move_head(s, &undo_label, &line.old, &line.new)?;
            Ok(false)
        }
        Kind::Reset | Kind::Rebase => {
            let line = line.expect("a reset's HEAD line");
            let from = item.from.as_deref().unwrap_or(&line.old);
            reset(s, &payload(s, item)?, kind, &undo_label, from, &line.new, action, stash)
        }
        Kind::Checkout => {
            let line = line.expect("a checkout's HEAD line");
            let (from, _) = moved(&item.label);
            checkout(s, from, &line.old, &undo_label)?;
            Ok(false)
        }
        Kind::Discard => discard_back(s, item, action),
        Kind::DeleteBranch => {
            branch_back(s, item)?;
            Ok(false)
        }
    }
}

/// Does `again.original` once more; whether work in progress went to a stash first.
pub(super) fn redo(s: &Session, again: &Again, action: &str, stash: bool) -> Result<bool, Refused> {
    let (original, label) = (&again.original, again.original.label.as_str());
    match again.kind {
        Kind::Commit | Kind::Amend => {
            let (line, now) = (original.head.as_ref().expect("a commit's HEAD line"), undone_to(again));
            move_head(s, label, &line.new, &now)?;
            Ok(false)
        }
        Kind::Reset | Kind::Rebase => {
            let line = original.head.as_ref().expect("a reset's HEAD line");
            let payload = payload(s, original)?;
            let stashed = reset(s, &payload, again.kind, label, &line.new, &undone_to(again), action, stash)?;
            if let Some(payload) = payload {
                rewrite(s, label, &payload, false)?;
            }
            Ok(stashed)
        }
        Kind::Checkout => {
            let line = original.head.as_ref().expect("a checkout's HEAD line");
            checkout(s, moved(label).1, &line.new, label)?;
            Ok(false)
        }
        Kind::Discard => {
            let Some(Payload { action: Action::Discard { paths, from_index }, .. }) = payload(s, original)? else { return Err(gone(original)) };
            discard_from_index(s, &paths, &from_index)?;
            Ok(false)
        }
        Kind::DeleteBranch => {
            let Some(Payload { action: Action::DeleteBranch { name, .. }, .. }) = payload(s, original)? else { return Err(gone(original)) };
            if exists(s, &format!("refs/tags/{name}"))? || !exists(s, &format!("refs/heads/{name}"))? {
                return Err(Refused::Stale { what: format!("delete branch {name} again"), why: "it is no longer a branch, or a tag has its name".into() });
            }
            delete_branch(s, &name)?;
            Ok(false)
        }
    }
}

/// Where HEAD went when the newest undo ran.
fn undone_to(again: &Again) -> String {
    again.undo.head.as_ref().map(|l| l.new.clone()).unwrap_or_default()
}

fn gone(item: &Item) -> Refused {
    Refused::Gone(format!("what {} kept", item.label))
}

/// `checkout: moving from <a> to <b>`'s two ends.
fn moved(label: &str) -> (&str, &str) {
    let moved = label.strip_prefix("checkout: moving from ").unwrap_or(label);
    moved.split_once(" to ").unwrap_or((moved, ""))
}

fn exists(s: &Session, name: &str) -> Result<bool, WriteError> {
    Ok(s.run(&Call::new(&["rev-parse", "-q", "--verify", name]))?.success())
}

fn object_exists(s: &Session, sha: &str) -> Result<bool, WriteError> {
    Ok(s.run(&Call::new(&["cat-file", "-e", &format!("{sha}^{{commit}}")]))?.success())
}

/// HEAD (and the branch it is on) moved from `expected` to `to`, `message` in their reflogs; index and files as they are.
fn move_head(s: &Session, message: &str, to: &str, expected: &str) -> Result<(), WriteError> {
    s.output(&Call::new(&["update-ref", "-m", message, "HEAD", to, expected])).map(drop)
}

/// A reset or GitMan's rebase taken back or done again: HEAD moved from `now` to `to`; a hard one stashes the work
/// in progress first, resets the files and puts back what its snapshot held.
#[allow(clippy::too_many_arguments)]
fn reset(s: &Session, payload: &Option<Payload>, kind: Kind, message: &str, to: &str, now: &str, action: &str, stash: bool) -> Result<bool, Refused> {
    let (hard, snapshot) = match payload {
        Some(Payload { action: Action::Reset { hard }, snapshot }) => (*hard, snapshot.as_deref()),
        Some(Payload { snapshot, .. }) => (kind == Kind::Rebase, snapshot.as_deref()),
        None => (false, None),
    };
    if !hard {
        move_head(s, message, to, now)?;
        return Ok(false);
    }
    let restoring = snapshot.filter(|_| message.starts_with(UNDO));
    let index = restoring.map(|snapshot| index_of(s, snapshot)).transpose()?;
    let stashed = stash_all(s, action, stash, restoring.is_some())?;
    move_head(s, message, to, now)?;
    s.output(&Call::new(&["reset", "-q", "--hard"]))?;
    drop_noop_reset(s)?;
    if let (Some(snapshot), Some(index)) = (restoring, index) {
        s.output(&Call::new(&["restore", &format!("--source={snapshot}"), "--worktree", "--", "."]))?;
        s.output(&Call::new(&["read-tree", &format!("{index}^{{tree}}")]))?;
        refresh(s)?;
    }
    Ok(stashed)
}

/// The commit of the index a snapshot holds (its last parent), checking both are still there.
fn index_of(s: &Session, snapshot: &str) -> Result<String, Refused> {
    let gone = || Refused::Gone(format!("the snapshot {}", crate::history::short_sha(snapshot)));
    if !object_exists(s, snapshot)? {
        return Err(gone());
    }
    let parents = text(&s.output(&Call::new(&["rev-parse", &format!("{snapshot}^@")]))?);
    let index = parents.lines().last().ok_or_else(gone)?.to_string();
    if !object_exists(s, &index)? {
        return Err(gone());
    }
    Ok(index)
}

/// Stashes the changes to tracked files, and with `untracked` the untracked files a snapshot may overwrite, when
/// there are any; with `allowed` false that is a refusal.
fn stash_all(s: &Session, action: &str, allowed: bool, untracked: bool) -> Result<bool, Refused> {
    let entries = crate::status(s.writer().repo()).map_err(WriteError::from)?.entries;
    let files = entries.iter().filter(|e| untracked || e.kind != crate::EntryKind::Untracked).count();
    if files == 0 {
        return Ok(false);
    }
    if !allowed {
        return Err(Refused::WouldStash { files, action: action.to_string() });
    }
    let mut args = vec!["stash", "push", "-q"];
    if untracked {
        args.push("--include-untracked");
    }
    let message = format!("GitMan: before {action}");
    args.extend(["-m", &message]);
    s.output(&Call::new(&args))?;
    Ok(true)
}

/// Takes out the newest line of HEAD's and its branch's reflog when it is the reset just run, which moved nothing.
pub(crate) fn drop_noop_reset(s: &Session) -> Result<(), WriteError> {
    let branch = text(&s.run(&Call::new(&["symbolic-ref", "-q", "HEAD"]))?.stdout).trim().to_string();
    let logs = s.writer().dirs().git_dir.join("logs");
    let common = s.writer().dirs().common_dir.join("logs");
    for (name, file) in [("HEAD".to_string(), logs.join("HEAD")), (branch.clone(), common.join(&branch))].into_iter().filter(|(n, _)| !n.is_empty()) {
        let newest = log(&file)?.pop();
        if newest.is_some_and(|l| l.old == l.new && l.message == "reset: moving to HEAD") {
            s.output(&Call::new(&["reflog", "delete", &format!("{name}@{{0}}")]))?;
        }
    }
    Ok(())
}

/// The index's stat cache brought up to date after a restore.
fn refresh(s: &Session) -> Result<(), WriteError> {
    s.run(&Call::new(&["update-index", "-q", "--refresh"])).map(drop)
}

/// Checks out the branch `name` when there is one, else `sha` detached; `message` in HEAD's reflog.
fn checkout(s: &Session, name: &str, sha: &str, message: &str) -> Result<(), Refused> {
    let args: Vec<&str> = if !name.is_empty() && exists(s, &format!("refs/heads/{name}"))? { vec!["checkout", "-q", name, "--"] } else { vec!["checkout", "-q", "--detach", sha, "--"] };
    let out = s.run(&Call::new(&args).env("GIT_REFLOG_ACTION", message))?;
    if out.success() {
        return Ok(());
    }
    let stderr = text(&out.stderr);
    if stderr.contains("would be overwritten by checkout") {
        let files = stderr.lines().filter(|l| l.starts_with('\t')).count();
        return Err(Refused::Conflicts { target: if name.is_empty() { sha.to_string() } else { name.to_string() }, files });
    }
    Err(WriteError::from(crate::GitError::Failed { args: args.join(" "), code: out.code, stderr: stderr.trim().to_string() }).into())
}

/// Every path in `tree` (a tree-ish) or, with `index`, in the index.
fn paths_in(s: &Session, tree: Option<&str>, index: bool) -> Result<HashSet<String>, WriteError> {
    let mut paths = HashSet::new();
    if let Some(tree) = tree {
        let out = s.run(&Call::new(&["ls-tree", "-r", "-z", "--full-tree", "--name-only", tree]))?;
        paths.extend(nul_records(&out.stdout).map(text));
    }
    if index {
        paths.extend(nul_records(&s.output(&Call::new(&["ls-files", "-z"]))?).map(text));
    }
    Ok(paths)
}

/// Throws away the changes to `paths` (staged, unstaged and untracked), kept first in a snapshot, and records it.
pub fn discard(s: &Session, paths: &[String]) -> Result<(), WriteError> {
    discard_from_index(s, paths, &[])
}

/// [`discard`], but the working files of `from_index` (some of `paths`) go back to the index's, their staged change
/// kept.
pub fn discard_from_index(s: &Session, paths: &[String], from_index: &[String]) -> Result<(), WriteError> {
    let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
    let snapshot = s.snapshot(&refs, "Before discard")?;
    let known = paths_in(s, Some("HEAD"), true)?;
    let to_index: HashSet<&String> = from_index.iter().collect();
    let (tracked, untracked): (Vec<&String>, Vec<&String>) = paths.iter().partition(|p| known.contains(*p));
    let (worktree, both): (Vec<&String>, Vec<&String>) = tracked.into_iter().partition(|p| to_index.contains(p));
    if !both.is_empty() {
        s.output(&Call::new(&["restore", "--source=HEAD", "--staged", "--worktree"]).paths(both.iter().map(|p| p.to_string())))?;
    }
    if !worktree.is_empty() {
        s.output(&Call::new(&["restore", "--worktree"]).paths(worktree.iter().map(|p| p.to_string())))?;
    }
    for path in untracked {
        let file = s.writer().dirs().top.join(path);
        if let Err(source) = std::fs::remove_file(&file).or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) }) {
            return Err(WriteError::Io { what: format!("remove {}", file.display()), source });
        }
    }
    record(s, Action::Discard { paths: paths.to_vec(), from_index: from_index.to_vec() }, Some(&snapshot))
}

/// A discard taken back: what changed in its paths since goes to a stash, then the paths' files and index entries
/// are the snapshot's again.
fn discard_back(s: &Session, item: &Item, action: &str) -> Result<bool, Refused> {
    let payload = payload(s, item)?.ok_or_else(|| gone(item))?;
    let (Action::Discard { paths, from_index }, Some(snapshot)) = (&payload.action, payload.snapshot.as_deref()) else { return Err(gone(item)) };
    let index = index_of(s, snapshot)?;
    let wanted: HashSet<&str> = paths.iter().map(String::as_str).collect();
    let staged_kept: HashSet<&str> = from_index.iter().map(String::as_str).collect();
    let status = crate::status(s.writer().repo()).map_err(WriteError::from)?;
    let changed: Vec<String> = status
        .entries
        .iter()
        .filter(|e| e.unstaged.is_some() || !staged_kept.contains(e.path.as_str()))
        .flat_map(|e| [Some(&e.path), e.orig_path.as_ref()])
        .flatten()
        .filter(|p| wanted.contains(p.as_str()))
        .cloned()
        .collect();
    let stashed = !changed.is_empty();
    if stashed {
        s.output(&Call::new(&["stash", "push", "-q", "--include-untracked", "-m", &format!("GitMan: before undo of {action}")]).paths(changed))?;
    }
    let in_snapshot = paths_in(s, Some(snapshot), true)?;
    let files: Vec<String> = paths.iter().filter(|p| in_snapshot.contains(*p)).cloned().collect();
    if !files.is_empty() {
        s.output(&Call::new(&["restore", &format!("--source={snapshot}"), "--worktree"]).paths(files))?;
    }
    s.output(&Call::new(&["update-index", "-z", "--index-info"]).input(index_info(s, &index, paths)?))?;
    refresh(s)?;
    rewrite(s, &item.label, &payload, true)?;
    Ok(stashed)
}

/// `update-index --index-info` lines that make `paths`' index entries those of the commit `index`.
fn index_info(s: &Session, index: &str, paths: &[String]) -> Result<Vec<u8>, WriteError> {
    let listed = s.output(&Call::new(&["ls-tree", "-r", "-z", "--full-tree", index]))?;
    let entries: std::collections::HashMap<String, String> =
        nul_records(&listed).map(text).filter_map(|r| r.split_once('\t').map(|(meta, path)| (path.to_string(), meta.to_string()))).collect();
    let zero = "0".repeat(entries.values().next().and_then(|m| m.split(' ').nth(2)).map_or(40, str::len));
    Ok(paths
        .iter()
        .flat_map(|path| {
            let meta = entries.get(path).cloned().unwrap_or_else(|| format!("0 {zero}"));
            format!("{meta}\t{path}\0").into_bytes()
        })
        .collect())
}

/// Deletes the branch `name` and records it with its tip and upstream.
pub fn delete_branch(s: &Session, name: &str) -> Result<(), WriteError> {
    let sha = text(&s.output(&Call::new(&["rev-parse", "--verify", &format!("refs/heads/{name}")]))?).trim().to_string();
    let config = |key: &str| -> Result<Option<String>, WriteError> {
        let out = s.run(&Call::new(&["config", "--get", &format!("branch.{name}.{key}")]))?;
        Ok(out.success().then(|| text(&out.stdout).trim().to_string()))
    };
    let (remote, merge) = (config("remote")?, config("merge")?);
    s.output(&Call::new(&["branch", "-D", name]))?;
    record(s, Action::DeleteBranch { name: name.to_string(), sha, remote, merge }, None)
}

/// A deleted branch made again at its tip, with its upstream.
fn branch_back(s: &Session, item: &Item) -> Result<(), Refused> {
    let payload = payload(s, item)?.ok_or_else(|| gone(item))?;
    let Action::DeleteBranch { name, sha, remote, merge } = &payload.action else { return Err(gone(item)) };
    if exists(s, &format!("refs/heads/{name}"))? {
        return Err(Refused::Stale { what: format!("restore branch {name}"), why: "a branch of that name exists".into() });
    }
    if is_zero(sha) || !object_exists(s, sha)? {
        return Err(Refused::Gone(format!("the commit {}", crate::history::short_sha(sha))));
    }
    let message = format!("{UNDO}{}", item.label);
    s.output(&Call::new(&["update-ref", "-m", &message, &format!("refs/heads/{name}"), sha, ""]))?;
    for (key, value) in [("remote", remote), ("merge", merge)] {
        if let Some(value) = value {
            s.output(&Call::new(&["config", &format!("branch.{name}.{key}"), value]))?;
        }
    }
    rewrite(s, &item.label, &payload, true)?;
    Ok(())
}
