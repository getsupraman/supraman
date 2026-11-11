//! The live sessions kept from each poll, so every Git tab's agent rows and background work answer from memory.

use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use supraman_core::paths::within;
use supraman_gitman::write::Occupancy;
use supraman_ui::nudge::{Nudge, Nudged, nudge};
pub use supraman_ui::session::LiveSession;

#[derive(Default)]
struct Shared {
    sessions: Vec<LiveSession>,
    watchers: Vec<Nudge>,
}

#[derive(Clone, Default)]
pub struct Live(Arc<Mutex<Shared>>);

impl Live {
    /// Every watcher is nudged when the sessions differ from the last set.
    pub fn set_sessions(&self, sessions: Vec<LiveSession>) {
        let mut shared = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if shared.sessions == sessions {
            return;
        }
        shared.sessions = sessions;
        shared.watchers.retain(|w| !w.is_closed());
        for w in &shared.watchers {
            w.wake();
        }
    }

    pub fn sessions(&self) -> Vec<LiveSession> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).sessions.clone()
    }

    /// The sessions whose cwd is `worktree` or below, by name.
    pub fn working_in(&self, worktree: &Path) -> Vec<LiveSession> {
        let shared = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let mut sessions: Vec<LiveSession> = shared.sessions.iter().filter(|s| within(&s.cwd, worktree)).cloned().collect();
        sessions.sort_by(|a, b| a.name.cmp(&b.name));
        sessions
    }

    /// Woken after each change of the sessions.
    pub fn watch(&self) -> Nudged {
        let (wake, woken) = nudge();
        self.0.lock().unwrap_or_else(PoisonError::into_inner).watchers.push(wake);
        woken
    }
}

impl Occupancy for Live {
    fn who_works_in(&self, worktree: &Path) -> Vec<String> {
        self.working_in(worktree).into_iter().map(|s| s.name).collect()
    }
}
