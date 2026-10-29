use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use foreman_claude::{Claude, LaunchSpec, Session, SessionIdentity};

use crate::{
    Action, Budget, DesiredState, Failure, HttpMinter, MintRepo, MintRequest, MintedSession, MintedSessions, Millis, Minter, NeedsKind,
    NeedsYou, Policy, Remedy, SECOND, SupervisorMemory, WantedSession, api_base, is_live, mid_turn, reconcile,
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

/// The bearer of the mint, PATCH and DELETE of a session's own token ([`crate::mint`]): the app's
/// OAuth access token, never written anywhere.
#[derive(Clone, PartialEq, Eq)]
pub struct RoleToken {
    /// The WorkState MCP URL; its origin is the API's base.
    pub url: String,
    pub token: String,
    /// The WorkState role the session is minted with (controller, orchestrator, worker).
    pub role: String,
}

impl std::fmt::Debug for RoleToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RoleToken").field("url", &self.url).field("token", &"<redacted>").field("role", &self.role).finish()
    }
}

/// Where the bearer comes from: the app's sign-in.
pub trait TokenProvider {
    /// `Ok(None)`: not signed in, so the role launches without a WorkState server. `Err`: signed in but
    /// no token now (the server unreachable, the sign-in to redo); a launch or resume fails with it.
    fn token(&self, role: &str) -> Result<Option<RoleToken>, String>;
}

