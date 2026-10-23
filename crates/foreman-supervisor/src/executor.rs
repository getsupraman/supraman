use std::path::{Path, PathBuf};

use foreman_claude::{Claude, LaunchSpec, Session, SessionIdentity};

use crate::{
    Action, Budget, DesiredState, Failure, Millis, NeedsYou, Policy, SupervisorMemory, WantedSession, is_live, mid_turn,
    reconcile,
};

/// The three `claude` writes the supervisor performs. Implemented for [`Claude`]; tests use a fake.
pub trait Launcher {
    fn launch(&self, spec: &LaunchSpec) -> foreman_claude::Result<String>;
    /// Must pass no flag but `--bg --resume <session_id> [-- <prompt>]` (any flag forks a copy, m2/1).
    fn resume(&self, session_id: &str, cwd: &Path, prompt: Option<&str>) -> foreman_claude::Result<String>;
    fn stop(&self, id: &str) -> foreman_claude::Result<String>;
}

impl Launcher for Claude {
    fn launch(&self, spec: &LaunchSpec) -> foreman_claude::Result<String> {
        Claude::launch(self, spec)
    }

    fn resume(&self, session_id: &str, cwd: &Path, prompt: Option<&str>) -> foreman_claude::Result<String> {
        Claude::resume(self, session_id, cwd, prompt)
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

/// Launch options every session of this supervisor gets. Every launch is `--strict-mcp-config`: a
/// session gets the role's WorkState server or none, never the user-scope one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaunchConfig {
    /// `<FOREMAN_HOME>/mcp`, where [`LaunchSpec::with_session_token`] writes each session's owner-only config.
    pub mcp_dir: PathBuf,
    pub plugin_dirs: Vec<PathBuf>,
    /// Passed to the session as `FOREMAN_HOME`, with `FOREMAN_ROLE`, `FOREMAN_WORKSPACE` (and `FOREMAN_REPO` for a
    /// repo session), through
    /// `--settings {"env":…}` (env on `claude --bg` itself does not reach it, m2/1).
    pub foreman_home: Option<PathBuf>,
    /// `--settings {"worktree":{"bgIsolation":"none"}}`: in a main checkout a `--bg` session otherwise
    /// moves its writes into a worktree of its own that a plain `claude rm` refuses to delete (m2/2).
    pub bg_isolation_none: bool,
    /// `--allowedTools`: permission rules every session gets, so `fm` (Foreman's CLI, run by controllers
    /// themselves) never parks for permission (m3/2: `Bash(fm:*)` matched; PowerShell rules did not).
    pub allowed_tools: Vec<String>,
}

impl LaunchConfig {
    /// The `--settings` JSON for one session, `None` when there is nothing to set.
    pub fn session_settings(&self, workspace: &str, role: &str, repo: Option<&str>) -> Option<String> {
        let mut settings = serde_json::Map::new();
        if let Some(home) = &self.foreman_home {
            let mut env = serde_json::json!({
                "FOREMAN_HOME": home.display().to_string(),
                "FOREMAN_ROLE": role,
                "FOREMAN_WORKSPACE": workspace,
            });
            if let Some(repo) = repo {
                env["FOREMAN_REPO"] = repo.into();
            }
            settings.insert("env".into(), env);
        }
        if self.bg_isolation_none {
            settings.insert("worktree".into(), serde_json::json!({ "bgIsolation": "none" }));
        }
        (!settings.is_empty()).then(|| serde_json::Value::Object(settings).to_string())
    }
}

/// One workspace's supervisor: desired state + memory, driven by [`Supervisor::tick`].
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

    /// Wanted sessions started less than `settle` ago that the snapshot does not show mid-turn: each
    /// holds a slot of the shared [`Budget`] until it is counted as working, or `settle` passes. A fresh
    /// process shows `status=idle` for a second before its first prompt (m3/2), so "live" is not enough.
    pub fn starting(&self, observed: &[Session], now: Millis) -> usize {
        if !self.desired.started {
            return 0;
        }
        self.desired
            .sessions
            .iter()
            .filter(|w| !w.exempt_from_budget())
            .filter(|w| {
                let settling = self
                    .memory
                    .get(&w.name)
                    .and_then(|a| a.started_at)
                    .is_some_and(|t| now - t < self.policy.settle);
                settling && !observed.iter().any(|s| s.name == w.name && mid_turn(s, now))
            })
            .count()
    }

