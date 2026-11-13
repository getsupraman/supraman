use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use supraman_claude::{Claude, LaunchSpec, Session, SessionIdentity};
use supraman_core::AGENTS_SERVER;
use supraman_core::home::SessionEnv;

use crate::mint::{Absent, Bearer, Keeper};
use crate::{
    Action, Budget, DesiredState, Failure, HttpMinter, MintRepo, MintRequest, Minted, MintedSession, MintedSessions, Millis, Minter, NeedsKind,
    NeedsYou, Policy, Remedy, SupervisorMemory, WantedSession, adopt_prompt, is_live, mid_turn, reconcile,
};

/// The three `claude` writes the supervisor performs.
pub trait Launcher {
    fn launch(&self, spec: &LaunchSpec) -> supraman_claude::Result<String>;
    /// Must pass no flag but `--bg --resume <session_id> [-- <prompt>]` (any flag forks a copy).
    fn resume(&self, session_id: &str, cwd: &Path, prompt: Option<&str>) -> supraman_claude::Result<String>;
    fn stop(&self, id: &str) -> supraman_claude::Result<String>;
}

impl Launcher for Claude {
    fn launch(&self, spec: &LaunchSpec) -> supraman_claude::Result<String> {
        Claude::launch(self, spec)
    }

    fn resume(&self, session_id: &str, cwd: &Path, prompt: Option<&str>) -> supraman_claude::Result<String> {
        Claude::resume(self, session_id, cwd, prompt)
    }

    fn stop(&self, id: &str) -> supraman_claude::Result<String> {
        Claude::stop(self, id)
    }
}

/// The bearer of a session token's mint, PATCH and DELETE: the app's OAuth access token, never written anywhere.
#[derive(Clone, PartialEq, Eq)]
pub struct RoleToken {
    /// The Workman MCP URL; its origin is the API's base.
    pub url: String,
    pub token: String,
    /// The Workman role the session is minted with (controller, orchestrator, worker).
    pub role: String,
}

impl std::fmt::Debug for RoleToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RoleToken").field("url", &self.url).field("token", &"<redacted>").field("role", &self.role).finish()
    }
}

/// Where the bearer comes from: the app's sign-in.
pub trait TokenProvider {
    /// `Ok(None)`: not signed in or Workman not set up (a token-less config); `Err`: no token now, which fails the launch.
    fn token(&self, role: &str) -> Result<Option<RoleToken>, String>;

    /// The MCP URL a session's config names while it has no token: [`crate::mcp_url`]; `Err` is Workman not set up.
    fn mcp_url(&self) -> Result<String, String>;
}

/// Launch options every session gets; every launch is `--strict-mcp-config`, never the user-scope Workman.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaunchConfig {
    /// `<SUPRAMAN_HOME>/mcp`, where [`crate::Keeper`] writes each session's helped config.
    pub mcp_dir: PathBuf,
    pub plugin_dirs: Vec<PathBuf>,
    /// The session's `SUPRAMAN_HOME`, through `--settings {"env":…}`: env on `claude --bg` itself does not reach it.
    pub supraman_home: Option<PathBuf>,
    /// `bgIsolation: none`, else a `--bg` session in a main checkout writes into a worktree `claude rm` refuses to delete.
    pub bg_isolation_none: bool,
    /// `--allowedTools` every session gets, so `sm` never parks (`Bash(sm:*)` matches; PowerShell rules do not).
    pub allowed_tools: Vec<String>,
    /// Claude Code's config dir, for [`supraman_claude::plugin_server`]; `None` gives no plugin server.
    pub claude_dir: Option<PathBuf>,
}

impl LaunchConfig {
    /// The `claude --bg` of `w` before its Workman token, for the supervisor and a schedule's run; `env` adds to its env.
    pub fn launch_spec(&self, workspace: &str, w: &WantedSession, env: &[(&str, &str)]) -> LaunchSpec {
        let mut spec = LaunchSpec::new(&w.name, &w.cwd, &w.prompt);
        spec.permission_mode = w.permission_mode.clone();
        spec.strict_mcp_config = true;
        spec.plugin_dirs = self.plugin_dirs.clone();
        spec.mcp_servers = self.plugin_servers(&w.name);
        spec.add_dirs = w.add_dirs.clone();
        if let Some(settings) = self.settings_with(workspace, &w.name, &w.role, w.repo.as_deref(), env) {
            spec.extra_args = vec!["--settings".into(), settings];
        }
        spec.extra_args.extend(self.allowed_tools_args());
        spec
    }