/// Launch options every session of this supervisor gets. Every launch is `--strict-mcp-config`: a
/// session gets the role's WorkState server or none, never the user-scope one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaunchConfig {
    /// `<FOREMAN_HOME>/mcp`, where [`LaunchSpec::with_minted_token`] writes each session's owner-only config.
    pub mcp_dir: PathBuf,
    pub plugin_dirs: Vec<PathBuf>,
    /// Passed to the session as `FOREMAN_HOME`, with `FOREMAN_ROLE`, `FOREMAN_WORKSPACE`, `FOREMAN_SESSION` (its
    /// name, what `fm tell` signs with) and `FOREMAN_REPO` for a repo session, through
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
    pub fn session_settings(&self, workspace: &str, name: &str, role: &str, repo: Option<&str>) -> Option<String> {
        let mut settings = serde_json::Map::new();
        if let Some(home) = &self.foreman_home {
            let mut env = serde_json::json!({
                "FOREMAN_HOME": home.display().to_string(),
                "FOREMAN_ROLE": role,
                "FOREMAN_WORKSPACE": workspace,
                "FOREMAN_SESSION": name,
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
    minter: Box<dyn Minter + Send>,
    /// The `repo` of a repo session's mint, from its cwd: [`crate::git_repo`]; tests use a fake.
    repo_of: fn(&Path) -> Option<MintRepo>,
    /// The sessions this supervisor minted a token for, by name.
    minted: MintedSessions,
    needs: Vec<NeedsYou>,
    notices: Vec<String>,
    /// Names whose Re-mint was pressed: the next tick stops their live row, and the resume that
    /// follows mints a fresh token. A name stays until its stop succeeds or it has no live row.
    remint_due: Vec<String>,
    /// Consecutive failed `claude stop`s, by name: from [`STOP_FAILURES_SHOWN`] on, an Inbox row.
    stop_failures: BTreeMap<String, StopFailure>,
    /// Resumes whose output named no id ([`Resumed::Unknown`]): (name, id, when), settled against the
    /// next snapshots ([`Self::confirm_resumes`]).
    unconfirmed: Vec<(String, String, Millis)>,
    /// The server Foreman is signed in to, as [`Self::server_changed`] last saw it.
    origin: Option<String>,
    /// The server [`Self::server_changed`] last scheduled re-mints for.
    checked: Option<String>,
    /// While set, no launch or resume goes: they would only fail after 45 s. Why, and since when.
    held: Option<(String, Millis)>,
    /// Why a `--bg` that could not reach the background service will keep failing, if it will:
    /// [`foreman_claude::daemon_mismatch`]; tests use a fake.
    daemon_check: fn() -> Option<String>,
}

impl<L: Launcher, T: TokenProvider> Supervisor<L, T> {
    /// Mints session tokens over HTTP ([`HttpMinter`]); the minted sessions are kept at
    /// [`MintedSessions::path`] when `config.foreman_home` is set.
    pub fn new(desired: DesiredState, config: LaunchConfig, launcher: L, tokens: T) -> Self {
        let path = config.foreman_home.as_deref().map(|home| MintedSessions::path(home, &desired.workspace));
        let mut notices = Vec::new();
        let mut minted = MintedSessions::load(path.clone()).unwrap_or_else(|e| {
            let shown = path.as_deref().map(|p| p.display().to_string()).unwrap_or_default();
            notices.push(format!("Cannot read {shown} ({e}): the WorkState sessions minted before are forgotten"));
            MintedSessions::empty(path)
        });
        if let Err(e) = minted.adopt_origins(&config.mcp_dir) {
            notices.push(format!("Cannot save the minted WorkState sessions of {}: {e}", desired.workspace));
        }
        Self {
            desired,
            memory: SupervisorMemory::default(),
            policy: Policy::default(),
            config,
            launcher,
            tokens,
            minter: Box::new(HttpMinter::default()),
            repo_of: crate::git_repo,
            minted,
            needs: Vec::new(),
            notices,
            remint_due: Vec::new(),
            stop_failures: BTreeMap::new(),
            unconfirmed: Vec::new(),
            origin: None,
            checked: None,
            held: None,
            daemon_check: foreman_claude::daemon_mismatch,
        }
    }

    pub fn with_daemon_check(mut self, check: fn() -> Option<String>) -> Self {
        self.daemon_check = check;
        self
    }

    /// The app's own [`foreman_claude::daemon_mismatch`] before launching: `Some` holds every
    /// launch and resume and shows one Inbox row saying why; `None` lets them go again.
    pub fn hold(&mut self, why: Option<String>, now: Millis) {
        self.held = why.map(|why| match self.held.take() {
            Some((_, since)) => (why, since),
            None => (why, now),
        });
    }

    pub fn held(&self) -> Option<&str> {
        self.held.as_ref().map(|(why, _)| why.as_str())
    }

    pub fn with_minter(mut self, minter: impl Minter + Send + 'static) -> Self {
        self.minter = Box::new(minter);
        self
    }

    pub fn with_repo_of(mut self, repo_of: fn(&Path) -> Option<MintRepo>) -> Self {
        self.repo_of = repo_of;
        self
    }

    /// What went wrong outside a launch since the last call (a failed PATCH or revoke): not Inbox
    /// rows, nothing the human must do.
    pub fn take_notices(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notices)
    }

    pub fn minted(&self) -> &MintedSessions {
        &self.minted
    }

    pub fn launcher(&self) -> &L {
        &self.launcher
    }

    /// The Inbox rows of the last tick.
    pub fn needs_you(&self) -> &[NeedsYou] {
        &self.needs
    }

    /// The human's Retry: forgets `name`'s failures (incl. trust and give-up); the next tick acts.
    /// On a [`NeedsKind::TokenExpired`] row it is the Re-mint: the next tick stops the live row.
    pub fn retry(&mut self, name: &str) {
        self.memory.retry(name);
        self.held = None;
        if self.needs.iter().any(|n| matches!(n.kind, NeedsKind::TokenExpired | NeedsKind::OtherServer) && n.name == name)
            && !self.remint_due.iter().any(|n| n == name)
        {
            self.remint_due.push(name.to_string());
        }
    }

    /// The server Foreman is signed in to (`None`: not signed in), on every tick. The first time it is
    /// `origin`, every wanted session minted elsewhere (or on an unknown server) is stopped at the next
    /// tick and resumed (by its full uuid) with a token minted on `origin`, `replace: true`, in the same
    /// config file; a stopped one gets it at its next resume anyway. Returns those (name, its origin).
    pub fn server_changed(&mut self, origin: Option<&str>) -> Vec<(String, Option<String>)> {
        self.origin = origin.map(api_base);
        let Some(origin) = self.origin.clone() else {
            return Vec::new();
        };
        if self.checked.as_deref() == Some(origin.as_str()) {
            return Vec::new();
        }
        self.checked = Some(origin.clone());
        let stale: Vec<(String, Option<String>)> = self
            .minted
            .elsewhere(&origin)
            .into_iter()
            .filter(|(n, _)| self.desired.sessions.iter().any(|w| &w.name == n))
            .filter(|(n, _)| !self.remint_due.contains(n))
            .collect();
        self.remint_due.extend(stale.iter().map(|(n, _)| n.clone()));
        stale
    }

    /// Whether `m`'s token is from another server than `url`'s: its PATCH and DELETE would name another
    /// server's session, so they are left to that server's grant revoke.
    fn elsewhere(m: &MintedSession, url: &str) -> bool {
        m.origin.as_deref().is_some_and(|o| o != api_base(url))
    }

    #[cfg(test)]
    fn tokens_mut(&mut self) -> &mut T {
        &mut self.tokens
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
        let mut stops = BTreeMap::new();
        let reminting = self.stop_for_remint(observed, &mut stops, now);
        self.confirm_resumes(observed, now);
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
                Action::Launch(_) | Action::Resume { .. } if self.held.is_some() => {}
                Action::Launch(w) => {
                    let result = self.launch(&w, now).map(drop);
                    let result = self.unless_held(result, now);
                    if matches!(result, Some(Ok(()))) && !w.exempt_from_budget() {
                        budget.take(1);
                    }
                    if let Some(result) = result {
                        self.memory.record_result(&w.name, result, &self.policy, now);
                    }
                }
                Action::Resume {
                    name, id, session_id, cwd,
                } => {
                    let wanted = self.desired.sessions.iter().find(|w| w.name == name).cloned();
                    let prompt = wanted.as_ref().and_then(|w| w.resume_prompt.clone());
                    let exempt = wanted.as_ref().is_some_and(|w| w.exempt_from_budget());
                    // the resume keeps its `--mcp-config` path: a fresh token goes into the same file
                    let result = match &wanted {
                        Some(w) => self.remint(w, &session_id, now),
                        None => Ok(()),
                    }
                    .and_then(|()| self.launcher.resume(&session_id, &cwd, prompt.as_deref()).map_err(|e| Failure::from(&e)))
                    .and_then(|out| match resumed(&out, &id) {
                        Resumed::InPlace => Ok(()),
                        Resumed::Copy(other) => Err(copy_failure(&id, &other)),
                        Resumed::Unknown => {
                            self.notices.push(format!(
                                "claude printed {out:?} for the resume of {name} ({id}); the next session list shows whether it woke in place"
                            ));
                            self.unconfirmed.push((name.clone(), id.clone(), now));
                            Ok(())
                        }
                    });
                    let result = self.unless_held(result, now);
                    if matches!(result, Some(Ok(()))) && !exempt {
                        budget.take(1);
                    }
                    if let Some(result) = result {
                        self.memory.record_result(&name, result, &self.policy, now);
                    }
                }
                // a failed stop is retried by the next tick: the row is still live
                Action::Stop { name, id } => {
                    let result = self.launcher.stop(&id).map(drop).map_err(|e| e.to_string());
                    stops.insert(name, (id, result));
                }
                Action::Wait { .. } | Action::NeedsYou(_) => {}
            }
        }
        self.record_stops(stops, now);
        self.bind_claude_sessions(observed, now);
        self.revoke_unwanted();
        self.track_activity(observed, now);
        let mut needs: Vec<NeedsYou> = reconcile(&self.desired, observed, &self.memory, &self.policy, budget.free, now)
            .into_iter()
            .filter_map(|a| match a {
                Action::NeedsYou(n) => Some(n),
                _ => None,
            })
            .collect();
        needs.extend(self.expired_tokens(observed, &reminting, now));
        needs.extend(self.other_server_tokens(observed, &reminting, now));
        needs.extend(self.failed_stops());
        if let Some(need) = self.held_need(observed, now) {
            needs.retain(|n| !matches!(n.kind, NeedsKind::LaunchFailed | NeedsKind::GaveUp));
            needs.push(need);
        }
        for n in &mut needs {
            if let Some(prev) = self.needs.iter().find(|p| p.kind == n.kind && p.name == n.name) {
                n.since = n.since.min(prev.since);
            }
        }
        self.needs = needs.clone();
        needs
    }

    /// `None` when `result` could not reach the background service and [`Self::daemon_check`] says it
    /// never will from here: then the supervisor holds instead of counting a failure.
    fn unless_held(&mut self, result: Result<(), Failure>, now: Millis) -> Option<Result<(), Failure>> {
        if matches!(result, Err(Failure::DaemonUnreachable(_)))
            && let Some(why) = (self.daemon_check)()
        {
            self.hold(Some(why), now);
            return None;
        }
        Some(result)
    }

    /// While held, the workspace's one Inbox row: on its first wanted session that is not live.
    fn held_need(&self, observed: &[Session], now: Millis) -> Option<NeedsYou> {
        let (why, since) = self.held.as_ref()?;
        if !self.desired.started {
            return None;
        }
        let waiting = self
            .desired
            .sessions
            .iter()
            .find(|w| !observed.iter().any(|s| s.name == w.name && is_live(s, now)))?;
        Some(NeedsYou {
            kind: NeedsKind::LaunchFailed,
            workspace: self.desired.workspace.clone(),
            name: waiting.name.clone(),
            since: *since,
            detail: why.clone(),
            action: Some(Remedy::RestartElevated),
        })
    }

    fn launch(&mut self, w: &WantedSession, now: Millis) -> Result<String, Failure> {
        let mut spec = LaunchSpec::new(&w.name, &w.cwd, &w.prompt);
        spec.permission_mode = w.permission_mode.clone();
        spec.strict_mcp_config = true;
        spec.plugin_dirs = self.config.plugin_dirs.clone();
        spec.add_dirs = w.add_dirs.clone();
        if let Some(settings) = self.config.session_settings(&self.desired.workspace, &w.name, &w.role, w.repo.as_deref()) {
            spec.extra_args = vec!["--settings".into(), settings];
        }
        if !self.config.allowed_tools.is_empty() {
            spec.extra_args.push("--allowedTools".into());
            spec.extra_args.extend(self.config.allowed_tools.iter().cloned());
        }
        if let Some((url, token)) = self.mint(w, None, now)? {
            spec = spec
                .with_minted_token(&self.config.mcp_dir, &w.name, &url, &token)
                .map_err(|e| Failure::Other(format!("cannot write the mcp config of {}: {e}", w.name)))?;
        }
        self.launcher.launch(&spec).map_err(|e| Failure::from(&e))
    }

    /// Mints `w`'s own token with the signed-in bearer, always with `replace` (a relaunch or resume gets a
    /// fresh token on the same WorkState row), and remembers its `sessionId`. Returns the MCP URL and the
    /// minted token; `None` when the role has no WorkState (not signed in). A failed mint fails the launch.
    ///
    /// `claude_session_id` is the uuid a resume wakes, which `--bg --resume` keeps; a `--bg` launch
    /// ignores `--session-id` (m5/40 probe), so a launch sends none and its row's id is PATCHed later
    /// ([`Self::bind_claude_sessions`]).
    fn mint(&mut self, w: &WantedSession, claude_session_id: Option<&str>, now: Millis) -> Result<Option<(String, String)>, Failure> {
        let Some(t) = self
            .tokens
            .token(&w.role)
            .map_err(|e| Failure::Other(format!("cannot mint a WorkState token for {}: {e}", w.name)))?
        else {
            return Ok(None);
        };
        let identity = session_identity(&self.desired.workspace, w);
        let request = MintRequest {
            name: w.name.clone(),
            role: t.role.clone(),
            launched_by: identity.launched_by,
            origin: identity.origin,
            cwd: identity.cwd,
            external_role: w.role.clone(),
            replace: true,
            repo: w.repo.as_ref().and_then(|_| (self.repo_of)(&w.cwd)),
            claude_session_id: claude_session_id.map(str::to_string),
        };
        let minted = self
            .minter
            .mint(&api_base(&t.url), &t.token, &request)
            .map_err(|e| Failure::Other(format!("cannot mint a WorkState token for {}: {e}", w.name)))?;
        self.minted.insert(
            &w.name,
            MintedSession {
                session_id: minted.session_id,
                role: w.role.clone(),
                claude_session_id: claude_session_id.map(str::to_string),
                last_active: Some(now),
                origin: Some(api_base(&t.url)),
            },
        );
        self.save_minted();
        Ok(Some((t.url, minted.token)))
    }

    /// Before the resume of `claude_session_id`: a fresh token into the session's config file.
    fn remint(&mut self, w: &WantedSession, claude_session_id: &str, now: Millis) -> Result<(), Failure> {
        if let Some((url, token)) = self.mint(w, Some(claude_session_id), now)? {
            foreman_claude::write_mcp_config(&self.config.mcp_dir, &w.name, &url, &token, &[])
                .map_err(|e| Failure::Other(format!("cannot write the mcp config of {}: {e}", w.name)))?;
        }
        Ok(())
    }

    /// PATCHes the claude session uuid onto each minted session once its single live row shows it:
    /// once per mint, failed or not (a failure is a notice).
    fn bind_claude_sessions(&mut self, observed: &[Session], now: Millis) {
        let mut due = Vec::new();
        for w in &self.desired.sessions {
            let Some(m) = self.minted.get(&w.name) else {
                continue;
            };
            let mut rows = observed.iter().filter(|s| s.name == w.name && is_live(s, now));
            if let (Some(row), None) = (rows.next(), rows.next())
                && !row.session_id.is_empty()
                && m.claude_session_id.as_deref() != Some(row.session_id.as_str())
            {
                due.push((w.name.clone(), row.session_id.clone()));
            }
        }
        for (name, claude_session_id) in due {
            let Some(m) = self.minted.get_mut(&name) else {
                continue;
            };
            m.claude_session_id = Some(claude_session_id.clone());
            let (role, session_id, m) = (m.role.clone(), m.session_id.clone(), m.clone());
            let result = match self.tokens.token(&role) {
                Ok(Some(t)) if Self::elsewhere(&m, &t.url) => Ok(()),
                Ok(Some(t)) => self.minter.bind(&api_base(&t.url), &t.token, &session_id, &claude_session_id).map_err(|e| e.to_string()),
                Ok(None) => Err(format!("role {role} has no WorkState token any more")),
                Err(e) => Err(e),
            };
            if let Err(e) = result {
                self.notices.push(format!("Cannot tell WorkState the claude session of {name} (session {session_id}): {e}"));
            }
            self.save_minted();
        }
    }

    /// Revokes the token of every minted session no longer kept alive (Stop, `fm unwant` (R14), gave
    /// up, gone from the desired state) and deletes its config file. A failed revoke is a notice: the
    /// unused token lapses by itself after its idle window.
    fn revoke_unwanted(&mut self) {
        let kept = |name: &str| {
            self.desired.started
                && self.desired.sessions.iter().any(|w| w.name == name)
                && !self.memory.get(name).is_some_and(|a| a.gave_up)
        };
        let gone: Vec<String> = self.minted.names().filter(|n| !kept(n)).cloned().collect();
        for name in gone {
            let Some(m) = self.minted.remove(&name) else {
                continue;
            };
            let result = match self.tokens.token(&m.role) {
                Ok(Some(t)) if Self::elsewhere(&m, &t.url) => Ok(()),
                Ok(Some(t)) => self.minter.revoke(&api_base(&t.url), &t.token, &m.session_id).map_err(|e| e.to_string()),
                Ok(None) => Err(format!("role {} has no WorkState token any more", m.role)),
                Err(e) => Err(e),
            };
            if let Err(e) = result {
                self.notices.push(format!(
                    "Cannot revoke the WorkState token of {name} (session {}): {e}; unused, it lapses after its idle window",
                    m.session_id
                ));
            }
            match foreman_claude::mcp_config_path(&self.config.mcp_dir, &name) {
                Ok(file) => match std::fs::remove_file(&file) {
                    Err(e) if e.kind() != ErrorKind::NotFound => self.notices.push(format!("Cannot delete {}: {e}", file.display())),
                    _ => {}
                },
                Err(e) => self.notices.push(format!("Cannot delete the WorkState config of {name}: {e}")),
            }
            self.save_minted();
        }
    }

    /// The Re-mint: stops the single live row of each name whose Re-mint was pressed. The supervisor
    /// resumes it after the respawn grace (R13, R3), and the resume mints a fresh token ([`Self::remint`]);
    /// a `--bg --resume` of a live row would start a copy instead (m2/4). Returns the names stopped.
    fn stop_for_remint(&mut self, observed: &[Session], stops: &mut Stops, now: Millis) -> Vec<String> {
        let mut stopped = Vec::new();
        for name in std::mem::take(&mut self.remint_due) {
            if !self.desired.sessions.iter().any(|w| w.name == name) {
                continue;
            }
            let rows: Vec<&Session> = observed.iter().filter(|s| s.name == name && is_live(s, now)).collect();
            match rows.as_slice() {
                [] => {}
                [row] => {
                    let result = self.launcher.stop(&row.id).map(drop).map_err(|e| e.to_string());
                    if result.is_ok() {
                        stopped.push(name.clone());
                    } else {
                        self.remint_due.push(name.clone());
                    }
                    stops.insert(name, (row.id.clone(), result));
                }
                _ => self.remint_due.push(name),
            }
        }
        stopped
    }

    /// This tick's `claude stop`s: a failure adds to its name's count, anything else clears it.
    fn record_stops(&mut self, stops: Stops, now: Millis) {
        let mut failures = BTreeMap::new();
        for (name, (id, result)) in stops {
            if let Err(error) = result {
                let prev = self.stop_failures.remove(&name);
                let (count, since) = prev.map_or((0, now), |p| (p.failures, p.since));
                failures.insert(name, StopFailure { id, failures: count + 1, since, error });
            }
        }
        self.stop_failures = failures;
    }

    /// [`NeedsKind::StopFailed`] for each name whose stop failed [`STOP_FAILURES_SHOWN`] ticks in a row.
    fn failed_stops(&self) -> Vec<NeedsYou> {
        self.stop_failures
            .iter()
            .filter(|(_, f)| f.failures >= STOP_FAILURES_SHOWN)
            .map(|(name, f)| NeedsYou {
                kind: NeedsKind::StopFailed,
                workspace: self.desired.workspace.clone(),
                name: name.clone(),
                since: f.since,
                detail: format!("it keeps running: claude stop failed {} times; last: {}", f.failures, f.error),
                action: Some(Remedy::Attach { id: f.id.clone() }),
            })
            .collect()
    }

    /// A [`Resumed::Unknown`] resume is confirmed once the snapshot shows its own id live, and fails when
    /// it shows another live row of the name (a copy); unseen, it waits up to `settle`.
    fn confirm_resumes(&mut self, observed: &[Session], now: Millis) {
        for (name, id, at) in std::mem::take(&mut self.unconfirmed) {
            let copies: Vec<&str> =
                observed.iter().filter(|s| s.name == name && s.id != id && is_live(s, now)).map(|s| s.id.as_str()).collect();
            if !copies.is_empty() {
                self.memory.record_result(&name, Err(copy_failure(&id, &copies.join(", "))), &self.policy, now);
            } else if !observed.iter().any(|s| s.name == name && s.id == id && is_live(s, now)) && now - at < self.policy.settle {
                self.unconfirmed.push((name, id, at));
            }
        }
    }

    /// A turn seen on a minted session's single live row slides its token's expiry, unless the token
    /// already lapsed (use does not renew a lapsed token). Saved at most once a minute, as the server
    /// renews it.
    fn track_activity(&mut self, observed: &[Session], now: Millis) {
        let window = self.policy.token_window;
        let mut changed = false;
        for w in &self.desired.sessions {
            let Some(m) = self.minted.get_mut(&w.name) else {
                continue;
            };
            let mut rows = observed.iter().filter(|s| s.name == w.name && is_live(s, now));
            let (Some(row), None) = (rows.next(), rows.next()) else {
                continue;
            };
            let due = match m.last_active {
                None => true,
                Some(t) => mid_turn(row, now) && now - t <= window && now - t >= 60 * SECOND,
            };
            if due {
                m.last_active = Some(now);
                changed = true;
            }
        }
        if changed {
            self.save_minted();
        }
    }

    /// [`NeedsKind::TokenExpired`] for each wanted, minted session whose single live row had no turn
    /// within the token window, except those stopped for a Re-mint this tick.
    fn expired_tokens(&self, observed: &[Session], reminting: &[String], now: Millis) -> Vec<NeedsYou> {
        if !self.desired.started {
            return Vec::new();
        }
        let window = self.policy.token_window;
        let mut out = Vec::new();
        for w in &self.desired.sessions {
            let Some(last) = self.minted.get(&w.name).and_then(|m| m.last_active) else {
                continue;
            };
            if now - last <= window || reminting.contains(&w.name) {
                continue;
            }
            let mut rows = observed.iter().filter(|s| s.name == w.name && is_live(s, now));
            let (Some(row), None) = (rows.next(), rows.next()) else {
                continue;
            };
            out.push(NeedsYou {
                kind: NeedsKind::TokenExpired,
                workspace: self.desired.workspace.clone(),
                name: w.name.clone(),
                since: last + window,
                detail: format!(
                    "no turn for {} h: its WorkState token lapsed after {} h idle, so its next WorkState call gets HTTP 401; \
                     Re-mint stops it and resumes it with a fresh token",
                    (now - last) / (3_600 * SECOND),
                    window / (3_600 * SECOND)
                ),
                action: Some(Remedy::Remint { id: row.id.clone() }),
            });
        }
        out
    }

    /// [`NeedsKind::OtherServer`] for each wanted, minted session whose single live row holds a token
    /// from another server than the signed-in one, except those stopped for a Re-mint this tick.
    fn other_server_tokens(&self, observed: &[Session], reminting: &[String], now: Millis) -> Vec<NeedsYou> {
        let Some(origin) = self.origin.as_deref().filter(|_| self.desired.started) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for (name, from) in self.minted.elsewhere(origin) {
            if reminting.contains(&name) || !self.desired.sessions.iter().any(|w| w.name == name) {
                continue;
            }
            let mut rows = observed.iter().filter(|s| s.name == name && is_live(s, now));
            let (Some(row), None) = (rows.next(), rows.next()) else {
                continue;
            };
            let from = from.unwrap_or_else(|| "an unknown server".into());
            out.push(NeedsYou {
                kind: NeedsKind::OtherServer,
                workspace: self.desired.workspace.clone(),
                name,
                since: now,
                detail: format!(
                    "its WorkState token is from {from}, and Foreman uses {origin}; Re-mint stops it and resumes it with a token from {origin}"
                ),
                action: Some(Remedy::Remint { id: row.id.clone() }),
            });
        }
        out
    }

    fn save_minted(&mut self) {
        if let Err(e) = self.minted.save() {
            self.notices.push(format!("Cannot save the minted WorkState sessions of {}: {e}", self.desired.workspace));
        }
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

/// From this many failed `claude stop`s in a row, the name gets a [`NeedsKind::StopFailed`] row.
pub const STOP_FAILURES_SHOWN: u32 = 3;

/// This tick's `claude stop`s by name: the row id and how it went.
type Stops = BTreeMap<String, (String, Result<(), String>)>;

#[derive(Debug, Clone, PartialEq, Eq)]
struct StopFailure {
    id: String,
    failures: u32,
    since: Millis,
    error: String,
}

/// What a `--bg --resume` printed it woke.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Resumed {
    InPlace,
    /// A session the daemon still counts as running starts a copy under a new id instead (m2/4).
    Copy(String),
    /// No `backgrounded · <id8>`: [`Supervisor::confirm_resumes`] asks the next snapshot.
    Unknown,
}

fn resumed(out: &str, id: &str) -> Resumed {
    match out.split_once("backgrounded · ").and_then(|(_, rest)| rest.split_whitespace().next()) {
        Some(woke) if woke == id => Resumed::InPlace,
        Some(woke) if woke.len() == id.len() && woke.chars().all(|c| c.is_ascii_hexdigit()) => Resumed::Copy(woke.to_string()),
        _ => Resumed::Unknown,
    }
}

fn copy_failure(id: &str, copy: &str) -> Failure {
    Failure::Other(format!("the resume of {id} started a copy {copy} (the daemon still counted it running)"))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use foreman_claude::{ClaudeError, Kind, SessionState, Status};

    use super::*;
    use crate::{MintError, Minted, NeedsKind, Remedy, SECOND, tick_all};

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
        /// Popped per stop; empty means success.
        stop_errors: RefCell<Vec<ClaudeError>>,
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
            self.stop_errors.borrow_mut().pop().map_or(Ok(String::new()), Err)
        }
    }

    /// The same bearer for every role, with the WorkState role the app maps it to; `.1` an error instead.
    struct FakeTokens(Option<RoleToken>, Option<String>);

    impl TokenProvider for FakeTokens {
        fn token(&self, role: &str) -> Result<Option<RoleToken>, String> {
            if let Some(e) = &self.1 {
                return Err(e.clone());
            }
            let Some(mut t) = self.0.clone() else { return Ok(None) };
            t.role = match role {
                "workspace" => "controller".into(),
                "repo" => "orchestrator".into(),
                other => other.into(),
            };
            Ok(Some(t))
        }
    }

    const STATIC: &str = "fake-static-role-token";

    fn static_token() -> Option<RoleToken> {
        Some(RoleToken {
            url: "http://127.0.0.1:5290/mcp".into(),
            token: STATIC.into(),
            role: String::new(),
        })
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum MintCall {
        Mint { base: String, bearer: String, request: MintRequest },
        Bind { bearer: String, session_id: String, claude_session_id: String },
        Revoke { bearer: String, session_id: String },
    }

    #[derive(Default)]
    struct MintLog {
        calls: Vec<MintCall>,
        /// Popped per mint; empty means success.
        mint_errors: Vec<MintError>,
        revoke_error: Option<MintError>,
        /// One WorkState row per name, as `replace` keeps it.
        rows: Vec<String>,
        minted: u32,
    }

    /// Never touches the network: every supervisor in these tests has one.
    #[derive(Clone, Default)]
    struct FakeMinter(std::sync::Arc<std::sync::Mutex<MintLog>>);

    impl FakeMinter {
        fn take(&self) -> Vec<MintCall> {
            std::mem::take(&mut self.0.lock().unwrap().calls)
        }
    }

    impl Minter for FakeMinter {
        fn mint(&self, base: &str, bearer: &str, request: &MintRequest) -> Result<Minted, MintError> {
            let mut log = self.0.lock().unwrap();
            log.calls.push(MintCall::Mint { base: base.into(), bearer: bearer.into(), request: request.clone() });
            if let Some(e) = log.mint_errors.pop() {
                return Err(e);
            }
            let row = match log.rows.iter().position(|n| n == &request.name) {
                Some(i) => i,
                None => {
                    log.rows.push(request.name.clone());
                    log.rows.len() - 1
                }
            };
            log.minted += 1;
            Ok(Minted {
                session_id: format!("ws-{row}"),
                token: format!("wss_fake{}", log.minted),
                expires_at: None,
            })
        }

        fn bind(&self, _base: &str, bearer: &str, session_id: &str, claude_session_id: &str) -> Result<(), MintError> {
            self.0.lock().unwrap().calls.push(MintCall::Bind {
                bearer: bearer.into(),
                session_id: session_id.into(),
                claude_session_id: claude_session_id.into(),
            });
            Ok(())
        }

        fn revoke(&self, _base: &str, bearer: &str, session_id: &str) -> Result<(), MintError> {
            let mut log = self.0.lock().unwrap();
            log.calls.push(MintCall::Revoke { bearer: bearer.into(), session_id: session_id.into() });
            log.revoke_error.clone().map_or(Ok(()), Err)
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
        Supervisor::new(desired, config, FakeLauncher::default(), FakeTokens(tokens, None))
            .with_minter(FakeMinter::default())
            .with_repo_of(fake_repo_of)
            .with_daemon_check(|| None)
    }

    /// Alpha's checkout has an origin, Beta's none; any other cwd is not a repo.
    fn fake_repo_of(dir: &Path) -> Option<MintRepo> {
        match dir.file_name()?.to_str()? {
            "Alpha" => Some(MintRepo { root: "C:/demo/Alpha".into(), remote: Some("https://example.invalid/demo/alpha.git".into()) }),
            "Beta" => Some(MintRepo { root: "C:/demo/Beta".into(), remote: None }),
            _ => None,
        }
    }

    /// [`supervisor`] with the static role token, and a handle on its fake minter.
    fn minting(started: bool, mcp_dir: &Path) -> (Supervisor<FakeLauncher, FakeTokens>, FakeMinter) {
        let minter = FakeMinter::default();
        (supervisor(started, static_token(), mcp_dir).with_minter(minter.clone()), minter)
    }

    fn config_text(mcp_dir: &Path, name: &str) -> String {
        std::fs::read_to_string(mcp_dir.join(format!("workstate-{name}.json"))).unwrap()
    }

    fn launch_argv(calls: &[Call]) -> String {
        match calls {
            [Call::Launch { argv, .. }] => argv.join(" "),
            other => panic!("{other:?}"),
        }
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
    fn mint_launch_writes_only_the_minted_token_and_settles() {
        let dir = tempfile::tempdir().unwrap();
        let mcp = dir.path().join("mcp");
        let (mut sup, minter) = minting(true, &mcp);
        assert_eq!(sup.tick(&[], &mut free(), NOW), []);
        match sup.launcher().take().as_slice() {
            [Call::Launch { name, argv }] => {
                assert_eq!(name, "fm-demo-workspace");
                let argv = argv.join(" ");
                assert!(argv.contains("--strict-mcp-config"), "{argv}");
                assert!(argv.contains("workstate-fm-demo-workspace.json"), "{argv}");
                assert!(argv.contains(r"--plugin-dir C:\fm\plugins\foreman"), "{argv}");
                assert!(argv.ends_with("-- be the controller"), "{argv}");
                assert!(!argv.contains(STATIC) && !argv.contains("wss_"), "no token in argv: {argv}");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            minter.take(),
            [MintCall::Mint {
                base: "http://127.0.0.1:5290".into(),
                bearer: STATIC.into(),
                request: MintRequest {
                    name: "fm-demo-workspace".into(),
                    role: "controller".into(),
                    launched_by: "foreman/demo".into(),
                    origin: "foreman/demo".into(),
                    cwd: r"C:\demo\cwd".into(),
                    external_role: "workspace".into(),
                    replace: true,
                    repo: None,
                    claude_session_id: None,
                },
            }]
        );
        let text = config_text(&mcp, "fm-demo-workspace");
        assert!(!text.contains(STATIC), "the static token never reaches the file: {text}");
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["mcpServers"]["workstate"]["url"], "http://127.0.0.1:5290/mcp");
        assert_eq!(v["mcpServers"]["workstate"]["headers"], serde_json::json!({ "Authorization": "Bearer wss_fake1" }));
        assert_eq!(sup.minted().get("fm-demo-workspace").unwrap().session_id, "ws-0");
        assert_eq!(sup.tick(&[], &mut free(), NOW + 5 * SECOND), []);
        assert_eq!(sup.launcher().take(), [], "the row has not shown yet: no second launch");
        let live = row(SessionState::Working, Some(Status::Busy), Some(4242));
        sup.tick(std::slice::from_ref(&live), &mut free(), NOW + 61 * SECOND);
        assert_eq!(sup.launcher().take(), []);
        assert_eq!(sup.memory.get("fm-demo-workspace").unwrap().started_at, None, "stable");
    }

    fn alpha() -> WantedSession {
        WantedSession {
            name: "fm-demo-alpha".into(),
            role: "repo".into(),
            cwd: r"C:\demo\Alpha".into(),
            repo: Some("Alpha".into()),
            prompt: "run the queue".into(),
            permission_mode: "auto".into(),
            add_dirs: Vec::new(),
            resume_prompt: None,
        }
    }

    #[test]
    fn mint_each_session_gets_its_own_token_with_its_identity_in_the_mint() {
        let dir = tempfile::tempdir().unwrap();
        let mcp = dir.path().join("mcp");
        let (mut sup, minter) = minting(true, &mcp);
        sup.desired.sessions.push(alpha());
        sup.tick(&[], &mut free(), NOW);
        assert_eq!(sup.launcher().take().len(), 2);
        let requests: Vec<MintRequest> = minter
            .take()
            .into_iter()
            .map(|c| match c {
                MintCall::Mint { request, .. } => request,
                other => panic!("{other:?}"),
            })
            .collect();
        let repo = &requests[1];
        assert_eq!(
            (repo.name.as_str(), repo.role.as_str(), repo.external_role.as_str()),
            ("fm-demo-alpha", "orchestrator", "repo")
        );
        assert_eq!((repo.launched_by.as_str(), repo.origin.as_str(), repo.cwd.as_str()), ("foreman/demo", "foreman/demo/Alpha", r"C:\demo\Alpha"));
        assert_eq!((requests[0].role.as_str(), requests[0].origin.as_str()), ("controller", "foreman/demo"));
        assert_eq!(
            repo.repo,
            Some(MintRepo { root: "C:/demo/Alpha".into(), remote: Some("https://example.invalid/demo/alpha.git".into()) }),
            "a repo session is minted with its repo"
        );
        assert_eq!(requests[0].repo, None, "the workspace root is not a repo");

        let ws = config_text(&mcp, "fm-demo-workspace");
        let repo = config_text(&mcp, "fm-demo-alpha");
        assert!(ws.contains("wss_fake1") && repo.contains("wss_fake2"), "{ws} {repo}");
        assert!(!ws.contains("X-WorkState-") && !repo.contains("X-WorkState-"), "identity is on the row, not in headers: {repo}");
    }

    #[test]
    fn mint_relaunch_and_resume_replace_the_token_on_the_same_session() {
        let dir = tempfile::tempdir().unwrap();
        let mcp = dir.path().join("mcp");
        let (mut sup, minter) = minting(true, &mcp);
        sup.tick(&[], &mut free(), NOW);
        sup.launcher().take();
        minter.take();
        sup.tick(&[row(SessionState::Working, Some(Status::Busy), Some(1))], &mut free(), NOW + 61 * SECOND);
        minter.take();
        // the session died and its row is stopped: resumed with a fresh token in the same file
        let stopped = row(SessionState::Stopped, None, None);
        sup.tick(std::slice::from_ref(&stopped), &mut free(), NOW + 100 * SECOND);
        assert!(matches!(sup.launcher().take().as_slice(), [Call::Resume { .. }]));
        match minter.take().as_slice() {
            [MintCall::Mint { request, bearer, .. }] => {
                assert!(request.replace);
                assert_eq!(bearer, STATIC);
                assert_eq!(request.claude_session_id, Some(stopped.session_id.clone()), "the uuid --bg --resume keeps");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(sup.minted().get("fm-demo-workspace").unwrap().session_id, "ws-0", "same WorkState row");
        let text = config_text(&mcp, "fm-demo-workspace");
        assert!(text.contains("wss_fake2") && !text.contains("wss_fake1") && !text.contains(STATIC), "{text}");
    }

    #[test]
    fn mint_on_a_new_server_stops_and_resumes_every_minted_session_with_its_new_url() {
        let dir = tempfile::tempdir().unwrap();
        let mcp = dir.path().join("mcp");
        let (mut sup, minter) = minting(true, &mcp);
        sup.tick(&[], &mut free(), NOW);
        sup.launcher().take();
        minter.take();
        let live = row(SessionState::Working, Some(Status::Busy), Some(1));
        sup.tick(std::slice::from_ref(&live), &mut free(), NOW + 61 * SECOND);
        sup.launcher().take();
        minter.take();

        assert!(sup.server_changed(Some("http://127.0.0.1:5290")).is_empty(), "minted there");
        sup.tokens_mut().0.as_mut().unwrap().url = "https://workstate.example/mcp".into();
        assert!(sup.server_changed(None).is_empty(), "not signed in: nothing to mint with");
        assert_eq!(
            sup.server_changed(Some("https://workstate.example")),
            [("fm-demo-workspace".to_string(), Some("http://127.0.0.1:5290".to_string()))]
        );
        assert!(sup.server_changed(Some("https://workstate.example")).is_empty(), "once");
        sup.tick(std::slice::from_ref(&live), &mut free(), NOW + 70 * SECOND);
        assert!(matches!(sup.launcher().take().as_slice(), [Call::Stop { .. }]), "the live row is stopped");
        let stopped = row(SessionState::Stopped, None, None);
        sup.tick(std::slice::from_ref(&stopped), &mut free(), NOW + 130 * SECOND);
        let calls = sup.launcher().take();
        assert!(matches!(calls.as_slice(), [Call::Resume { session_id, .. }] if *session_id == stopped.session_id), "{calls:?}");
        match minter.take().as_slice() {
            [MintCall::Mint { base, request, .. }] => {
                assert_eq!(base, "https://workstate.example");
                assert!(request.replace);
                assert_eq!(request.claude_session_id, Some(stopped.session_id.clone()));
            }
            other => panic!("{other:?}"),
        }
        let text = config_text(&mcp, "fm-demo-workspace");
        assert!(text.contains("https://workstate.example/mcp") && text.contains("wss_fake2"), "{text}");
    }

    /// A supervisor of `home` whose sessions file holds `json`, with the static role token.
    fn minting_from(home: &Path, json: &str) -> (Supervisor<FakeLauncher, FakeTokens>, FakeMinter) {
        let path = MintedSessions::path(home, "demo");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, json).unwrap();
        let mut desired = DesiredState::workspace_controller("demo", r"C:\demo\cwd", "be the controller");
        desired.started = true;
        let config = LaunchConfig { mcp_dir: home.join("mcp"), foreman_home: Some(home.into()), ..LaunchConfig::default() };
        let minter = FakeMinter::default();
        let sup = Supervisor::new(desired, config, FakeLauncher::default(), FakeTokens(static_token(), None))
            .with_minter(minter.clone())
            .with_repo_of(fake_repo_of)
            .with_daemon_check(|| None);
        (sup, minter)
    }

    #[test]
    fn mint_of_unknown_origin_is_reminted_once_unless_its_config_names_the_server() {
        const OLD: &str = r#"{"fm-demo-workspace":{"sessionId":"ws-0","role":"workspace","lastActive":1800000000000}}"#;
        let live = row(SessionState::Working, Some(Status::Busy), Some(1));
        let dir = tempfile::tempdir().unwrap();
        let (mut sup, minter) = minting_from(dir.path(), OLD);
        assert_eq!(sup.server_changed(Some("http://127.0.0.1:5290")), [("fm-demo-workspace".to_string(), None)], "no config file: unknown");
        assert!(sup.server_changed(Some("http://127.0.0.1:5290")).is_empty(), "once");
        sup.tick(std::slice::from_ref(&live), &mut free(), NOW + 10 * SECOND);
        assert!(matches!(sup.launcher().take().as_slice(), [Call::Stop { .. }]));
        minter.take();
        let stopped = row(SessionState::Stopped, None, None);
        sup.tick(std::slice::from_ref(&stopped), &mut free(), NOW + 70 * SECOND);
        assert!(matches!(sup.launcher().take().as_slice(), [Call::Resume { .. }]));
        let calls = minter.take();
        assert!(matches!(calls.as_slice(), [MintCall::Mint { .. }]), "{calls:?}");
        assert_eq!(sup.minted().get("fm-demo-workspace").unwrap().origin.as_deref(), Some("http://127.0.0.1:5290"), "recorded now");

        let dir = tempfile::tempdir().unwrap();
        foreman_claude::write_mcp_config(&dir.path().join("mcp"), "fm-demo-workspace", "http://127.0.0.1:5290/mcp", "wss_old", &[]).unwrap();
        let (mut sup, _) = minting_from(dir.path(), OLD);
        assert!(sup.server_changed(Some("http://127.0.0.1:5290")).is_empty(), "its config file already names the server");
        sup.tick(std::slice::from_ref(&live), &mut free(), NOW + 10 * SECOND);
        assert_eq!(sup.launcher().take(), []);
        let saved = std::fs::read_to_string(MintedSessions::path(dir.path(), "demo")).unwrap();
        assert!(saved.contains(r#""origin": "http://127.0.0.1:5290""#), "{saved}");
    }

    #[test]
    fn mint_on_another_server_offers_re_mint_and_is_never_revoked_or_patched_on_this_one() {
        let dir = tempfile::tempdir().unwrap();
        let mcp = dir.path().join("mcp");
        let (mut sup, minter) = minting(true, &mcp);
        sup.tick(&[], &mut free(), NOW);
        sup.launcher().take();
        minter.take();
        sup.tokens_mut().0.as_mut().unwrap().url = "https://workstate.example/mcp".into();
        // checked while its row was not live: nothing to stop, and then it shows live with the old token
        assert_eq!(sup.server_changed(Some("https://workstate.example")).len(), 1);
        sup.tick(&[], &mut free(), NOW + SECOND);
        sup.launcher().take();
        let live = row(SessionState::Working, Some(Status::Busy), Some(1));
        let needs = sup.tick(std::slice::from_ref(&live), &mut free(), NOW + 61 * SECOND);
        let other: Vec<_> = needs.iter().filter(|n| n.kind == NeedsKind::OtherServer).collect();
        let [need] = other.as_slice() else { panic!("{needs:?}") };
        assert_eq!(need.action, Some(Remedy::Remint { id: live.id.clone() }));
        assert!(need.detail.contains("http://127.0.0.1:5290") && need.detail.contains("https://workstate.example"), "{}", need.detail);
        assert!(minter.take().is_empty(), "no PATCH of the old server's session on the new one");

        sup.retry("fm-demo-workspace");
        sup.tick(std::slice::from_ref(&live), &mut free(), NOW + 62 * SECOND);
        assert!(matches!(sup.launcher().take().as_slice(), [Call::Stop { .. }]), "Re-mint stops it");

        sup.desired.started = false;
        sup.tick(&[], &mut free(), NOW + 63 * SECOND);
        assert!(minter.take().is_empty(), "its server's grant revoke covers it, not a DELETE here");
        assert!(sup.minted().get("fm-demo-workspace").is_none());
        assert!(!mcp.join("workstate-fm-demo-workspace.json").exists());
    }

    #[test]
    fn mint_without_a_bearer_now_fails_the_launch_with_why() {
        let dir = tempfile::tempdir().unwrap();
        let (mut sup, minter) = minting(true, dir.path());
        sup.tokens_mut().1 = Some("sign in to WorkState at http://127.0.0.1:5290 again: the server refused the refresh (invalid_grant)".into());
        let needs = sup.tick(&[], &mut free(), NOW);
        assert!(sup.launcher().take().is_empty(), "nothing launched without WorkState");
        assert!(minter.take().is_empty());
        assert!(
            needs.iter().any(|n| n.kind == NeedsKind::LaunchFailed && n.detail.contains("sign in to WorkState") && n.detail.contains("again")),
            "{needs:?}"
        );
    }

    #[test]
    fn mint_patches_the_claude_session_once_per_mint() {
        let dir = tempfile::tempdir().unwrap();
        let (mut sup, minter) = minting(true, dir.path());
        sup.tick(&[], &mut free(), NOW);
        minter.take();
        let live = row(SessionState::Working, Some(Status::Busy), Some(4242));
        sup.tick(std::slice::from_ref(&live), &mut free(), NOW + 5 * SECOND);
        assert_eq!(
            minter.take(),
            [MintCall::Bind {
                bearer: STATIC.into(),
                session_id: "ws-0".into(),
                claude_session_id: "aaaa0001-0000-4000-8000-000000000001".into(),
            }]
        );
        sup.tick(std::slice::from_ref(&live), &mut free(), NOW + 10 * SECOND);
        sup.tick(std::slice::from_ref(&live), &mut free(), NOW + 70 * SECOND);
        assert_eq!(minter.take(), [], "once");
        assert_eq!(
            sup.minted().get("fm-demo-workspace").unwrap().claude_session_id.as_deref(),
            Some("aaaa0001-0000-4000-8000-000000000001")
        );
        // a resume mints again with the uuid in its body, so there is nothing left to PATCH
        sup.tick(&[row(SessionState::Stopped, None, None)], &mut free(), NOW + 200 * SECOND);
        sup.tick(std::slice::from_ref(&live), &mut free(), NOW + 205 * SECOND);
        let calls = minter.take();
        match calls.as_slice() {
            [MintCall::Mint { request, .. }] => assert_eq!(request.claude_session_id.as_deref(), Some("aaaa0001-0000-4000-8000-000000000001")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn mint_stop_revokes_the_token_and_deletes_the_config() {
        let dir = tempfile::tempdir().unwrap();
        let (mut sup, minter) = minting(true, dir.path());
        sup.tick(&[], &mut free(), NOW);
        minter.take();
        let live = row(SessionState::Working, Some(Status::Busy), Some(1));
        sup.tick(std::slice::from_ref(&live), &mut free(), NOW + SECOND);
        minter.take();
        assert!(dir.path().join("workstate-fm-demo-workspace.json").is_file());
        sup.desired.started = false;
        sup.tick(std::slice::from_ref(&live), &mut free(), NOW + 2 * SECOND);
        assert_eq!(sup.launcher().take().last(), Some(&Call::Stop { id: "aaaa0001".into() }));
        assert_eq!(minter.take(), [MintCall::Revoke { bearer: STATIC.into(), session_id: "ws-0".into() }]);
        assert!(!dir.path().join("workstate-fm-demo-workspace.json").exists());
        assert_eq!(sup.minted().get("fm-demo-workspace"), None);
        sup.tick(std::slice::from_ref(&live), &mut free(), NOW + 3 * SECOND);
        assert_eq!(minter.take(), [], "revoked once");
        assert!(sup.take_notices().is_empty());
    }

    #[test]
    fn mint_unwant_and_give_up_revoke_too() {
        let dir = tempfile::tempdir().unwrap();
        let (mut sup, minter) = minting(true, dir.path());
        sup.desired.sessions.push(alpha());
        sup.tick(&[], &mut free(), NOW);
        minter.take();
        // `fm unwant` (R14): retired while the workspace runs
        sup.desired.unwant("fm-demo-alpha");
        sup.tick(&[], &mut free(), NOW + SECOND);
        assert_eq!(minter.take(), [MintCall::Revoke { bearer: STATIC.into(), session_id: "ws-1".into() }]);
        assert!(!dir.path().join("workstate-fm-demo-alpha.json").exists());
        assert!(dir.path().join("workstate-fm-demo-workspace.json").exists(), "the others keep theirs");
        // the workspace session dies within 20 s of each start until the supervisor gives up
        let mut now = NOW + 30 * SECOND;
        while !sup.memory.get("fm-demo-workspace").is_some_and(|a| a.gave_up) {
            sup.tick(&[], &mut free(), now);
            now += 30 * SECOND;
        }
        let revokes: Vec<MintCall> = minter.take().into_iter().filter(|c| matches!(c, MintCall::Revoke { .. })).collect();
        assert_eq!(revokes, [MintCall::Revoke { bearer: STATIC.into(), session_id: "ws-0".into() }]);
        assert!(!dir.path().join("workstate-fm-demo-workspace.json").exists());
    }

    #[test]
    fn mint_failure_fails_the_launch_without_falling_back_to_the_static_token() {
        let dir = tempfile::tempdir().unwrap();
        let (mut sup, minter) = minting(true, dir.path());
        minter.0.lock().unwrap().mint_errors.push(MintError::Refused { status: 401, reason: "unknown token".into() });
        let needs = sup.tick(&[], &mut free(), NOW);
        assert_eq!(sup.launcher().take(), [], "nothing launched");
        assert_eq!(needs.len(), 1);
        assert_eq!(needs[0].kind, NeedsKind::LaunchFailed);
        assert_eq!(needs[0].action, Some(Remedy::Retry));
        assert!(
            needs[0].detail.contains("cannot mint a WorkState token for fm-demo-workspace: WorkState answered HTTP 401: unknown token"),
            "{}",
            needs[0].detail
        );
        assert!(!needs[0].detail.contains(STATIC));
        assert!(!dir.path().join("workstate-fm-demo-workspace.json").exists(), "no file");
        // the server is back: Retry launches with a minted token
        sup.retry("fm-demo-workspace");
        sup.tick(&[], &mut free(), NOW + SECOND);
        assert!(!launch_argv(&sup.launcher().take()).contains(STATIC));
        assert!(config_text(dir.path(), "fm-demo-workspace").contains("wss_fake1"));
    }

    #[test]
    fn mint_a_failed_revoke_is_a_notice_and_the_file_still_goes() {
        let dir = tempfile::tempdir().unwrap();
        let (mut sup, minter) = minting(true, dir.path());
        sup.tick(&[], &mut free(), NOW);
        minter.0.lock().unwrap().revoke_error = Some(MintError::Unreachable { base: "http://127.0.0.1:5290".into(), reason: "refused".into() });
        sup.desired.started = false;
        sup.tick(&[], &mut free(), NOW + SECOND);
        let notices = sup.take_notices();
        assert_eq!(notices.len(), 1, "{notices:?}");
        assert!(notices[0].starts_with("Cannot revoke the WorkState token of fm-demo-workspace (session ws-0)"), "{}", notices[0]);
        assert!(!dir.path().join("workstate-fm-demo-workspace.json").exists());
    }

    #[test]
    fn mint_sessions_survive_a_restart_and_are_revoked_on_stop() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let (mut sup, minter) = minting(true, &dir.path().join("mcp"));
        sup.config.foreman_home = Some(home.clone());
        let config = sup.config.clone();
        sup.tick(&[], &mut free(), NOW);
        // no persistence without a home at construction: rebuild the way the app does
        let mut sup = Supervisor::new(sup.desired.clone(), config.clone(), FakeLauncher::default(), FakeTokens(static_token(), None))
            .with_minter(minter.clone());
        assert_eq!(sup.minted().get("fm-demo-workspace"), None, "nothing saved yet");
        sup.tick(&[], &mut free(), NOW);
        minter.take();
        let saved = std::fs::read_to_string(MintedSessions::path(&home, "demo")).unwrap();
        assert!(saved.contains("ws-0") && !saved.contains("wss_") && !saved.contains(STATIC), "{saved}");
        // the app restarts, then the workspace is stopped
        let mut desired = sup.desired.clone();
        desired.started = false;
        let mut sup = Supervisor::new(desired, config, FakeLauncher::default(), FakeTokens(static_token(), None)).with_minter(minter.clone());
        sup.tick(&[], &mut free(), NOW + SECOND);
        assert_eq!(minter.take(), [MintCall::Revoke { bearer: STATIC.into(), session_id: "ws-0".into() }]);
        assert_eq!(std::fs::read_to_string(MintedSessions::path(&home, "demo")).unwrap().trim(), "{}");
    }

    #[test]
    fn mint_a_live_session_idle_past_its_token_window_asks_for_a_remint() {
        const HOUR: Millis = 3_600 * SECOND;
        let dir = tempfile::tempdir().unwrap();
        let (mut sup, minter) = minting(true, dir.path());
        sup.tick(&[], &mut free(), NOW);
        let busy = row(SessionState::Working, Some(Status::Busy), Some(7));
        let idle = row(SessionState::Working, Some(Status::Idle), Some(7));
        sup.tick(std::slice::from_ref(&busy), &mut free(), NOW + 61 * SECOND);
        sup.launcher().take();
        minter.take();
        // a turn at +50 h slides the window: idle until +122 h is fine
        sup.tick(std::slice::from_ref(&busy), &mut free(), NOW + 50 * HOUR);
        assert_eq!(sup.tick(std::slice::from_ref(&idle), &mut free(), NOW + 122 * HOUR), []);
        let needs = sup.tick(std::slice::from_ref(&idle), &mut free(), NOW + 122 * HOUR + SECOND);
        assert_eq!(needs.len(), 1, "{needs:?}");
        assert_eq!(needs[0].kind, NeedsKind::TokenExpired);
        assert_eq!(needs[0].since, NOW + 122 * HOUR);
        assert_eq!(needs[0].action, Some(Remedy::Remint { id: "aaaa0001".into() }));
        assert!(needs[0].detail.contains("next WorkState call gets HTTP 401"), "{}", needs[0].detail);
        // a turn after the lapse does not renew it: the row stays, its since kept
        let needs = sup.tick(std::slice::from_ref(&busy), &mut free(), NOW + 130 * HOUR);
        assert_eq!((needs.len(), needs[0].since), (1, NOW + 122 * HOUR));
        assert_eq!(sup.launcher().take(), [], "nothing is done without the human");
        assert_eq!(minter.take(), []);
        // Re-mint: stopped this tick (no row for it), resumed with a fresh token after the respawn grace
        sup.retry("fm-demo-workspace");
        let t = NOW + 131 * HOUR;
        assert_eq!(sup.tick(std::slice::from_ref(&idle), &mut free(), t), []);
        assert_eq!(sup.launcher().take(), [Call::Stop { id: "aaaa0001".into() }]);
        let stopped = row(SessionState::Stopped, None, None);
        assert_eq!(sup.tick(std::slice::from_ref(&stopped), &mut free(), t + 5 * SECOND), []);
        assert_eq!(sup.launcher().take(), [], "respawn grace");
        sup.tick(std::slice::from_ref(&stopped), &mut free(), t + 31 * SECOND);
        assert!(matches!(sup.launcher().take().as_slice(), [Call::Resume { .. }]));
        assert!(matches!(minter.take().as_slice(), [MintCall::Mint { request, .. }] if request.replace));
        assert_eq!(sup.minted().get("fm-demo-workspace").unwrap().last_active, Some(t + 31 * SECOND));
        assert!(config_text(dir.path(), "fm-demo-workspace").contains("wss_fake2"));
        assert_eq!(sup.tick(std::slice::from_ref(&idle), &mut free(), t + 40 * SECOND), [], "fresh window");
        // a plain Retry elsewhere never stops a session
        sup.retry("fm-demo-workspace");
        sup.tick(std::slice::from_ref(&idle), &mut free(), t + 50 * SECOND);
        assert_eq!(sup.launcher().take(), []);
    }

    #[test]
    fn mint_no_token_row_for_a_stopped_workspace_or_an_unminted_session() {
        let dir = tempfile::tempdir().unwrap();
        let idle = row(SessionState::Working, Some(Status::Idle), Some(7));
        let later = NOW + 100 * 3_600 * SECOND;
        // no role token: nothing minted, so no window to lapse
        let mut sup = supervisor(true, None, dir.path());
        sup.tick(&[], &mut free(), NOW);
        assert_eq!(sup.tick(std::slice::from_ref(&idle), &mut free(), later), []);
        // minted, then the workspace is stopped: its session is stopped, not re-minted
        let (mut sup, _minter) = minting(true, dir.path());
        sup.tick(&[], &mut free(), NOW);
        sup.launcher().take();
        sup.desired.started = false;
        assert_eq!(sup.tick(std::slice::from_ref(&idle), &mut free(), later), []);
        assert_eq!(sup.launcher().take(), [Call::Stop { id: "aaaa0001".into() }]);
    }

    #[test]
    fn mint_nothing_without_a_role_token() {
        let dir = tempfile::tempdir().unwrap();
        let minter = FakeMinter::default();
        let mut sup = supervisor(true, None, dir.path()).with_minter(minter.clone());
        sup.tick(&[], &mut free(), NOW);
        assert_eq!(sup.launcher().take().len(), 1);
        assert_eq!(minter.take(), []);
        assert!(!dir.path().join("workstate-fm-demo-workspace.json").exists());
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
                "env": {
                    "FOREMAN_HOME": r"C:\fm\home", "FOREMAN_ROLE": "workspace", "FOREMAN_WORKSPACE": "demo",
                    "FOREMAN_SESSION": "fm-demo-workspace",
                },
                "worktree": { "bgIsolation": "none" }
            })
        );
        assert_eq!(LaunchConfig::default().session_settings("demo", "fm-demo-workspace", "workspace", None), None);
    }

    fn workspace(name: &str, mcp_dir: &Path) -> Supervisor<FakeLauncher, FakeTokens> {
        let mut desired = DesiredState::workspace_controller(name, r"C:\demo\cwd", "be the controller");
        desired.started = true;
        let config = LaunchConfig {
            mcp_dir: mcp_dir.into(),
            ..LaunchConfig::default()
        };
        Supervisor::new(desired, config, FakeLauncher::default(), FakeTokens(None, None))
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
        tick_all(sups.iter_mut(), &[], &[], 1, NOW);
        assert_eq!(launched(&sups), [1, 0]);
        // alpha's row has not shown yet: it still holds the slot, beta waits
        tick_all(sups.iter_mut(), &[], &[], 1, NOW + 2 * SECOND);
        assert_eq!(launched(&sups), [0, 0]);
        // alpha is working: beta waits
        let alpha_busy = live("fm-alpha-workspace", 1, SessionState::Working, Status::Busy);
        tick_all(sups.iter_mut(), &[], std::slice::from_ref(&alpha_busy), 1, NOW + 4 * SECOND);
        assert_eq!(launched(&sups), [0, 0]);
        // alpha's turn ended (done + live pid = idle): the slot frees, beta goes
        let alpha_idle = live("fm-alpha-workspace", 1, SessionState::Done, Status::Idle);
        tick_all(sups.iter_mut(), &[], std::slice::from_ref(&alpha_idle), 1, NOW + 30 * SECOND);
        assert_eq!(launched(&sups), [0, 1]);
        // the same snapshot again: beta, settling, holds the slot
        tick_all(sups.iter_mut(), &[], std::slice::from_ref(&alpha_idle), 1, NOW + 32 * SECOND);
        assert_eq!(launched(&sups), [0, 0]);

        // budget 2: both in one tick
        let mut sups = [workspace("alpha", dir.path()), workspace("beta", dir.path())];
        tick_all(sups.iter_mut(), &[], &[], 2, NOW);
        assert_eq!(launched(&sups), [1, 1]);

        // budget 0 pauses every launch; a working session of a registered workspace nobody here supervises counts too
        let mut sups = [workspace("alpha", dir.path()), workspace("beta", dir.path())];
        tick_all(sups.iter_mut(), &["alpha", "beta", "gamma"], &[], 0, NOW);
        assert_eq!(launched(&sups), [0, 0]);
        let other = live("fm-gamma-workspace", 3, SessionState::Working, Status::Busy);
        tick_all(sups.iter_mut(), &["alpha", "beta", "gamma"], std::slice::from_ref(&other), 1, NOW + SECOND);
        assert_eq!(launched(&sups), [0, 0]);
        let needs = tick_all(sups.iter_mut(), &["alpha", "beta", "gamma"], std::slice::from_ref(&other), 2, NOW + 2 * SECOND);
        assert_eq!(launched(&sups), [1, 0]);
        assert!(needs.is_empty(), "a cap wait is not an Inbox row: {needs:?}");
    }

    #[test]
    fn another_homes_busy_sessions_take_no_slot() {
        let dir = tempfile::tempdir().unwrap();
        let launched = |sups: &[Supervisor<FakeLauncher, FakeTokens>]| -> Vec<usize> {
            sups.iter().map(|s| s.launcher().take().len()).collect()
        };
        let foreign = [
            live("fm-nomos-workspace", 7, SessionState::Working, Status::Busy),
            live("fm-nomos-nomos", 8, SessionState::Working, Status::Busy),
        ];
        assert_eq!(Budget::new(1, &foreign, &[], NOW).free, 0, "unfiltered, they would fill the budget");

        // --selftest-supervise step f with the limit at 1 while another home is mid-turn: alpha goes, then beta
        let mut sups = [workspace("alpha", dir.path()), workspace("beta", dir.path())];
        tick_all(sups.iter_mut(), &["alpha", "beta"], &foreign, 1, NOW);
        assert_eq!(launched(&sups), [1, 0]);
        let mut rows = foreign.to_vec();
        rows.push(live("fm-alpha-workspace", 1, SessionState::Working, Status::Busy));
        tick_all(sups.iter_mut(), &["alpha", "beta"], &rows, 1, NOW + 30 * SECOND);
        assert_eq!(launched(&sups), [0, 0], "alpha's own busy row holds the slot");
        rows.pop();
        rows.push(live("fm-alpha-workspace", 1, SessionState::Done, Status::Idle));
        tick_all(sups.iter_mut(), &["alpha", "beta"], &rows, 1, NOW + 60 * SECOND);
        assert_eq!(launched(&sups), [0, 1]);

        // a session this home wants under another workspace's name is its own
        let mut sups = [workspace("alpha", dir.path()), workspace("beta", dir.path())];
        sups[1].desired.sessions[0].name = "fm-nomos-workspace".into();
        tick_all(sups.iter_mut(), &["alpha", "beta"], &foreign[..1], 1, NOW);
        assert_eq!(launched(&sups), [0, 0]);
    }

    #[test]
    fn a_workspace_session_idling_on_the_relay_frees_its_slot() {
        let dir = tempfile::tempdir().unwrap();
        let mut sups = [workspace("alpha", dir.path()), workspace("beta", dir.path())];
        tick_all(sups.iter_mut(), &[], &[], 1, NOW);
        assert_eq!(sups.iter().map(|s| s.launcher().take().len()).collect::<Vec<_>>(), [1, 0]);
        // just started, no prompt taken yet: `status=idle` on a live pid still holds the slot while settling
        let fresh = live("fm-alpha-workspace", 1, SessionState::Working, Status::Idle);
        tick_all(sups.iter_mut(), &[], std::slice::from_ref(&fresh), 1, NOW + 2 * SECOND);
        assert_eq!(sups.iter().map(|s| s.launcher().take().len()).collect::<Vec<_>>(), [0, 0]);
        // its turn ended, `state` stayed `working`: the slot is free once it settled
        tick_all(sups.iter_mut(), &[], std::slice::from_ref(&fresh), 1, NOW + 30 * SECOND);
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
        tick_all(sups.iter_mut(), &[], &[], 1, NOW);
        let first: Vec<Vec<Call>> = sups.iter().map(|s| s.launcher().take()).collect();
        assert_eq!(first[0].len(), 2, "alpha: relay and workspace {first:?}");
        assert_eq!(first[1].len(), 1, "beta: relay only {first:?}");
        let rows = [
            live("fm-alpha-relay", 1, SessionState::Working, Status::Busy),
            live("fm-beta-relay", 2, SessionState::Working, Status::Busy),
            live("fm-alpha-workspace", 3, SessionState::Done, Status::Idle),
        ];
        tick_all(sups.iter_mut(), &[], &rows, 1, NOW + 30 * SECOND);
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

    fn unreachable() -> ClaudeError {
        ClaudeError::DaemonUnreachable {
            code: Some(1),
            stderr: "Starting background service… Couldn't reach the background service (background service did not become reachable within 45s) — run 'claude daemon status'".into(),
        }
    }

    /// An unelevated Foreman facing an elevated service gets one row saying why, at once, and
    /// no more launches — not five 45 s failures.
    #[test]
    fn an_unreachable_service_foreman_cannot_reach_holds_with_one_row() {
        let dir = tempfile::tempdir().unwrap();
        let mut sup = supervisor(true, None, dir.path()).with_daemon_check(|| Some("runs as administrator".into()));
        sup.launcher.errors.borrow_mut().push(unreachable());
        let needs = sup.tick(&[], &mut free(), NOW);
        assert_eq!(sup.launcher().take().len(), 1);
        assert_eq!(needs.len(), 1, "{needs:?}");
        assert_eq!((needs[0].kind, needs[0].detail.as_str(), needs[0].since), (NeedsKind::LaunchFailed, "runs as administrator", NOW));
        assert_eq!(needs[0].action, Some(Remedy::RestartElevated));
        assert_eq!(sup.memory.get("fm-demo-workspace").map(|a| a.failures).unwrap_or(0), 0, "not a counted failure");
        for t in 1..600 {
            assert_eq!(sup.tick(&[], &mut free(), NOW + t * SECOND).len(), 1);
        }
        assert_eq!(sup.launcher().take(), [], "held: nothing launched");
        // the app's check finds it reachable again
        sup.hold(None, NOW + 600 * SECOND);
        assert_eq!(sup.tick(&[], &mut free(), NOW + 601 * SECOND), []);
        assert_eq!(sup.launcher().take().len(), 1);
    }

    /// The app's check before any launch: held from the start, never launched.
    #[test]
    fn a_hold_from_the_apps_check_comes_before_any_launch() {
        let dir = tempfile::tempdir().unwrap();
        let mut sup = supervisor(true, None, dir.path());
        sup.hold(Some("runs as administrator".into()), NOW);
        let needs = sup.tick(&[], &mut free(), NOW);
        assert_eq!(sup.launcher().take(), []);
        assert_eq!(needs.len(), 1);
        assert_eq!(needs[0].detail, "runs as administrator");
        sup.retry(&needs[0].name);
        assert_eq!(sup.held(), None, "Retry tries again");
        sup.tick(&[], &mut free(), NOW + SECOND);
        assert_eq!(sup.launcher().take().len(), 1);
    }

    /// Unreachable for another reason (the check says elevation is not why): counted and backed off.
    #[test]
    fn an_unreachable_service_with_no_elevation_mismatch_backs_off_as_before() {
        let dir = tempfile::tempdir().unwrap();
        let mut sup = supervisor(true, None, dir.path()).with_daemon_check(|| None);
        sup.launcher.errors.borrow_mut().push(unreachable());
        let needs = sup.tick(&[], &mut free(), NOW);
        assert_eq!(sup.held(), None);
        assert_eq!(sup.memory.get("fm-demo-workspace").unwrap().failures, 1);
        assert!(needs[0].detail.starts_with("failure 1 of 5: claude failed (exit 1): Starting background service…"), "{needs:?}");
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
        assert_eq!(resumed("backgrounded · aaaa0001 · fm-demo-workspace", "aaaa0001"), Resumed::InPlace);
        assert_eq!(resumed("backgrounded · bbbb0002 · fm-demo-workspace", "aaaa0001"), Resumed::Copy("bbbb0002".into()));
        assert_eq!(resumed("", "aaaa0001"), Resumed::Unknown);
        assert_eq!(resumed("backgrounded · resumed fm-demo-workspace", "aaaa0001"), Resumed::Unknown);
        assert_eq!(resumed("Session resumed in the background", "aaaa0001"), Resumed::Unknown);
    }

    #[test]
    fn a_resume_with_unrecognised_output_is_settled_by_the_next_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let name = "fm-demo-workspace";
        let stopped = row(SessionState::Stopped, None, None);
        let mut copy = row(SessionState::Working, Some(Status::Busy), Some(9));
        copy.id = "bbbb0002".into();
        let woke = row(SessionState::Working, Some(Status::Busy), Some(9));

        let mut sup = supervisor(true, None, dir.path());
        *sup.launcher.output.borrow_mut() = Some("Resumed in the background".into());
        assert_eq!(sup.tick(std::slice::from_ref(&stopped), &mut free(), NOW), []);
        assert!(matches!(sup.launcher().take().as_slice(), [Call::Resume { .. }]));
        let notices = sup.take_notices();
        assert!(notices.len() == 1 && notices[0].contains("Resumed in the background") && notices[0].contains("aaaa0001"), "{notices:?}");
        sup.tick(std::slice::from_ref(&copy), &mut free(), NOW + 2 * SECOND);
        let a = sup.memory.get(name).unwrap();
        assert_eq!((a.failures, a.started_at), (1, None));
        assert_eq!(a.last_error.as_deref(), Some("the resume of aaaa0001 started a copy bbbb0002 (the daemon still counted it running)"));

        let mut sup = supervisor(true, None, dir.path());
        *sup.launcher.output.borrow_mut() = Some(String::new());
        sup.tick(std::slice::from_ref(&stopped), &mut free(), NOW);
        sup.tick(&[], &mut free(), NOW + SECOND);
        sup.tick(std::slice::from_ref(&woke), &mut free(), NOW + 2 * SECOND);
        sup.tick(std::slice::from_ref(&copy), &mut free(), NOW + 3 * SECOND);
        assert_eq!(sup.memory.get(name).unwrap().failures, 0, "confirmed by its own id: a later row is not this resume's");
    }

    #[test]
    fn a_stop_that_keeps_failing_is_an_inbox_row_until_it_stops() {
        let dir = tempfile::tempdir().unwrap();
        let mut sup = supervisor(false, None, dir.path());
        let live = row(SessionState::Working, Some(Status::Busy), Some(1));
        let fail = || ClaudeError::CliFailed { code: Some(1), stderr: "boom".into() };
        sup.launcher.stop_errors.borrow_mut().extend([fail(), fail(), fail(), fail()]);
        for t in 0..2 {
            assert_eq!(sup.tick(std::slice::from_ref(&live), &mut free(), NOW + t * SECOND), [], "not after {}", t + 1);
        }
        let needs = sup.tick(std::slice::from_ref(&live), &mut free(), NOW + 2 * SECOND);
        assert_eq!(needs.len(), 1, "{needs:?}");
        assert_eq!((needs[0].kind, needs[0].since), (NeedsKind::StopFailed, NOW));
        assert_eq!(needs[0].action, Some(Remedy::Attach { id: "aaaa0001".into() }));
        assert_eq!(needs[0].detail, "it keeps running: claude stop failed 3 times; last: claude failed (exit 1): boom");
        let needs = sup.tick(std::slice::from_ref(&live), &mut free(), NOW + 3 * SECOND);
        assert_eq!((needs.len(), needs[0].since), (1, NOW), "since kept");
        assert_eq!(sup.launcher().take(), vec![Call::Stop { id: "aaaa0001".into() }; 4], "stop, never rm, every tick");
        assert_eq!(sup.tick(std::slice::from_ref(&live), &mut free(), NOW + 4 * SECOND), [], "stopped");
        assert_eq!(sup.tick(&[row(SessionState::Stopped, None, None)], &mut free(), NOW + 5 * SECOND), []);
    }

    #[test]
    fn a_remint_whose_stop_fails_is_tried_again() {
        let dir = tempfile::tempdir().unwrap();
        let mcp = dir.path().join("mcp");
        let (mut sup, minter) = minting(true, &mcp);
        sup.tick(&[], &mut free(), NOW);
        let live = row(SessionState::Working, Some(Status::Busy), Some(1));
        sup.tick(std::slice::from_ref(&live), &mut free(), NOW + 61 * SECOND);
        sup.launcher().take();
        minter.take();
        sup.tokens_mut().0.as_mut().unwrap().url = "https://workstate.example/mcp".into();
        assert_eq!(sup.server_changed(Some("https://workstate.example")).len(), 1);
        sup.launcher.stop_errors.borrow_mut().push(ClaudeError::CliFailed { code: Some(1), stderr: "busy".into() });
        let needs = sup.tick(std::slice::from_ref(&live), &mut free(), NOW + 70 * SECOND);
        assert_eq!(sup.launcher().take(), [Call::Stop { id: "aaaa0001".into() }]);
        assert!(needs.iter().any(|n| n.kind == NeedsKind::OtherServer), "still on the other server: {needs:?}");
        assert!(sup.server_changed(Some("https://workstate.example")).is_empty(), "not scheduled twice");
        sup.tick(std::slice::from_ref(&live), &mut free(), NOW + 71 * SECOND);
        assert_eq!(sup.launcher().take(), [Call::Stop { id: "aaaa0001".into() }], "tried again");
        sup.tick(std::slice::from_ref(&live), &mut free(), NOW + 72 * SECOND);
        assert_eq!(sup.launcher().take(), [], "stopped: done");
        let stopped = row(SessionState::Stopped, None, None);
        sup.tick(std::slice::from_ref(&stopped), &mut free(), NOW + 130 * SECOND);
        assert!(matches!(sup.launcher().take().as_slice(), [Call::Resume { .. }]));
        assert!(matches!(minter.take().as_slice(), [MintCall::Mint { base, .. }] if base == "https://workstate.example"));
    }

    #[test]
    fn a_remint_without_a_live_row_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let (mut sup, _minter) = minting(true, dir.path());
        sup.remint_due.push("fm-demo-workspace".into());
        sup.remint_due.push("fm-demo-gone".into());
        let mut gone = row(SessionState::Working, Some(Status::Busy), Some(2));
        gone.name = "fm-demo-gone".into();
        sup.tick(&[row(SessionState::Stopped, None, None), gone], &mut free(), NOW);
        assert!(sup.remint_due.is_empty(), "{:?}", sup.remint_due);
        assert!(!sup.launcher().take().iter().any(|c| matches!(c, Call::Stop { .. })));
    }

    #[test]
    fn starting_counts_only_a_started_workspace_and_ignores_a_hold() {
        let dir = tempfile::tempdir().unwrap();
        let mut sup = supervisor(true, None, dir.path());
        sup.tick(&[], &mut free(), NOW);
        assert_eq!(sup.starting(&[], NOW + SECOND), 1);
        sup.hold(Some("runs as administrator".into()), NOW + SECOND);
        assert_eq!(sup.starting(&[], NOW + 2 * SECOND), 1, "launched before the hold: still settling");
        assert_eq!(sup.starting(&[], NOW + 20 * SECOND), 0, "settled");
        sup.desired.started = false;
        assert_eq!(sup.starting(&[], NOW + 2 * SECOND), 0);
        let mut held = supervisor(true, None, dir.path());
        held.hold(Some("runs as administrator".into()), NOW);
        held.tick(&[], &mut free(), NOW);
        assert_eq!(held.starting(&[], NOW + SECOND), 0, "held: nothing launched");
    }

    #[test]
    fn a_trust_refusal_on_resume_waits_for_retry() {
        let dir = tempfile::tempdir().unwrap();
        let mut sup = supervisor(true, None, dir.path());
        let stopped = row(SessionState::Stopped, None, None);
        sup.launcher.errors.borrow_mut().push(ClaudeError::TrustNeeded { cwd: r"C:\demo\cwd".into() });
        let needs = sup.tick(std::slice::from_ref(&stopped), &mut free(), NOW);
        assert!(matches!(sup.launcher().take().as_slice(), [Call::Resume { .. }]));
        assert_eq!(needs.len(), 1);
        assert_eq!(needs[0].kind, NeedsKind::TrustNeeded);
        sup.tick(std::slice::from_ref(&stopped), &mut free(), NOW + 3_600 * SECOND);
        assert_eq!(sup.launcher().take(), [], "never retried automatically");
        sup.retry("fm-demo-workspace");
        sup.tick(std::slice::from_ref(&stopped), &mut free(), NOW + 3_601 * SECOND);
        assert!(matches!(sup.launcher().take().as_slice(), [Call::Resume { .. }]));
    }

    #[test]
    fn a_row_whose_cwd_is_gone_backs_off_and_is_never_launched_afresh() {
        let dir = tempfile::tempdir().unwrap();
        let mut sup = supervisor(true, None, dir.path());
        let mut stopped = row(SessionState::Stopped, None, None);
        stopped.cwd = dir.path().join("gone");
        let gone = || ClaudeError::Spawn {
            command: "claude --bg --resume".into(),
            source: std::io::Error::new(std::io::ErrorKind::NotADirectory, "The directory name is invalid."),
        };
        sup.launcher.errors.borrow_mut().extend([gone(), gone()]);
        let needs = sup.tick(std::slice::from_ref(&stopped), &mut free(), NOW);
        assert_eq!(
            sup.launcher().take(),
            [Call::Resume { session_id: stopped.session_id.clone(), cwd: stopped.cwd.clone(), prompt: None }]
        );
        assert_eq!(needs[0].kind, NeedsKind::LaunchFailed);
        assert!(needs[0].detail.contains("directory name is invalid"), "{}", needs[0].detail);
        sup.tick(std::slice::from_ref(&stopped), &mut free(), NOW + 4 * SECOND);
        assert_eq!(sup.launcher().take(), [], "backing off");
        sup.tick(std::slice::from_ref(&stopped), &mut free(), NOW + 5 * SECOND);
        assert!(matches!(sup.launcher().take().as_slice(), [Call::Resume { .. }]), "a resume again, not a launch");
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
