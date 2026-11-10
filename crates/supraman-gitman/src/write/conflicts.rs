//! The conflict panel's writes.

use std::collections::{BTreeMap, BTreeSet};

use super::rebase::{Autostash, Rebased};
use super::{Call, Session, WriteError, wip};
use crate::changes::{ChangeKind, FileChange};
use crate::operation::{OperationState, operation_state};
use crate::parse::{nul_records, text};
use crate::{EntryKind, GitError};

pub const RESOLVE_FIRST: &str = "Mark every conflicted file resolved first";
pub const NO_SIDES: &str = "Nothing under way says which commits the conflict came from";
pub const NOT_IN_SIDES: &str = "None of the conflict's commits has this file";

/// A conflicted file's side: git's "ours" (stage 2) or "theirs" (stage 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Current,
    Incoming,
}

impl Side {
    fn stage(self) -> u8 {
        match self {
            Side::Current => 2,
            Side::Incoming => 3,
        }
    }

    fn flag(self) -> &'static str {
        match self {
            Side::Current => "--ours",
            Side::Incoming => "--theirs",
        }
    }
}

/// What [`mark_resolved`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Marked {
    /// How many files it staged.
    Staged(usize),
    /// Nothing staged: these files still hold conflict markers.
    Markers(Vec<String>),
}

/// The stages each unmerged path has in the index.
fn stages(s: &Session) -> Result<BTreeMap<String, BTreeSet<u8>>, WriteError> {
    let out = s.output(&Call::new(&["ls-files", "-u", "-z"]))?;
    let mut stages: BTreeMap<String, BTreeSet<u8>> = BTreeMap::new();
    for record in nul_records(&out).map(text) {
        let Some((info, path)) = record.split_once('\t') else { continue };
        if let Some(stage) = info.rsplit(' ').next().and_then(|n| n.parse().ok()) {
            stages.entry(path.to_string()).or_default().insert(stage);
        }
    }
    Ok(stages)
}

/// Takes `side` of each conflicted file in `paths` whole and stages it; a file that side deleted is deleted.
pub fn take(s: &Session, paths: &[String], side: Side) -> Result<(), WriteError> {
    let stages = stages(s)?;
    let conflicted: Vec<&String> = paths.iter().filter(|p| stages.contains_key(*p)).collect();
    let (kept, gone): (Vec<&String>, Vec<&String>) = conflicted.into_iter().partition(|p| stages[*p].contains(&side.stage()));
    if !kept.is_empty() {
        s.output(&Call::new(&["checkout", side.flag()]).paths(kept.iter().map(|p| p.as_str())))?;
        s.output(&Call::new(&["add"]).paths(kept.iter().map(|p| p.as_str())))?;
    }
    if !gone.is_empty() {
        s.output(&Call::new(&["rm", "-q", "-f"]).paths(gone.iter().map(|p| p.as_str())))?;
    }
    Ok(())
}

/// Whether `bytes` hold a line that starts with a conflict marker: `<<<<<<<`, `=======` or `>>>>>>>`.
pub fn has_markers(bytes: &[u8]) -> bool {
    bytes.split(|b| *b == b'\n').any(|line| [&b"<<<<<<<"[..], b"=======", b">>>>>>>"].iter().any(|m| line.starts_with(m)))
}

/// Stages the conflicted files among `paths` (`None`: every one); unless `anyway`, nothing while one holds conflict markers.
pub fn mark_resolved(s: &Session, paths: Option<&[String]>, anyway: bool) -> Result<Marked, WriteError> {
    let status = crate::status(s.writer().repo())?;
    let files: Vec<FileChange> = status
        .entries
        .iter()
        .filter(|e| e.kind == EntryKind::Unmerged && paths.is_none_or(|p| p.contains(&e.path)))
        .map(|e| FileChange { kind: ChangeKind::Unmerged, path: e.path.clone(), old_path: None, similarity: None })
        .collect();
    if !anyway {
        let top = &s.writer().dirs().top;
        let marked: Vec<String> = files.iter().filter(|f| std::fs::read(top.join(&f.path)).is_ok_and(|b| has_markers(&b))).map(|f| f.path.clone()).collect();
        if !marked.is_empty() {
            return Ok(Marked::Markers(marked));
        }
    }
    wip::stage(s, &files)?;
    Ok(Marked::Staged(files.len()))
}

