use std::collections::HashSet;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

type Listener = Arc<dyn Fn() + Send + Sync>;

/// GitMan's own writes under way on one repository: watchers drop batches meanwhile, listeners hear when the last ends.
#[derive(Default)]
pub struct Quiet {
    state: Mutex<QuietState>,
}

#[derive(Default)]
struct QuietState {
    writes: HashSet<u64>,
    next: u64,
    listeners: Vec<(u64, Listener)>,
}

impl Quiet {
    fn state(&self) -> MutexGuard<'_, QuietState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn begin(&self, write: u64) {
        self.state().writes.insert(write);
    }

    /// Ends `write`; the listeners run (on this thread) when no write remains.
    pub fn end(&self, write: u64) {
        let settled = {
            let mut state = self.state();
            state.writes.remove(&write) && state.writes.is_empty()
        };
        if settled {
            let listeners: Vec<Listener> = self.state().listeners.iter().map(|(_, l)| l.clone()).collect();
            listeners.iter().for_each(|l| l());
        }
    }

    pub fn paused(&self) -> bool {
        !self.state().writes.is_empty()
    }

    /// Runs `listener` each time the writes settle, until the returned [`Settled`] drops.
    pub fn on_settled(self: &Arc<Self>, listener: impl Fn() + Send + Sync + 'static) -> Settled {
        let mut state = self.state();
        state.next += 1;
        let id = state.next;
        state.listeners.push((id, Arc::new(listener)));
        Settled { quiet: Arc::downgrade(self), id }
    }
}

/// A listener of [`Quiet::on_settled`]; dropping it stops it.
pub struct Settled {
    quiet: Weak<Quiet>,
    id: u64,
}

impl Drop for Settled {
    fn drop(&mut self) {
        if let Some(quiet) = self.quiet.upgrade() {
            quiet.state().listeners.retain(|(id, _)| *id != self.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn one_settle_when_the_last_of_overlapping_writes_ends() {
        let quiet = Arc::new(Quiet::default());
        let heard = Arc::new(AtomicUsize::new(0));
        let counter = heard.clone();
        let listener = quiet.on_settled(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        quiet.begin(1);
        quiet.begin(2);
        quiet.end(1);
        assert!(quiet.paused());
        assert_eq!(heard.load(Ordering::SeqCst), 0);
        quiet.end(2);
        assert!(!quiet.paused());
        assert_eq!(heard.load(Ordering::SeqCst), 1);
        quiet.end(2);
        assert_eq!(heard.load(Ordering::SeqCst), 1, "an unknown write settles nothing");
        drop(listener);
        quiet.begin(3);
        quiet.end(3);
        assert_eq!(heard.load(Ordering::SeqCst), 1);
    }
}
