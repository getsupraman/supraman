//! The earlier per-workspace schedule files, moved once to their folders' files. Compat code removed in 0.1.19 with
//! `end_controllers` (backlog/end-controllers-removal).

use std::fs;
use std::path::{Path, PathBuf};

use super::store::{edit_raw, folder_key, schedules_dir, toml_stems};
use super::{Schedule, Status, append_run, live_dir, read_runs, runs_path, secret_key, state_dir};
use crate::secret::SecretStore;
use crate::{CoreError, Result};

/// The file a migration leaves in the schedules dir for the app's next `sync_all`, which removes it.
pub const RESYNC: &str = "resync";
const UNPLACED: &str = "unplaced";

/// What one [`migrate`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Migrated {
    /// `(workspace, id, folder)` of each schedule now in its folder's file, pending.
    pub moved: Vec<(String, String, PathBuf)>,
    /// Each schedule no folder took, in words; it is kept under `schedules/unplaced/`.
    pub unplaced: Vec<String>,
}

/// Moves each `<home>/schedules/<workspace>.toml` of the earlier shape (no `folder`) to its folders' files, once:
/// each schedule to the file of its workspace's folder by name, pending (C100: nothing re-signs an approval), with
/// its run records, live and state files and secrets; one whose folder is not found is kept as [`unplaced`] says.
pub fn migrate(home: &Path, store: &dyn SecretStore) -> Result<Migrated> {
    let dir = schedules_dir(home);
    let old = |ws: &String| is_old(&dir.join(format!("{ws}.toml")));
    if !toml_stems(&dir).iter().any(old) {
        return Ok(Migrated::default());
    }
    let lock = dir.join("migrate.lock");
    let _held = crate::fs::lock_file(&lock).map_err(|e| CoreError::io(&lock, e))?;
    let workspaces = crate::folders::migrate::workspace_folders(home);
    let mut out = Migrated::default();
    for ws in toml_stems(&dir).into_iter().filter(old) {
        let folders = workspaces.iter().find(|(name, _)| *name == ws).map(|(_, f)| f.as_slice()).unwrap_or_default();
        workspace(home, store, &ws, folders, &mut out)?;
    }
    if !out.moved.is_empty() {
        let resync = dir.join(RESYNC);
        crate::fs::write_atomic(&resync, "").map_err(|e| CoreError::io(&resync, e))?;
    }
    Ok(out)
}

/// A schedules file without its folder: the per-workspace shape.
fn is_old(path: &Path) -> bool {
    fs::read_to_string(path).ok().and_then(|t| t.parse::<toml::Table>().ok()).is_some_and(|t| !t.contains_key("folder"))
}

fn workspace(home: &Path, store: &dyn SecretStore, ws: &str, folders: &[(String, PathBuf)], out: &mut Migrated) -> Result<()> {
    let path = schedules_dir(home).join(format!("{ws}.toml"));
    let text = fs::read_to_string(&path).map_err(|e| CoreError::io(&path, e))?;
    let table: toml::Table = text.parse().map_err(|source| CoreError::Parse { path: path.clone(), source })?;
    let tables = table.get("schedule").and_then(toml::Value::as_array).cloned().unwrap_or_default();
    let (mut placed, mut kept): (Vec<(PathBuf, Schedule)>, Vec<toml::Value>) = (Vec::new(), Vec::new());
    for t in tables {
        let named = t.get("folder").and_then(toml::Value::as_str).unwrap_or_default().to_string();
        match (t.clone().try_into::<Schedule>(), folders.iter().find(|(name, _)| *name == named)) {
            (Ok(s), Some((_, folder))) => placed.push((folder.clone(), Schedule { status: Status::Pending, ..s })),
            _ => {
                out.unplaced.push(format!("schedule {} of the former workspace {ws} names folder '{named}', which it no longer has", id_of(&t)));
                kept.push(t);
            }
        }
    }
    let mut moved = Vec::new();
    for (folder, s) in placed {
        let added = edit_raw(home, &folder, |all| match all.iter().find(|o| o.id == s.id) {
            Some(o) => Ok(o.created_at == s.created_at),
            None => {
                all.push(s.clone());
                Ok(true)
            }
        })?;
        if added {
            moved.push((s, folder));
        } else {
            out.unplaced.push(format!("schedule {} of the former workspace {ws}: {} has a schedule of that id already", s.id, folder.display()));
            kept.push(toml::Value::try_from(&s).map_err(|source| CoreError::SerializeSchedules { folder: folder.display().to_string(), source })?);
        }
    }
    for (s, folder) in &moved {
        files(home, store, ws, s, &folder_key(folder))?;
        out.moved.push((ws.to_string(), s.id.clone(), folder.clone()));
    }
    runs(home, ws, &moved, kept.is_empty())?;
    if !kept.is_empty() {
        keep(home, ws, kept)?;
    }
    fs::remove_file(&path).map_err(|e| CoreError::io(&path, e))?;
    let _ = fs::remove_file(schedules_dir(home).join(format!("{ws}.lock")));
    for dir in [live_dir(home, ws)?, schedules_dir(home).join("state").join(ws)] {
        let _ = fs::remove_dir(dir);
    }
    Ok(())
}

