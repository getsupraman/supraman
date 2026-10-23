use foreman_claude::{Session, SessionState, Status};

use crate::{Launcher, Millis, NeedsYou, RELAY_ROLE, Supervisor, TokenProvider, is_live};

/// The machine-wide concurrency budget of one tick, shared by every workspace's supervisor: at most
/// `max` `fm-` sessions busy at once. Starts as `max` minus the live `fm-` rows that are mid-turn ([`mid_turn`]);
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

/// Live `fm-` rows that are mid-turn, relays aside ([`crate::WantedSession::exempt_from_budget`]).
pub fn working(observed: &[Session], now: Millis) -> usize {
    observed
        .iter()
        .filter(|s| s.name.starts_with("fm-") && !s.name.ends_with(&format!("-{RELAY_ROLE}")) && mid_turn(s, now))
        .count()
}

/// The process is in a turn: `status=busy`. `state` is not the turn: a session that ended its turn
/// waiting for messages stays `state=working` with `status=idle` (m0/4, m3/1, measured m3/2), and
/// `status` flipped with the UserPromptSubmit/Stop hooks within one poll. Only a live row with no
/// `status` yet (right after `--bg`) falls back to `state=working`.
pub fn mid_turn(s: &Session, now: Millis) -> bool {
    is_live(s, now)
        && match &s.status {
            Some(status) => *status == Status::Busy,
            None => s.state == Some(SessionState::Working),
        }
}

/// One tick of every workspace on one snapshot, sequentially with one [`Budget`] of `max_working`.
/// Sessions still settling after a start are reserved first (they are working by intent but not yet
/// `working` in the snapshot), so no workspace spends a slot another one already used. Returns all
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
