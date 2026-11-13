//! What the diff viewer reads for one file, blocking: its two sides decoded, and the language that highlights it.

use std::sync::Arc;

use gpui::{ImageFormat, SvgRenderer};
use supraman_core::text::{BINARY_PROBE, Encoding, decode};
use supraman_gitman::{Blob, ChangeKind, FileChange, Repo};
use supraman_editor::language_for;

use super::image::{self, Images};
use super::invisible;

/// What a file's two sides are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Of {
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
    pub(crate) of: Of,
    pub(crate) path: String,
    pub(crate) change: Option<FileChange>,
}

impl Target {
    pub fn commit(sha: String, parent: Option<String>, path: String, change: Option<FileChange>) -> Target {
        Target { of: Of::Commit { sha, parent }, path, change }
    }

    pub(crate) fn kind(&self) -> Option<ChangeKind> {
        self.change.as_ref().map(|c| c.kind)
    }

    /// The path before the change: a rename's or copy's source, else the same path.
    pub(crate) fn old_path(&self) -> &str {
        self.change.as_ref().and_then(|c| c.old_path.as_deref()).unwrap_or(&self.path)
    }

    /// The working-tree side shown, for the header; `None` for a commit's file.
    pub(crate) fn side_word(&self) -> Option<&'static str> {
        match self.of {
            Of::Commit { .. } => None,
            Of::Staged => Some("Staged"),
            Of::Unstaged => Some("Unstaged"),
            Of::Untracked => Some("Untracked"),
            Of::Conflicted => Some("Conflicted"),
        }
    }
}

/// Each side's size in bytes; `None` for a side that does not exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Sizes {
    pub(crate) old: Option<usize>,
    pub(crate) new: Option<usize>,
}

/// The largest file the viewer reads; a larger side shows only its size.
pub(crate) const MAX_SHOWN_BYTES: u64 = 6 * 1024 * 1024;

/// Why a file is not shown: a side over [`MAX_SHOWN_BYTES`], or an image side over [`image::MAX_PIXELS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TooLarge {
    Bytes(Sizes),
    Pixels { old: Option<(u32, u32)>, new: Option<(u32, u32)> },
}

pub(crate) enum Loaded {
    Binary(Sizes),
    TooLarge(TooLarge),
    Image(Box<Images>),
    /// `invisible` says what changed when the bytes differ but the texts do not.
    Text { old: String, new: String, encoding: String, invisible: Vec<String>, language: Option<Arc<supraman_editor::Language>> },
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

/// A side whole, or with `head` its first [`BINARY_PROBE`] bytes, all `classify` needs to call it binary.
fn read(repo: &Repo, side: Side<'_>, head: bool) -> Result<Option<Blob>, String> {
    use supraman_gitman::{blob, blob_head, staged_blob, staged_blob_head, worktree_file, worktree_head};
    let blob = match (side, head) {
        (Side::Absent | Side::SameAsNew, _) => return Ok(None),
        (Side::Rev(rev, path), false) => blob(repo, rev, path),
        (Side::Rev(rev, path), true) => blob_head(repo, rev, path),
        (Side::Index(path), false) => staged_blob(repo, path),
        (Side::Index(path), true) => staged_blob_head(repo, path),
        (Side::Worktree(path), false) => worktree_file(repo, path),
        (Side::Worktree(path), true) => worktree_head(repo, path),
    };
    blob.map(Some).map_err(|e| e.to_string())
}

/// The side whole: its head when that was all of it, else read again.
fn read_rest(repo: &Repo, side: Side<'_>, head: Option<Blob>) -> Result<Option<Blob>, String> {
    match head {
        Some(head) if head.bytes.len() < BINARY_PROBE => Ok(Some(head)),
        _ => read(repo, side, false),
    }
}

/// Both sides whole, but only their heads once a head says a file that is no image is binary.
pub(super) fn read_sides(repo: &Repo, target: &Target) -> Result<(Option<Blob>, Option<Blob>), String> {
    let (old_side, new_side) = sides(target);
    let (old, new) = if image::format_of(&target.path).is_some() {
        (read(repo, old_side, false)?, read(repo, new_side, false)?)
    } else {
        let (old, new) = (read(repo, old_side, true)?, read(repo, new_side, true)?);
        if [&old, &new].into_iter().flatten().any(|b| b.binary) {
            return Ok((old, new));
        }
        (read_rest(repo, old_side, old)?, read_rest(repo, new_side, new)?)
    };
    Ok((if old_side == Side::SameAsNew { new.clone() } else { old }, new))
}

/// Each side's size in bytes, asked before either is read: one `cat-file --batch-check` for the git sides.
fn sizes(repo: &Repo, old: Side<'_>, new: Side<'_>) -> Result<Sizes, String> {
    let object = |side: Side<'_>| match side {
        Side::Rev(rev, path) => Some(format!("{rev}:{path}")),
        Side::Index(path) => Some(format!(":0:{path}")),
        _ => None,
    };
    let objects: Vec<String> = [old, new].into_iter().filter_map(object).collect();
    let objects: Vec<&str> = objects.iter().map(String::as_str).collect();
    let mut asked = supraman_gitman::object_sizes(repo, &objects).map_err(|e| e.to_string())?.into_iter();
    let mut size = |side: Side<'_>| -> Result<Option<usize>, String> {
        let bytes = match side {
            Side::Absent | Side::SameAsNew => None,
            Side::Rev(..) | Side::Index(_) => asked.next().flatten(),
            Side::Worktree(path) => supraman_gitman::worktree_size(repo, path).map_err(|e| e.to_string())?,
        };
        Ok(bytes.map(|b| usize::try_from(b).unwrap_or(usize::MAX)))
    };
    let old_size = size(old)?;
    let new_size = size(new)?;
    Ok(Sizes { old: if old == Side::SameAsNew { new_size } else { old_size }, new: new_size })
}

