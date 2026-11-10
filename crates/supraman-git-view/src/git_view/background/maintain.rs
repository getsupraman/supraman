//! `git maintenance run` in the background, [`AFTER`] a repository's first tab opens and then at most once per
//! [`EVERY`], its last run per common git dir kept in `<home>/gitman/maintenance.json` so a restart does not run it
//! again. It is a write: it waits for nothing and asks nobody, it is skipped while a live session works in the
//! repository, a git holds an index, a build or selftest holds the heavy-job lock, or another write holds the
//! repository, and tried again [`RETRY`] later. Its failures are logged only.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use gpui::{App, AsyncApp};
use supraman_core::fs::{load_json_or_notice, lock_file};
use supraman_core::store::{JsonStore, Save};
use supraman_gitman::write::{LockSet, Writer, maintain};
use supraman_ui::nudge::Nudged;

use super::{Background, HeavyLock, IDLE, Step};

pub(crate) const AFTER: Duration = Duration::from_secs(15);
pub(crate) const EVERY: Duration = Duration::from_secs(3600);
pub(crate) const RETRY: Duration = Duration::from_secs(60);

const LOST: &str = "GitMan runs git maintenance again on each repository";

type Runs = BTreeMap<String, i64>;

/// The last run per common git dir, epoch milliseconds; read on the first maintenance, off the UI thread.
#[derive(Clone, Default)]
pub(crate) struct Ran(Arc<Mutex<Option<JsonStore<Runs>>>>);

pub(crate) fn path(home: &Path) -> PathBuf {
    home.join("gitman").join("maintenance.json")
}

impl Ran {
    /// When `key` last ran; blocking (the first call reads the file).
    fn last(&self, home: Option<&Path>, key: &str) -> Option<i64> {
        let mut store = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let store = store.get_or_insert_with(|| {
            let (store, notice) = JsonStore::load_kept(home.map(path), LOST);
            if let Some(notice) = notice {
                log::warn!("{notice}");
            }
            store
        });
        store.get().get(key).copied()
    }

    /// `key` ran at `now`; blocking.
    fn record(&self, key: &str, now: i64) {
        let mut store = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(Err(why)) = store.as_mut().map(|store| record_in(store, key, now)) {
            log::warn!("git maintenance: {why}");
        }
    }
}

/// `key` ran at `now`, written under the file's lock over the runs other processes wrote since; a file that
/// cannot be read is left as it is.
fn record_in(store: &mut JsonStore<Runs>, key: &str, now: i64) -> Result<(), String> {
    let ran = |runs: &mut Runs| runs.insert(key.to_string(), now) != Some(now);
    let Some(path) = store.path().map(Path::to_path_buf) else {
        let _ = store.edit(ran);
        return Ok(());
    };
    let lock = path.with_extension("lock");
    let _held = lock_file(&lock).map_err(|e| format!("cannot lock {}: {e}", lock.display()))?;
    let (on_disk, notice) = load_json_or_notice::<Runs>(&path, LOST);
    if let Some(notice) = notice {
        let _ = store.edit(ran);
        return Err(notice);
    }
    let save = store.edit(|runs| {
        on_disk.into_iter().for_each(|(k, v)| {
            runs.entry(k).and_modify(|last| *last = (*last).max(v)).or_insert(v);
        });
        ran(runs);
        true
    });
    save.map_or(Ok(()), Save::write)
}

struct Job {
    writer: Writer,
    home: Option<PathBuf>,
    ran: Ran,
    heavy: HeavyLock,
    now: i64,
}

/// What the tabs of `key` allow now: nothing while maintenance is off, a tab's write is under way or a session
/// works in one of the repository's worktrees.
fn job(key: &str, cx: &App) -> Step<Option<Job>> {
    let Some(bg) = cx.try_global::<Background>() else { return Step::End };
    let views = bg.views(key);
    if views.is_empty() {
        return Step::End;
    }
    if !bg.settings.is_some_and(|s| s.maintains()) {
        return Step::Run(None);
    }
    let Some(writer) = views.iter().find_map(|v| v.read(cx).gate.read(cx).writer().cloned()) else { return Step::Skip };
    for view in &views {
        let view = view.read(cx);
        let remote = view.remote.read(cx);
        let occupancy = view.gate.read(cx).occupancy();
        let mut folders = remote.worktrees();
        folders.push(writer.dirs().top.clone());
        if remote.busy(cx).is_some() || folders.iter().any(|f| !occupancy.who_works_in(f).is_empty()) {
            return Step::Skip;
        }
    }
    Step::Run(Some(Job { writer, home: bg.home.clone(), ran: bg.ran.clone(), heavy: bg.heavy.clone(), now: (bg.clock)() }))
}

/// How long until the next look: [`EVERY`] after a run, the rest of it when one ran lately, [`RETRY`] when skipped.
fn maintain(job: Job) -> Duration {
    let key = supraman_core::paths::path_key(&job.writer.dirs().common_dir);
    let every = EVERY.as_millis() as i64;
    if let Some(last) = job.ran.last(job.home.as_deref(), &key).filter(|last| job.now - last < every) {
        return Duration::from_millis((every - (job.now - last)).max(0) as u64);
    }
    let Some(_heavy) = (job.heavy)() else { return RETRY };
    if maintain::index_locked(job.writer.dirs()) {
        return RETRY;
    }
    match job.writer.try_run_watched(LockSet::maintenance(), maintain::run) {
        None => return RETRY,
        Some(Err(e)) => log::warn!("git maintenance of {}: {e}", job.writer.dirs().common_dir.display()),
        Some(Ok(())) => {}
    }
    job.ran.record(&key, job.now);
    EVERY
}

pub(super) async fn run_loop(key: String, mut nudged: Nudged, cx: &mut AsyncApp) {
    let executor = cx.background_executor().clone();
    executor.timer(AFTER).await;
    loop {
        let wait = match cx.update(|cx| job(&key, cx)) {
            Step::End => return,
            Step::Skip => RETRY,
            Step::Run(None) => IDLE,
            Step::Run(Some(job)) => executor.spawn(async move { maintain(job) }).await,
        };
        nudged.wait(&executor, wait).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_in(home: &Path, text: &str) -> PathBuf {
        let file = path(home);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, text).unwrap();
        file
    }

    #[test]
    fn an_unreadable_record_keeps_the_runs_in_memory_and_is_never_written_over() {
        let home = tempfile::tempdir().unwrap();
        let file = file_in(home.path(), "not json");
        let ran = Ran::default();
        assert_eq!(ran.last(Some(home.path()), "repo"), None);
        assert!(ran.0.lock().unwrap().as_ref().is_some_and(|store| store.path().is_none()), "the store lives in memory");
        ran.record("repo", 5);
        assert_eq!(ran.last(Some(home.path()), "repo"), Some(5));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "not json");
    }

    #[test]
    fn a_record_keeps_what_another_process_wrote_and_leaves_a_file_broken_since_as_it_is() {
        let home = tempfile::tempdir().unwrap();
        let ran = Ran::default();
        assert_eq!(ran.last(Some(home.path()), "a"), None);
        let file = file_in(home.path(), r#"{"b": 7}"#);
        ran.record("a", 5);
        let on_disk: Runs = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(on_disk, Runs::from([("a".to_string(), 5), ("b".to_string(), 7)]));
        std::fs::write(&file, "broken").unwrap();
        ran.record("a", 9);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "broken");
        assert_eq!(ran.last(Some(home.path()), "a"), Some(9));
    }
}
