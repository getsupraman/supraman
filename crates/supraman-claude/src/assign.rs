use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use supraman_core::folders::{Entries, FolderEntry};
use supraman_core::paths::{key_within, path_key};

use crate::Session;

/// The sessions by the [`path_key`] of the listed folder they show under; the rest are in none.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Placement {
    pub by_folder: HashMap<String, Vec<Session>>,
    pub unplaced: Vec<Session>,
}

pub fn place(sessions: &[Session], entries: &Entries) -> Placement {
    place_with(sessions, entries, main_repo)
}

/// [`place`] with `main_of` in place of [`main_repo`]: the app renders every frame from a table
/// filled once per poll, so the filesystem walk stays off the UI thread.
pub fn place_with(sessions: &[Session], entries: &Entries, main_of: impl Fn(&Path) -> Option<PathBuf>) -> Placement {
    let keys: Vec<String> = entries.folders().map(|f| path_key(&f.path)).collect();
    let mut out = Placement::default();
    for s in sessions {
        match folder_key(&s.cwd, &keys, &main_of) {
            Some(key) => out.by_folder.entry(key).or_default().push(s.clone()),
            None => out.unplaced.push(s.clone()),
        }
    }
    out
}

/// The listed folder a session in `cwd` shows under ([`place_with`]'s rule); a path listed twice is its first.
pub fn holding<'a>(cwd: &Path, entries: &'a Entries, main_of: impl Fn(&Path) -> Option<PathBuf>) -> Option<&'a FolderEntry> {
    let keys: Vec<String> = entries.folders().map(|f| path_key(&f.path)).collect();
    let key = folder_key(cwd, &keys, main_of)?;
    entries.folders().find(|f| path_key(&f.path) == key)
}

/// The deepest of `folders` holding `cwd` or the main checkout of the linked worktree `cwd` lies in.
fn folder_key(cwd: &Path, folders: &[String], main_of: impl Fn(&Path) -> Option<PathBuf>) -> Option<String> {
    let mut at = vec![path_key(cwd)];
    at.extend(main_of(cwd).map(|m| path_key(&m)));
    folders.iter().filter(|f| at.iter().any(|a| key_within(a, f))).max_by_key(|f| f.len()).cloned()
}

static MAIN_REPO_WALKS: AtomicU64 = AtomicU64::new(0);

/// How many times [`main_repo`] has walked the filesystem in this process; `--selftest` checks that
/// idle frames add none.
pub fn main_repo_walks() -> u64 {
    MAIN_REPO_WALKS.load(Ordering::Relaxed)
}

/// [`supraman_core::paths::main_checkout`], counted by [`main_repo_walks`].
pub fn main_repo(dir: &Path) -> Option<PathBuf> {
    MAIN_REPO_WALKS.fetch_add(1, Ordering::Relaxed);
    supraman_core::paths::main_checkout(dir)
}

#[cfg(test)]
mod tests;