/// `\r\n` as `\n`, as the editor's buffers hold it, so a worktree file checked out with CRLF matches its blob.
fn text_of(blob: &Blob) -> (String, Encoding) {
    let (text, encoding) = decode(&blob.bytes);
    (if text.contains('\r') { text.replace("\r\n", "\n") } else { text }, encoding)
}

fn images(target: &Target, old: Option<&Blob>, new: Option<&Blob>, svg: &SvgRenderer) -> Option<Loaded> {
    let format = image::format_of(&target.path)?;
    let dimensions = |blob: Option<&Blob>| blob.and_then(|b| image::dimensions(format, &b.bytes));
    let (old_px, new_px) = (dimensions(old), dimensions(new));
    if [old_px, new_px].into_iter().flatten().any(|px| image::decoded_pixels(format, px) > image::MAX_PIXELS) {
        return Some(Loaded::TooLarge(TooLarge::Pixels { old: old_px, new: new_px }));
    }
    if format == ImageFormat::Svg && [(old, old_px), (new, new_px)].into_iter().any(|(blob, px)| blob.is_some() && px.is_none()) {
        return None;
    }
    let decoded = |blob: Option<&Blob>| blob.map(|b| image::decode(format, &b.bytes, svg).ok_or(())).transpose();
    let images = decoded(old).and_then(|old| decoded(new).map(|new| Images { format, old, new }));
    images.ok().map(|images| Loaded::Image(Box::new(images)))
}

pub(crate) fn load(repo: &Repo, target: &Target, svg: &SvgRenderer) -> Result<Loaded, String> {
    let (old_side, new_side) = sides(target);
    let sizes = sizes(repo, old_side, new_side)?;
    if [sizes.old, sizes.new].into_iter().flatten().any(|b| b as u64 > MAX_SHOWN_BYTES) {
        return Ok(Loaded::TooLarge(TooLarge::Bytes(sizes)));
    }
    let (old, new) = read_sides(repo, target)?;
    if image::format_of(&target.path).is_some() || [&old, &new].into_iter().flatten().any(|b| b.binary) {
        return Ok(images(target, old.as_ref(), new.as_ref(), svg).unwrap_or(Loaded::Binary(sizes)));
    }
    let (new_text, new_enc) = new.as_ref().map(text_of).unzip();
    let (old_text, old_enc) = old.as_ref().map(text_of).unzip();
    let invisible = match (&old, &new) {
        (Some(o), Some(n)) if old_text == new_text => invisible::changes(&o.bytes, &n.bytes),
        _ => Vec::new(),
    };
    let encoding = invisible::encoding_label(old_enc, new_enc);
    Ok(Loaded::Text { old: old_text.unwrap_or_default(), new: new_text.unwrap_or_default(), encoding, invisible, language: language_for(&target.path) })
}
