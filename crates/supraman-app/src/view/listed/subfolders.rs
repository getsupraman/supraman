//! The folders that show their sub folders, each watched alone (not below), so a child folder made, renamed or
//! removed is read again without waiting for the poll.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use gpui::{App, AppContext as _, Context, Entity, Task, WeakEntity};
use supraman_core::paths::path_key;
use supraman_core::watch::{Watch, Watcher};
use supraman_ui::nudge::{self, Nudge, Nudged};

use super::super::FleetView;

const DEBOUNCE: Duration = Duration::from_millis(300);
/// The loop wakes only when nudged; this is how long it waits between checks that its window is still there.
const IDLE: Duration = Duration::from_secs(3600);

/// One watcher per folder; blocking to change, so the app syncs it off the UI thread.
#[derive(Default)]
pub struct Watchers {
    by_key: Vec<(String, Watcher)>,
}

impl Watchers {
    /// Watching exactly `folders`, each change calling `changed`; a folder it cannot watch is left out.
    pub fn sync(&mut self, folders: &[PathBuf], changed: Arc<dyn Fn() + Send + Sync>) {
        let wanted: Vec<String> = folders.iter().map(|p| path_key(p)).collect();
        self.by_key.retain(|(key, _)| wanted.contains(key));
        for (key, folder) in wanted.into_iter().zip(folders) {
            if self.by_key.iter().any(|(k, _)| *k == key) {
                continue;
            }
            let changed = changed.clone();
            match Watcher::new(&[Watch::only(folder)], DEBOUNCE, move |_| changed()) {
                Ok(w) => self.by_key.push((key, w)),
                Err(e) => log::warn!("cannot watch {} for its sub folders: {e}", folder.display()),
            }
        }
    }

    #[cfg(test)]
    pub fn watched(&self) -> Vec<&str> {
        self.by_key.iter().map(|(k, _)| k.as_str()).collect()
    }
}

/// The watchers of the folders that show their sub folders, and the loop that reads the list again on a change.
pub struct SubfolderWatch {
    watchers: Arc<Mutex<Watchers>>,
    /// The newest folders asked for: a sync job that runs late still watches these.
    latest: Arc<Mutex<Vec<PathBuf>>>,
    wanted: Vec<String>,
    nudge: Nudge,
    nudged: Option<Nudged>,
    _reading: Option<Task<()>>,
}

impl SubfolderWatch {
    pub fn new(cx: &mut App) -> Entity<Self> {
        let (nudge, nudged) = nudge::nudge();
        cx.new(|_| Self { watchers: Arc::default(), latest: Arc::default(), wanted: Vec::new(), nudge, nudged: Some(nudged), _reading: None })
    }

    /// Watching `folders` from now on; the first call starts the loop that has `fleet` read the list on each change.
    pub fn sync(&mut self, folders: Vec<PathBuf>, fleet: WeakEntity<FleetView>, cx: &mut Context<Self>) {
        if let Some(mut nudged) = self.nudged.take() {
            self._reading = Some(cx.spawn(async move |_, cx| {
                loop {
                    if nudged.wait(cx.background_executor(), IDLE).await && fleet.update(cx, |v, cx| v.read_listed(cx)).is_err() {
                        return;
                    }
                    if fleet.upgrade().is_none() {
                        return;
                    }
                }
            }));
        }
        let wanted: Vec<String> = folders.iter().map(|p| path_key(p)).collect();
        if wanted == self.wanted {
            return;
        }
        self.wanted = wanted;
        *self.latest.lock().unwrap_or_else(PoisonError::into_inner) = folders;
        let (watchers, latest, nudge) = (self.watchers.clone(), self.latest.clone(), self.nudge.clone());
        let changed: Arc<dyn Fn() + Send + Sync> = Arc::new(move || nudge.wake());
        cx.background_executor()
            .spawn(async move {
                let mut watchers = watchers.lock().unwrap_or_else(PoisonError::into_inner);
                let folders = latest.lock().unwrap_or_else(PoisonError::into_inner).clone();
                watchers.sync(&folders, changed);
            })
            .detach();
    }
}

#[cfg(test)]
mod tests;
