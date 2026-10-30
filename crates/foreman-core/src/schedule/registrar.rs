use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, Local, NaiveDate, TimeZone};
use sha2::{Digest, Sha256};

use super::task_xml::{RunLevel, TaskSpec, definition_digest, task_xml, version_of};
use super::{Schedule, Status, Trigger, next_fire, verify_approval};
use crate::secret::SecretStore;
use crate::{CoreError, Result, Workspace};

/// The windowless fire binary (sched/5); it ships next to `fm.exe` and `foreman.exe`.
pub const FMW_EXE: &str = "fmw.exe";

/// `fmw.exe` next to the running exe.
pub fn fmw_exe() -> PathBuf {
    let dir = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)).unwrap_or_default();
    dir.join(FMW_EXE)
}

/// `\Foreman` for the default home (`%APPDATA%\Foreman`), `\Foreman-<hex8>` for any other.
pub fn task_root(home: &Path) -> String {
    task_root_for(home, crate::home_from(None, std::env::var_os("APPDATA")).as_deref())
}

pub fn task_root_for(home: &Path, default_home: Option<&Path>) -> String {
    let key = crate::paths::path_key(home);
    if default_home.is_some_and(|d| crate::paths::path_key(d) == key) {
        return r"\Foreman".to_string();
    }
    format!(r"\Foreman-{}", &super::approval::hex(&Sha256::digest(key.as_bytes()))[..8])
}

/// Task Scheduler as [`sync`] sees it. Folders are `\`-separated paths from the root, such as `\Foreman\demo`.
pub trait Registrar {
    /// The SID every task runs as: the current user.
    fn user(&self) -> Result<String>;
    /// The tasks directly in `folder`, by name with their XML; none when the folder does not exist.
    fn tasks(&self, folder: &str) -> Result<Vec<(String, String)>>;
    /// Creates or replaces `folder\name`, creating the folders on the way.
    fn register(&self, folder: &str, name: &str, xml: &str) -> Result<()>;
    fn delete(&self, folder: &str, name: &str) -> Result<()>;
    /// Deletes `folder` when it holds no task and no folder; `true` when it is gone.
    fn delete_empty_folder(&self, folder: &str) -> Result<bool>;
}

/// What a [`sync`] runs against besides the schedules.
pub struct SyncTarget<'a> {
    pub home: &'a Path,
    pub store: &'a dyn SecretStore,
    pub level: RunLevel,
    pub fmw: &'a Path,
    pub now_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Synced {
    pub folder: String,
    /// The run level every task in the folder now has.
    pub level: RunLevel,
    pub registered: Vec<String>,
    pub updated: Vec<String>,
    pub removed: Vec<String>,
    pub folder_deleted: bool,
}

/// Makes `<task_root>\<workspace>` hold one task per schedule whose approval verifies and whose status is
/// active, and nothing else. A task whose [`definition_digest`] matches stays; a changed definition, run
/// level included, is registered again. No task outside that folder is read or written.
pub fn sync(registrar: &dyn Registrar, target: &SyncTarget, workspace: &Workspace, schedules: &[Schedule]) -> Result<Synced> {
    let root = task_root(target.home);
    let folder = format!(r"{root}\{}", workspace.name);
    let existing: BTreeMap<String, String> = registrar.tasks(&folder)?.into_iter().collect();
    let user = registrar.user()?;
    let now = Local.timestamp_millis_opt(target.now_ms).single().ok_or_else(|| CoreError::Tasks(format!("no local time for {}", target.now_ms)))?;
    let mut out = Synced { folder: folder.clone(), level: target.level, registered: vec![], updated: vec![], removed: vec![], folder_deleted: false };
    let mut wanted = BTreeSet::new();
    for s in schedules.iter().filter(|s| s.status == Status::Active) {
        let Ok(covered) = verify_approval(target.store, workspace, s) else { continue };
        let spec = TaskSpec {
            user: &user,
            level: target.level,
            fmw: target.fmw,
            home: target.home,
            workspace: &workspace.name,
            schedule: s,
            fingerprint: &covered.fingerprint,
            start: start_date(&s.trigger, &now),
        };
        wanted.insert(s.id.clone());
        let digest = definition_digest(&spec);
        match existing.get(&s.id) {
            Some(xml) if version_of(xml) == Some(digest.as_str()) => {}
            had => {
                registrar.register(&folder, &s.id, &task_xml(&spec))?;
                if had.is_some() { &mut out.updated } else { &mut out.registered }.push(s.id.clone());
            }
        }
    }
    for name in existing.keys().filter(|n| !wanted.contains(*n)) {
        registrar.delete(&folder, name)?;
        out.removed.push(name.clone());
    }
    if wanted.is_empty() {
        out.folder_deleted = registrar.delete_empty_folder(&folder)?;
        if out.folder_deleted {
            registrar.delete_empty_folder(&root)?;
        }
    }
    Ok(out)
}

