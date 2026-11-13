//! `<home>/folders.toml`: the sidebar's ordered folders and named groups, and their one reader and writer.

mod edits;
pub(crate) mod migrate;
mod subfolders;

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::paths::path_key;
use crate::{CoreError, Result};

pub use edits::{Place, Pos};
pub use subfolders::{SKIPPED, Shown, shown, subfolders};

pub const FILE: &str = "folders.toml";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FolderKind {
    #[default]
    Git,
    Plain,
}

const LOCK: &str = "folders.lock";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderEntry {
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "is_false")]
    pub show_subfolders: bool,
}

impl FolderEntry {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into(), show_subfolders: false }
    }
}

fn is_false(b: &bool) -> bool {
    !b
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Stored", into = "Stored")]
pub enum Entry {
    Folder(FolderEntry),
    Group { name: String, folders: Vec<FolderEntry> },
}

impl Entry {
    /// The folders this entry lists: itself, or a group's members.
    pub fn folders(&self) -> &[FolderEntry] {
        match self {
            Entry::Folder(f) => std::slice::from_ref(f),
            Entry::Group { folders, .. } => folders,
        }
    }

    pub fn group_name(&self) -> Option<&str> {
        match self {
            Entry::Group { name, .. } => Some(name),
            Entry::Folder(_) => None,
        }
    }
}

/// An entry as the file holds it: `path` for a folder, `group` with its `folders` for a group.
#[derive(Serialize, Deserialize)]
struct Stored {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "is_false")]
    show_subfolders: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    folders: Option<Vec<FolderEntry>>,
}

impl TryFrom<Stored> for Entry {
    type Error = String;

    fn try_from(s: Stored) -> std::result::Result<Self, String> {
        match (s.path, s.group) {
            (Some(path), None) if s.folders.is_none() => Ok(Entry::Folder(FolderEntry { path, show_subfolders: s.show_subfolders })),
            (None, Some(name)) => Ok(Entry::Group { name, folders: s.folders.unwrap_or_default() }),
            _ => Err("an entry is a folder (`path`) or a group (`group` with its `folders`)".into()),
        }
    }
}

impl From<Entry> for Stored {
    fn from(e: Entry) -> Self {
        match e {
            Entry::Folder(f) => Stored { path: Some(f.path), show_subfolders: f.show_subfolders, group: None, folders: None },
            Entry::Group { name, folders } => Stored { path: None, show_subfolders: false, group: Some(name), folders: Some(folders) },
        }
    }
}

/// The sidebar's entries in order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entries {
    #[serde(default, rename = "entry")]
    pub list: Vec<Entry>,
}

impl Entries {
    pub fn new(list: Vec<Entry>) -> Self {
        Self { list }
    }

    /// Every listed folder, in order; a path listed twice comes twice.
    pub fn folders(&self) -> impl Iterator<Item = &FolderEntry> {
        self.list.iter().flat_map(Entry::folders)
    }

    pub fn groups(&self) -> impl Iterator<Item = (&str, &[FolderEntry])> {
        self.list.iter().filter_map(|e| match e {
            Entry::Group { name, folders } => Some((name.as_str(), folders.as_slice())),
            Entry::Folder(_) => None,
        })
    }

    /// Whether `path` is listed anywhere, compared by [`path_key`].
    pub fn lists(&self, path: &Path) -> bool {
        let key = path_key(path);
        self.folders().any(|f| path_key(&f.path) == key)
    }
}

/// What the disk says of a listed folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnDisk {
    pub kind: FolderKind,
    pub branch: Option<String>,
}

/// `path`'s kind (git when it holds a `.git` folder or file) and checked-out branch; blocking.
pub fn on_disk(path: &Path) -> OnDisk {
    let kind = if path.join(".git").exists() { FolderKind::Git } else { FolderKind::Plain };
    OnDisk { kind, branch: (kind == FolderKind::Git).then(|| crate::branch_at(path)).flatten() }
}

pub fn path(home: &Path) -> PathBuf {
    home.join(FILE)
}

/// The entries, created once from the workspaces when the file does not exist; a file that does not
/// parse is an error and stays as it is.
pub fn load(home: &Path) -> Result<Entries> {
    match read(home)? {
        Some(entries) => Ok(entries),
        None => edit(home, |_| Ok(())).map(|(entries, ())| entries),
    }
}

/// Load, `change`, save, under `<home>/folders.lock`; nothing is written when `change` refuses.
pub fn edit<T>(home: &Path, change: impl FnOnce(&mut Entries) -> Result<T>) -> Result<(Entries, T)> {
    let lock = home.join(LOCK);
    let _held = crate::fs::lock_file(&lock).map_err(|e| CoreError::io(&lock, e))?;
    let mut entries = match read(home)? {
        Some(entries) => entries,
        None => migrate::from_workspaces(home),
    };
    let out = change(&mut entries)?;
    let file = path(home);
    let text = toml::to_string(&entries).map_err(|source| CoreError::Serialize { name: FILE.into(), source })?;
    crate::fs::write_atomic(&file, text).map_err(|e| CoreError::io(&file, e))?;
    Ok((entries, out))
}

fn read(home: &Path) -> Result<Option<Entries>> {
    let file = path(home);
    match fs::read_to_string(&file) {
        Ok(text) => toml::from_str(&text).map(Some).map_err(|source| CoreError::Parse { path: file, source }),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(CoreError::io(&file, e)),
    }
}

#[cfg(test)]
mod tests;
