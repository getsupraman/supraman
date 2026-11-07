//! GitMan's undo journal (docs/research/gitkraken-12.6.1.md, "Undo/redo"): HEAD's reflog merged with the reflog of
//! [`JOURNAL`], per worktree. Each undoable write [`record`]s an entry: an empty-tree marker commit holding the
//! action (its snapshot a parent, so the journal's reflog keeps it alive) and a reflog line naming the HEAD line
//! it follows; the ref then rests on another commit, so no marker outlives its reflog line.

mod restore;
mod walk;

use std::path::Path;

use super::{Call, LockSet, Session, WriteError, Writer};
use crate::history::short_sha;
use crate::operation::{OperationState, operation_state};
use crate::parse::text;

pub use restore::{delete_branch, discard, discard_from_index};
pub(crate) use restore::drop_noop_reset;
pub use walk::Kind;
use walk::{Entry, Item, Line, Next, REST, UNDO};

pub const JOURNAL: &str = "refs/worktree/supraman/undo";

/// A write GitMan can undo, as [`record`] keeps it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Action {
    Commit,
    Amend,
    Checkout,
    /// `hard` with the snapshot of the work it threw away.
    Reset { hard: bool },
    /// GitMan's own rebase of the branch that was at `from`.
    Rebase { from: String },
    /// `paths` from the checkout's top, `/`-separated, with the snapshot of them; `from_index` those of them whose
    /// working file went back to the index's, their staged change kept.
    Discard {
        paths: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        from_index: Vec<String>,
    },
    DeleteBranch { name: String, sha: String, remote: Option<String>, merge: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Payload {
    action: Action,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    snapshot: Option<String>,
}

/// What Undo or Redo would do: its kind and the action in the user's words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub kind: Kind,
    pub action: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Undo {
    Step(Step),
    /// The newest action is one GitMan cannot undo: its reflog message.
    Cannot(String),
}

/// What the toolbar's Undo and Redo show.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Preview {
    pub undo: Option<Undo>,
    pub redo: Option<Step>,
    pub state: OperationState,
}

pub const SUPPORTED: &str = "only commits, checkouts, resets, discards, branch deletes and GitMan's rebases can be undone";

/// The operation under way in the words "Cannot undo during …" ends with.
fn under_way(state: OperationState) -> Option<&'static str> {
    match state {
        OperationState::None => None,
        OperationState::Merge => Some("a merge"),
        OperationState::Rebase { .. } => Some("a rebase"),
        OperationState::Am => Some("an am"),
        OperationState::Revert { .. } => Some("a revert"),
        OperationState::CherryPick { .. } => Some("a cherry-pick"),
    }
}

impl Preview {
    pub fn can_undo(&self) -> bool {
        self.state == OperationState::None && matches!(self.undo, Some(Undo::Step(_)))
    }

    pub fn can_redo(&self) -> bool {
        self.state == OperationState::None && self.redo.is_some()
    }

    pub fn undo_tooltip(&self) -> String {
        if let Some(op) = under_way(self.state) {
            return format!("Cannot undo during {op}");
        }
        match &self.undo {
            None => "Nothing to undo".into(),
            Some(Undo::Step(step)) => format!("Undo {}", step.action),
            Some(Undo::Cannot(message)) => format!("Cannot undo {message}: {SUPPORTED}"),
        }
    }