/// The local date of the trigger's first start after `now`, so a new task has no missed start to catch up.
fn start_date<Tz: TimeZone>(trigger: &Trigger, now: &DateTime<Tz>) -> NaiveDate {
    let first = match trigger {
        Trigger::Interval { at, .. } => next_fire(&Trigger::Daily { at: *at }, now),
        t => next_fire(t, now),
    };
    first.map_or_else(|| now.date_naive(), |t| t.date_naive())
}

/// Task Scheduler in memory, keyed by the task's full path; every test and selftest registers here (C101).
#[derive(Debug, Default)]
pub struct FakeRegistrar {
    state: Mutex<FakeState>,
}

#[derive(Debug, Default)]
struct FakeState {
    tasks: BTreeMap<String, String>,
    folders: BTreeSet<String>,
    writes: Vec<String>,
}

impl FakeRegistrar {
    pub const USER: &'static str = "S-1-5-21-1000-2000-3000-1001";

    pub fn xml(&self, path: &str) -> Option<String> {
        self.lock().tasks.get(path).cloned()
    }

    /// Every task's full path.
    pub fn paths(&self) -> Vec<String> {
        self.lock().tasks.keys().cloned().collect()
    }

    pub fn has_folder(&self, folder: &str) -> bool {
        self.lock().folders.contains(folder)
    }

    /// `register <path>`, `delete <path>` and `rmdir <folder>`, in order.
    pub fn writes(&self) -> Vec<String> {
        self.lock().writes.clone()
    }

    /// Puts a task in place without recording a write, as another program would.
    pub fn plant(&self, folder: &str, name: &str, xml: &str) {
        let mut st = self.lock();
        add_folders(&mut st.folders, folder);
        st.tasks.insert(format!(r"{folder}\{name}"), xml.to_string());
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, FakeState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn add_folders(folders: &mut BTreeSet<String>, folder: &str) {
    let mut path = String::new();
    for part in folder.split('\\').filter(|p| !p.is_empty()) {
        path.push('\\');
        path.push_str(part);
        folders.insert(path.clone());
    }
}

fn parent_of(path: &str) -> Option<&str> {
    path.rsplit_once('\\').map(|(parent, _)| parent)
}

impl Registrar for FakeRegistrar {
    fn user(&self) -> Result<String> {
        Ok(Self::USER.to_string())
    }

    fn tasks(&self, folder: &str) -> Result<Vec<(String, String)>> {
        let st = self.lock();
        Ok(st.tasks.iter().filter(|(p, _)| parent_of(p) == Some(folder)).map(|(p, x)| (p[folder.len() + 1..].to_string(), x.clone())).collect())
    }

    fn register(&self, folder: &str, name: &str, xml: &str) -> Result<()> {
        let mut st = self.lock();
        let path = format!(r"{folder}\{name}");
        add_folders(&mut st.folders, folder);
        st.writes.push(format!("register {path}"));
        st.tasks.insert(path, xml.to_string());
        Ok(())
    }

    fn delete(&self, folder: &str, name: &str) -> Result<()> {
        let mut st = self.lock();
        let path = format!(r"{folder}\{name}");
        st.writes.push(format!("delete {path}"));
        st.tasks.remove(&path).map(|_| ()).ok_or_else(|| CoreError::Tasks(format!("no task {path}")))
    }

    fn delete_empty_folder(&self, folder: &str) -> Result<bool> {
        let mut st = self.lock();
        if !st.folders.contains(folder) {
            return Ok(true);
        }
        let below = format!("{folder}\\");
        if st.tasks.keys().chain(st.folders.iter()).any(|p| p.starts_with(&below)) {
            return Ok(false);
        }
        st.folders.remove(folder);
        st.writes.push(format!("rmdir {folder}"));
        Ok(true)
    }
}
