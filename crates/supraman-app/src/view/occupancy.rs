//! The live sessions' names and cwds, kept from each poll, so every Git tab's write gate answers from memory.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use supraman_claude::Session;
use supraman_gitman::write::{Occupancy, working_in};

use crate::fleet;

#[derive(Clone, Default)]
pub struct Live(Arc<Mutex<Vec<(String, PathBuf)>>>);

impl Live {
    /// The sessions of a poll at `now`: the live ones are kept.
    pub fn keep(&self, sessions: &[Session], now: i64) {
        self.set(sessions.iter().filter(|s| fleet::is_alive(&s.health(now))).map(|s| (s.name.clone(), s.cwd.clone())).collect());
    }

    pub fn set(&self, live: Vec<(String, PathBuf)>) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = live;
    }
}

impl Occupancy for Live {
    fn who_works_in(&self, worktree: &Path) -> Vec<String> {
        let live = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        working_in(live.iter().map(|(name, cwd)| (name.as_str(), cwd.as_path())), worktree)
    }
}
