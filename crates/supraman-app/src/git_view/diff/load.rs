//! What the diff viewer reads for one file, blocking, for `cx.background_spawn`: its two sides (a commit and
//! its parent, HEAD and the index, the index and the worktree), decoded, and the language that highlights it.

use std::sync::Arc;

use supraman_core::text::{Encoding, decode};
use supraman_gitman::{Blob, ChangeKind, FileChange, Repo};
use supraman_editor::language_for;

/// What a file's two sides are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Of {
    /// The commit `sha` against `parent` (a merge's first, none for a root commit).
    Commit { sha: String, parent: Option<String> },
    /// HEAD against the index.
    Staged,
    /// The index against the worktree.
    Unstaged,
    /// Nothing against the worktree.
    Untracked,
    /// The worktree file as it stands, conflict markers and all.
    Conflicted,
}

/// The file to show: `path` in `of`, and its change there (`None` for a file a commit left as it was).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub of: Of,
    pub path: String,
    pub change: Option<FileChange>,
}

impl Target {
    pub fn commit(sha: String, parent: Option<String>, path: String, change: Option<FileChange>) -> Target {
        Target { of: Of::Commit { sha, parent }, path, change }
    }

    pub fn kind(&self) -> Option<ChangeKind> {
        self.change.as_ref().map(|c| c.kind)
    }

    /// The path before the change: a rename's or copy's source, else the same path.
    pub fn old_path(&self) -> &str {
        self.change.as_ref().and_then(|c| c.old_path.as_deref()).unwrap_or(&self.path)
    }

    /// The working-tree side shown, for the header; `None` for a commit's file.
    pub fn side_word(&self) -> Option<&'static str> {
        match self.of {
            Of::Commit { .. } => None,
            Of::Staged => Some("Staged"),
            Of::Unstaged => Some("Unstaged"),
            Of::Untracked => Some("Untracked"),
            Of::Conflicted => Some("Conflicted"),
        }
    }
}

pub enum Loaded {
    Binary,
    Text { old: String, new: String, encoding: Encoding, language: Option<Arc<supraman_editor::Language>> },
}

/// Where one side of a file is read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side<'a> {
    /// The side does not exist: an added file before, a deleted one after.
    Absent,
    Rev(&'a str, &'a str),
    Index(&'a str),
    Worktree(&'a str),
    /// Both sides are the new one: a file a commit left as it was, or a conflicted one.
    SameAsNew,
}

fn sides(target: &Target) -> (Side<'_>, Side<'_>) {
    let (path, old_path, kind) = (target.path.as_str(), target.old_path(), target.kind());
    let deleted = kind == Some(ChangeKind::Deleted);
    let added = kind == Some(ChangeKind::Added);
    match &target.of {
        Of::Commit { sha, parent } => {
            let new = if deleted { Side::Absent } else { Side::Rev(sha, path) };
            let old = match (kind, parent) {
                (None, _) => Side::SameAsNew,
                (Some(ChangeKind::Added), _) | (_, None) => Side::Absent,
                (Some(_), Some(parent)) => Side::Rev(parent, old_path),
            };
            (old, new)
        }
        Of::Staged => (if added { Side::Absent } else { Side::Rev("HEAD", old_path) }, if deleted { Side::Absent } else { Side::Index(path) }),
        Of::Unstaged => (if added { Side::Absent } else { Side::Index(path) }, if deleted { Side::Absent } else { Side::Worktree(path) }),
        Of::Untracked => (Side::Absent, Side::Worktree(path)),
        Of::Conflicted => (Side::SameAsNew, Side::Worktree(path)),
    }
}

fn read(repo: &Repo, side: Side<'_>) -> Result<Option<Blob>, String> {
    let blob = match side {
        Side::Absent | Side::SameAsNew => return Ok(None),
        Side::Rev(rev, path) => supraman_gitman::blob(repo, rev, path),
        Side::Index(path) => supraman_gitman::staged_blob(repo, path),
        Side::Worktree(path) => supraman_gitman::worktree_file(repo, path),
    };
    blob.map(Some).map_err(|e| e.to_string())
}

/// `\r\n` as `\n`, as the editor's buffers hold it, so a worktree file checked out with CRLF matches its blob.
fn text_of(blob: &Blob) -> (String, Encoding) {
    let (text, encoding) = decode(&blob.bytes);
    (if text.contains('\r') { text.replace("\r\n", "\n") } else { text }, encoding)
}

pub fn load(repo: &Repo, target: &Target) -> Result<Loaded, String> {
    let (old_side, new_side) = sides(target);
    let new = read(repo, new_side)?;
    let old = read(repo, old_side)?;
    if [&old, &new].into_iter().flatten().any(|b| b.binary) {
        return Ok(Loaded::Binary);
    }
    let (new_text, new_enc) = new.as_ref().map(text_of).unzip();
    let (old_text, old_enc) = old.as_ref().map(text_of).unzip();
    let encoding = new_enc.or(old_enc).unwrap_or(Encoding::Utf8);
    let new = new_text.unwrap_or_default();
    let old = if old_side == Side::SameAsNew { new.clone() } else { old_text.unwrap_or_default() };
    Ok(Loaded::Text { old, new, encoding, language: language_for(&target.path) })
}
