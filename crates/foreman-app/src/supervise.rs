//! The app's supervisor: one [`foreman_supervisor::Supervisor`] per workspace that has a desired-state
//! file, ticked after every `claude agents --json --all` poll on that snapshot, all workspaces with one
//! shared budget ([`tick_all`]). Runs on the refresh loop's background thread; the UI only writes
//! files (desired state, settings) and queues Retries.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context as _, anyhow};
use foreman_claude::{Claude, Session};
use foreman_core::{
    HOOK_EXE, OutboxMessage, Settings, Workspace, clear_ready, ensure_relay_dirs, find_plugin_dir, install_hook_exe,
    migrate_home,
    read_answered, read_outbox, relay_demand, relay_ready,
};
use foreman_supervisor::{
    DesiredState, LaunchConfig, Millis, NeedsYou, RELAY_ROLE, REPO_ROLE, WORKSPACE_ROLE, Supervisor, fill_paths, is_live,
    relay_session, relay_session_name, role_prompt, tick_all,
};

use crate::tokens::AppTokens;

/// Off: no tick acts. The selftest's cleanup turns it off, then takes [`TICK`] to wait out a tick
/// in flight, so nothing it removes is resumed behind its back.
pub static ENABLED: AtomicBool = AtomicBool::new(true);
pub static TICK: Mutex<()> = Mutex::new(());

pub fn disable_and_wait() {
    ENABLED.store(false, Ordering::SeqCst);
    drop(TICK.lock().unwrap_or_else(|e| e.into_inner()));
}

/// What the app found at start: where the plugin is and whether its hook exe is current.
pub struct Setup {
    pub plugin_dir: Option<PathBuf>,
    pub notices: Vec<String>,
}

/// [`migrate_home`] (before anything reads the registry), `fm plugin install` for the `fm.exe` built next
/// to `foreman.exe`, and the role templates at `<home>/roles/{workspace,relay,repo}.md` (the SessionStart hook
/// fills in `{workspace}`/`{repo}` from `FOREMAN_WORKSPACE`/`FOREMAN_REPO`; `{fm}` and `{home}` are filled here).
pub fn setup(home: &Path) -> Setup {
    let mut notices = migrate_home(home)
        .unwrap_or_else(|e| vec![format!("Cannot migrate {} to workspace names: {e}", home.display())]);
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(e) => {
            notices.push(format!("Cannot locate foreman.exe ({e}); Start is unavailable."));
            return Setup { plugin_dir: None, notices };
        }
    };
    let plugin_dir = match find_plugin_dir(&exe) {
        Ok(dir) => dir,
        Err(e) => {
            notices.push(format!("{e}: sessions cannot be started from this build."));
            return Setup { plugin_dir: None, notices };
        }
    };
    let fm = exe.with_file_name(HOOK_EXE);
    if fm.is_file() {
        if let Err(e) = install_hook_exe(&plugin_dir, &fm) {
            notices.push(format!("Cannot install {} into {}: {e}", fm.display(), plugin_dir.display()));
        }
    } else {
        notices.push(format!(
            "{} is missing (build it with cargo build -p fm): started sessions get no hook signals and no role re-injection.",
            fm.display()
        ));
    }
    for role in [WORKSPACE_ROLE, RELAY_ROLE, REPO_ROLE] {
        if let Err(e) = write_role_template(home, &plugin_dir, role) {
            notices.push(format!("Cannot write the {role} role into {}: {e:#}", home.display()));
        }
    }
    Setup {
        plugin_dir: Some(plugin_dir),
        notices,
    }
}

