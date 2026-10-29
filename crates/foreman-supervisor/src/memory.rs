use std::collections::BTreeMap;
use std::path::PathBuf;

use foreman_claude::ClaudeError;

/// Epoch milliseconds, the unit of `Session::started_at` and `foreman_claude::now_ms`.
pub type Millis = i64;

pub const SECOND: Millis = 1_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    /// Wait after failure n (1-based) is `backoff[min(n, len) - 1]`.
    pub backoff: Vec<Millis>,
    /// Consecutive failures after which the supervisor stops trying until Retry.
    pub give_up_after: u32,
    /// After a successful launch/resume, how long a missing or pid-less row is "still starting"
    /// (the row trails `--bg` by ~1 s and the pid by up to ~6 s, m0/2, m1/7).
    pub settle: Millis,
    /// A session that is dead again this soon after a successful start counts as a failure; one
    /// alive this long clears the failure count.
    pub stable_after: Millis,
    /// A row that lost its pid this soon after it was last seen alive is not resumed yet: the daemon
    /// respawns a session killed mid-turn by itself (~12 s), and for a few seconds after the death of
    /// an idle one still counts it as running, so a resume then starts a copy (5 s: copy, 10 s: woke
    /// the original; m2/4).
    pub respawn_grace: Millis,
    /// A minted token's idle window: its expiry slides to last use + this (rlarch 2.2.0, 72 h).
    pub token_window: Millis,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            backoff: vec![5 * SECOND, 30 * SECOND, 120 * SECOND, 600 * SECOND],
            give_up_after: 5,
            settle: 20 * SECOND,
            stable_after: 60 * SECOND,
            respawn_grace: 30 * SECOND,
            token_window: 72 * 3_600 * SECOND,
        }
    }
}

impl Policy {
    pub fn backoff_after(&self, failures: u32) -> Millis {
        let i = (failures.max(1) as usize - 1).min(self.backoff.len().saturating_sub(1));
        self.backoff.get(i).copied().unwrap_or(0)
    }
}

/// Why a launch or resume did not take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// Never retried automatically; only Retry clears it.
    TrustNeeded { cwd: PathBuf },
    /// `--bg` could not reach the background service; [`crate::Supervisor`] asks whether Foreman's
    /// elevation is why (m5/25), else it counts like [`Failure::Other`].
    DaemonUnreachable(String),
    Other(String),
}