fn id_of(t: &toml::Value) -> &str {
    t.get("id").and_then(toml::Value::as_str).unwrap_or("?")
}

/// The live files, state dir and secrets of schedule `s` of `ws`, moved to `key`.
fn files(home: &Path, store: &dyn SecretStore, ws: &str, s: &Schedule, key: &str) -> Result<()> {
    let (from, to) = (live_dir(home, ws)?, live_dir(home, key)?);
    if let Ok(entries) = fs::read_dir(&from) {
        let prefix = format!("{}.", s.id);
        for e in entries.flatten().filter(|e| e.file_name().to_string_lossy().starts_with(&prefix)) {
            fs::create_dir_all(&to).map_err(|err| CoreError::io(&to, err))?;
            fs::rename(e.path(), to.join(e.file_name())).map_err(|err| CoreError::io(e.path(), err))?;
        }
    }
    let (from, to) = (state_dir(home, ws, &s.id)?, state_dir(home, key, &s.id)?);
    if from.is_dir() {
        fs::create_dir_all(to.parent().expect("under the home")).map_err(|e| CoreError::io(&to, e))?;
        fs::rename(&from, &to).map_err(|e| CoreError::io(&from, e))?;
    }
    for name in &s.permissions.secrets {
        let old = secret_key(ws, &s.id, name);
        if let Some(value) = store.get(&old).map_err(CoreError::Secret)? {
            store.set(&secret_key(key, &s.id, name), &value).map_err(CoreError::Secret)?;
            store.delete(&old).map_err(CoreError::Secret)?;
        }
    }
    Ok(())
}

/// Each moved schedule's run records appended to its folder's; `ws`'s records go once nothing of it stays behind.
fn runs(home: &Path, ws: &str, moved: &[(Schedule, PathBuf)], all_moved: bool) -> Result<()> {
    for r in read_runs(home, ws)? {
        if let Some((_, folder)) = moved.iter().find(|(s, _)| s.id == r.schedule) {
            append_run(home, &folder_key(folder), &r)?;
        }
    }
    if all_moved {
        let path = runs_path(home, ws)?;
        for file in [crate::fs::rotated(&path), path] {
            let _ = fs::remove_file(file);
        }
    }
    Ok(())
}

fn keep(home: &Path, ws: &str, kept: Vec<toml::Value>) -> Result<()> {
    let path = schedules_dir(home).join(UNPLACED).join(format!("{ws}.toml"));
    let mut table = toml::Table::new();
    table.insert("schedule".into(), toml::Value::Array(kept));
    let text = toml::to_string(&table).map_err(|source| CoreError::SerializeSchedules { folder: ws.into(), source })?;
    fs::create_dir_all(path.parent().expect("under the home")).map_err(|e| CoreError::io(&path, e))?;
    crate::fs::write_atomic(&path, text).map_err(|e| CoreError::io(&path, e))
}

/// A line for each schedule a migration found no folder for, naming it and where it is kept.
pub(super) fn unplaced(home: &Path) -> Vec<String> {
    let dir = schedules_dir(home).join(UNPLACED);
    let mut out = Vec::new();
    for ws in toml_stems(&dir) {
        let path = dir.join(format!("{ws}.toml"));
        let table = fs::read_to_string(&path).ok().and_then(|t| t.parse::<toml::Table>().ok()).unwrap_or_default();
        for t in table.get("schedule").and_then(toml::Value::as_array).into_iter().flatten() {
            let named = t.get("folder").and_then(toml::Value::as_str).unwrap_or("?");
            out.push(format!("Schedule {} of the former workspace {ws} has no folder (it named '{named}'); kept in {}", id_of(t), path.display()));
        }
    }
    out
}
