use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context as _, anyhow};
use supraman_claude::{Claude, Session};
use supraman_core::{
    OutboxMessage, Registry, Settings, Workspace, clear_ready, ensure_relay_dirs, find_plugin_dir, install_hook_exe, hook_exe,
    read_answered, read_outbox, relay_demand, relay_ready,
};
use supraman_supervisor::{
    DesiredState, HAND_ROLE, LaunchConfig, Millis, NeedsYou, RELAY_ROLE, REPO_ROLE, WORKSPACE_ROLE, Supervisor, adopted_session, fill_paths, is_live,
    relay_session, relay_session_name, render_repo_role, role_prompt, tick_all,
};

pub use supraman_core::sm_allowed_tools;

use supraman_supervisor::{Absent, AppTokens, Bearer, HttpMinter, Keeper, bearer, revoke_deleted_runs, runs_signed_out, sign_in_generation};

/// The selftest's cleanup turns this off and takes [`TICK`], so nothing it removes is resumed.
pub static ENABLED: AtomicBool = AtomicBool::new(true);
pub static TICK: Mutex<()> = Mutex::new(());

/// Origins signed out of since the last tick, each with its sign-in generation: the sessions minted there before it
/// end at the next.
static SIGNED_OUT: Mutex<Vec<(String, u64)>> = Mutex::new(Vec::new());

pub fn signed_out(origin: &str) {
    SIGNED_OUT.lock().unwrap_or_else(|e| e.into_inner()).push((origin.to_string(), supraman_supervisor::sign_in_generation()));
}

/// Signs out of `auth`'s configured server, whose grant revoke takes every session token minted there; the hands,
/// supervised sessions and scheduled runs minted there are marked signed out for the next sign-in. What failed.
pub fn sign_out(auth: &supraman_workman::Auth, hands: Option<std::sync::Arc<Mutex<crate::hand::Hands>>>) -> Vec<String> {
    let origin = match auth.configured_origin() {
        Ok(origin) => origin,
        Err(e) => return vec![e.to_string()],
    };
    let mut notices: Vec<String> = auth.sign_out(&origin).err().into_iter().collect();
    signed_out(&origin);
    if let Some(hands) = hands {
        notices.extend(hands.lock().unwrap_or_else(|e| e.into_inner()).signed_out(&origin));
    }
    notices
}

#[cfg(feature = "selftest")]
pub fn disable_and_wait() {
    ENABLED.store(false, Ordering::SeqCst);
    drop(TICK.lock().unwrap_or_else(|e| e.into_inner()));
}

pub struct Setup {
    pub plugin_dir: Option<PathBuf>,
    pub notices: Vec<String>,
}

pub fn setup(home: &Path) -> Setup {
    let mut notices: Vec<String> = Vec::new();
    let sm = match supraman_core::install::sm_exe() {
        Ok(sm) => sm,
        Err(e) => {
            notices.push(format!("Cannot locate supraman.exe ({e}); Start is unavailable."));
            return Setup { plugin_dir: None, notices };
        }
    };
    let plugin_dir = match find_plugin_dir(&sm) {
        Ok(dir) => dir,
        Err(e) => {
            notices.push(format!("{e}: sessions cannot be started from this build."));
            return Setup { plugin_dir: None, notices };
        }
    };
    if sm.is_file() {
        if let Err(e) = install_hook_exe(&plugin_dir, &sm) {
            notices.push(format!("Cannot install {} into {}: {e}", sm.display(), plugin_dir.display()));
        }
    } else {
        notices.push(format!(
            "{} is missing (build it with cargo build -p sm): started sessions get no hook signals and no role re-injection.",
            sm.display()
        ));
    }
    for role in [WORKSPACE_ROLE, RELAY_ROLE, REPO_ROLE, HAND_ROLE] {
        if let Err(e) = write_role_template(home, &plugin_dir, role) {
            notices.push(format!("Cannot write the {role} role into {}: {e:#}", home.display()));
        }
    }
    Setup {
        plugin_dir: Some(plugin_dir),
        notices,
    }
}

pub fn write_role_template(home: &Path, plugin_dir: &Path, role: &str) -> anyhow::Result<()> {
    let file = format!("{role}.md");
    let template = fill_paths(
        &std::fs::read_to_string(plugin_dir.join("roles").join(&file))?,
        &plugin_dir.join("bin").join(hook_exe()),
        home,
    );
    let dir = home.join("roles");
    std::fs::create_dir_all(&dir)?;
    let target = dir.join(file);
    if std::fs::read_to_string(&target).ok().as_deref() != Some(template.as_str()) {
        std::fs::write(&target, template)?;
    }
    Ok(())
}