impl From<&ClaudeError> for Failure {
    fn from(e: &ClaudeError) -> Self {
        match e {
            ClaudeError::TrustNeeded { cwd } => Self::TrustNeeded { cwd: cwd.clone() },
            e @ ClaudeError::DaemonUnreachable { .. } => Self::DaemonUnreachable(e.to_string()),
            other => Self::Other(other.to_string()),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Attempts {
    /// Consecutive failures: errors, plus starts that died within `stable_after`.
    pub failures: u32,
    pub first_failure: Option<Millis>,
    pub last_attempt: Option<Millis>,
    pub last_error: Option<String>,
    /// No launch/resume before this.
    pub next_allowed: Option<Millis>,
    pub trust_needed: Option<PathBuf>,
    pub gave_up: bool,
    /// The last successful launch/resume, until it proves stable or dies.
    pub started_at: Option<Millis>,
    /// The last tick that saw the session live.
    pub last_alive: Option<Millis>,
}

/// Per wanted name, what the supervisor tried and how it went. In-process only: a restart of the
/// app starts every name with a clean slate.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SupervisorMemory {
    names: BTreeMap<String, Attempts>,
}

impl SupervisorMemory {
    pub fn get(&self, name: &str) -> Option<&Attempts> {
        self.names.get(name)
    }

    pub fn record_result(&mut self, name: &str, result: Result<(), Failure>, policy: &Policy, now: Millis) {
        let a = self.names.entry(name.to_string()).or_default();
        a.last_attempt = Some(now);
        match result {
            Ok(()) => {
                a.started_at = Some(now);
                a.next_allowed = None;
            }
            Err(Failure::TrustNeeded { cwd }) => {
                a.last_error = Some(format!("workspace {} is not trusted", cwd.display()));
                a.trust_needed = Some(cwd);
                a.started_at = None;
            }
            Err(Failure::Other(e) | Failure::DaemonUnreachable(e)) => {
                a.started_at = None;
                fail(a, e, policy, now);
            }
        }
    }

    /// Feeds one observation of the wanted name: remembers when it was last alive, confirms a start
    /// as stable, or counts it as a failure when it is dead after `settle` but within `stable_after`.
    pub fn observe(&mut self, name: &str, alive: bool, policy: &Policy, now: Millis) {
        if alive {
            self.names.entry(name.to_string()).or_default().last_alive = Some(now);
        }
        let Some(a) = self.names.get_mut(name) else {
            return;
        };
        let Some(started) = a.started_at else {
            return;
        };
        let age = now - started;
        if alive && age >= policy.stable_after {
            *a = Attempts {
                last_attempt: a.last_attempt,
                last_alive: a.last_alive,
                ..Attempts::default()
            };
        } else if !alive && age >= policy.settle {
            a.started_at = None;
            fail(a, format!("exited within {} s of starting", age / SECOND), policy, now);
        }
    }

    /// The human pressed Retry: forget everything about `name`.
    pub fn retry(&mut self, name: &str) {
        self.names.remove(name);
    }
}

fn fail(a: &mut Attempts, error: String, policy: &Policy, now: Millis) {
    a.failures += 1;
    a.first_failure.get_or_insert(now);
    a.last_error = Some(error);
    if a.failures >= policy.give_up_after {
        a.gave_up = true;
        a.next_allowed = None;
    } else {
        a.next_allowed = Some(now + policy.backoff_after(a.failures));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: Millis = 1_800_000_000_000;

    fn other(e: &str) -> Result<(), Failure> {
        Err(Failure::Other(e.into()))
    }

    #[test]
    fn backoff_schedule_is_5s_30s_2min_then_10min() {
        let p = Policy::default();
        let waits: Vec<Millis> = (1..=6).map(|n| p.backoff_after(n) / SECOND).collect();
        assert_eq!(waits, [5, 30, 120, 600, 600, 600]);
    }

    #[test]
    fn failures_back_off_then_give_up_on_the_fifth() {
        let p = Policy::default();
        let mut m = SupervisorMemory::default();
        let mut now = T0;
        for (n, wait) in [(1, 5), (2, 30), (3, 120), (4, 600)] {
            m.record_result("a", other("boom"), &p, now);
            let a = m.get("a").unwrap();
            assert_eq!(a.failures, n);
            assert_eq!(a.next_allowed, Some(now + wait * SECOND));
            assert!(!a.gave_up);
            now += wait * SECOND;
        }
        m.record_result("a", other("boom 5"), &p, now);
        let a = m.get("a").unwrap();
        assert!(a.gave_up);
        assert_eq!(a.next_allowed, None);
        assert_eq!(a.first_failure, Some(T0));
        assert_eq!(a.last_error.as_deref(), Some("boom 5"));
    }

    #[test]
    fn retry_clears_the_counter() {
        let p = Policy::default();
        let mut m = SupervisorMemory::default();
        for _ in 0..5 {
            m.record_result("a", other("x"), &p, T0);
        }
        m.retry("a");
        assert_eq!(m.get("a"), None);
    }

    #[test]
    fn trust_needed_is_kept_not_backed_off() {
        let p = Policy::default();
        let mut m = SupervisorMemory::default();
        m.record_result("a", Err(Failure::TrustNeeded { cwd: r"C:\x".into() }), &p, T0);
        let a = m.get("a").unwrap();
        assert_eq!(a.trust_needed.as_deref(), Some(std::path::Path::new(r"C:\x")));
        assert_eq!((a.failures, a.next_allowed), (0, None));
    }

    #[test]
    fn success_waits_for_proof_before_clearing_failures() {
        let p = Policy::default();
        let mut m = SupervisorMemory::default();
        m.record_result("a", other("x"), &p, T0);
        m.record_result("a", Ok(()), &p, T0 + 10 * SECOND);
        assert_eq!(m.get("a").unwrap().failures, 1);
        m.observe("a", true, &p, T0 + 69 * SECOND);
        assert_eq!(m.get("a").unwrap().failures, 1, "not yet stable");
        m.observe("a", true, &p, T0 + 70 * SECOND);
        let a = m.get("a").unwrap();
        assert_eq!((a.failures, a.started_at, a.last_error.as_deref()), (0, None, None));
    }

    #[test]
    fn dying_soon_after_a_start_is_a_failure_but_not_before_settle() {
        let p = Policy::default();
        let mut m = SupervisorMemory::default();
        m.record_result("a", Ok(()), &p, T0);
        m.observe("a", false, &p, T0 + 19 * SECOND);
        assert_eq!(m.get("a").unwrap().failures, 0, "still settling");
        m.observe("a", false, &p, T0 + 25 * SECOND);
        let a = m.get("a").unwrap();
        assert_eq!(a.failures, 1);
        assert_eq!(a.next_allowed, Some(T0 + 30 * SECOND));
        assert_eq!(a.last_error.as_deref(), Some("exited within 25 s of starting"));
        m.observe("a", false, &p, T0 + 26 * SECOND);
        assert_eq!(m.get("a").unwrap().failures, 1, "counted once");
    }

    #[test]
    fn observing_an_unknown_dead_name_records_nothing() {
        let mut m = SupervisorMemory::default();
        m.observe("a", false, &Policy::default(), T0);
        assert_eq!(m, SupervisorMemory::default());
    }

    #[test]
    fn alive_observations_are_remembered_across_stability() {
        let p = Policy::default();
        let mut m = SupervisorMemory::default();
        m.observe("a", true, &p, T0);
        assert_eq!(m.get("a").unwrap().last_alive, Some(T0));
        m.record_result("a", Ok(()), &p, T0 + SECOND);
        m.observe("a", true, &p, T0 + 70 * SECOND);
        let a = m.get("a").unwrap();
        assert_eq!((a.started_at, a.last_alive), (None, Some(T0 + 70 * SECOND)));
        m.observe("a", false, &p, T0 + 71 * SECOND);
        assert_eq!(m.get("a").unwrap().last_alive, Some(T0 + 70 * SECOND));
    }

    #[test]
    fn claude_errors_map_to_failures() {
        let trust = ClaudeError::TrustNeeded { cwd: r"C:\x".into() };
        assert_eq!(Failure::from(&trust), Failure::TrustNeeded { cwd: r"C:\x".into() });
        let cli = ClaudeError::CliFailed { code: Some(1), stderr: "boom".into() };
        assert_eq!(Failure::from(&cli), Failure::Other("claude failed (exit 1): boom".into()));
    }
}
