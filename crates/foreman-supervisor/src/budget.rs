use foreman_claude::{Session, SessionState, Status};

use foreman_core::FM_PREFIX;

use crate::{Launcher, Millis, NeedsYou, Supervisor, TokenProvider, is_foreign, is_live, is_relay};

/// The concurrency budget of one tick, shared by every workspace's supervisor of this home; see the crate docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    pub max: usize,
    pub free: usize,
}

impl Budget {
    pub fn new<'a>(max: usize, observed: impl IntoIterator<Item = &'a Session>, workspaces: &[&str], now: Millis) -> Self {
        Self {
            max,
            free: max.saturating_sub(working(observed, workspaces, now)),
        }
    }

    pub fn take(&mut self, n: usize) {
        self.free = self.free.saturating_sub(n);
    }
}

/// Live `fm-` rows that are mid-turn, the relays of `workspaces` aside ([`crate::WantedSession::exempt_from_budget`]).
pub fn working<'a>(observed: impl IntoIterator<Item = &'a Session>, workspaces: &[&str], now: Millis) -> usize {
    observed
        .into_iter()
        .filter(|s| s.name.starts_with(FM_PREFIX) && !is_relay(&s.name, workspaces.iter().copied()) && mid_turn(s, now))
        .count()
}

/// `status=busy`: a session waiting for messages stays `state=working` with `status=idle` (LEARNINGS m3/2).
/// Only a live row with no `status` yet (right after `--bg`) falls back to `state=working`.
pub fn mid_turn(s: &Session, now: Millis) -> bool {
    is_live(s, now)
        && match &s.status {
            Some(status) => *status == Status::Busy,
            None => s.state == Some(SessionState::Working),
        }
}

/// One tick of every workspace on one snapshot with one [`Budget`] (the crate docs' shared budget); all
/// Inbox rows, in supervisor order.
pub fn tick_all<'a, L, T>(
    supervisors: impl IntoIterator<Item = &'a mut Supervisor<L, T>>,
    workspaces: &[&str],
    observed: &[Session],
    max_working: usize,
    now: Millis,
) -> Vec<NeedsYou>
where
    L: Launcher + 'a,
    T: TokenProvider + 'a,
{
    let mut supervisors: Vec<&mut Supervisor<L, T>> = supervisors.into_iter().collect();
    let names: Vec<String> = workspaces.iter().map(|w| w.to_string()).chain(supervisors.iter().map(|s| s.desired.workspace.clone())).collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let own = |s: &&Session| {
        let wanted = supervisors.iter().flat_map(|s| s.desired.sessions.iter().map(|w| w.name.as_str()));
        !is_foreign(&s.name, names.iter().copied(), wanted)
    };
    let mut budget = Budget::new(max_working, observed.iter().filter(own), &names, now);
    for s in &supervisors {
        budget.take(s.starting(observed, now));
    }
    supervisors.iter_mut().flat_map(|s| s.tick(observed, &mut budget, now)).collect()
}