/// The relay is not wanted here: [`sync_relay`] wants it on demand.
pub fn start_workspace(home: &Path, plugin_dir: Option<&Path>, workspace: &Workspace) -> anyhow::Result<DesiredState> {
    let plugin_dir = plugin_dir.ok_or_else(|| anyhow!("no plugins/supraman found for this build"))?;
    let prompt = role_prompt(plugin_dir, WORKSPACE_ROLE, &workspace.name)?;
    let relay = relay_wanted(home, workspace)?;
    clear_ready(home, &workspace.name)?;
    let mut controller = DesiredState::workspace_controller(&workspace.name, home_of(workspace)?, &prompt)
        .sessions
        .into_iter()
        .next()
        .context("no controller session")?;
    controller.add_dirs = workspace.other_dirs();
    let (desired, ()) = DesiredState::update(home, &workspace.name, |desired| {
        desired.sessions.retain(|w| w.name != controller.name);
        desired.retired.retain(|n| n != &controller.name);
        let at = usize::from(desired.wants_relay());
        if let Some(r) = desired.sessions.iter_mut().find(|w| w.name == relay.name) {
            *r = relay.clone();
        }
        desired.sessions.insert(at, controller.clone());
        desired.started = true;
    })?;
    Ok(desired)
}

/// Its dirs are created here: its `--add-dir` must exist before it launches.
pub fn relay_wanted(home: &Path, workspace: &Workspace) -> anyhow::Result<supraman_supervisor::WantedSession> {
    let relay_dir = ensure_relay_dirs(home, &workspace.name)?;
    Ok(relay_session(&workspace.name, home_of(workspace)?, relay_dir, role_prompt(home, RELAY_ROLE, &workspace.name)?))
}

fn home_of(workspace: &Workspace) -> anyhow::Result<&Path> {
    workspace.home().ok_or_else(|| anyhow!("workspace {} has no home and no folder to run its sessions in", workspace.name))
}

pub fn sync_relay(home: &Path, workspace: &Workspace, desired: &DesiredState, idle_ms: Millis, now: Millis) -> anyhow::Result<Option<DesiredState>> {
    let demand = desired.started && relay_demand(home, &workspace.name, now, idle_ms)?;
    if demand == desired.wants_relay() {
        return Ok(None);
    }
    let relay = demand.then(|| relay_wanted(home, workspace)).transpose()?;
    let (saved, _) = DesiredState::update(home, &workspace.name, |d| d.set_relay(relay.clone()))?;
    Ok(Some(saved))
}

/// Wants `fm-<workspace>-<folder>` continuing the conversation of `session_id` (a full uuid), with the repo role
/// [`setup`] wrote for a fresh start; true when the workspace is started, so the supervisor goes at its next tick.
pub fn adopt(home: &Path, workspace: &str, folder: &str, session_id: &str) -> anyhow::Result<bool> {
    let ws = Registry::at(home).load(workspace)?;
    let f = ws.folder(folder).ok_or_else(|| anyhow!("workspace {workspace} has no folder {folder}"))?;
    let role = home.join("roles").join(format!("{REPO_ROLE}.md"));
    let template = std::fs::read_to_string(&role).with_context(|| format!("cannot read {}", role.display()))?;
    let wanted = adopted_session(workspace, &f.name, &f.path, render_repo_role(&template, workspace, &f.name), session_id)
        .ok_or_else(|| anyhow!("folder {folder} would take a name of the workspace's own sessions"))?;
    let (desired, _) = DesiredState::update(home, workspace, |d| d.want(wanted.clone()))?;
    Ok(desired.started)
}

pub fn stop_workspace(home: &Path, workspace: &str) -> anyhow::Result<()> {
    if DesiredState::load(home, workspace)?.is_some() {
        DesiredState::update(home, workspace, |desired| desired.started = false)?;
    }
    Ok(())
}

pub fn set_max_working(home: &Path, max_working: usize) -> anyhow::Result<()> {
    Settings::update(home, |settings| settings.sessions.max_working = max_working)?;
    Ok(())
}

#[derive(Debug, Clone, Default)]
pub struct Report {
    pub needs: Vec<NeedsYou>,
    pub desired: BTreeMap<String, (bool, Vec<String>)>,
    pub max_working: usize,
    pub terminal: supraman_core::TerminalSettings,
    pub errors: Vec<String>,
    pub last_attempt: HashMap<String, Millis>,
    pub relays: BTreeMap<String, RelayStatus>,
    pub messages: BTreeMap<String, Vec<OutboxMessage>>,
    /// outbox file → the answer's first line.
    pub answered: BTreeMap<String, BTreeMap<String, String>>,
    /// Why no `claude --bg` from here can reach the background service.
    pub daemon_hold: Option<String>,
    pub left_servers: Vec<String>,
}

pub fn signed_in_server(auth: &supraman_workman::Auth) -> Option<String> {
    let origin = auth.configured_origin().ok()?;
    matches!(auth.status(&origin), supraman_workman::AuthStatus::SignedIn { .. }).then_some(origin)
}

pub const DAEMON_CHECK_EVERY: Millis = 15_000;

pub struct DaemonWatch {
    check: fn() -> Option<String>,
    checked_at: Option<Millis>,
    why: Option<String>,
}

impl DaemonWatch {
    pub fn new(check: fn() -> Option<String>) -> Self {
        Self { check, checked_at: None, why: None }
    }

    pub fn current(&mut self, force: bool, now: Millis) -> Option<String> {
        if force || self.checked_at.is_none_or(|t| now - t >= DAEMON_CHECK_EVERY) {
            self.why = (self.check)();
            self.checked_at = Some(now);
        }
        self.why.clone()
    }