    /// `observed` is one fresh `claude agents --json --all`. Observes, reconciles, executes the
    /// launches/resumes/stops, records their results, and returns the Inbox rows as they stand
    /// after execution (a launch refused for trust shows in the same tick). Launches and resumes
    /// go only while `budget` has free slots; each one that starts takes one.
    pub fn tick(&mut self, observed: &[Session], budget: &mut Budget, now: Millis) -> Vec<NeedsYou> {
        for w in &self.desired.sessions {
            let mut rows = observed.iter().filter(|s| s.name == w.name);
            let alive = match (rows.next(), rows.next()) {
                (Some(s), None) => is_live(s, now),
                _ => false,
            };
            self.memory.observe(&w.name, alive, &self.policy, now);
        }
        for action in reconcile(&self.desired, observed, &self.memory, &self.policy, budget.free, now) {
            match action {
                Action::Launch(w) => {
                    let result = self.launch(&w).map(drop);
                    if result.is_ok() && !w.exempt_from_budget() {
                        budget.take(1);
                    }
                    self.memory.record_result(&w.name, result, &self.policy, now);
                }
                Action::Resume {
                    name, id, session_id, cwd,
                } => {
                    let prompt = self.desired.sessions.iter().find(|w| w.name == name).and_then(|w| w.resume_prompt.clone());
                    let exempt = self.desired.sessions.iter().any(|w| w.name == name && w.exempt_from_budget());
                    let result = self
                        .launcher
                        .resume(&session_id, &cwd, prompt.as_deref())
                        .map_err(|e| Failure::from(&e))
                        .and_then(|out| woke_in_place(&out, &id));
                    if result.is_ok() && !exempt {
                        budget.take(1);
                    }
                    self.memory.record_result(&name, result, &self.policy, now);
                }
                // A failed stop is retried by the next tick: the row is still live.
                Action::Stop { id, .. } => {
                    let _ = self.launcher.stop(&id);
                }
                Action::Wait { .. } | Action::NeedsYou(_) => {}
            }
        }
        let needs: Vec<NeedsYou> = reconcile(&self.desired, observed, &self.memory, &self.policy, budget.free, now)
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
        spec.permission_mode = w.permission_mode.clone();
        spec.strict_mcp_config = true;
        spec.plugin_dirs = self.config.plugin_dirs.clone();
        spec.add_dirs = w.add_dirs.clone();
        if let Some(settings) = self.config.session_settings(&self.desired.workspace, &w.role, w.repo.as_deref()) {
            spec.extra_args = vec!["--settings".into(), settings];
        }
        if !self.config.allowed_tools.is_empty() {
            spec.extra_args.push("--allowedTools".into());
            spec.extra_args.extend(self.config.allowed_tools.iter().cloned());
        }
        if let Some(t) = self.tokens.token(&w.role) {
            spec = spec
                .with_session_token(&self.config.mcp_dir, &session_identity(&self.desired.workspace, w), &t.url, &t.token)
                .map_err(|e| Failure::Other(format!("cannot write the mcp config of {}: {e}", w.name)))?;
        }
        self.launcher.launch(&spec).map_err(|e| Failure::from(&e))
    }
}

/// What `w` tells WorkState about itself: its name, `foreman/<workspace>` as launcher, the repo in its
/// origin, its cwd and Foreman role.
pub fn session_identity(workspace: &str, w: &WantedSession) -> SessionIdentity {
    let launched_by = format!("foreman/{workspace}");
    SessionIdentity {
        agent: w.name.clone(),
        session: None,
        origin: match &w.repo {
            Some(repo) => format!("{launched_by}/{repo}"),
            None => launched_by.clone(),
        },
        launched_by,
        cwd: w.cwd.display().to_string(),
        foreman_role: w.role.clone(),
    }
}

