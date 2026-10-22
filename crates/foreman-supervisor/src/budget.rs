use foreman_claude::{Session, SessionState};

use crate::{Launcher, Millis, NeedsYou, Supervisor, TokenProvider, is_live};

/// The machine-wide concurrency budget of one tick, shared by every platform's supervisor: at most
/// `max` `fm-` sessions busy at once. Starts as `max` minus the live `fm-` rows with `state=working`;
/// every start the supervisors are still waiting to see, and every launch/resume of this tick, takes
/// one more slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    pub max: usize,
    pub free: usize,
}

impl Budget {
    pub fn new(max: usize, observed: &[Session], now: Millis) -> Self {
        Self {
            max,
            free: max.saturating_sub(working(observed, now)),
        }
    }

    pub fn take(&mut self, n: usize) {
        self.free = self.free.saturating_sub(n);
    }
}

/// Live `fm-` rows whose turn is `working`.
pub fn working(observed: &[Session], now: Millis) -> usize {
    observed
        .iter()
        .filter(|s| s.name.starts_with("fm-") && s.state == Some(SessionState::Working) && is_live(s, now))
        .count()
}

/// One tick of every platform on one snapshot, sequentially with one [`Budget`] of `max_working`.
/// Sessions still settling after a start are reserved first (they are working by intent but not yet
/// `working` in the snapshot), so no platform spends a slot another one already used. Returns all
/// Inbox rows, in supervisor order.
pub fn tick_all<'a, L, T>(
    supervisors: impl IntoIterator<Item = &'a mut Supervisor<L, T>>,
    observed: &[Session],
    max_working: usize,
    now: Millis,
) -> Vec<NeedsYou>
where
    L: Launcher + 'a,
    T: TokenProvider + 'a,
{
    let mut supervisors: Vec<&mut Supervisor<L, T>> = supervisors.into_iter().collect();
    let mut budget = Budget::new(max_working, observed, now);
    for s in &supervisors {
        budget.take(s.starting(observed, now));
    }
    supervisors.iter_mut().flat_map(|s| s.tick(observed, &mut budget, now)).collect()
}