    /// The plugin servers `--strict-mcp-config` would leave out, for `name`'s own config; a miss is logged, not failed.
    pub fn plugin_servers(&self, name: &str) -> Vec<(String, String)> {
        let Some(dir) = &self.claude_dir else {
            return Vec::new();
        };
        match supraman_claude::plugin_server(dir, AGENTS_PLUGIN, AGENTS_SERVER) {
            Ok(server) => vec![(AGENTS_SERVER.to_string(), server)],
            Err(why) => {
                log::info!("{name}: no {AGENTS_SERVER} MCP server in its config: {why}");
                Vec::new()
            }
        }
    }

    fn allowed_tools_args(&self) -> Vec<String> {
        if self.allowed_tools.is_empty() {
            return Vec::new();
        }
        std::iter::once("--allowedTools".to_string()).chain(self.allowed_tools.iter().cloned()).collect()
    }

    /// The plugin and `--allowedTools` flags of a foreground `claude` (a [+] session); its env is [`Self::session_env`].
    pub fn interactive_args(&self) -> Vec<String> {
        let plugins = self.plugin_dirs.iter().flat_map(|d| ["--plugin-dir".to_string(), d.to_string_lossy().into_owned()]);
        plugins.chain(self.allowed_tools_args()).collect()
    }

    /// The session's `SUPRAMAN_HOME` and `SUPRAMAN_*` vars; empty without a home.
    pub fn session_env(&self, workspace: &str, name: &str, role: &str, repo: Option<&str>, extra: &[(&str, &str)]) -> Vec<(String, String)> {
        let Some(home) = &self.supraman_home else {
            return Vec::new();
        };
        let session = SessionEnv {
            role: Some(role.to_string()),
            workspace: Some(workspace.to_string()),
            session: Some(name.to_string()),
            repo: repo.map(str::to_string),
        };
        let mut env: Vec<(String, String)> = session.vars().into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        env.extend(supraman_core::home::home_env(home).map(|(k, v)| (k.to_string(), v)));
        for (k, v) in extra {
            env.retain(|(have, _)| have != k);
            env.push((k.to_string(), v.to_string()));
        }
        env
    }