/// The base, current and incoming revisions of the conflict under way; `None` for a side git keeps none of.
fn sides(s: &Session) -> Result<[Option<String>; 3], WriteError> {
    let rev = |name: &str| s.verify(&format!("{name}^{{commit}}"));
    let state = operation_state(&s.writer().dirs().git_dir);
    let (base, incoming) = match state {
        OperationState::Merge => {
            let base = s.run(&Call::new(&["merge-base", "HEAD", "MERGE_HEAD"]))?;
            (base.success().then(|| text(&base.stdout).trim().to_string()), rev("MERGE_HEAD")?)
        }
        OperationState::CherryPick { .. } => (rev("CHERRY_PICK_HEAD^")?, rev("CHERRY_PICK_HEAD")?),
        OperationState::Revert { .. } => (rev("REVERT_HEAD")?, rev("REVERT_HEAD^")?),
        OperationState::Rebase { .. } => (rev("REBASE_HEAD^")?, rev("REBASE_HEAD")?),
        OperationState::Am | OperationState::None => return Err(WriteError::Refused(NO_SIDES)),
    };
    if incoming.is_none() {
        return Err(WriteError::Refused(NO_SIDES));
    }
    Ok([base, rev("HEAD")?, incoming])
}

/// Each of `paths`' mode and blob in `rev`.
fn blobs(s: &Session, rev: &str, paths: &[String]) -> Result<BTreeMap<String, (String, String)>, WriteError> {
    let mut args = vec!["ls-tree".to_string(), "-z".into(), rev.into()];
    args.extend(paths.iter().map(|p| format!(":(literal){p}")));
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = s.output(&Call::new(&args))?;
    Ok(nul_records(&out)
        .map(text)
        .filter_map(|r| {
            let (info, path) = r.split_once('\t')?;
            let mut fields = info.split(' ');
            let (mode, kind, sha) = (fields.next()?, fields.next()?, fields.next()?);
            (kind == "blob").then(|| (path.to_string(), (mode.to_string(), sha.to_string())))
        })
        .collect())
}

/// Puts each of `paths` back in conflict in the index (stages 1 to 3 from the operation under way); the work tree is untouched.
pub fn mark_conflicted(s: &Session, paths: &[String]) -> Result<(), WriteError> {
    if paths.is_empty() {
        return Ok(());
    }
    let sides = sides(s)?;
    let mut info = Vec::new();
    let mut lines = Vec::new();
    for (stage, rev) in (1u8..).zip(&sides) {
        let Some(rev) = rev else { continue };
        for (path, (mode, sha)) in blobs(s, rev, paths)? {
            lines.push(format!("{mode} {sha} {stage}\t{path}"));
        }
    }
    let zero = "0".repeat(sides[1].as_ref().map_or(40, String::len));
    for path in paths {
        if !lines.iter().any(|l| l.split_once('\t').is_some_and(|(_, p)| p == path)) {
            return Err(WriteError::Refused(NOT_IN_SIDES));
        }
        info.push(format!("0 {zero} 0\t{path}"));
    }
    info.extend(lines);
    let input: Vec<u8> = info.iter().flat_map(|l| l.bytes().chain([0])).collect();
    s.output(&Call::new(&["update-index", "-z", "--index-info"]).input(input))?;
    Ok(())
}

/// An operation git runs commit by commit with its own continue, skip and abort.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sequence {
    CherryPick,
    Revert,
    Am,
}

impl Sequence {
    pub fn of(state: OperationState) -> Option<Sequence> {
        match state {
            OperationState::CherryPick { .. } => Some(Sequence::CherryPick),
            OperationState::Revert { .. } => Some(Sequence::Revert),
            OperationState::Am => Some(Sequence::Am),
            _ => None,
        }
    }

    pub fn command(self) -> &'static str {
        match self {
            Sequence::CherryPick => "cherry-pick",
            Sequence::Revert => "revert",
            Sequence::Am => "am",
        }
    }
}

/// `git <sequence> --continue`, `--skip` or `--abort`; a continue is refused while a file is still conflicted.
pub(super) fn sequence_step(s: &Session, sequence: Sequence, step: &'static str) -> Result<Rebased, WriteError> {
    if step == "--continue" && !stages(s)?.is_empty() {
        return Err(WriteError::Refused(RESOLVE_FIRST));
    }
    let out = s.run(&Call::new(&[sequence.command(), step]).editor("true"))?;
    let under_way = Sequence::of(operation_state(&s.writer().dirs().git_dir)) == Some(sequence);
    if under_way && !stages(s)?.is_empty() {
        return Ok(Rebased::Conflicted);
    }
    if !out.success() {
        return Err(GitError::said(format!("{} {step}", sequence.command()), &out, None).into());
    }
    let head = s.rev("HEAD")?;
    Ok(Rebased::Done { head, autostash: Autostash::None })
}

/// Whether the index holds a conflicted path.
pub fn conflicted(s: &Session) -> Result<bool, WriteError> {
    Ok(!stages(s)?.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_starting_with_a_marker_is_a_marker() {
        assert!(has_markers(b"a\n<<<<<<< HEAD\nb\n"));
        assert!(has_markers(b"=======\r\n"));
        assert!(has_markers(b">>>>>>> main"));
        assert!(!has_markers(b"a <<<<<<< b\n======\n"));
    }
}
