use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use supraman_core::folders::{Entries, FolderEntry, Shown};
use supraman_core::paths::{key_within, path_key};

use crate::Session;

/// The sessions by the [`path_key`] of the listed or shown folder they show under; the rest are in none.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Placement {
    pub by_folder: HashMap<String, Vec<Session>>,
    pub unplaced: Vec<Session>,
}

pub fn place(sessions: &[Session], entries: &Entries, shown: &Shown) -> Placement {
    place_with(sessions, entries, shown, main_repo)
}

/// [`place`] with `main_of` in place of [`main_repo`]: the app renders every frame from a table
/// filled once per poll, so the filesystem walk stays off the UI thread.
pub fn place_with(sessions: &[Session], entries: &Entries, shown: &Shown, main_of: impl Fn(&Path) -> Option<PathBuf>) -> Placement {
    let keys: Vec<String> = entries.placed(shown).map(path_key).collect();
    let mut out = Placement::default();
    for s in sessions {
        match folder_key(&s.cwd, &keys, &main_of) {
            Some(key) => out.by_folder.entry(key).or_default().push(s.clone()),
            None => out.unplaced.push(s.clone()),
        }
    }
    out
}

/// The folder a session in `cwd` shows under, by [`place_with`]'s rule, and the listed folder it sits in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Holding<'a> {
    /// The deepest listed folder or shown child holding `cwd`: the session's folder.
    pub folder: PathBuf,
    /// `folder` itself when listed, else the listed folder showing it: the entry's and the schedules' folder.
    pub listed: &'a FolderEntry,
}

/// The folder holding `cwd` among `entries` and their `shown` children ([`Entries::placed`]); a path listed twice
/// is its first. `sm` has no shown children and passes an empty [`Shown`].
pub fn holding<'a>(cwd: &Path, entries: &'a Entries, shown: &Shown, main_of: impl Fn(&Path) -> Option<PathBuf>) -> Option<Holding<'a>> {
    let placed: Vec<&Path> = entries.placed(shown).collect();
    let keys: Vec<String> = placed.iter().map(|p| path_key(p)).collect();
    let key = folder_key(cwd, &keys, main_of)?;
    let folder = placed[keys.iter().position(|k| *k == key)?].to_path_buf();
    let shows = |f: &FolderEntry| f.show_subfolders && shown.get(&path_key(&f.path)).is_some_and(|c| c.iter().any(|c| path_key(c) == key));
    let listed = entries.folders().find(|f| path_key(&f.path) == key).or_else(|| entries.folders().find(|f| shows(f)))?;
    Some(Holding { folder, listed })
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