    fn settings_with(&self, workspace: &str, name: &str, role: &str, repo: Option<&str>, extra: &[(&str, &str)]) -> Option<String> {
        let mut settings = serde_json::Map::new();
        let mut env = self.session_env(workspace, name, role, repo, extra);
        if !env.is_empty() {
            env.extend(supraman_core::install::sm_first_path().map(|path| ("PATH".to_string(), path)));
            let env: serde_json::Map<String, serde_json::Value> = env.into_iter().map(|(k, v)| (k, v.into())).collect();
            settings.insert("env".into(), env.into());
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
    /// Where its state files are: the minted sessions, the kept tokens, the wanted sessions.
    home: PathBuf,
    pub config: LaunchConfig,
    launcher: L,
    tokens: T,
    minter: Box<dyn Minter + Send>,
    /// The `repo` of a repo session's mint, from its cwd: [`crate::git_repo`].
    repo_of: fn(&Path) -> Option<MintRepo>,
    /// The sessions this supervisor minted a token for, by name.
    minted: MintedSessions,
    /// The [`crate::sign_in_generation`] each was minted in by this process; one loaded from the file predates them all.
    generations: BTreeMap<String, u64>,
    /// Where their tokens are kept and their configs written.
    keeper: Keeper,
    /// Their claude sessions missing from the snapshots: a deleted one revokes.
    absent: Absent,
    /// Unbound records whose name is neither wanted nor listed, with the non-empty snapshots in a row that showed so.
    unwanted: BTreeMap<String, u32>,
    needs: Vec<NeedsYou>,
    notices: Vec<String>,
    /// Names whose Re-mint was pressed, until their stop succeeds or they have no live row.
    remint_due: Vec<String>,
    /// Consecutive failed `claude stop`s, by name: from [`STOP_FAILURES_SHOWN`] on, an Inbox row.
    stop_failures: BTreeMap<String, StopFailure>,
    /// Resumes whose output named no id: (name, id, when), settled by [`Self::confirm_resumes`].
    unconfirmed: Vec<(String, String, Millis)>,
    /// The server Supraman is signed in to, as [`Self::server_changed`] last saw it.
    origin: Option<String>,
    /// The server [`Self::server_changed`] last scheduled re-mints for.
    checked: Option<String>,
    /// Set by a sign-in ([`Self::server_changed`]): the next tick runs [`Self::mint_unminted`].
    mint_unminted_due: bool,
    /// While set, no launch or resume goes: they would only fail after 45 s. Why, and since when.
    held: Option<(String, Millis)>,
    /// Why a `--bg` that could not reach the background service will keep failing: [`supraman_claude::daemon_mismatch`].
    daemon_check: fn() -> Option<String>,
}

impl<L: Launcher, T: TokenProvider> Supervisor<L, T> {
    /// Mints over HTTP; the minted sessions are kept at [`MintedSessions::path`] under `home`.
    pub fn new(desired: DesiredState, home: &Path, config: LaunchConfig, launcher: L, tokens: T) -> Self {
        let path = MintedSessions::path(home, &desired.workspace);
        let mut notices = Vec::new();
        let mut minted = MintedSessions::load(path.clone()).unwrap_or_else(|e| {
            let kept = match MintedSessions::set_aside(&path) {
                Ok(aside) => format!("it is kept as {}", aside.display()),
                Err(moved) => format!("it could not be set aside ({moved})"),
            };
            notices.push(format!("Cannot read {} ({e}); {kept}: the Workman sessions minted before are not revoked", path.display()));
            MintedSessions::empty(path)
        });
        if let Err(e) = minted.adopt_origins(&config.mcp_dir) {
            notices.push(format!("Cannot save the minted Workman sessions of {}: {e}", desired.workspace));
        }
        let keeper = Keeper::new(home, &config.mcp_dir, supraman_core::secret::default_store(home));
        Self {
            home: home.to_path_buf(),
            desired,
            memory: SupervisorMemory::default(),
            policy: Policy::default(),
            config,
            launcher,
            tokens,
            minter: Box::new(HttpMinter::default()),
            repo_of: crate::git_repo,
            minted,
            generations: BTreeMap::new(),
            keeper,
            absent: Absent::default(),
            unwanted: BTreeMap::new(),
            needs: Vec::new(),
            notices,
            remint_due: Vec::new(),
            stop_failures: BTreeMap::new(),
            unconfirmed: Vec::new(),
            origin: None,
            checked: None,
            mint_unminted_due: false,
            held: None,
            daemon_check: supraman_claude::daemon_mismatch,
        }
    }

    #[cfg(test)]
    pub fn with_daemon_check(mut self, check: fn() -> Option<String>) -> Self {
        self.daemon_check = check;
        self
    }

    /// The app's own [`supraman_claude::daemon_mismatch`]: `Some` holds every launch and resume behind one Inbox row.
    pub fn hold(&mut self, why: Option<String>, now: Millis) {
        self.held = why.map(|why| match self.held.take() {
            Some((_, since)) => (why, since),
            None => (why, now),
        });
    }

    pub fn held(&self) -> Option<&str> {
        self.held.as_ref().map(|(why, _)| why.as_str())
    }

    #[cfg(test)]
    pub fn with_minter(mut self, minter: impl Minter + Send + 'static) -> Self {
        self.minter = Box::new(minter);
        self
    }

    #[cfg(test)]
    pub fn with_repo_of(mut self, repo_of: fn(&Path) -> Option<MintRepo>) -> Self {
        self.repo_of = repo_of;
        self
    }

    /// What went wrong outside a launch since the last call (a failed PATCH or revoke); nothing for the Inbox.
    pub fn take_notices(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notices)
    }

    pub fn minted(&self) -> &MintedSessions {
        &self.minted
    }

    pub fn keeper(&self) -> &Keeper {
        &self.keeper
    }

    /// The sign-out of `origin`: each session minted there is [`Keeper::signed_out`].
    pub fn signed_out(&mut self, origin: &str) {
        self.signed_out_as_of(origin, u64::MAX);
    }

    /// [`Self::signed_out`] queued in sign-in `generation`: a session minted on a later sign-in stays.
    pub fn signed_out_as_of(&mut self, origin: &str, generation: u64) {
        let origin = supraman_core::url::origin(origin);
        let names: Vec<String> = self
            .minted
            .names()
            .filter(|n| self.minted.get(n).is_some_and(|m| m.origin.as_deref() == Some(origin.as_str())))
            .filter(|n| self.generations.get(*n).is_none_or(|g| *g <= generation))
            .cloned()
            .collect();
        let notices = self.keeper.signed_out(&names);
        self.notices.extend(notices);
    }

    pub fn launcher(&self) -> &L {
        &self.launcher
    }

    /// The Inbox rows of the last tick.
    pub fn needs_you(&self) -> &[NeedsYou] {
        &self.needs
    }

    /// The human's Retry: forgets `name`'s failures; on a [`NeedsKind::OtherServer`] row it is the Re-mint.
    pub fn retry(&mut self, name: &str) {
        self.memory.retry(name);
        self.held = None;
        if self.needs.iter().any(|n| n.kind == NeedsKind::OtherServer && n.name == name)
            && !self.remint_due.iter().any(|n| n == name)
        {
            self.remint_due.push(name.to_string());
        }
    }

    /// The server signed in to, every tick; on a new one, each wanted session minted elsewhere is re-minted there. Returns those.
    pub fn server_changed(&mut self, origin: Option<&str>) -> Vec<(String, Option<String>)> {
        let was = std::mem::replace(&mut self.origin, origin.map(supraman_core::url::origin));
        self.mint_unminted_due |= was.is_none() && self.origin.is_some();
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

    /// Whether `m`'s token is from another server than `url`'s, whose PATCH and DELETE are that server's grant revoke's.
    fn elsewhere(m: &MintedSession, url: &str) -> bool {
        m.origin.as_deref().is_some_and(|o| o != supraman_core::url::origin(url))
    }

    #[cfg(test)]
    fn tokens_mut(&mut self) -> &mut T {
        &mut self.tokens
    }

    /// Wanted sessions started less than `settle` ago, not yet mid-turn: a fresh process shows `status=idle` first.
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

    /// One tick on a fresh `claude agents --json --all`; returns the Inbox rows as they stand after execution.
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
                    let result = self.launch(&w, observed, now).map(drop);
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
                        Some(w) => self.remint(w, &session_id),
                        None => Ok(()),
                    }
                    .and_then(|()| self.launcher.resume(&session_id, &cwd, prompt.as_deref()).map_err(|e| Failure::from(&e)))
                    .and_then(|out| match resumed(&out, &id) {
                        Resumed::InPlace => Ok(()),
                        Resumed::Copy(other) => Err(copy_failure(&id, &other)),
                        Resumed::Unknown => {
                            log::info!("claude printed {out:?} for the resume of {name} ({id}); the next session list shows whether it woke in place");
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
        self.settle_minted(observed, now);
        self.inbox(observed, &reminting, budget.free, now)
    }

    fn settle_minted(&mut self, observed: &[Session], now: Millis) {
        self.mint_unminted(observed, now);
        self.settle_forks(observed);
        self.bind_claude_sessions(observed, now);
        self.end_gone(observed, now);
        self.end_unwanted_unbound(observed);
    }

    fn inbox(&mut self, observed: &[Session], reminting: &[String], free: usize, now: Millis) -> Vec<NeedsYou> {
        let mut needs: Vec<NeedsYou> = reconcile(&self.desired, observed, &self.memory, &self.policy, free, now)
            .into_iter()
            .filter_map(|a| match a {
                Action::NeedsYou(n) => Some(n),
                _ => None,
            })
            .collect();
        needs.extend(self.other_server_tokens(observed, reminting, now));
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

    /// `None` when `result` could not reach the background service and [`Self::daemon_check`] says it never will.
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

    fn launch(&mut self, w: &WantedSession, observed: &[Session], now: Millis) -> Result<String, Failure> {
        let mut spec = self.config.launch_spec(&self.desired.workspace, w, &[]);
        if let Some(from) = &w.fork_from {
            // a copy of a live conversation would leave two sessions going on from one history
            if let Some(s) = observed.iter().find(|s| &s.session_id == from && is_live(s, now)) {
                return Err(Failure::Other(format!("the session to adopt ({}) runs again: stop it, then Retry", s.id)));
            }
            spec.resume = Some(from.clone());
            spec.prompt = adopt_prompt(&self.desired.workspace, w);
        }
        spec = match self.mint(w, None)? {
            Some((url, minted)) => self.keeper.launch(spec, &url, &minted.token, minted.expires_at.as_deref()),
            None => self.keeper.launch_unminted(spec, self.tokens.mcp_url().ok().as_deref()),
        }
        .map_err(Failure::Other)?;
        self.launcher.launch(&spec).map_err(|e| Failure::from(&e))
    }

    /// Mints `w`'s own token, always with `replace`; a launch sends no `claude_session_id`, as `--bg` ignores `--session-id`.
    fn mint(&mut self, w: &WantedSession, claude_session_id: Option<&str>) -> Result<Option<(String, Minted)>, Failure> {
        let Some(t) = self
            .tokens
            .token(&w.role)
            .map_err(|e| Failure::Other(format!("cannot mint a Workman token for {}: {e}", w.name)))?
        else {
            return Ok(None);
        };
        let generation = crate::sign_in_generation();
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
            takeover_from: None,
        };
        let minted = crate::mint::mint_as(&*self.minter, &supraman_core::url::origin(&t.url), &t.token, &request, self.minted.launched_by(&w.name))
            .map_err(|e| Failure::Other(format!("cannot mint a Workman token for {}: {e}", w.name)))?;
        let session = MintedSession {
            session_id: minted.session_id.clone(),
            role: w.role.clone(),
            claude_session_id: claude_session_id.map(str::to_string),
            origin: Some(supraman_core::url::origin(&t.url)),
        };
        self.minted.insert_minted(&w.name, session, &request);
        self.generations.insert(w.name.clone(), generation);
        self.save_minted();
        Ok(Some((t.url, minted)))
    }

    /// Before the resume of `claude_session_id`: a fresh token into the session's config file.
    fn remint(&mut self, w: &WantedSession, claude_session_id: &str) -> Result<(), Failure> {
        if let Some((url, minted)) = self.mint(w, Some(claude_session_id))? {
            self.keeper.write(&w.name, &url, &minted.token, minted.expires_at.as_deref(), &self.config.plugin_servers(&w.name)).map_err(Failure::Other)?;
        }
        Ok(())
    }

    /// Once per sign-in: each live wanted session with a token-less config gets its token kept, without a stop.
    fn mint_unminted(&mut self, observed: &[Session], now: Millis) {
        if !std::mem::take(&mut self.mint_unminted_due) {
            return;
        }
        let due: Vec<(WantedSession, String)> = self
            .desired
            .sessions
            .iter()
            .filter(|w| self.minted.get(&w.name).is_none() && supraman_claude::mcp_config_url(self.keeper.mcp_dir(), &w.name).is_some())
            .filter_map(|w| {
                let mut rows = observed.iter().filter(|s| s.name == w.name && is_live(s, now));
                match (rows.next(), rows.next()) {
                    (Some(row), None) if !row.session_id.is_empty() => Some((w.clone(), row.session_id.clone())),
                    _ => None,
                }
            })
            .collect();
        for (w, claude_session_id) in due {
            if let Err(Failure::Other(e)) = self.remint(&w, &claude_session_id) {
                self.notices.push(format!("{} runs without its Workman token after the sign-in: {e}", w.name));
            }
        }
    }

    /// [`DesiredState::forked`] for each adopted session whose own row shows, in the file too.
    fn settle_forks(&mut self, observed: &[Session]) {
        let landed: Vec<(String, String)> = self
            .desired
            .sessions
            .iter()
            .filter_map(|w| {
                let from = w.fork_from.as_ref()?;
                let row = observed.iter().find(|s| s.name == w.name && !s.session_id.is_empty() && &s.session_id != from)?;
                Some((w.name.clone(), row.session_id.clone()))
            })
            .collect();
        for (name, session_id) in landed {
            self.desired.forked(&name, &session_id);
            if let Err(e) = DesiredState::update(&self.home, &self.desired.workspace, |d| d.forked(&name, &session_id)) {
                self.notices.push(format!("Cannot record that {name} continues as {session_id}: {e}"));
            }
        }
    }

    /// PATCHes the claude session uuid onto each minted session once its single live row shows it, once per mint.
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
                Ok(Some(t)) => self.minter.bind(&supraman_core::url::origin(&t.url), &t.token, &session_id, &claude_session_id).map_err(|e| e.to_string()),
                Ok(None) => Err(format!("role {role} has no Workman token any more")),
                Err(e) => Err(e),
            };
            if let Err(e) = result {
                log::warn!("Cannot tell Workman the claude session of {name} (session {session_id}): {e}");
            }
            self.save_minted();
        }
    }

