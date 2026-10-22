use std::path::{Path, PathBuf};

use foreman_claude::{Claude, LaunchSpec, Session};

use crate::{
    Action, DesiredState, Failure, Millis, NeedsYou, Policy, SupervisorMemory, WantedSession, is_live, reconcile,
};

/// The three `claude` writes the supervisor performs. Implemented for [`Claude`]; tests use a fake.
pub trait Launcher {
    fn launch(&self, spec: &LaunchSpec) -> foreman_claude::Result<String>;
    /// Must pass no flag but `--bg --resume <session_id>` (any flag forks a copy, m2/1).
    fn resume(&self, session_id: &str, cwd: &Path) -> foreman_claude::Result<String>;
    fn stop(&self, id: &str) -> foreman_claude::Result<String>;
}

impl Launcher for Claude {
    fn launch(&self, spec: &LaunchSpec) -> foreman_claude::Result<String> {
        Claude::launch(self, spec)
    }

    fn resume(&self, session_id: &str, cwd: &Path) -> foreman_claude::Result<String> {
        Claude::resume(self, session_id, cwd, None)
    }

    fn stop(&self, id: &str) -> foreman_claude::Result<String> {
        Claude::stop(self, id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleToken {
    pub url: String,
    pub token: String,
}

/// Where role tokens come from; the supervisor never reads `.env` itself.
pub trait TokenProvider {
    /// `None`: launch the role without a WorkState server.
    fn token(&self, role: &str) -> Option<RoleToken>;
}

/// Launch options every session of this supervisor gets.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaunchConfig {
    /// `<FOREMAN_HOME>/mcp`, where [`LaunchSpec::with_role_token`] writes the owner-only config.
    pub mcp_dir: PathBuf,
    pub plugin_dirs: Vec<PathBuf>,
}

/// One platform's supervisor: desired state + memory, driven by [`Supervisor::tick`].
pub struct Supervisor<L, T> {
    pub desired: DesiredState,
    pub memory: SupervisorMemory,
    pub policy: Policy,
    pub config: LaunchConfig,
    launcher: L,
    tokens: T,
    needs: Vec<NeedsYou>,
}

impl<L: Launcher, T: TokenProvider> Supervisor<L, T> {
    pub fn new(desired: DesiredState, config: LaunchConfig, launcher: L, tokens: T) -> Self {
        Self {
            desired,
            memory: SupervisorMemory::default(),
            policy: Policy::default(),
            config,
            launcher,
            tokens,
            needs: Vec::new(),
        }
    }

    pub fn launcher(&self) -> &L {
        &self.launcher
    }

    /// The Inbox rows of the last tick.
    pub fn needs_you(&self) -> &[NeedsYou] {
        &self.needs
    }

    /// The human's Retry: forgets `name`'s failures (incl. trust and give-up); the next tick acts.
    pub fn retry(&mut self, name: &str) {
        self.memory.retry(name);
    }

    /// `observed` is one fresh `claude agents --json --all`. Observes, reconciles, executes the
    /// launches/resumes/stops, records their results, and returns the Inbox rows as they stand
    /// after execution (a launch refused for trust shows in the same tick).
    pub fn tick(&mut self, observed: &[Session], now: Millis) -> Vec<NeedsYou> {
        for w in &self.desired.sessions {
            let mut rows = observed.iter().filter(|s| s.name == w.name);
            let alive = match (rows.next(), rows.next()) {
                (Some(s), None) => is_live(s, now),
                _ => false,
            };
            self.memory.observe(&w.name, alive, &self.policy, now);
        }
        for action in reconcile(&self.desired, observed, &self.memory, &self.policy, now) {
            match action {
                Action::Launch(w) => {
                    let result = self.launch(&w).map(drop);
                    self.memory.record_result(&w.name, result, &self.policy, now);
                }
                Action::Resume {
                    name, session_id, cwd, ..
                } => {
                    let result = self.launcher.resume(&session_id, &cwd).map(drop).map_err(|e| Failure::from(&e));
                    self.memory.record_result(&name, result, &self.policy, now);
                }
                // A failed stop is retried by the next tick: the row is still live.
                Action::Stop { id, .. } => {
                    let _ = self.launcher.stop(&id);
                }
                Action::Wait { .. } | Action::NeedsYou(_) => {}
            }
        }
        let needs: Vec<NeedsYou> = reconcile(&self.desired, observed, &self.memory, &self.policy, now)
            .into_iter()
            .filter_map(|a| match a {
                Action::NeedsYou(mut n) => {
                    if let Some(prev) = self.needs.iter().find(|p| p.kind == n.kind && p.name == n.name) {
                        n.since = n.since.min(prev.since);
                    }
                    Some(n)
                }
                _ => None,
            })
            .collect();
        self.needs = needs.clone();
        needs
    }

    fn launch(&self, w: &WantedSession) -> Result<String, Failure> {
        let mut spec = LaunchSpec::new(&w.name, &w.cwd, &w.prompt);
        spec.plugin_dirs = self.config.plugin_dirs.clone();
        if let Some(t) = self.tokens.token(&w.role) {
            spec = spec
                .with_role_token(&self.config.mcp_dir, &w.role, &t.url, &t.token)
                .map_err(|e| Failure::Other(format!("cannot write the {} mcp config: {e}", w.role)))?;
        }
        self.launcher.launch(&spec).map_err(|e| Failure::from(&e))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use foreman_claude::{ClaudeError, Kind, SessionState, Status};

    use super::*;
    use crate::{NeedsKind, Remedy, SECOND};

    const NOW: Millis = 1_800_000_000_000;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Call {
        Launch { name: String, argv: Vec<String> },
        Resume { session_id: String, cwd: PathBuf },
        Stop { id: String },
    }

    #[derive(Default)]
    struct FakeLauncher {
        calls: RefCell<Vec<Call>>,
        /// Popped per launch/resume; empty means success.
        errors: RefCell<Vec<ClaudeError>>,
    }

    impl FakeLauncher {
        fn result(&self) -> foreman_claude::Result<String> {
            match self.errors.borrow_mut().pop() {
                Some(e) => Err(e),
                None => Ok("backgrounded · aaaa0001 · x".into()),
            }
        }

        fn take(&self) -> Vec<Call> {
            std::mem::take(&mut self.calls.borrow_mut())
        }
    }

    impl Launcher for FakeLauncher {
        fn launch(&self, spec: &LaunchSpec) -> foreman_claude::Result<String> {
            let argv = spec.argv().iter().map(|a| a.to_string_lossy().into_owned()).collect();
            self.calls.borrow_mut().push(Call::Launch { name: spec.name.clone(), argv });
            self.result()
        }

        fn resume(&self, session_id: &str, cwd: &Path) -> foreman_claude::Result<String> {
            self.calls.borrow_mut().push(Call::Resume { session_id: session_id.into(), cwd: cwd.into() });
            self.result()
        }

        fn stop(&self, id: &str) -> foreman_claude::Result<String> {
            self.calls.borrow_mut().push(Call::Stop { id: id.into() });
            Ok(String::new())
        }
    }

    struct FakeTokens(Option<RoleToken>);

    impl TokenProvider for FakeTokens {
        fn token(&self, _role: &str) -> Option<RoleToken> {
            self.0.clone()
        }
    }

    fn supervisor(started: bool, tokens: Option<RoleToken>, mcp_dir: &Path) -> Supervisor<FakeLauncher, FakeTokens> {
        let mut desired = DesiredState::platform_controller("demo", r"C:\demo\cwd", "be the controller");
        desired.started = started;
        let config = LaunchConfig {
            mcp_dir: mcp_dir.into(),
            plugin_dirs: vec![r"C:\fm\plugins\foreman".into()],
        };
        Supervisor::new(desired, config, FakeLauncher::default(), FakeTokens(tokens))
    }

    fn row(state: SessionState, status: Option<Status>, pid: Option<u32>) -> Session {
        Session {
            id: "aaaa0001".into(),
            session_id: "aaaa0001-0000-4000-8000-000000000001".into(),
            name: "fm-demo-platform".into(),
            cwd: r"C:\demo\cwd".into(),
            kind: Kind::Background,
            state: Some(state),
            status,
            waiting_for: None,
            pid,
            started_at: Some(NOW - 3_600_000),
        }
    }

    #[test]
    fn launches_with_role_token_file_and_plugin_dir_then_settles() {
        let dir = tempfile::tempdir().unwrap();
        let token = RoleToken {
            url: "http://localhost:5290/mcp".into(),
            token: "fake-token".into(),
        };
        let mut sup = supervisor(true, Some(token), &dir.path().join("mcp"));
        assert_eq!(sup.tick(&[], NOW), []);
        match sup.launcher().take().as_slice() {
            [Call::Launch { name, argv }] => {
                assert_eq!(name, "fm-demo-platform");
                let argv = argv.join(" ");
                assert!(argv.contains("--strict-mcp-config"), "{argv}");
                assert!(argv.contains("workstate-platform.json"), "{argv}");
                assert!(argv.contains(r"--plugin-dir C:\fm\plugins\foreman"), "{argv}");
                assert!(argv.ends_with("-- be the controller"), "{argv}");
                assert!(!argv.contains("fake-token"), "{argv}");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(sup.tick(&[], NOW + 5 * SECOND), []);
        assert_eq!(sup.launcher().take(), [], "the row has not shown yet: no second launch");
        let live = row(SessionState::Working, Some(Status::Busy), Some(4242));
        sup.tick(std::slice::from_ref(&live), NOW + 61 * SECOND);
        assert_eq!(sup.launcher().take(), []);
        assert_eq!(sup.memory.get("fm-demo-platform").unwrap().started_at, None, "stable");
    }

    #[test]
    fn no_token_launches_without_mcp_config() {
        let dir = tempfile::tempdir().unwrap();
        let mut sup = supervisor(true, None, dir.path());
        sup.tick(&[], NOW);
        match sup.launcher().take().as_slice() {
            [Call::Launch { argv, .. }] => assert!(!argv.iter().any(|a| a == "--mcp-config"), "{argv:?}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn stopped_row_is_resumed_from_its_cwd_with_its_session_id() {
        let dir = tempfile::tempdir().unwrap();
        let mut sup = supervisor(true, None, dir.path());
        sup.tick(&[row(SessionState::Stopped, None, None)], NOW);
        assert_eq!(
            sup.launcher().take(),
            [Call::Resume {
                session_id: "aaaa0001-0000-4000-8000-000000000001".into(),
                cwd: r"C:\demo\cwd".into()
            }]
        );
    }

    #[test]
    fn idle_done_with_pid_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let mut sup = supervisor(true, None, dir.path());
        assert_eq!(sup.tick(&[row(SessionState::Done, Some(Status::Idle), Some(27304))], NOW), []);
        assert_eq!(sup.launcher().take(), []);
    }

    #[test]
    fn stop_stops_live_and_never_removes() {
        let dir = tempfile::tempdir().unwrap();
        let mut sup = supervisor(false, None, dir.path());
        sup.tick(&[row(SessionState::Working, Some(Status::Busy), Some(1))], NOW);
        assert_eq!(sup.launcher().take(), [Call::Stop { id: "aaaa0001".into() }]);
        sup.tick(&[row(SessionState::Stopped, None, None)], NOW + SECOND);
        assert_eq!(sup.launcher().take(), []);
    }

    #[test]
    fn trust_refusal_shows_in_the_same_tick_and_waits_for_retry() {
        let dir = tempfile::tempdir().unwrap();
        let mut sup = supervisor(true, None, dir.path());
        sup.launcher.errors.borrow_mut().push(ClaudeError::TrustNeeded { cwd: r"C:\demo\cwd".into() });
        let needs = sup.tick(&[], NOW);
        assert_eq!(needs.len(), 1);
        assert_eq!(needs[0].kind, NeedsKind::TrustNeeded);
        assert_eq!(needs[0].since, NOW);
        assert_eq!(
            needs[0].action,
            Some(Remedy::RunInTerminal { cwd: r"C:\demo\cwd".into(), command: "claude".into() })
        );
        sup.launcher().take();
        let later = sup.tick(&[], NOW + 3_600 * SECOND);
        assert_eq!(later[0].since, NOW, "since is kept");
        assert_eq!(sup.launcher().take(), [], "never retried automatically");
        sup.retry("fm-demo-platform");
        assert_eq!(sup.tick(&[], NOW + 3_601 * SECOND), []);
        assert_eq!(sup.launcher().take().len(), 1);
    }

    #[test]
    fn repeated_failures_back_off_then_give_up_until_retry() {
        let dir = tempfile::tempdir().unwrap();
        let mut sup = supervisor(true, None, dir.path());
        let fail = || ClaudeError::CliFailed { code: Some(1), stderr: "boom".into() };
        let mut now = NOW;
        let mut attempts = Vec::new();
        for _ in 0..5 {
            sup.launcher.errors.borrow_mut().push(fail());
        }
        // Tick every second for an hour: launches only at 0, +5 s, +35 s, +155 s, +755 s.
        while now < NOW + 3_600 * SECOND {
            sup.tick(&[], now);
            if !sup.launcher().take().is_empty() {
                attempts.push((now - NOW) / SECOND);
            }
            now += SECOND;
        }
        assert_eq!(attempts, [0, 5, 35, 155, 755]);
        let needs = sup.needs_you();
        assert_eq!(needs.len(), 1);
        assert_eq!(needs[0].kind, NeedsKind::GaveUp);
        assert_eq!(needs[0].since, NOW + 755 * SECOND);
        assert_eq!(needs[0].detail, "gave up after 5 failures; last: claude failed (exit 1): boom");
        sup.retry("fm-demo-platform");
        sup.tick(&[], now);
        assert_eq!(sup.launcher().take().len(), 1);
    }

    #[test]
    fn launch_failed_is_listed_while_backing_off() {
        let dir = tempfile::tempdir().unwrap();
        let mut sup = supervisor(true, None, dir.path());
        sup.launcher.errors.borrow_mut().push(ClaudeError::NotInstalled);
        let needs = sup.tick(&[], NOW);
        assert_eq!(needs.len(), 1);
        assert_eq!(needs[0].kind, NeedsKind::LaunchFailed);
        assert_eq!(needs[0].action, Some(Remedy::Retry));
        assert!(needs[0].detail.contains("not on PATH"), "{}", needs[0].detail);
    }

    #[test]
    fn a_resume_that_dies_quickly_counts_as_a_failure() {
        let dir = tempfile::tempdir().unwrap();
        let mut sup = supervisor(true, None, dir.path());
        let dead = row(SessionState::Stopped, None, None);
        sup.tick(std::slice::from_ref(&dead), NOW);
        assert_eq!(sup.launcher().take().len(), 1);
        sup.tick(&[row(SessionState::Working, Some(Status::Busy), Some(9))], NOW + 10 * SECOND);
        let needs = sup.tick(std::slice::from_ref(&dead), NOW + 30 * SECOND);
        assert_eq!(sup.launcher().take(), [], "backing off 5 s");
        assert_eq!(needs[0].kind, NeedsKind::LaunchFailed);
        assert!(needs[0].detail.contains("exited within 30 s"), "{}", needs[0].detail);
        sup.tick(&[dead], NOW + 35 * SECOND);
        assert_eq!(sup.launcher().take().len(), 1);
    }
}
