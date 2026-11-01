use std::collections::HashSet;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use super::approval::{covered, covered_toml};
use super::{Schedule, ScheduleFile, Status};
use crate::{CoreError, Result, Workspace, validate_name};

pub fn schedules_dir(home: &Path) -> PathBuf {
    home.join("schedules")
}

/// `<home>/schedules/<workspace>.toml`.
pub fn file_path(home: &Path, workspace: &str) -> Result<PathBuf> {
    validate_name(workspace)?;
    Ok(schedules_dir(home).join(format!("{workspace}.toml")))
}

fn lock_path(home: &Path, workspace: &str) -> PathBuf {
    schedules_dir(home).join(format!("{workspace}.lock"))
}

/// The workspace's schedules; none when its file is missing.
pub fn load(home: &Path, workspace: &str) -> Result<Vec<Schedule>> {
    Ok(load_file(&file_path(home, workspace)?)?.schedule)
}

fn load_file(path: &Path) -> Result<ScheduleFile> {
    match fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text).map_err(|source| CoreError::Parse { path: path.to_path_buf(), source }),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(ScheduleFile::default()),
        Err(e) => Err(CoreError::io(path, e)),
    }
}

/// Applies `change` to `workspace`'s schedules and saves them under `<workspace>.lock`; nothing is written
/// when `change` or a check fails. `approval` stays as it was, a covered change sets `pending` and clears
/// `worktree`, and `active` holds only where the approval's fingerprint is the current one (C100).
pub fn edit<T>(home: &Path, workspace: &Workspace, change: impl FnOnce(&mut Vec<Schedule>) -> Result<T>) -> Result<T> {
    edit_raw(home, workspace, |schedules| {
        let before = schedules.clone();
        let out = change(schedules)?;
        let mut ids = HashSet::new();
        for s in schedules.iter_mut() {
            if !ids.insert(s.id.clone()) {
                return Err(CoreError::InvalidSchedule { id: s.id.clone(), why: "the id is taken in this workspace".into() });
            }
            let old = before.iter().find(|o| o.id == s.id);
            s.approval = old.and_then(|o| o.approval.clone());
            if old != Some(&*s) {
                s.validate(workspace).map_err(|why| CoreError::InvalidSchedule { id: s.id.clone(), why })?;
            }
            if old.is_none() && s.created_at == 0 {
                s.created_at = crate::time::now_ms();
            }
            let changed = old.is_some_and(|o| covered_toml(&workspace.name, o) != covered_toml(&workspace.name, s));
            if changed {
                s.status = Status::Pending;
                s.action.worktree = false;
            } else if s.status == Status::Active && old.is_none_or(|o| o.status != Status::Active) {
                let approved = s.approval.as_ref().is_some_and(|a| covered(workspace, s).is_ok_and(|c| c.fingerprint == a.fingerprint));
                if !approved {
                    s.status = Status::Pending;
                }
            }
        }
        Ok(out)
    })
}

/// [`edit`] without its rules: for [`super::approve`] alone.
pub(super) fn edit_raw<T>(home: &Path, workspace: &Workspace, change: impl FnOnce(&mut Vec<Schedule>) -> Result<T>) -> Result<T> {
    let path = file_path(home, &workspace.name)?;
    let lock = lock_path(home, &workspace.name);
    let _held = crate::fs::lock_file(&lock).map_err(|e| CoreError::io(&lock, e))?;
    let mut file = load_file(&path)?;
    let out = change(&mut file.schedule)?;
    let text = toml::to_string(&file).map_err(|source| CoreError::SerializeSchedules { workspace: workspace.name.clone(), source })?;
    crate::fs::write_atomic(&path, text).map_err(|e| CoreError::io(&path, e))?;
    Ok(out)
}