    /// Ends each minted session whose claude session and transcript are deleted, and forgets an unlisted one whose token lapsed ([`Keeper::gone`]).
    fn end_gone(&mut self, observed: &[Session], now: Millis) {
        let keeper = self.keeper.clone().with_claude_dir(self.config.claude_dir.clone());
        let gone = keeper.gone(&mut self.absent, &self.minted.bound(), observed, now);
        self.notices.extend(gone.notices);
        for (name, sid) in gone.deleted {
            log::info!("{name}: its claude session {sid} is deleted, so its Workman token is revoked");
            self.end(&name);
        }
        for (name, sid) in gone.lapsed {
            log::info!("{name}: its claude session {sid} stayed unlisted until its Workman token lapsed, so its record goes");
            if self.minted.remove(&name).is_some() {
                self.generations.remove(&name);
                self.notices.extend(self.keeper.forget(&name));
                self.save_minted();
            }
        }
    }

    /// Ends each never-bound record unwanted and unlisted for [`UNWANTED_UNBOUND_TICKS`] non-empty snapshots in a row.
    fn end_unwanted_unbound(&mut self, observed: &[Session]) {
        let mut counts = BTreeMap::new();
        let mut due = Vec::new();
        if !observed.is_empty() {
            for name in self.minted.names() {
                let unbound = self.minted.get(name).is_some_and(|m| m.claude_session_id.is_none());
                if !unbound || self.desired.sessions.iter().any(|w| &w.name == name) || observed.iter().any(|s| &s.name == name) {
                    continue;
                }
                let times = self.unwanted.get(name).copied().unwrap_or(0) + 1;
                if times >= UNWANTED_UNBOUND_TICKS {
                    due.push(name.clone());
                } else {
                    counts.insert(name.clone(), times);
                }
            }
        }
        self.unwanted = counts;
        for name in due {
            log::info!("{name}: never bound to a claude session and no longer wanted, so its Workman token is revoked");
            self.end(&name);
        }
    }