    pub fn redo_tooltip(&self) -> String {
        if let Some(op) = under_way(self.state) {
            return format!("Cannot redo during {op}");
        }
        match &self.redo {
            None => "No recent undo to redo".into(),
            Some(step) => format!("Redo {}", step.action),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Refused {
    #[error("Nothing to undo")]
    Nothing,
    #[error("No recent undo to redo")]
    NoRedo,
    #[error("Cannot undo {0}: {SUPPORTED}")]
    Cannot(String),
    #[error("Cannot change the history during {0}")]
    Busy(&'static str),
    /// The work in progress must be stashed first: ask, then call again with the stash allowed.
    #[error("{files} changed files would be stashed first")]
    WouldStash { files: usize, action: String },
    #[error("Cannot check out {target}: it would overwrite local changes in {files} files")]
    Conflicts { target: String, files: usize },
    #[error("Cannot {what}: {why}")]
    Stale { what: String, why: String },
    #[error("Cannot undo: {0} is gone (probably garbage-collected)")]
    Gone(String),
    #[error(transparent)]
    Write(#[from] WriteError),
}

/// An undo or a redo done: the toast's words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Done {
    pub toast: String,
}

/// Keeps `action`, just written by `s`, for Undo; `snapshot` is the one taken before it ([`Session::snapshot`]).
pub fn record(s: &Session, action: Action, snapshot: Option<&str>) -> Result<(), WriteError> {
    let label = label(s, &action)?;
    write_entry(s, &label, &Payload { action, snapshot: snapshot.map(str::to_string) })
}

/// What Undo and Redo would act on now; blocking.
pub fn preview(writer: &Writer) -> Result<Preview, WriteError> {
    writer.run(LockSet::refresh(), |s| {
        let state = operation_state(&s.writer().dirs().git_dir);
        let walk = walk::walk(&read(s)?);
        let undo = match walk.undo {
            None => None,
            Some(Next::Cannot(message)) => Some(Undo::Cannot(message)),
            Some(Next::Undo(item, kind)) => Some(Undo::Step(Step { kind, action: friendly(s, &item)? })),
        };
        let redo = walk.redo.map(|again| Ok::<_, WriteError>(Step { kind: again.kind, action: friendly(s, &again.original)? })).transpose()?;
        Ok(Preview { undo, redo, state })
    })
}

/// Undoes the newest action not undone; with `stash` false it refuses with [`Refused::WouldStash`] where work in
/// progress would be stashed first.
pub fn undo(s: &Session, stash: bool) -> Result<Done, Refused> {
    s.nested(LockSet::undo(), |s| Ok(undo_held(s, stash)))?
}

/// Redoes the newest undo while nothing came after it; `stash` as for [`undo`].
pub fn redo(s: &Session, stash: bool) -> Result<Done, Refused> {
    s.nested(LockSet::undo(), |s| Ok(redo_held(s, stash)))?
}

fn idle(s: &Session) -> Result<(), Refused> {
    match under_way(operation_state(&s.writer().dirs().git_dir)) {
        Some(op) => Err(Refused::Busy(op)),
        None => Ok(()),
    }
}

fn undo_held(s: &Session, stash: bool) -> Result<Done, Refused> {
    idle(s)?;
    let (item, kind) = match walk::walk(&read(s)?).undo {
        None => return Err(Refused::Nothing),
        Some(Next::Cannot(message)) => return Err(Refused::Cannot(message)),
        Some(Next::Undo(item, kind)) => (item, kind),
    };
    let action = friendly(s, &item)?;
    let stashed = restore::undo(s, &item, kind, &action, stash)?;
    Ok(done("Undid", &action, stashed))
}

fn redo_held(s: &Session, stash: bool) -> Result<Done, Refused> {
    idle(s)?;
    let again = walk::walk(&read(s)?).redo.ok_or(Refused::NoRedo)?;
    let action = friendly(s, &again.original)?;
    let stashed = restore::redo(s, &again, &action, stash)?;
    Ok(done("Redid", &action, stashed))
}

fn done(verb: &str, action: &str, stashed: bool) -> Done {
    let stashed = if stashed { "; your changes are in a stash" } else { "" };
    Done { toast: format!("{verb} {action}{stashed}") }
}

fn log(path: &Path) -> Result<Vec<Line>, WriteError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(walk::parse_log(&text(&bytes))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(source) => Err(WriteError::Io { what: format!("read {}", path.display()), source }),
    }
}

/// The kind of HEAD's newest reflog line, if it is one the journal knows.
pub fn newest_kind(s: &Session) -> Result<Option<Kind>, WriteError> {
    Ok(head_log(s)?.pop().and_then(|l| Kind::of(&l.message)))
}

fn head_log(s: &Session) -> Result<Vec<Line>, WriteError> {
    log(&s.writer().dirs().git_dir.join("logs").join("HEAD"))
}

/// HEAD's reflog and the journal's, merged newest first.
fn read(s: &Session) -> Result<Vec<Item>, WriteError> {
    let journal = log(&s.writer().dirs().git_dir.join("logs").join(JOURNAL))?;
    let entries: Vec<Entry> = journal.iter().filter_map(Entry::parse).collect();
    Ok(walk::items(&head_log(s)?, &entries))
}

fn label(s: &Session, action: &Action) -> Result<String, WriteError> {
    let expected = match action {
        Action::Rebase { from } => return Ok(format!("rebase: from {from}")),
        Action::Discard { paths, .. } => return Ok(discard_label(paths)),
        Action::DeleteBranch { name, .. } => return Ok(format!("delete branch: {name}")),
        Action::Commit => Kind::Commit,
        Action::Amend => Kind::Amend,
        Action::Checkout => Kind::Checkout,
        Action::Reset { .. } => Kind::Reset,
    };
    let newest = head_log(s)?.pop().map(|l| l.message).unwrap_or_default();
    if Kind::of(&newest) != Some(expected) {
        return Err(WriteError::Journal(format!("HEAD's newest reflog line is not the {expected:?} to record: {newest}")));
    }
    Ok(newest)
}

fn discard_label(paths: &[String]) -> String {
    let first = paths.first().map_or("", String::as_str);
    match paths.len() {
        0 | 1 => format!("discard: {first}"),
        2 => format!("discard: {first} (and 1 other file)"),
        n => format!("discard: {first} (and {} other files)", n - 1),
    }
}

/// The action in the user's words: `commit "Fix it"`, `reset to "<its target's subject>"`, ….
fn friendly(s: &Session, item: &Item) -> Result<String, WriteError> {
    let label = item.label.as_str();
    let quoted = |prefix: &str, rest: &str| format!("{prefix} \"{rest}\"");
    Ok(match Kind::of(label) {
        Some(Kind::Commit) => quoted("commit", &label["commit: ".len()..]),
        Some(Kind::Amend) => quoted("commit amend", &label["commit (amend): ".len()..]),
        Some(Kind::Checkout) => {
            let moved = &label["checkout: moving from ".len()..];
            let (from, to) = moved.split_once(" to ").unwrap_or((moved, ""));
            format!("checkout from \"{from}\" to \"{to}\"")
        }
        Some(Kind::Reset) => {
            let target = item.head.as_ref().map(|l| l.new.clone()).unwrap_or_default();
            let subject = s.run(&Call::new(&["log", "-1", "--format=%s", &target]))?;
            let subject = text(&subject.stdout).trim().to_string();
            quoted("reset to", if subject.is_empty() { &label["reset: moving to ".len()..] } else { &subject })
        }
        Some(Kind::Rebase) => quoted("rebase from", short_sha(&label["rebase: from ".len()..])),
        Some(Kind::Discard) => {
            let rest = &label["discard: ".len()..];
            match rest.rsplit_once(" (and ") {
                Some((file, others)) => format!("discard \"{file}\" (and {others}"),
                None => quoted("discard", rest),
            }
        }
        Some(Kind::DeleteBranch) => quoted("delete branch", &label["delete branch: ".len()..]),
        None => label.to_string(),
    })
}

/// Git run with no hooks: the journal's own writes.
fn quiet_call(s: &Session, args: &[&str]) -> Call {
    let none = s.writer().dirs().git_dir.join("supraman-no-hooks");
    let hooks = format!("core.hooksPath={}", none.display());
    Call::new(&[&["-c", hooks.as_str()][..], args].concat())
}

fn write_entry(s: &Session, label: &str, payload: &Payload) -> Result<(), WriteError> {
    let empty = text(&s.output(&Call::new(&["mktree"]).input(Vec::new()))?).trim().to_string();
    let json = serde_json::to_string(payload).map_err(|e| WriteError::Journal(e.to_string()))?;
    let message = format!("{label}\n\n{json}\n\nSupraman-Undo-Id: {}", uuid::Uuid::new_v4());
    let marker = s.commit_tree(&empty, &payload.snapshot.iter().map(String::as_str).collect::<Vec<_>>(), &message)?;
    let head = text(&s.run(&Call::new(&["rev-parse", "-q", "--verify", "HEAD"]))?.stdout).trim().to_string();
    let reflog = Entry::message(label, if head.is_empty() { "none" } else { &head }, head_log(s)?.last());
    s.output(&quiet_call(s, &["update-ref", "--create-reflog", "-m", &reflog, JOURNAL, &marker]))?;
    let rest = s.commit_tree(&empty, &[], "Supraman's undo journal")?;
    s.output(&quiet_call(s, &["update-ref", "--create-reflog", "-m", REST, JOURNAL, &rest]))?;
    Ok(())
}

/// The journal entry of `item` (an action GitMan recorded), read from its marker.
fn payload(s: &Session, item: &Item) -> Result<Option<Payload>, Refused> {
    let Some(entry) = &item.entry else { return Ok(None) };
    let out = s.run(&Call::new(&["cat-file", "commit", &entry.marker]))?;
    if !out.success() {
        return Err(Refused::Gone(format!("the journal entry {}", short_sha(&entry.marker))));
    }
    let body = text(&out.stdout);
    let json = body.lines().find(|l| l.starts_with('{')).ok_or_else(|| Refused::Gone(format!("the action of {}", short_sha(&entry.marker))))?;
    serde_json::from_str(json).map(Some).map_err(|e| Refused::Write(WriteError::Journal(e.to_string())))
}

/// Writes `label`'s entry again with `payload`, `undo: ` before it when `undo`.
fn rewrite(s: &Session, label: &str, payload: &Payload, undo: bool) -> Result<(), WriteError> {
    let label = if undo { format!("{UNDO}{label}") } else { label.to_string() };
    write_entry(s, &label, payload)
}