    pub fn found(&mut self, why: &str, now: Millis) {
        if self.why.is_none() {
            self.why = Some(why.to_string());
            self.checked_at = Some(now);
        }
    }
}

pub const MESSAGES_SHOWN: usize = 20;
/// Monitor expires after 30 min; a relay that has not re-armed a minute later has lost it.
pub const REARM_WITHIN_MS: Millis = 31 * 60 * 1000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayStatus {
    pub started: bool,
    pub wanted: bool,
    pub live: bool,
    pub ready_at: Option<Millis>,
}

impl RelayStatus {
    pub fn label(&self, now: Millis) -> Option<(String, bool)> {
        if !self.started && !self.live {
            return None;
        }
        Some(match (self.wanted, self.live, self.ready_at) {
            // a relay being retired: messages go direct already
            (false, _, _) => ("direct".into(), true),
            (true, false, _) | (true, true, None) => ("relay starting".into(), false),
            (true, true, Some(t)) if now - t > REARM_WITHIN_MS => {
                (format!("relay watcher expired, not re-armed (armed {} min ago)", (now - t) / 60_000), false)
            }
            (true, true, Some(_)) => ("relay ready".into(), true),
        })
    }
}

pub struct Supervision {
    home: PathBuf,
    plugin_dir: Option<PathBuf>,
    config: LaunchConfig,
    supervisors: BTreeMap<String, Supervisor<Claude, AppTokens>>,
    notices: Vec<String>,
    pub daemon: DaemonWatch,
    daemon_logged: Option<String>,
    pruned_at: Option<Millis>,
    /// The scheduled runs' claude sessions missing from the snapshots.
    runs_absent: Absent,
}

pub const NOTICES_KEPT: usize = 5;
pub const PRUNE_EVERY: Millis = 60 * 60 * 1000;

pub fn plugin_dirs(plugin_dir: Option<&Path>, settings: &Settings) -> Vec<PathBuf> {
    plugin_dir.map(Path::to_path_buf).into_iter().chain(settings.sessions.extra_plugin_dirs.iter().cloned()).collect()
}

/// The launch pieces of this home's supervised and [+] sessions.
pub fn launch_config(home: Option<PathBuf>, plugin_dir: Option<&Path>, settings: &Settings) -> LaunchConfig {
    LaunchConfig {
        mcp_dir: home.as_deref().map(|h| h.join("mcp")).unwrap_or_default(),
        plugin_dirs: plugin_dirs(plugin_dir, settings),
        supraman_home: home,
        bg_isolation_none: true,
        allowed_tools: plugin_dir.map(sm_allowed_tools).unwrap_or_default(),
        claude_dir: supraman_core::paths::claude_dir(),
    }
}

impl Supervision {
    /// A sign-out since the last tick marks the sessions minted there before it signed out. A scheduled run's record
    /// is marked only while no sign-in followed: `sm` mints it, so its generation is unknown here.
    fn end_signed_out(&mut self) -> Vec<String> {
        let keeper = Keeper::open(&self.home);
        let mut notices = Vec::new();
        for (origin, generation) in std::mem::take(&mut *SIGNED_OUT.lock().unwrap_or_else(|e| e.into_inner())) {
            for s in self.supervisors.values_mut() {
                s.signed_out_as_of(&origin, generation);
            }
            if sign_in_generation() == generation {
                notices.extend(runs_signed_out(&keeper, &origin));
            }
        }
        notices
    }

    /// A scheduled run whose claude session is deleted is revoked. `observed` was read without error.
    fn end_deleted_runs(&mut self, observed: &[Session]) -> Vec<String> {
        let keeper = Keeper::open(&self.home).with_claude_dir(supraman_core::paths::claude_dir());
        let auth = crate::signin::auth(&self.home);
        revoke_deleted_runs(&keeper, &HttpMinter::default(), || Bearer::of(bearer(&auth)), &mut self.runs_absent, observed)
    }

    pub fn new(home: PathBuf, plugin_dir: Option<PathBuf>) -> Self {
        let config = launch_config(Some(home.clone()), plugin_dir.as_deref(), &Settings::default());
        Self {
            home,
            plugin_dir,
            config,
            supervisors: BTreeMap::new(),
            notices: Vec::new(),
            daemon: DaemonWatch::new(supraman_claude::daemon_mismatch),
            daemon_logged: None,
            pruned_at: None,
            runs_absent: Absent::default(),
        }
    }

    pub fn tick(&mut self, workspaces: &[Workspace], observed: &[Session], retries: Vec<(String, String)>, now: Millis) -> Report {
        let mut report = Report::default();
        let settings = self.read_settings(&mut report, now);
        self.read_desired(workspaces, &settings, &mut report, now);
        self.follow_server(&mut report);
        self.run_supervisors(workspaces, observed, retries, &settings, &mut report, now);
        let ended = self.end_deleted_runs(observed);
        self.notices.extend(ended);
        for s in self.supervisors.values_mut() {
            self.notices.extend(s.take_notices());
        }
        self.notices.drain(..self.notices.len().saturating_sub(NOTICES_KEPT));
        report.errors.extend(self.notices.iter().cloned());
        self.report_desired(observed, &mut report, now);
        self.read_messages(workspaces, &mut report);
        report
    }

