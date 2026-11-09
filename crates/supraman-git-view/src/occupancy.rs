//! The live sessions' names and cwds, kept from each poll, so every Git tab's write gate answers from memory.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use supraman_gitman::write::{Occupancy, working_in};

#[derive(Clone, Default)]
pub struct Live(Arc<Mutex<Vec<(String, PathBuf)>>>);

impl Live {
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