/// Forward slashes: the relay's Monitor runs its command under Git Bash (m0/4).
fn forward(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

/// The permission rules every session gets so `fm` never parks (m3/2): the plugin's `bin/` is on the
/// session's PATH, and a session may also call the exe by its absolute path.
pub fn fm_allowed_tools(plugin_dir: &Path) -> Vec<String> {
    vec!["Bash(fm:*)".into(), format!("Bash({}:*)", forward(&plugin_dir.join("bin").join(HOOK_EXE)))]
}

/// `<home>/roles/<role>.md` from the plugin's template, with `{fm}` (the plugin's hook exe) and `{home}`
/// filled in; `{workspace}` and `{repo}` stay for the hook, [`role_prompt`] and `fm want`.
pub fn write_role_template(home: &Path, plugin_dir: &Path, role: &str) -> anyhow::Result<()> {
    let file = format!("{role}.md");
    let template = fill_paths(
        &std::fs::read_to_string(plugin_dir.join("roles").join(&file))?,
        &plugin_dir.join("bin").join(HOOK_EXE),
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

/// Start: the workspace's desired state with its `fm-<workspace>-workspace` session first (cwd = the
/// workspace root, prompt = the role), `started = true`, saved. Other wanted sessions (repo sessions
/// `fm want` added) are kept, after it. The relay is not wanted here: the tick wants it on demand
/// ([`sync_relay`]); one that is wanted already follows the root. Its dirs are created and its last
/// `ready.json` forgotten. Read-modify-write through [`DesiredState::update`]: `fm want` writes the same file.
pub fn start_workspace(home: &Path, plugin_dir: Option<&Path>, workspace: &Workspace) -> anyhow::Result<DesiredState> {
    let plugin_dir = plugin_dir.ok_or_else(|| anyhow!("no plugins/foreman found for this build"))?;
    let prompt = role_prompt(plugin_dir, WORKSPACE_ROLE, &workspace.name)?;
    let relay = relay_wanted(home, workspace)?;
    clear_ready(home, &workspace.name)?;
    let controller = DesiredState::workspace_controller(&workspace.name, &workspace.root, &prompt)
        .sessions
        .into_iter()
        .next()
        .context("no controller session")?;
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

/// `fm-<workspace>-relay` as the tick wants it: its dirs created (its `--add-dir` must exist before it
/// launches), and the rendered role from `<home>/roles`, which has the relay's paths filled in.
pub fn relay_wanted(home: &Path, workspace: &Workspace) -> anyhow::Result<foreman_supervisor::WantedSession> {
    let relay_dir = ensure_relay_dirs(home, &workspace.name)?;
    Ok(relay_session(&workspace.name, &workspace.root, relay_dir, role_prompt(home, RELAY_ROLE, &workspace.name)?))
}

/// The relay on demand (m5/32): wanted while the workspace is started and a fallback drop waits in its
/// inbox or the last one is younger than `idle_ms` ([`relay_demand`]); otherwise a wanted relay is retired,
/// which R14 stops, also one an older desired state lists. Queued drops stay in the inbox for the next relay.
/// `Some` with the saved state when it changed.
pub fn sync_relay(home: &Path, workspace: &Workspace, desired: &DesiredState, idle_ms: Millis, now: Millis) -> anyhow::Result<Option<DesiredState>> {
    let demand = desired.started && relay_demand(home, &workspace.name, now, idle_ms)?;
    if demand == desired.wants_relay() {
        return Ok(None);
    }
    let relay = demand.then(|| relay_wanted(home, workspace)).transpose()?;
    let (saved, _) = DesiredState::update(home, &workspace.name, |d| d.set_relay(relay.clone()))?;
    Ok(Some(saved))
}

/// Stop: `started = false`, saved. The next tick stops the live wanted sessions (never removes them):
/// the workspace session, then the repo sessions, the relay last.
pub fn stop_workspace(home: &Path, workspace: &str) -> anyhow::Result<()> {
    if DesiredState::load(home, workspace)?.is_some() {
        DesiredState::update(home, workspace, |desired| desired.started = false)?;
    }
    Ok(())
}

pub fn set_max_working(home: &Path, max_working: usize) -> anyhow::Result<()> {
    let mut settings = Settings::load(home)?;
    settings.max_working = max_working;
    settings.save(home)?;
    Ok(())
}

/// One tick's outcome, for the UI.
#[derive(Debug, Clone, Default)]
pub struct Report {
    pub needs: Vec<NeedsYou>,
    /// Per workspace with a desired-state file: started, and the wanted session names.
    pub desired: BTreeMap<String, (bool, Vec<String>)>,
    pub max_working: usize,
    pub errors: Vec<String>,
    /// Per wanted name, the supervisor's last launch/resume attempt.
    pub last_attempt: HashMap<String, Millis>,
    /// Per workspace with a desired-state file: its messaging path.
    pub relays: BTreeMap<String, RelayStatus>,
    /// Per registered workspace, the last [`MESSAGES_SHOWN`] outbox messages, oldest first.
    pub messages: BTreeMap<String, Vec<OutboxMessage>>,
    /// Per workspace with messages, its `answered.json` (outbox file → the answer's first line).
    pub answered: BTreeMap<String, BTreeMap<String, String>>,
    /// Why no `claude --bg` from here can reach the background service ([`DaemonWatch`]); a tab
    /// that would run one says so.
    pub daemon_hold: Option<String>,
}

/// How often an unelevated Foreman asks `claude daemon status` whether it can reach the service.
pub const DAEMON_CHECK_EVERY: Millis = 15_000;

/// The app's check before launching (m5/25): [`foreman_claude::daemon_mismatch`] at most every
/// [`DAEMON_CHECK_EVERY`], at once after a Retry, its result handed to every supervisor. An elevated
/// Foreman reaches every service, and the check runs nothing for it.
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

    /// A supervisor's own launch found the mismatch: the others hold too, until the next check.
    pub fn found(&mut self, why: &str, now: Millis) {
        if self.why.is_none() {
            self.why = Some(why.to_string());
            self.checked_at = Some(now);
        }
    }
}

pub const MESSAGES_SHOWN: usize = 20;
/// Monitor expires after 30 min (m0/4); a relay that has not re-armed a minute later has lost it.
pub const REARM_WITHIN_MS: Millis = 31 * 60 * 1000;

/// What the workspace row says about its messaging path (m5/32): `direct` while no relay is wanted, else
/// how far the relay is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayStatus {
    pub started: bool,
    /// The desired state wants `fm-<w>-relay`: a message fell back to it within the idle timeout.
    pub wanted: bool,
    /// The `fm-<w>-relay` row is live ([`is_live`]).
    pub live: bool,
    /// When it last armed its watcher (`ready.json`); forgotten whenever the row is not live.
    pub ready_at: Option<Millis>,
}

impl RelayStatus {
    /// `(text, healthy)`; `None` when the workspace is stopped and its relay with it.
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
    /// The last [`NOTICES_KEPT`] supervisor notices (a failed revoke or PATCH), shown until the app restarts.
    notices: Vec<String>,
    pub daemon: DaemonWatch,
    /// The hold as last logged.
    daemon_logged: Option<String>,
}

pub const NOTICES_KEPT: usize = 5;

/// Foreman's own plugin first, then `settings.toml` `extra_plugin_dirs` in their order.
pub fn plugin_dirs(plugin_dir: Option<&Path>, settings: &Settings) -> Vec<PathBuf> {
    plugin_dir.map(Path::to_path_buf).into_iter().chain(settings.extra_plugin_dirs.iter().cloned()).collect()
}

impl Supervision {
    pub fn new(home: PathBuf, plugin_dir: Option<PathBuf>) -> Self {
        let config = LaunchConfig {
            mcp_dir: home.join("mcp"),
            plugin_dirs: plugin_dir.clone().into_iter().collect(),
            foreman_home: Some(home.clone()),
            bg_isolation_none: true,
            allowed_tools: plugin_dir.as_deref().map(fm_allowed_tools).unwrap_or_default(),
        };
        Self {
            home,
            plugin_dir,
            config,
            supervisors: BTreeMap::new(),
            notices: Vec::new(),
            daemon: DaemonWatch::new(foreman_claude::daemon_mismatch),
            daemon_logged: None,
        }
    }

    /// Re-reads settings and every registered workspace's desired state, applies the queued Retries
    /// (`(workspace, name)`), then ticks every supervisor on `observed`.
    pub fn tick(&mut self, workspaces: &[Workspace], observed: &[Session], retries: Vec<(String, String)>, now: Millis) -> Report {
        let mut report = Report::default();
        let settings = Settings::load(&self.home).unwrap_or_else(|e| {
            report.errors.push(format!("Cannot read the settings, using the defaults: {e}"));
            Settings::default()
        });
        report.max_working = settings.max_working;
        self.config.plugin_dirs = plugin_dirs(self.plugin_dir.as_deref(), &settings);
        self.supervisors.retain(|name, _| workspaces.iter().any(|p| &p.name == name));
        for p in workspaces {
            match DesiredState::load(&self.home, &p.name) {
                Ok(Some(desired)) => match self.supervisors.get_mut(&p.name) {
                    Some(s) => {
                        s.desired = desired;
                        s.config.plugin_dirs = self.config.plugin_dirs.clone();
                    }
                    None => {
                        let tokens = AppTokens { home: self.home.clone() };
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
                match sync_relay(&self.home, p, &s.desired, settings.relay_idle_ms(), now) {
                    Ok(Some(saved)) => {
                        log::info!("{}: relay {}", p.name, if saved.wants_relay() { "wanted (a message falls back)" } else { "retired (idle)" });
                        s.desired = saved;
                    }
                    Ok(None) => {}
                    Err(e) => report.errors.push(format!("Cannot start or retire the relay of {}: {e:#}", p.name)),
                }
            }
        }
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
                report.needs = tick_all(self.supervisors.values_mut(), &names, observed, settings.max_working, now);
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
        for s in self.supervisors.values_mut() {
            self.notices.extend(s.take_notices());
        }
        self.notices.drain(..self.notices.len().saturating_sub(NOTICES_KEPT));
        report.errors.extend(self.notices.iter().cloned());
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
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/foreman")
    }

    #[test]
    fn finds_the_plugin_above_the_exe() {
        let exe = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/release/foreman.exe");
        let found = find_plugin_dir(&exe).unwrap();
        assert!(found.join("roles").join("workspace.md").is_file(), "{}", found.display());
        assert!(find_plugin_dir(Path::new(r"C:\nowhere\foreman.exe")).is_err());
    }

    #[test]
    fn launches_get_extra_plugin_dirs_after_foremans_and_the_fm_rules() {
        let settings = Settings {
            extra_plugin_dirs: vec![r"C:\skills\plugins\rlarch".into()],
            ..Settings::default()
        };
        let own = Path::new(r"C:\fm\plugins\foreman");
        assert_eq!(plugin_dirs(Some(own), &settings), [own.to_path_buf(), r"C:\skills\plugins\rlarch".into()]);
        assert_eq!(plugin_dirs(None, &Settings::default()), Vec::<PathBuf>::new());
        assert_eq!(fm_allowed_tools(own), ["Bash(fm:*)", "Bash(C:/fm/plugins/foreman/bin/fm.exe:*)"]);
    }

    #[test]
    fn controller_roles_load_their_skills_as_rendered_and_injected() {
        let home = tempfile::tempdir().unwrap();
        for (role, skill, repo) in [(WORKSPACE_ROLE, "workspace-controller", None), (REPO_ROLE, "repo-controller", Some("Nomos.Pdf"))] {
            write_role_template(home.path(), &plugin_dir(), role).unwrap();
            let injected = foreman_core::role_injection_for(home.path(), role, Some("demo"), repo).unwrap();
            assert!(injected.contains(&format!("/foreman:{skill}")), "{injected}");
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
        assert!(launched.contains("/foreman:workspace-controller") && launched.contains("fm-demo-relay"), "{launched}");
        let repo = foreman_supervisor::render_repo_role(&std::fs::read_to_string(plugin_dir().join("roles").join("repo.md")).unwrap(), "demo", "Alpha");
        assert!(repo.contains("/foreman:repo-controller") && repo.contains("/rlarch:controller"), "{repo}");
    }

    #[test]
    fn start_keeps_repo_sessions_after_its_own() {
        let home = tempfile::tempdir().unwrap();
        write_role_template(home.path(), &plugin_dir(), RELAY_ROLE).unwrap();
        write_role_template(home.path(), &plugin_dir(), REPO_ROLE).unwrap();
        let rendered = std::fs::read_to_string(home.path().join("roles").join("repo.md")).unwrap();
        assert!(rendered.contains("{repo}") && rendered.contains("{workspace}"), "{rendered}");
        let alpha = foreman_supervisor::repo_session("demo", "Alpha", r"C:\demo\Alpha", "p").unwrap();
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

        // a relay wanted on demand, and a park
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
        assert_eq!(d.sessions[0].cwd, PathBuf::from(r"C:\moved"), "the relay follows the root");

        stop_workspace(home.path(), "demo").unwrap();
        let d = DesiredState::load(home.path(), "demo").unwrap().unwrap();
        assert!(!d.started);
        assert_eq!(d.sessions.len(), 3);
        stop_workspace(home.path(), "never-started").unwrap();
        assert!(DesiredState::load(home.path(), "never-started").unwrap().is_none());
        assert!(start_workspace(home.path(), None, &workspace).is_err());
    }

    #[test]
    fn the_relay_role_gets_its_paths_and_the_wanted_relay_its_dir() {
        let home = tempfile::tempdir().unwrap();
        write_role_template(home.path(), &plugin_dir(), RELAY_ROLE).unwrap();
        let rendered = std::fs::read_to_string(home.path().join("roles").join("relay.md")).unwrap();
        let fm = forward(&plugin_dir().join("bin").join("fm.exe"));
        let command = format!("\"{fm}\" inbox --dir \"{}/relay/{{workspace}}/inbox\"", forward(home.path()));
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

        // beta wants a relay that is not running: its stale ready.json is forgotten; alpha has a message
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

    /// m5/32: no relay by default; a fallback drop wants it (first), it stays wanted while a drop waits and
    /// until the idle timeout after the last one, then it is retired; an older desired state that lists it
    /// retires it the same way, and keeps a queued drop for the next relay.
    #[test]
    fn the_relay_is_wanted_on_a_fallback_and_retired_after_the_idle_timeout() {
        let home = tempfile::tempdir().unwrap();
        write_role_template(home.path(), &plugin_dir(), RELAY_ROLE).unwrap();
        let workspace = Workspace::new("demo", r"C:\demo", Vec::new());
        let idle = 60_000;
        let mut d = DesiredState::workspace_controller("demo", r"C:\demo", "p");
        d.started = true;
        d.save(home.path()).unwrap();
        let now = foreman_claude::now_ms();
        assert!(sync_relay(home.path(), &workspace, &d, idle, now).unwrap().is_none(), "no fallback: no relay");

        foreman_core::send(home.path(), "demo", "fm-demo-workspace", "fell back").unwrap();
        let d = sync_relay(home.path(), &workspace, &d, idle, now).unwrap().expect("wanted");
        assert_eq!(d.sessions[0].name, "fm-demo-relay");
        assert_eq!(DesiredState::load(home.path(), "demo").unwrap().unwrap(), d, "saved");
        assert!(sync_relay(home.path(), &workspace, &d, idle, now + 10 * idle).unwrap().is_none(), "the drop still waits");

        let inbox = foreman_core::inbox_dir(home.path(), "demo").unwrap();
        foreman_core::InboxWatcher::new(&inbox).scan().unwrap();
        let last = foreman_core::last_drop(home.path(), "demo").unwrap();
        assert!(sync_relay(home.path(), &workspace, &d, idle, last + idle - 1).unwrap().is_none(), "within the timeout");
        let d = sync_relay(home.path(), &workspace, &d, idle, last + idle).unwrap().expect("retired");
        assert!(!d.wants_relay());
        assert_eq!(d.retired, ["fm-demo-relay"]);
        assert_eq!(d.sessions.len(), 1);

        // upgrade: a desired state written before m5/32 lists the relay, with a drop queued
        let mut old = DesiredState::workspace_controller("demo", r"C:\demo", "p");
        old.started = true;
        old.sessions.insert(0, relay_wanted(home.path(), &workspace).unwrap());
        old.save(home.path()).unwrap();
        let queued = foreman_core::send(home.path(), "demo", "fm-demo-workspace", "queued").unwrap();
        let later = foreman_core::last_drop(home.path(), "demo").unwrap() + 2 * idle;
        assert!(sync_relay(home.path(), &workspace, &old, idle, later).unwrap().is_none(), "kept while the drop waits");
        assert!(queued.is_file());
        foreman_core::InboxWatcher::new(&inbox).scan().unwrap();
        let d = sync_relay(home.path(), &workspace, &old, idle, later).unwrap().expect("retired on upgrade");
        assert_eq!(d.retired, ["fm-demo-relay"]);
        // and without a drop, at once
        old.save(home.path()).unwrap();
        assert!(sync_relay(home.path(), &workspace, &old, idle, later).unwrap().is_some_and(|d| !d.wants_relay()));
    }

    static CHECKS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    fn counted_mismatch() -> Option<String> {
        CHECKS.fetch_add(1, Ordering::SeqCst);
        Some("runs as administrator".into())
    }

    /// m5/25: a started workspace whose service Foreman cannot reach is held before its first launch,
    /// with one Inbox row and `daemon_hold` for the tabs; the check runs every 15 s, and at once on Retry.
    #[test]
    fn an_unreachable_service_holds_a_started_workspace_before_any_launch() {
        let home = tempfile::tempdir().unwrap();
        let workspaces = [Workspace::new("beta", r"C:", Vec::new())];
        let mut d = DesiredState::workspace_controller("beta", r"C:", "p");
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
