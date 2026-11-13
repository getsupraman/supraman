use std::collections::HashSet;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::approval::{covered, covered_toml};
use super::{Schedule, ScheduleFile, Status};
use crate::paths::{folder_name, path_key};
use crate::{CoreError, Result, slugify, validate_name};

/// A [`ScheduleFile`] in the shape keyed by folder.
pub const FILE_VERSION: u32 = 2;
const KEY_SLUG_MAX: usize = 20;
const KEY_HASH_LEN: usize = 8;

pub fn schedules_dir(home: &Path) -> PathBuf {
    home.join("schedules")
}

/// The one key of a folder's schedules: its name's slug and the first hex digits of its [`path_key`]'s sha256,
/// such as `gaia-1a2b3c4d`; a [`validate_name`] name, so it is safe in a file, task folder and session name.
pub fn folder_key(folder: &Path) -> String {
    let hash = crate::hex::encode(&Sha256::digest(path_key(folder).as_bytes()));
    let slug: String = slugify(&folder_name(folder)).chars().take(KEY_SLUG_MAX).collect();
    match slug.trim_end_matches('-') {
        "" => format!("folder-{}", &hash[..KEY_HASH_LEN]),
        slug => format!("{slug}-{}", &hash[..KEY_HASH_LEN]),
    }
}

/// Whether `key` has [`folder_key`]'s shape.
pub fn is_folder_key(key: &str) -> bool {
    validate_name(key).is_ok()
        && key.rsplit_once('-').is_some_and(|(slug, hash)| !slug.is_empty() && hash.len() == KEY_HASH_LEN && hash.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
}

/// `<home>/schedules/<key>.toml`.
pub fn file_path(home: &Path, key: &str) -> Result<PathBuf> {
    validate_name(key)?;
    Ok(schedules_dir(home).join(format!("{key}.toml")))
}

fn lock_path(home: &Path, key: &str) -> PathBuf {
    schedules_dir(home).join(format!("{key}.lock"))
}

/// One folder's schedules as its file holds them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scheduled {
    pub folder: PathBuf,
    pub key: String,
    pub schedules: Vec<Schedule>,
}

/// `folder`'s schedules; none when its file is missing.
pub fn load(home: &Path, folder: &Path) -> Result<Vec<Schedule>> {
    Ok(load_file(&file_path(home, &folder_key(folder))?)?.schedule)
}

/// The schedules filed under `key`, with their folder; `None` when there is no such file.
pub fn load_key(home: &Path, key: &str) -> Result<Option<Scheduled>> {
    let path = file_path(home, key)?;
    if !path.exists() {
        return Ok(None);
    }
    let file = load_file(&path)?;
    if file.version < FILE_VERSION || folder_key(&file.folder) != key {
        return Err(CoreError::InvalidSchedule { id: key.into(), why: format!("{} does not hold the schedules of one folder", path.display()) });
    }
    Ok(Some(Scheduled { folder: file.folder, key: key.into(), schedules: file.schedule }))
}

/// Every folder's schedules, by key, and a line for each file that cannot be read or holds schedules no folder took.
pub fn load_all(home: &Path) -> (Vec<Scheduled>, Vec<String>) {
    let dir = schedules_dir(home);
    let (mut out, mut errors) = (Vec::new(), Vec::new());
    for key in toml_stems(&dir) {
        match load_key(home, &key) {
            Ok(Some(s)) => out.push(s),
            Ok(None) => {}
            Err(e) => errors.push(e.to_string()),
        }
    }
    errors.extend(super::migrate::unplaced(home));
    (out, errors)
}

/// The stems of `dir`'s `*.toml` files that are names, sorted.
pub(super) fn toml_stems(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<String> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .filter_map(|p| p.file_stem().and_then(|s| s.to_str()).map(str::to_string))
        .filter(|stem| validate_name(stem).is_ok())
        .collect();
    out.sort();
    out
}

fn load_file(path: &Path) -> Result<ScheduleFile> {
    match fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text).map_err(|source| CoreError::Parse { path: path.to_path_buf(), source }),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(ScheduleFile::default()),
        Err(e) => Err(CoreError::io(path, e)),
    }
}

/// Applies `change` to `folder`'s schedules and saves them under `<key>.lock`; nothing is written when
/// `change` or a check fails. `approval` stays as it was, a covered change sets `pending` and clears
/// `worktree`, and `active` holds only where the approval's fingerprint is the current one (C100).
pub fn edit<T>(home: &Path, folder: &Path, change: impl FnOnce(&mut Vec<Schedule>) -> Result<T>) -> Result<T> {
    edit_raw(home, folder, |schedules| {
        let before = schedules.clone();
        let out = change(schedules)?;
        let mut ids = HashSet::new();
        for s in schedules.iter_mut() {
            if !ids.insert(s.id.clone()) {
                return Err(CoreError::InvalidSchedule { id: s.id.clone(), why: "the id is taken in this folder".into() });
            }
            let old = before.iter().find(|o| o.id == s.id);
            s.approval = old.and_then(|o| o.approval.clone());
            if old != Some(&*s) {
                s.validate().map_err(|why| CoreError::InvalidSchedule { id: s.id.clone(), why })?;
            }
            if old.is_none() && s.created_at == 0 {
                s.created_at = crate::time::now_ms();
            }
            let changed = old.is_some_and(|o| covered_toml(folder, o) != covered_toml(folder, s));
            if changed {
                s.status = Status::Pending;
                s.action.worktree = false;
            } else if s.status == Status::Active && old.is_none_or(|o| o.status != Status::Active) {
                let approved = s.approval.as_ref().is_some_and(|a| covered(folder, s).is_ok_and(|c| c.fingerprint == a.fingerprint));
                if !approved {
                    s.status = Status::Pending;
                }
            }
        }
        Ok(out)
    })
}

/// [`edit`] without its rules: for [`super::approve`] and the migration alone.
pub(super) fn edit_raw<T>(home: &Path, folder: &Path, change: impl FnOnce(&mut Vec<Schedule>) -> Result<T>) -> Result<T> {
    let key = folder_key(folder);
    let path = file_path(home, &key)?;
    let lock = lock_path(home, &key);
    let _held = crate::fs::lock_file(&lock).map_err(|e| CoreError::io(&lock, e))?;
    let mut file = load_file(&path)?;
    let out = change(&mut file.schedule)?;
    file.version = FILE_VERSION;
    file.folder = folder.to_path_buf();
    let text = toml::to_string(&file).map_err(|source| CoreError::SerializeSchedules { folder: folder.display().to_string(), source })?;
    crate::fs::write_atomic(&path, text).map_err(|e| CoreError::io(&path, e))?;
    Ok(out)
}
