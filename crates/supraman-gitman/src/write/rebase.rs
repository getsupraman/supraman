//! GitMan's own rebase (docs/research/gitkraken-12.6.1.md §2 "Interactive rebase"): an older commit's message
//! reworded by `rebase -i` with a todo and a message Supraman prepared, and a rebase's continue, skip and abort.
//! A rebase GitMan stops in holds [`REBASE_MARKER`]: where the branch was and the autostash to put back.

use std::path::{Path, PathBuf};

use super::journal::{self, Action};
use super::stash::{self, Applied};
use super::{Call, Session, WriteError};
use crate::history::short_sha;
use crate::operation::{OperationState, REBASE_MARKER, operation_state};
use crate::parse::text;
use crate::GitError;

pub const DETACHED: &str = "HEAD is detached: check out a branch to reword its commits";
pub const NOT_REWORDABLE: &str = "Only a commit below HEAD on its first-parent line, above every merge and not the root, can be reworded";
pub const UNDER_WAY: &str = "Finish or abort the operation under way first";
pub const STAGE_FIRST: &str = "Stage the resolved files first";
pub const EMPTY_MESSAGE: &str = "A commit needs a message";

/// Every run of `rebase`: no autostash, no ref updates, no todo checks, commit messages as given.
const REBASE_PINS: &[&str] = &[
    "-c", "rebase.autoStash=false",
    "-c", "rebase.updateRefs=false",
    "-c", "rebase.autoSquash=false",
    "-c", "rebase.missingCommitsCheck=ignore",
    "-c", "commit.cleanup=verbatim",
    "-c", "commit.status=false",
];

/// The rebase's state GitMan keeps in [`REBASE_MARKER`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Marker {
    from: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    autostash: Option<Autostashed>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Autostashed {
    sha: String,
    name: String,
}