    fn read_settings(&mut self, report: &mut Report, now: Millis) -> Settings {
        let settings = Settings::load(&self.home).unwrap_or_else(|e| {
            report.errors.push(format!("Cannot read the settings, using the defaults: {e}"));
            Settings::default()
        });
        report.max_working = settings.sessions.max_working;
        report.terminal = settings.terminal.clone();
        if self.pruned_at.is_none_or(|at| now - at >= PRUNE_EVERY) {
            self.pruned_at = Some(now);
            report.errors.extend(supraman_core::prune::prune(&self.home, supraman_core::time::now_ms(), supraman_core::prune::KEEP_MS));
        }
        self.config.plugin_dirs = plugin_dirs(self.plugin_dir.as_deref(), &settings);
        settings
    }

    fn read_desired(&mut self, workspaces: &[Workspace], settings: &Settings, report: &mut Report, now: Millis) {
        self.supervisors.retain(|name, _| workspaces.iter().any(|p| &p.name == name));
        for p in workspaces {
            match DesiredState::load(&self.home, &p.name) {
                Ok(Some(desired)) => match self.supervisors.get_mut(&p.name) {
                    Some(s) => {
                        s.desired = desired;
                        s.config.plugin_dirs = self.config.plugin_dirs.clone();
                    }
                    None => {
                        let tokens = AppTokens { auth: crate::signin::auth(&self.home) };
                        let s = Supervisor::new(desired, self.config.clone(), Claude::default(), tokens);
                        self.supervisors.insert(p.name.clone(), s);
                    }
                },
                Ok(None) => {
                    self.supervisors.remove(&p.name);
                }
                Err(e) => report.errors.push(format!("Cannot read the desired state of {}: {e}", p.name)),
            }
            if let Some(s) = self.supervisors.get_mut(&p.name) {
                match sync_relay(&self.home, p, &s.desired, settings.sessions.relay_idle_ms(), now) {
                    Ok(Some(saved)) => {
                        log::info!("{}: relay {}", p.name, if saved.wants_relay() { "wanted (a message falls back)" } else { "retired (idle)" });
                        s.desired = saved;
                    }
                    Ok(None) => {}
                    Err(e) => report.errors.push(format!("Cannot start or retire the relay of {}: {e:#}", p.name)),
                }
            }
        }
    }

    fn follow_server(&mut self, report: &mut Report) {
        let signed_out = self.end_signed_out();
        self.notices.extend(signed_out);
        let server = signed_in_server(&crate::signin::auth(&self.home));
        for (name, s) in &mut self.supervisors {
            let stale = s.server_changed(server.as_deref());
            if !stale.is_empty() {
                let names: Vec<&str> = stale.iter().map(|(n, _)| n.as_str()).collect();
                log::info!("{name}: Workman is {}; restarting {} with tokens minted there", server.as_deref().unwrap_or_default(), names.join(", "));
            }
            for origin in stale.into_iter().filter_map(|(_, o)| o) {
                if !report.left_servers.contains(&origin) {
                    report.left_servers.push(origin);
                }
            }
        }
    }

    fn run_supervisors(&mut self, workspaces: &[Workspace], observed: &[Session], retries: Vec<(String, String)>, settings: &Settings, report: &mut Report, now: Millis) {
        let retried = !retries.is_empty();
        for (workspace, name) in retries {
            if let Some(s) = self.supervisors.get_mut(&workspace) {
                s.retry(&name);
            }
        }
        let held = self.daemon.current(retried, now);
        for s in self.supervisors.values_mut() {
            s.hold(held.clone(), now);
        }
        {
            let _tick = TICK.lock().unwrap_or_else(|e| e.into_inner());
            if ENABLED.load(Ordering::SeqCst) {
                let names: Vec<&str> = workspaces.iter().map(|p| p.name.as_str()).collect();
                report.needs = tick_all(self.supervisors.values_mut(), &names, observed, settings.sessions.max_working, now);
            }
        }
        if let Some(why) = self.supervisors.values().find_map(|s| s.held()) {
            self.daemon.found(why, now);
        }
        if self.daemon_logged != self.daemon.why {
            match &self.daemon.why {
                Some(why) => log::warn!("launches and resumes held: {why}"),
                None => log::info!("launches and resumes no longer held: the background service is reachable"),
            }
            self.daemon_logged = self.daemon.why.clone();
        }
        report.daemon_hold = self.daemon.why.clone();
    }

    fn report_desired(&self, observed: &[Session], report: &mut Report, now: Millis) {
        for (workspace, s) in &self.supervisors {
            let names: Vec<String> = s.desired.sessions.iter().map(|w| w.name.clone()).collect();
            for name in &names {
                if let Some(t) = s.memory.get(name).and_then(|a| a.last_attempt) {
                    report.last_attempt.insert(name.clone(), t);
                }
            }
            let relay = relay_session_name(workspace);
            let wanted = names.contains(&relay);
            let live = observed.iter().any(|r| r.name == relay && is_live(r, now));
            if !live && let Err(e) = clear_ready(&self.home, workspace) {
                report.errors.push(format!("Cannot reset the relay of {workspace}: {e}"));
            }
            let ready_at = live.then(|| relay_ready(&self.home, workspace)).flatten();
            report.relays.insert(workspace.clone(), RelayStatus { started: s.desired.started, wanted, live, ready_at });
            report.desired.insert(workspace.clone(), (s.desired.started, names));
        }
    }

