//! What the diff viewer reads for one file, blocking, for `cx.background_spawn`: its contents before and
//! after the commit, decoded, and the language that highlights it.

use std::sync::Arc;

use supraman_core::text::{Encoding, decode};
use supraman_gitman::{ChangeKind, FileChange, Repo};
use supraman_editor::language_for;

/// The file to show: `path` in commit `sha`, its change there (`None` for a file the commit left as it was),
/// and the parent it is compared with (a merge's first, none for a root commit).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub sha: String,
    pub parent: Option<String>,
    pub path: String,
    pub change: Option<FileChange>,
}

impl Target {
    pub fn kind(&self) -> Option<ChangeKind> {
        self.change.as_ref().map(|c| c.kind)
    }

    /// The path before the commit: a rename's or copy's source, else the same path.
    pub fn old_path(&self) -> &str {
        self.change.as_ref().and_then(|c| c.old_path.as_deref()).unwrap_or(&self.path)
    }
}

pub enum Loaded {
    Binary,
    Text { old: String, new: String, encoding: Encoding, language: Option<Arc<supraman_editor::Language>> },
}

/// `None` when the side does not exist: an added file before, a deleted one after.
fn side(repo: &Repo, rev: Option<&str>, path: &str) -> Result<Option<supraman_gitman::Blob>, String> {
    let Some(rev) = rev else { return Ok(None) };
    supraman_gitman::blob(repo, rev, path).map(Some).map_err(|e| e.to_string())
}

pub fn load(repo: &Repo, target: &Target) -> Result<Loaded, String> {
    let kind = target.kind();
    let new_rev = (kind != Some(ChangeKind::Deleted)).then_some(target.sha.as_str());
    let old_rev = match kind {
        None => None,
        Some(ChangeKind::Added) => None,
        Some(_) => target.parent.as_deref(),
    };
    let new = side(repo, new_rev, &target.path)?;
    let old = side(repo, old_rev, target.old_path())?;
    if [&old, &new].into_iter().flatten().any(|b| b.binary) {
        return Ok(Loaded::Binary);
    }
    let (new_text, new_enc) = new.as_ref().map(|b| decode(&b.bytes)).unzip();
    let (old_text, old_enc) = old.as_ref().map(|b| decode(&b.bytes)).unzip();
    let encoding = new_enc.or(old_enc).unwrap_or(Encoding::Utf8);
    let new = new_text.unwrap_or_default();
    let old = match kind {
        None => new.clone(),
        _ => old_text.unwrap_or_default(),
    };
    Ok(Loaded::Text { old, new, encoding, language: language_for(&target.path) })
}