/// What became of the stash a rebase made of the work in progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Autostash {
    None,
    Popped,
    /// Still in the stash list as `name`: why it was not put back.
    Kept { name: String, why: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rebased {
    /// No rebase is under way any more: HEAD's sha.
    Done { head: String, autostash: Autostash },
    /// Stopped on a conflict, the rebase still under way.
    Conflicted,
}

/// The autostash's message.
pub fn autostash_name(sha: &str) -> String {
    format!("GitMan autostash before rewording {}", short_sha(sha))
}

/// Rewords `sha` ([`crate::rewordable`]) to `message`: `rebase -i <sha>^` of HEAD's branch, the todo `reword
/// <sha>` and a `pick` of each commit above, the work in progress stashed first and popped after. A conflict
/// leaves the rebase under way; another failure while it is under way aborts it. Recorded for Undo.
pub fn reword(s: &Session, sha: &str, message: &str) -> Result<Rebased, WriteError> {
    if message.trim().is_empty() {
        return Err(WriteError::Refused(EMPTY_MESSAGE));
    }
    if operation_state(&s.writer().dirs().git_dir) != OperationState::None {
        return Err(WriteError::Refused(UNDER_WAY));
    }
    let branch = s.run(&Call::new(&["symbolic-ref", "-q", "--short", "HEAD"]))?;
    let branch = text(&branch.stdout).trim().to_string();
    if branch.is_empty() {
        return Err(WriteError::Refused(DETACHED));
    }
    if crate::rewordable(s.writer().repo(), sha)?.is_none() {
        return Err(WriteError::Refused(NOT_REWORDABLE));
    }
    let range = format!("{sha}^..HEAD");
    let commits: Vec<String> = text(&s.output(&Call::new(&["rev-list", "--reverse", "--first-parent", &range]))?).lines().map(str::to_string).collect();
    let todo: String = commits.iter().enumerate().map(|(i, c)| format!("{} {c}
", if i == 0 { "reword" } else { "pick" })).collect();
    start(s, &format!("{sha}^"), &branch, &todo, message, &autostash_name(sha))
}

/// `rebase -i <base> <branch>` with `todo` and `message` copied in by its editors, the work in progress stashed
/// as `stash_name` first and popped after.
pub(crate) fn start(s: &Session, base: &str, branch: &str, todo: &str, message: &str, stash_name: &str) -> Result<Rebased, WriteError> {
    let from = rev(s, "HEAD")?;
    let autostash = autostash(s, stash_name)?;
    let marker = Marker { from, autostash };
    let prepared = Prepared::new(&s.writer().dirs().git_dir, todo, message)?;
    let rebase = Call::new(&[REBASE_PINS, &["rebase", "-i", base, branch]].concat())
        .env("GIT_SEQUENCE_EDITOR", copy_over(&prepared.todo))
        .editor(copy_over(&prepared.message));
    let out = s.run(&rebase)?;
    drop(prepared);
    if out.success() && !under_way(s) {
        return done(s, &marker);
    }
    if !under_way(s) {
        return Err(failed("rebase -i", &out, put_back(s, &marker)));
    }
    if conflicted(s)? {
        write_marker(s, &marker)?;
        return Ok(Rebased::Conflicted);
    }
    s.output(&Call::new(&["rebase", "--abort"]))?;
    Err(failed("rebase -i", &out, put_back(s, &marker)))
}

/// `rebase --continue` with every commit's message kept: the stopped commit's without git's commented
/// "Conflicts:" section (in `MERGE_MSG`, or the rebase's `message` for an older git). Refused while a conflicted file is not staged.
pub fn continue_rebase(s: &Session) -> Result<Rebased, WriteError> {
    let marker = read_marker(s);
    strip_conflicts(&rebase_dir(s).join("message"))?;
    strip_conflicts(&s.writer().dirs().git_dir.join("MERGE_MSG"))?;
    let out = s.run(&step_call("--continue"))?;
    let said = format!("{}\n{}", text(&out.stderr), text(&out.stdout)).to_lowercase();
    if !out.success() && (said.contains("edit all merge conflicts") || said.contains("unmerged files")) {
        return Err(WriteError::Refused(STAGE_FIRST));
    }
    stepped(s, marker, "rebase --continue", &out)
}

/// `rebase --skip`: the stopped commit left out.
pub fn skip(s: &Session) -> Result<Rebased, WriteError> {
    let marker = read_marker(s);
    let out = s.run(&step_call("--skip"))?;
    stepped(s, marker, "rebase --skip", &out)
}

/// `rebase --abort`, then GitMan's autostash popped back onto the branch where it was made.
pub fn abort(s: &Session) -> Result<Rebased, WriteError> {
    let marker = read_marker(s);
    s.output(&Call::new(&["rebase", "--abort"]))?;
    let autostash = marker.map_or(Autostash::None, |m| pop(s, m.autostash.as_ref()));
    Ok(Rebased::Done { head: rev(s, "HEAD")?, autostash })
}

fn step_call(step: &str) -> Call {
    Call::new(&[REBASE_PINS, &["rebase", step]].concat()).editor("true")
}

/// After a continue or a skip: finished, recorded and the autostash popped when the rebase was GitMan's; stopped
/// again on a conflict; or git's error.
fn stepped(s: &Session, marker: Option<Marker>, what: &str, out: &supraman_core::process::RawOutput) -> Result<Rebased, WriteError> {
    if !under_way(s) {
        if !out.success() {
            return Err(failed(what, out, None));
        }
        return match marker {
            Some(marker) => done(s, &marker),
            None => Ok(Rebased::Done { head: rev(s, "HEAD")?, autostash: Autostash::None }),
        };
    }
    if conflicted(s)? {
        return Ok(Rebased::Conflicted);
    }
    Err(failed(what, out, None))
}

/// A finished rebase of GitMan's: the autostash popped, then recorded with a snapshot of the work in progress
/// that Undo puts back.
fn done(s: &Session, marker: &Marker) -> Result<Rebased, WriteError> {
    let autostash = pop(s, marker.autostash.as_ref());
    let status = crate::status(s.writer().repo())?;
    let paths: Vec<&str> = status.entries.iter().flat_map(|e| [Some(e.path.as_str()), e.orig_path.as_deref()]).flatten().collect();
    let snapshot = s.snapshot(&paths, "After reword")?;
    journal::record(s, Action::Rebase { from: marker.from.clone() }, Some(&snapshot))?;
    Ok(Rebased::Done { head: rev(s, "HEAD")?, autostash })
}

/// Stashes the work in progress as `name`, untracked files too, when there is any.
fn autostash(s: &Session, name: &str) -> Result<Option<Autostashed>, WriteError> {
    if crate::status(s.writer().repo())?.entries.is_empty() {
        return Ok(None);
    }
    stash::stash_all(s, name, false)?;
    Ok(Some(Autostashed { sha: rev(s, "refs/stash")?, name: name.to_string() }))
}

fn pop(s: &Session, autostash: Option<&Autostashed>) -> Autostash {
    let Some(Autostashed { sha, name }) = autostash else { return Autostash::None };
    let kept = |why: String| Autostash::Kept { name: name.clone(), why };
    match stash::apply(s, sha, true) {
        Ok(Applied::Clean) => Autostash::Popped,
        Ok(Applied::Conflicted) => kept("it conflicts with the rebased files".into()),
        Err(e) => kept(e.to_string()),
    }
}

/// The autostash back after a reword that did not happen; what to add to its error when it stays.
fn put_back(s: &Session, marker: &Marker) -> Option<String> {
    match pop(s, marker.autostash.as_ref()) {
        Autostash::Kept { name, why } => Some(format!("Your changes stay in the stash \"{name}\": {why}")),
        _ => None,
    }
}

fn rev(s: &Session, name: &str) -> Result<String, WriteError> {
    Ok(text(&s.output(&Call::new(&["rev-parse", "--verify", name]))?).trim().to_string())
}

fn rebase_dir(s: &Session) -> PathBuf {
    s.writer().dirs().git_dir.join("rebase-merge")
}

fn under_way(s: &Session) -> bool {
    matches!(operation_state(&s.writer().dirs().git_dir), OperationState::Rebase { .. })
}

fn conflicted(s: &Session) -> Result<bool, WriteError> {
    Ok(!s.output(&Call::new(&["ls-files", "-u", "-z"]))?.is_empty())
}

fn failed(what: &str, out: &supraman_core::process::RawOutput, note: Option<String>) -> WriteError {
    let said = [text(&out.stderr), text(&out.stdout)].map(|t| t.trim().to_string());
    let stderr = said.into_iter().chain(note).filter(|t| !t.is_empty()).collect::<Vec<_>>().join("\n");
    GitError::Failed { args: what.to_string(), code: out.code, stderr }.into()
}

fn write_marker(s: &Session, marker: &Marker) -> Result<(), WriteError> {
    let path = rebase_dir(s).join(REBASE_MARKER);
    let json = serde_json::to_string(marker).map_err(|e| WriteError::Journal(e.to_string()))?;
    std::fs::write(&path, json).map_err(|source| WriteError::Io { what: format!("write {}", path.display()), source })
}

/// GitMan's marker of the rebase under way; `None` for another's.
fn read_marker(s: &Session) -> Option<Marker> {
    let path = rebase_dir(s).join(REBASE_MARKER);
    std::fs::read_to_string(path).ok().and_then(|json| serde_json::from_str(&json).ok())
}

/// `message` without the commented `Conflicts:` section git adds to a stopped commit's message, whatever the
/// comment character.
pub fn without_conflicts(message: &str) -> String {
    let mut kept = Vec::new();
    let mut section = None;
    for line in message.lines() {
        let mut chars = line.chars();
        let lead = chars.next();
        if lead.is_some_and(|c| !c.is_alphanumeric()) && chars.as_str().trim_end() == " Conflicts:" {
            section = lead;
            continue;
        }
        if section.is_some_and(|c| line.starts_with(c)) {
            continue;
        }
        section = None;
        kept.push(line);
    }
    while kept.last().is_some_and(|l| l.trim().is_empty()) {
        kept.pop();
    }
    format!("{}
", kept.join("
"))
}

fn strip_conflicts(path: &Path) -> Result<(), WriteError> {
    let Ok(message) = std::fs::read_to_string(path) else { return Ok(()) };
    let stripped = without_conflicts(&message);
    if stripped == message {
        return Ok(());
    }
    std::fs::write(path, stripped).map_err(|source| WriteError::Io { what: format!("write {}", path.display()), source })
}

/// The todo and the message a reword's editors copy into place, in the git dir; removed when it drops.
struct Prepared {
    dir: PathBuf,
    todo: PathBuf,
    message: PathBuf,
}

impl Prepared {
    fn new(git_dir: &Path, todo: &str, message: &str) -> Result<Prepared, WriteError> {
        let dir = git_dir.join(format!("gitman-reword-{}", supraman_core::time::hex6()));
        let io = |what: &Path| {
            let what = format!("write {}", what.display());
            move |source| WriteError::Io { what, source }
        };
        std::fs::create_dir_all(&dir).map_err(io(&dir))?;
        let prepared = Prepared { todo: dir.join("todo"), message: dir.join("message"), dir };
        std::fs::write(&prepared.todo, todo).map_err(io(&prepared.todo))?;
        std::fs::write(&prepared.message, message).map_err(io(&prepared.message))?;
        Ok(prepared)
    }
}

impl Drop for Prepared {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

/// An editor command git's sh runs with the file to edit appended: `file` copied over it.
fn copy_over(file: &Path) -> String {
    let path = file.to_string_lossy().replace('\\', "/");
    format!("cp '{}'", path.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stopped_commits_conflicts_section_goes_and_the_rest_stays() {
        let message = "Fix it\n\nWhy\n# kept\n\n# Conflicts:\n#\ta.txt\n#\tb.txt\n";
        assert_eq!(without_conflicts(message), "Fix it\n\nWhy\n# kept\n");
        assert_eq!(without_conflicts("Plain\n"), "Plain\n");
    }

    #[test]
    fn the_copy_editor_quotes_its_path_for_sh() {
        assert_eq!(copy_over(Path::new(r"C:\a b\it's\todo")), r"cp 'C:/a b/it'\''s/todo'");
    }
}