/// `--bg --resume` of a session the daemon still counts as running starts a copy under a new id
/// (`backgrounded · <new id8> · …`) instead of waking it (m2/4): a failure, not a start.
fn woke_in_place(out: &str, id: &str) -> Result<(), Failure> {
    let woke = out
        .split_once("backgrounded · ")
        .and_then(|(_, rest)| rest.split_whitespace().next());
    match woke {
        Some(other) if other != id => Err(Failure::Other(format!(
            "the resume of {id} started a copy {other} (the daemon still counted it running)"
        ))),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use foreman_claude::{ClaudeError, Kind, SessionState, Status};

    use super::*;
    use crate::{NeedsKind, Remedy, SECOND, tick_all};

    const NOW: Millis = 1_800_000_000_000;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Call {
        Launch { name: String, argv: Vec<String> },
        Resume { session_id: String, cwd: PathBuf, prompt: Option<String> },
        Stop { id: String },
    }

    #[derive(Default)]
    struct FakeLauncher {
        calls: RefCell<Vec<Call>>,
        /// Popped per launch/resume; empty means success.
        errors: RefCell<Vec<ClaudeError>>,
        /// What a successful launch/resume prints; `None`: `backgrounded · aaaa0001 · x`.
        output: RefCell<Option<String>>,
    }

    impl FakeLauncher {
        fn result(&self) -> foreman_claude::Result<String> {
            match self.errors.borrow_mut().pop() {
                Some(e) => Err(e),
                None => Ok(self.output.borrow().clone().unwrap_or_else(|| "backgrounded · aaaa0001 · x".into())),
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

        fn resume(&self, session_id: &str, cwd: &Path, prompt: Option<&str>) -> foreman_claude::Result<String> {
            self.calls.borrow_mut().push(Call::Resume {
                session_id: session_id.into(),
                cwd: cwd.into(),
                prompt: prompt.map(str::to_string),
            });
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
        let mut desired = DesiredState::workspace_controller("demo", r"C:\demo\cwd", "be the controller");
        desired.started = started;
        let config = LaunchConfig {
            mcp_dir: mcp_dir.into(),
            plugin_dirs: vec![r"C:\fm\plugins\foreman".into()],
            ..LaunchConfig::default()
        };
        Supervisor::new(desired, config, FakeLauncher::default(), FakeTokens(tokens))
    }

    fn free() -> Budget {
        Budget { max: 3, free: 3 }
    }

    fn row(state: SessionState, status: Option<Status>, pid: Option<u32>) -> Session {
        Session {
            id: "aaaa0001".into(),
            session_id: "aaaa0001-0000-4000-8000-000000000001".into(),
            name: "fm-demo-workspace".into(),
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
            url: "http://127.0.0.1:5290/mcp".into(),
            token: "fake-token".into(),
        };
        let mut sup = supervisor(true, Some(token), &dir.path().join("mcp"));
        assert_eq!(sup.tick(&[], &mut free(), NOW), []);
        match sup.launcher().take().as_slice() {
            [Call::Launch { name, argv }] => {
                assert_eq!(name, "fm-demo-workspace");
                let argv = argv.join(" ");
                assert!(argv.contains("--strict-mcp-config"), "{argv}");
                assert!(argv.contains("workstate-fm-demo-workspace.json"), "{argv}");
                assert!(argv.contains(r"--plugin-dir C:\fm\plugins\foreman"), "{argv}");
                assert!(argv.ends_with("-- be the controller"), "{argv}");
                assert!(!argv.contains("fake-token"), "{argv}");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(sup.tick(&[], &mut free(), NOW + 5 * SECOND), []);
        assert_eq!(sup.launcher().take(), [], "the row has not shown yet: no second launch");
        let live = row(SessionState::Working, Some(Status::Busy), Some(4242));
        sup.tick(std::slice::from_ref(&live), &mut free(), NOW + 61 * SECOND);
        assert_eq!(sup.launcher().take(), []);
        assert_eq!(sup.memory.get("fm-demo-workspace").unwrap().started_at, None, "stable");
    }

    #[test]
    fn each_session_gets_its_own_config_with_its_identity_headers() {
        let dir = tempfile::tempdir().unwrap();
        let token = RoleToken { url: "http://127.0.0.1:5290/mcp".into(), token: "fake-token".into() };
        let mut sup = supervisor(true, Some(token), &dir.path().join("mcp"));
        sup.desired.sessions.push(WantedSession {
            name: "fm-demo-alpha".into(),
            role: "repo".into(),
            cwd: r"C:\demo\Alpha".into(),
            repo: Some("Alpha".into()),
            prompt: "run the queue".into(),
            permission_mode: "auto".into(),
            add_dirs: Vec::new(),
            resume_prompt: None,
        });
        sup.tick(&[], &mut free(), NOW);
        assert_eq!(sup.launcher().take().len(), 2);
        let headers = |name: &str| {
            let text = std::fs::read_to_string(dir.path().join("mcp").join(format!("workstate-{name}.json"))).unwrap();
            let v: serde_json::Value = serde_json::from_str(&text).unwrap();
            v["mcpServers"]["workstate"]["headers"].clone()
        };
        let repo = headers("fm-demo-alpha");
        assert_eq!(repo["X-WorkState-Agent"], "fm-demo-alpha");
        assert_eq!(repo["X-WorkState-Launched-By"], "foreman/demo");
        assert_eq!(repo["X-WorkState-Origin"], "foreman/demo/Alpha");
        assert_eq!(repo["X-WorkState-Cwd"], r"C:\demo\Alpha");
        assert_eq!(repo["X-WorkState-Foreman-Role"], "repo");
        assert!(repo.get("X-WorkState-Session").is_none());
        let ws = headers("fm-demo-workspace");
        assert_eq!(ws["X-WorkState-Origin"], "foreman/demo");
        assert_eq!(ws["X-WorkState-Foreman-Role"], "workspace");
        assert!(!dir.path().join("mcp").join("workstate-workspace.json").exists(), "no per-role file any more");
    }

    #[test]
    fn no_token_launches_without_mcp_config() {
        let dir = tempfile::tempdir().unwrap();
        let mut sup = supervisor(true, None, dir.path());
        sup.tick(&[], &mut free(), NOW);
        match sup.launcher().take().as_slice() {
            [Call::Launch { argv, .. }] => {
                assert!(!argv.iter().any(|a| a == "--mcp-config"), "{argv:?}");
                assert!(argv.iter().any(|a| a == "--strict-mcp-config"), "never the user-scope server: {argv:?}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn session_gets_its_env_isolation_and_permission_mode_through_settings() {
        let dir = tempfile::tempdir().unwrap();
        let mut sup = supervisor(true, None, dir.path());
        sup.config.foreman_home = Some(r"C:\fm\home".into());
        sup.config.bg_isolation_none = true;
        sup.config.allowed_tools = vec!["Bash(fm:*)".into(), "Bash(C:/fm/bin/fm.exe:*)".into()];
        sup.desired.sessions[0].permission_mode = "default".into();
        sup.tick(&[], &mut free(), NOW);
        let argv = match sup.launcher().take().as_slice() {
            [Call::Launch { argv, .. }] => argv.clone(),
            other => panic!("{other:?}"),
        };
        let at = |flag: &str| argv.iter().position(|a| a == flag).map(|i| argv[i + 1].clone());
        assert_eq!(at("--permission-mode").as_deref(), Some("default"));
        let tools = argv.iter().position(|a| a == "--allowedTools").unwrap();
        assert_eq!(argv[tools + 1..tools + 4], ["Bash(fm:*)", "Bash(C:/fm/bin/fm.exe:*)", "--"]);
        let settings: serde_json::Value = serde_json::from_str(&at("--settings").unwrap()).unwrap();
        assert_eq!(
            settings,
            serde_json::json!({
                "env": { "FOREMAN_HOME": r"C:\fm\home", "FOREMAN_ROLE": "workspace", "FOREMAN_WORKSPACE": "demo" },
                "worktree": { "bgIsolation": "none" }
            })
        );
        assert_eq!(LaunchConfig::default().session_settings("demo", "workspace", None), None);
    }

    fn workspace(name: &str, mcp_dir: &Path) -> Supervisor<FakeLauncher, FakeTokens> {
        let mut desired = DesiredState::workspace_controller(name, r"C:\demo\cwd", "be the controller");
        desired.started = true;
        let config = LaunchConfig {
            mcp_dir: mcp_dir.into(),
            ..LaunchConfig::default()
        };
        Supervisor::new(desired, config, FakeLauncher::default(), FakeTokens(None))
    }

    fn live(name: &str, n: u32, state: SessionState, status: Status) -> Session {
        Session {
            id: format!("bbbb000{n}"),
            session_id: format!("bbbb000{n}-0000-4000-8000-000000000000"),
            name: name.into(),
            cwd: r"C:\demo\cwd".into(),
            kind: Kind::Background,
            state: Some(state),
            status: Some(status),
            waiting_for: None,
            pid: Some(100 + n),
            started_at: Some(NOW),
        }
    }

    #[test]
    fn two_started_workspaces_never_exceed_the_shared_budget_together() {
        let dir = tempfile::tempdir().unwrap();
        let launched = |sups: &[Supervisor<FakeLauncher, FakeTokens>]| -> Vec<usize> {
            sups.iter().map(|s| s.launcher().take().len()).collect()
        };
        let mut sups = [workspace("alpha", dir.path()), workspace("beta", dir.path())];
        // budget 1, both need a launch: only alpha goes
        tick_all(sups.iter_mut(), &[], 1, NOW);
        assert_eq!(launched(&sups), [1, 0]);
        // alpha's row has not shown yet: it still holds the slot, beta waits
        tick_all(sups.iter_mut(), &[], 1, NOW + 2 * SECOND);
        assert_eq!(launched(&sups), [0, 0]);
        // alpha is working: beta waits
        let alpha_busy = live("fm-alpha-workspace", 1, SessionState::Working, Status::Busy);
        tick_all(sups.iter_mut(), std::slice::from_ref(&alpha_busy), 1, NOW + 4 * SECOND);
        assert_eq!(launched(&sups), [0, 0]);
        // alpha's turn ended (done + live pid = idle): the slot frees, beta goes
        let alpha_idle = live("fm-alpha-workspace", 1, SessionState::Done, Status::Idle);
        tick_all(sups.iter_mut(), std::slice::from_ref(&alpha_idle), 1, NOW + 30 * SECOND);
        assert_eq!(launched(&sups), [0, 1]);
        // the same snapshot again: beta, settling, holds the slot
        tick_all(sups.iter_mut(), std::slice::from_ref(&alpha_idle), 1, NOW + 32 * SECOND);
        assert_eq!(launched(&sups), [0, 0]);

        // budget 2: both in one tick
        let mut sups = [workspace("alpha", dir.path()), workspace("beta", dir.path())];
        tick_all(sups.iter_mut(), &[], 2, NOW);
        assert_eq!(launched(&sups), [1, 1]);

        // budget 0 pauses every launch; a working fm- session nobody here supervises counts too
        let mut sups = [workspace("alpha", dir.path()), workspace("beta", dir.path())];
        tick_all(sups.iter_mut(), &[], 0, NOW);
        assert_eq!(launched(&sups), [0, 0]);
        let other = live("fm-gamma-workspace", 3, SessionState::Working, Status::Busy);
        tick_all(sups.iter_mut(), std::slice::from_ref(&other), 1, NOW + SECOND);
        assert_eq!(launched(&sups), [0, 0]);
        let needs = tick_all(sups.iter_mut(), std::slice::from_ref(&other), 2, NOW + 2 * SECOND);
        assert_eq!(launched(&sups), [1, 0]);
        assert!(needs.is_empty(), "a cap wait is not an Inbox row: {needs:?}");
    }

    #[test]
    fn a_workspace_session_idling_on_the_relay_frees_its_slot() {
        let dir = tempfile::tempdir().unwrap();
        let mut sups = [workspace("alpha", dir.path()), workspace("beta", dir.path())];
        tick_all(sups.iter_mut(), &[], 1, NOW);
        assert_eq!(sups.iter().map(|s| s.launcher().take().len()).collect::<Vec<_>>(), [1, 0]);
        // just started, no prompt taken yet: `status=idle` on a live pid still holds the slot while settling
        let fresh = live("fm-alpha-workspace", 1, SessionState::Working, Status::Idle);
        tick_all(sups.iter_mut(), std::slice::from_ref(&fresh), 1, NOW + 2 * SECOND);
        assert_eq!(sups.iter().map(|s| s.launcher().take().len()).collect::<Vec<_>>(), [0, 0]);
        // its turn ended, `state` stayed `working`: the slot is free once it settled
        tick_all(sups.iter_mut(), std::slice::from_ref(&fresh), 1, NOW + 30 * SECOND);
        assert_eq!(sups.iter().map(|s| s.launcher().take().len()).collect::<Vec<_>>(), [0, 1]);
    }

    #[test]
    fn stopped_row_is_resumed_from_its_cwd_with_its_session_id() {
        let dir = tempfile::tempdir().unwrap();
        let mut sup = supervisor(true, None, dir.path());
        sup.tick(&[row(SessionState::Stopped, None, None)], &mut free(), NOW);
        assert_eq!(
            sup.launcher().take(),
            [Call::Resume {
                session_id: "aaaa0001-0000-4000-8000-000000000001".into(),
                cwd: r"C:\demo\cwd".into(),
                prompt: None,
            }]
        );
    }

    fn with_relay(sup: &mut Supervisor<FakeLauncher, FakeTokens>) {
        let relay = crate::relay_session("demo", r"C:\demo\cwd", r"C:\fm\home\relay\demo", "be the relay");
        sup.desired.sessions.insert(0, relay);
    }

    #[test]
    fn the_relay_launches_first_with_its_add_dir_and_never_takes_a_budget_slot() {
        let dir = tempfile::tempdir().unwrap();
        let mut sup = supervisor(true, None, dir.path());
        with_relay(&mut sup);
        let mut budget = Budget { max: 1, free: 1 };
        sup.tick(&[], &mut budget, NOW);
        let calls = sup.launcher().take();
        let names: Vec<&str> = calls
            .iter()
            .map(|c| match c {
                Call::Launch { name, .. } => name.as_str(),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(names, ["fm-demo-relay", "fm-demo-workspace"], "relay first");
        let Call::Launch { argv, .. } = &calls[0] else { unreachable!() };
        let at = argv.iter().position(|a| a == "--add-dir").expect("--add-dir");
        assert_eq!(argv[at + 1], r"C:\fm\home\relay\demo");
        assert_eq!(budget.free, 0, "only the workspace session took the slot");
        assert_eq!(sup.starting(&[], NOW + SECOND), 1, "a settling relay holds no slot");

        // budget 0: the relay still goes, the workspace waits
        let mut sup = supervisor(true, None, dir.path());
        with_relay(&mut sup);
        sup.tick(&[], &mut Budget { max: 0, free: 0 }, NOW);
        assert!(matches!(sup.launcher().take().as_slice(), [Call::Launch { name, .. }] if name == "fm-demo-relay"));
    }

    #[test]
    fn armed_relays_leave_the_budget_to_the_workspace_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let mut sups = [workspace("alpha", dir.path()), workspace("beta", dir.path())];
        for (s, name) in sups.iter_mut().zip(["alpha", "beta"]) {
            let relay = crate::relay_session(name, r"C:\demo\cwd", r"C:\fm\relay", "be the relay");
            s.desired.sessions.insert(0, relay);
        }
        tick_all(sups.iter_mut(), &[], 1, NOW);
        let first: Vec<Vec<Call>> = sups.iter().map(|s| s.launcher().take()).collect();
        assert_eq!(first[0].len(), 2, "alpha: relay and workspace {first:?}");
        assert_eq!(first[1].len(), 1, "beta: relay only {first:?}");
        let rows = [
            live("fm-alpha-relay", 1, SessionState::Working, Status::Busy),
            live("fm-beta-relay", 2, SessionState::Working, Status::Busy),
            live("fm-alpha-workspace", 3, SessionState::Done, Status::Idle),
        ];
        tick_all(sups.iter_mut(), &rows, 1, NOW + 30 * SECOND);
        let second: Vec<usize> = sups.iter().map(|s| s.launcher().take().len()).collect();
        assert_eq!(second, [0, 1], "beta's workspace session goes once alpha's is idle");
    }

    #[test]
    fn a_resumed_relay_is_told_to_rearm() {
        let dir = tempfile::tempdir().unwrap();
        let mut sup = supervisor(true, None, dir.path());
        with_relay(&mut sup);
        let mut relay = row(SessionState::Stopped, None, None);
        relay.name = "fm-demo-relay".into();
        let live = row(SessionState::Done, Some(Status::Idle), Some(5));
        sup.tick(&[relay, live], &mut Budget { max: 0, free: 0 }, NOW);
        match sup.launcher().take().as_slice() {
            [Call::Resume { prompt: Some(p), .. }] => assert!(p.contains("Re-arm"), "{p}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn stop_stops_the_relay_after_the_other_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let mut sup = supervisor(false, None, dir.path());
        with_relay(&mut sup);
        let mut relay = row(SessionState::Working, Some(Status::Busy), Some(2));
        relay.name = "fm-demo-relay".into();
        relay.id = "aaaa0002".into();
        sup.tick(&[relay, row(SessionState::Done, Some(Status::Idle), Some(1))], &mut free(), NOW);
        assert_eq!(
            sup.launcher().take(),
            [Call::Stop { id: "aaaa0001".into() }, Call::Stop { id: "aaaa0002".into() }]
        );
    }

    #[test]
    fn idle_done_with_pid_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let mut sup = supervisor(true, None, dir.path());
        assert_eq!(sup.tick(&[row(SessionState::Done, Some(Status::Idle), Some(27304))], &mut free(), NOW), []);
        assert_eq!(sup.launcher().take(), []);
    }

    #[test]
    fn stop_stops_live_and_never_removes() {
        let dir = tempfile::tempdir().unwrap();
        let mut sup = supervisor(false, None, dir.path());
        sup.tick(&[row(SessionState::Working, Some(Status::Busy), Some(1))], &mut free(), NOW);
        assert_eq!(sup.launcher().take(), [Call::Stop { id: "aaaa0001".into() }]);
        sup.tick(&[row(SessionState::Stopped, None, None)], &mut free(), NOW + SECOND);
        assert_eq!(sup.launcher().take(), []);
    }

    #[test]
    fn trust_refusal_shows_in_the_same_tick_and_waits_for_retry() {
        let dir = tempfile::tempdir().unwrap();
        let mut sup = supervisor(true, None, dir.path());
        sup.launcher.errors.borrow_mut().push(ClaudeError::TrustNeeded { cwd: r"C:\demo\cwd".into() });
        let needs = sup.tick(&[], &mut free(), NOW);
        assert_eq!(needs.len(), 1);
        assert_eq!(needs[0].kind, NeedsKind::TrustNeeded);
        assert_eq!(needs[0].since, NOW);
        assert_eq!(
            needs[0].action,
            Some(Remedy::RunInTerminal { cwd: r"C:\demo\cwd".into(), command: "claude".into() })
        );
        sup.launcher().take();
        let later = sup.tick(&[], &mut free(), NOW + 3_600 * SECOND);
        assert_eq!(later[0].since, NOW, "since is kept");
        assert_eq!(sup.launcher().take(), [], "never retried automatically");
        sup.retry("fm-demo-workspace");
        assert_eq!(sup.tick(&[], &mut free(), NOW + 3_601 * SECOND), []);
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
            sup.tick(&[], &mut free(), now);
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
        sup.retry("fm-demo-workspace");
        sup.tick(&[], &mut free(), now);
        assert_eq!(sup.launcher().take().len(), 1);
    }

    #[test]
    fn a_resume_that_starts_a_copy_is_a_failure() {
        let dir = tempfile::tempdir().unwrap();
        let mut sup = supervisor(true, None, dir.path());
        *sup.launcher.output.borrow_mut() = Some("backgrounded · bbbb0002 (idle — send a prompt to start)".into());
        let needs = sup.tick(&[row(SessionState::Stopped, None, None)], &mut free(), NOW);
        assert_eq!(sup.launcher().take().len(), 1);
        assert_eq!(needs[0].kind, NeedsKind::LaunchFailed);
        assert!(needs[0].detail.contains("started a copy bbbb0002"), "{}", needs[0].detail);
        assert_eq!(woke_in_place("backgrounded · aaaa0001 · fm-demo-workspace", "aaaa0001"), Ok(()));
        assert_eq!(woke_in_place("", "aaaa0001"), Ok(()));
    }

    #[test]
    fn launch_failed_is_listed_while_backing_off() {
        let dir = tempfile::tempdir().unwrap();
        let mut sup = supervisor(true, None, dir.path());
        sup.launcher.errors.borrow_mut().push(ClaudeError::NotInstalled);
        let needs = sup.tick(&[], &mut free(), NOW);
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
        sup.tick(std::slice::from_ref(&dead), &mut free(), NOW);
        assert_eq!(sup.launcher().take().len(), 1);
        sup.tick(&[row(SessionState::Working, Some(Status::Busy), Some(9))], &mut free(), NOW + 10 * SECOND);
        let needs = sup.tick(std::slice::from_ref(&dead), &mut free(), NOW + 30 * SECOND);
        assert_eq!(sup.launcher().take(), [], "backing off 5 s");
        assert_eq!(needs[0].kind, NeedsKind::LaunchFailed);
        assert!(needs[0].detail.contains("exited within 30 s"), "{}", needs[0].detail);
        // the backoff is over at +35 s, the respawn grace (last alive at +10 s) at +40 s
        sup.tick(std::slice::from_ref(&dead), &mut free(), NOW + 35 * SECOND);
        assert_eq!(sup.launcher().take(), [], "respawn grace");
        sup.tick(&[dead], &mut free(), NOW + 40 * SECOND);
        assert_eq!(sup.launcher().take().len(), 1);
    }
}