    /// [`crate::mint::revoke`] of `name` with its role's bearer; forgotten here.
    fn end(&mut self, name: &str) {
        let Some(m) = self.minted.remove(name) else {
            return;
        };
        self.generations.remove(name);
        let bearer = Bearer::of(self.tokens.token(&m.role).map(|t| t.map(|t| (t.url, t.token))));
        for notice in crate::mint::revoke(&self.keeper, &*self.minter, &bearer, name, &m).notices {
            log::warn!("{notice}");
        }
        self.save_minted();
    }

    /// The Re-mint: stops each pressed name's live row, as a resume of a live row starts a copy; R13 and R3 resume it.
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

    /// A [`Resumed::Unknown`] resume: its own id live confirms it, another live row of the name fails it.
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

    /// [`NeedsKind::OtherServer`] for each wanted, minted session whose live row holds another server's token.
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
                    "its Workman token is from {from}, and Supraman uses {origin}; Re-mint stops it and resumes it with a token from {origin}"
                ),
                action: Some(Remedy::Remint { id: row.id.clone() }),
            });
        }
        out
    }

    fn save_minted(&mut self) {
        if let Err(e) = self.minted.save() {
            self.notices.push(format!("Cannot save the minted Workman sessions of {}: {e}", self.desired.workspace));
        }
    }
}

pub fn session_identity(workspace: &str, w: &WantedSession) -> SessionIdentity {
    let launched_by = crate::mint::launched_by(workspace);
    SessionIdentity {
        agent: w.name.clone(),
        session: None,
        origin: match &w.repo {
            Some(repo) => format!("{launched_by}/{repo}"),
            None => launched_by.clone(),
        },
        launched_by,
        cwd: w.cwd.display().to_string(),
        supraman_role: w.role.clone(),
    }
}

/// From this many failed `claude stop`s in a row, the name gets a [`NeedsKind::StopFailed`] row.
pub const STOP_FAILURES_SHOWN: u32 = 3;
/// The non-empty snapshots in a row an unbound record's name must be unwanted and unlisted before it ends.
pub const UNWANTED_UNBOUND_TICKS: u32 = 30;
/// The plugin whose MCP server ([`AGENTS_SERVER`]) every supervised session gets in its own config.
const AGENTS_PLUGIN: &str = "workman";

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
    /// A session the daemon still counts as running starts a copy under a new id instead.
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
mod tests;