    fn read_messages(&self, workspaces: &[Workspace], report: &mut Report) {
        for p in workspaces {
            match read_outbox(&self.home, &p.name) {
                Ok(mut msgs) if !msgs.is_empty() => {
                    msgs.drain(..msgs.len().saturating_sub(MESSAGES_SHOWN));
                    report.messages.insert(p.name.clone(), msgs);
                    match read_answered(&self.home, &p.name) {
                        Ok(answered) => {
                            report.answered.insert(p.name.clone(), answered);
                        }
                        Err(e) => report.errors.push(format!("Cannot read which questions of {} were answered: {e}", p.name)),
                    }
                }
                Ok(_) => {}
                Err(e) => report.errors.push(format!("Cannot read the messages of {}: {e}", p.name)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The relay's Monitor runs its command under Git Bash (LEARNINGS m0/4).
    fn forward(path: &Path) -> String {
        path.display().to_string().replace('\\', "/")
    }

    fn plugin_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/supraman")
    }

    #[test]
    fn finds_the_plugin_above_the_exe() {
        let exe = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/release/supraman.exe");
        let found = find_plugin_dir(&exe).unwrap();
        assert!(found.join("roles").join("workspace.md").is_file(), "{}", found.display());
        assert!(find_plugin_dir(Path::new(r"C:\nowhere\supraman.exe")).is_err());
    }

    #[test]
    fn launches_get_extra_plugin_dirs_after_supramans_and_the_sm_rules() {
        let mut settings = Settings::default();
        settings.sessions.extra_plugin_dirs = vec![r"C:\skills\plugins\rlarch".into()];
        let own = Path::new(r"C:\fm\plugins\supraman");
        assert_eq!(plugin_dirs(Some(own), &settings), [own.to_path_buf(), r"C:\skills\plugins\rlarch".into()]);
        assert_eq!(plugin_dirs(None, &Settings::default()), Vec::<PathBuf>::new());
        assert_eq!(
            sm_allowed_tools(own),
            ["Bash(sm:*)", "Bash(C:/fm/plugins/supraman/bin/sm.exe:*)", "mcp__agents__agent_launch", "mcp__agents__agent_list", "mcp__agents__agent_stop"]
        );
    }

    #[test]
    fn controller_roles_load_their_skills_as_rendered_and_injected() {
        let home = tempfile::tempdir().unwrap();
        for (role, skill, repo) in [(WORKSPACE_ROLE, "workspace-controller", None), (REPO_ROLE, "repo-controller", Some("Nomos.Pdf"))] {
            write_role_template(home.path(), &plugin_dir(), role).unwrap();
            let injected = supraman_core::role_injection_for(home.path(), role, Some("demo"), repo).unwrap();
            assert!(injected.contains(&format!("/supraman:{skill}")), "{injected}");
            assert!(injected.contains("work_brief"), "after a compaction the role says how to resume: {injected}");
            assert!(injected.contains("fm-demo-workspace"), "{injected}");
            assert!(!injected.contains('{'), "every placeholder filled: {injected}");

            let text = std::fs::read_to_string(plugin_dir().join("skills").join(skill).join("SKILL.md")).unwrap().replace("\r\n", "\n");
            let front = text.strip_prefix("---\n").and_then(|t| t.split_once("\n---\n")).map(|(f, _)| f).unwrap_or_default();
            assert!(front.lines().any(|l| l == format!("name: {skill}")), "{skill}: {front}");
            let description = front.lines().find_map(|l| l.strip_prefix("description: ")).unwrap_or_default();
            assert!(description.contains("USE FOR") && description.contains("DO NOT USE FOR"), "{skill}: {description}");
        }
        let launched = role_prompt(&plugin_dir(), WORKSPACE_ROLE, "demo").unwrap();
        assert!(launched.contains("/supraman:workspace-controller") && launched.contains("fm-demo-relay"), "{launched}");
        let repo = supraman_supervisor::render_repo_role(&std::fs::read_to_string(plugin_dir().join("roles").join("repo.md")).unwrap(), "demo", "Alpha");
        assert!(repo.contains("/supraman:repo-controller") && repo.contains("/workman:controller"), "{repo}");
    }

    #[test]
    fn adopt_wants_the_folders_session_forking_the_uuid_with_the_repo_role() {
        let home = tempfile::tempdir().unwrap();
        let alpha = home.path().join("Alpha");
        std::fs::create_dir_all(&alpha).unwrap();
        Registry::at(home.path()).save(&Workspace::new("demo", home.path(), vec![supraman_core::Folder::new("Alpha", &alpha)]), true).unwrap();
        let from = "fc5bf027-48e3-4510-90b2-63a00eb76df8";
        assert!(adopt(home.path(), "demo", "Alpha", from).is_err(), "no role written yet");
        write_role_template(home.path(), &plugin_dir(), REPO_ROLE).unwrap();
        assert!(!adopt(home.path(), "demo", "Alpha", from).unwrap(), "not started");
        let d = DesiredState::load(home.path(), "demo").unwrap().unwrap();
        let w = &d.sessions[0];
        assert_eq!((w.name.as_str(), w.role.as_str(), w.repo.as_deref(), w.cwd.as_path(), w.fork_from.as_deref()), ("fm-demo-alpha", REPO_ROLE, Some("Alpha"), alpha.as_path(), Some(from)));
        assert!(w.prompt.starts_with("You are the repo controller for Alpha in the Supraman workspace demo"), "{}", w.prompt);
        assert!(adopt(home.path(), "demo", "Nope", from).is_err());
    }

    #[test]
    fn start_keeps_repo_sessions_after_its_own() {
        let home = tempfile::tempdir().unwrap();
        write_role_template(home.path(), &plugin_dir(), RELAY_ROLE).unwrap();
        write_role_template(home.path(), &plugin_dir(), REPO_ROLE).unwrap();
        let rendered = std::fs::read_to_string(home.path().join("roles").join("repo.md")).unwrap();
        assert!(rendered.contains("{repo}") && rendered.contains("{workspace}"), "{rendered}");
        let alpha = supraman_supervisor::repo_session("demo", "Alpha", r"C:\demo\Alpha", "p").unwrap();
        DesiredState::update(home.path(), "demo", |d| d.want(alpha.clone())).unwrap();
        let d = start_workspace(home.path(), Some(&plugin_dir()), &Workspace::new("demo", r"C:\demo", Vec::new())).unwrap();
        let names: Vec<&str> = d.sessions.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(names, ["fm-demo-workspace", "fm-demo-alpha"], "no relay: it starts on demand");
        stop_workspace(home.path(), "demo").unwrap();
        assert_eq!(DesiredState::load(home.path(), "demo").unwrap().unwrap().sessions.len(), 2);
    }

    #[test]
    fn start_keeps_extra_sessions_and_stop_keeps_the_file() {
        let home = tempfile::tempdir().unwrap();
        let workspace = Workspace::new("demo", r"C:\demo", Vec::new());
        write_role_template(home.path(), &plugin_dir(), RELAY_ROLE).unwrap();
        let d = start_workspace(home.path(), Some(&plugin_dir()), &workspace).unwrap();
        assert!(d.started);
        assert_eq!(d.sessions.len(), 1, "no relay wanted at Start");
        assert_eq!(d.sessions[0].name, "fm-demo-workspace");
        assert_eq!(d.sessions[0].cwd, PathBuf::from(r"C:\demo"));
        assert!(d.sessions[0].prompt.contains("workspace controller for demo"), "{}", d.sessions[0].prompt);

        let mut extra = d.clone();
        extra.started = false;
        let mut park = extra.sessions[0].clone();
        park.name = "fm-demo-park".into();
        extra.sessions.push(park);
        extra.set_relay(Some(relay_wanted(home.path(), &workspace).unwrap()));
        extra.save(home.path()).unwrap();
        let moved = Workspace::new("demo", r"C:\moved", Vec::new());
        let d = start_workspace(home.path(), Some(&plugin_dir()), &moved).unwrap();
        let names: Vec<&str> = d.sessions.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(names, ["fm-demo-relay", "fm-demo-workspace", "fm-demo-park"], "a wanted relay stays first");
        assert_eq!(d.sessions[1].cwd, PathBuf::from(r"C:\moved"));
        assert_eq!(d.sessions[0].cwd, PathBuf::from(r"C:\moved"), "the relay follows the home");

        stop_workspace(home.path(), "demo").unwrap();
        let d = DesiredState::load(home.path(), "demo").unwrap().unwrap();
        assert!(!d.started);
        assert_eq!(d.sessions.len(), 3);
        stop_workspace(home.path(), "never-started").unwrap();
        assert!(DesiredState::load(home.path(), "never-started").unwrap().is_none());
        assert!(start_workspace(home.path(), None, &workspace).is_err());
    }

    #[test]
    fn a_virtual_workspace_runs_in_its_home_and_adds_the_other_folders() {
        let home = tempfile::tempdir().unwrap();
        write_role_template(home.path(), &plugin_dir(), RELAY_ROLE).unwrap();
        let folders = vec![
            supraman_core::Folder::new("Gaia", r"P:\L\Gaia"),
            supraman_core::Folder::new("NodeApi", r"P:\L\NodeApi"),
            supraman_core::Folder::new("dotnet", r"Q:\node-api-dotnet"),
        ];
        let mut gaia = Workspace::new("gaia", r"P:\L\Gaia", folders);
        gaia.home = None;
        let d = start_workspace(home.path(), Some(&plugin_dir()), &gaia).unwrap();
        let controller = &d.sessions[0];
        assert_eq!(controller.cwd, PathBuf::from(r"P:\L\Gaia"), "home defaults to the first folder");
        assert_eq!(controller.add_dirs, [PathBuf::from(r"P:\L\NodeApi"), PathBuf::from(r"Q:\node-api-dotnet")]);
        assert_eq!(relay_wanted(home.path(), &gaia).unwrap().cwd, PathBuf::from(r"P:\L\Gaia"));

        gaia.home = Some(r"P:\L".into());
        let d = start_workspace(home.path(), Some(&plugin_dir()), &gaia).unwrap();
        assert_eq!((d.sessions[0].cwd.as_path(), d.sessions[0].add_dirs.as_slice()), (std::path::Path::new(r"P:\L"), &[PathBuf::from(r"Q:\node-api-dotnet")][..]));

        let empty = Workspace { home: None, folders: vec![], ..gaia };
        let err = start_workspace(home.path(), Some(&plugin_dir()), &empty).unwrap_err().to_string();
        assert!(err.contains("no home and no folder"), "{err}");
    }

    #[test]
    fn the_relay_role_gets_its_paths_and_the_wanted_relay_its_dir() {
        let home = tempfile::tempdir().unwrap();
        write_role_template(home.path(), &plugin_dir(), RELAY_ROLE).unwrap();
        let rendered = std::fs::read_to_string(home.path().join("roles").join("relay.md")).unwrap();
        let sm = forward(&plugin_dir().join("bin").join("sm.exe"));
        let command = format!("\"{sm}\" inbox --dir \"{}/relay/{{workspace}}/inbox\"", forward(home.path()));
        assert!(rendered.contains(&command), "{command}\n{rendered}");
        assert!(!rendered.contains('\\'), "forward slashes only: {rendered}");

        let outbox = ensure_relay_dirs(home.path(), "demo").unwrap().join("outbox");
        std::fs::write(outbox.join("ready.json"), "{}").unwrap();
        let workspace = Workspace::new("demo", r"C:\demo", Vec::new());
        start_workspace(home.path(), Some(&plugin_dir()), &workspace).unwrap();
        let relay = &relay_wanted(home.path(), &workspace).unwrap();
        assert_eq!((relay.name.as_str(), relay.role.as_str()), ("fm-demo-relay", "relay"));
        assert_eq!(relay.add_dirs, [home.path().join("relay").join("demo")]);
        assert!(relay.prompt.contains("/relay/demo/outbox/ready.json"), "{}", relay.prompt);
        assert!(!outbox.join("ready.json").exists(), "a new start forgets the last arm");
        assert!(home.path().join("relay").join("demo").join("inbox").is_dir());
    }

    #[test]
    fn relay_status_reads_direct_starting_ready_and_expired() {
        let now = 1_800_000_000_000;
        let st = |started, wanted, live, ready_at| RelayStatus { started, wanted, live, ready_at };
        assert_eq!(st(false, false, false, None).label(now), None);
        assert_eq!(st(false, true, false, None).label(now), None);
        assert_eq!(st(true, false, false, None).label(now), Some(("direct".into(), true)));
        assert_eq!(st(true, false, true, Some(now)).label(now), Some(("direct".into(), true)), "being retired");
        assert_eq!(st(true, true, false, None).label(now), Some(("relay starting".into(), false)));
        assert_eq!(st(true, true, true, None).label(now), Some(("relay starting".into(), false)));
        assert_eq!(st(true, true, true, Some(now - 60_000)).label(now), Some(("relay ready".into(), true)));
        let expired = st(true, true, true, Some(now - 40 * 60_000)).label(now).unwrap();
        assert_eq!(expired, ("relay watcher expired, not re-armed (armed 40 min ago)".into(), false));
    }

    #[test]
    fn tick_supervises_only_workspaces_with_a_desired_file() {
        let home = tempfile::tempdir().unwrap();
        let workspaces = [Workspace::new("alpha", r"C:\a", Vec::new()), Workspace::new("beta", r"C:\b", Vec::new())];
        DesiredState::workspace_controller("beta", r"C:\b", "p").save(home.path()).unwrap();
        set_max_working(home.path(), 1).unwrap();
        let mut s = Supervision::new(home.path().to_path_buf(), None);
        s.daemon = DaemonWatch::new(|| None);
        // beta is not started: nothing to launch, and no `claude` is run
        let report = s.tick(&workspaces, &[], Vec::new(), 1_800_000_000_000);
        assert_eq!(report.max_working, 1);
        assert_eq!(report.desired.keys().collect::<Vec<_>>(), ["beta"]);
        assert_eq!(report.desired["beta"], (false, vec!["fm-beta-workspace".to_string()]));
        assert!(report.needs.is_empty() && report.errors.is_empty(), "{report:?}");
        assert_eq!(report.relays["beta"], RelayStatus { started: false, wanted: false, live: false, ready_at: None });
        assert!(report.messages.is_empty(), "{report:?}");

        let mut d = DesiredState::workspace_controller("beta", r"C:\b", "p");
        d.sessions.insert(0, relay_session("beta", r"C:\b", home.path().join("relay").join("beta"), "r"));
        d.save(home.path()).unwrap();
        let outbox = ensure_relay_dirs(home.path(), "beta").unwrap().join("outbox");
        std::fs::write(outbox.join("ready.json"), "{}").unwrap();
        let alpha_outbox = ensure_relay_dirs(home.path(), "alpha").unwrap().join("outbox");
        let msg = r#"{"from": "u", "from_name": "fm-alpha-workspace", "body": "hi"}"#;
        std::fs::write(alpha_outbox.join("msg-0001.json"), msg).unwrap();
        let report = s.tick(&workspaces, &[], Vec::new(), 1_800_000_000_000);
        assert_eq!(report.relays["beta"], RelayStatus { started: false, wanted: false, live: false, ready_at: None }, "stopped: retired");
        assert_eq!(DesiredState::load(home.path(), "beta").unwrap().unwrap().retired, ["fm-beta-relay"]);
        assert!(!outbox.join("ready.json").exists());
        assert_eq!(report.messages["alpha"].len(), 1);
        assert_eq!(report.messages["alpha"][0].from_name, "fm-alpha-workspace");
    }

    #[test]
    fn the_relay_is_wanted_on_a_fallback_and_retired_after_the_idle_timeout() {
        let home = tempfile::tempdir().unwrap();
        write_role_template(home.path(), &plugin_dir(), RELAY_ROLE).unwrap();
        let workspace = Workspace::new("demo", r"C:\demo", Vec::new());
        let idle = 60_000;
        let mut d = DesiredState::workspace_controller("demo", r"C:\demo", "p");
        d.started = true;
        d.save(home.path()).unwrap();
        let now = supraman_claude::now_ms();
        assert!(sync_relay(home.path(), &workspace, &d, idle, now).unwrap().is_none(), "no fallback: no relay");

        supraman_core::send(home.path(), "demo", "fm-demo-workspace", "fell back").unwrap();
        let d = sync_relay(home.path(), &workspace, &d, idle, now).unwrap().expect("wanted");
        assert_eq!(d.sessions[0].name, "fm-demo-relay");
        assert_eq!(DesiredState::load(home.path(), "demo").unwrap().unwrap(), d, "saved");
        assert!(sync_relay(home.path(), &workspace, &d, idle, now + 10 * idle).unwrap().is_none(), "the drop still waits");

        let inbox = supraman_core::inbox_dir(home.path(), "demo").unwrap();
        supraman_core::InboxWatcher::new(&inbox).scan().unwrap();
        let last = supraman_core::last_drop(home.path(), "demo").unwrap();
        assert!(sync_relay(home.path(), &workspace, &d, idle, last + idle - 1).unwrap().is_none(), "within the timeout");
        let d = sync_relay(home.path(), &workspace, &d, idle, last + idle).unwrap().expect("retired");
        assert!(!d.wants_relay());
        assert_eq!(d.retired, ["fm-demo-relay"]);
        assert_eq!(d.sessions.len(), 1);

        let mut old = DesiredState::workspace_controller("demo", r"C:\demo", "p");
        old.started = true;
        old.sessions.insert(0, relay_wanted(home.path(), &workspace).unwrap());
        old.save(home.path()).unwrap();
        let queued = supraman_core::send(home.path(), "demo", "fm-demo-workspace", "queued").unwrap();
        let later = supraman_core::last_drop(home.path(), "demo").unwrap() + 2 * idle;
        assert!(sync_relay(home.path(), &workspace, &old, idle, later).unwrap().is_none(), "kept while the drop waits");
        assert!(queued.is_file());
        supraman_core::InboxWatcher::new(&inbox).scan().unwrap();
        let d = sync_relay(home.path(), &workspace, &old, idle, later).unwrap().expect("retired on upgrade");
        assert_eq!(d.retired, ["fm-demo-relay"]);
        old.save(home.path()).unwrap();
        assert!(sync_relay(home.path(), &workspace, &old, idle, later).unwrap().is_some_and(|d| !d.wants_relay()));
    }

    static CHECKS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    fn counted_mismatch() -> Option<String> {
        CHECKS.fetch_add(1, Ordering::SeqCst);
        Some("runs as administrator".into())
    }

    #[test]
    fn an_unreachable_service_holds_a_started_workspace_before_any_launch() {
        let home = tempfile::tempdir().unwrap();
        let workspaces = [Workspace::new("beta", r"C:\b", Vec::new())];
        let mut d = DesiredState::workspace_controller("beta", r"C:\b", "p");
        d.started = true;
        d.save(home.path()).unwrap();
        let mut s = Supervision::new(home.path().to_path_buf(), None);
        s.daemon = DaemonWatch::new(counted_mismatch);
        let t0 = 1_800_000_000_000;
        // a launch would run the real `claude`: the hold keeps it from running at all
        let report = s.tick(&workspaces, &[], Vec::new(), t0);
        assert_eq!(report.needs.len(), 1, "{report:?}");
        assert_eq!(report.needs[0].detail, "runs as administrator");
        assert_eq!(report.daemon_hold.as_deref(), Some("runs as administrator"));
        assert_eq!(CHECKS.load(Ordering::SeqCst), 1);
        s.tick(&workspaces, &[], Vec::new(), t0 + 2_000);
        assert_eq!(CHECKS.load(Ordering::SeqCst), 1, "not every tick");
        s.tick(&workspaces, &[], Vec::new(), t0 + DAEMON_CHECK_EVERY);
        assert_eq!(CHECKS.load(Ordering::SeqCst), 2);
        let report = s.tick(&workspaces, &[], vec![("beta".into(), "fm-beta-workspace".into())], t0 + DAEMON_CHECK_EVERY + 2_000);
        assert_eq!(CHECKS.load(Ordering::SeqCst), 3, "Retry checks at once");
        assert_eq!(report.needs.len(), 1, "still held: {report:?}");
    }
}
