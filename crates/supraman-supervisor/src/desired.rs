use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use supraman_core::FM_PREFIX;
use serde::{Deserialize, Serialize};

use crate::{Result, SupervisorError};

/// What the human asked for on one workspace. Persisted at `<FOREMAN_HOME>/supervisor/<workspace>.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesiredState {
    pub workspace: String,
    /// Start/Stop. `false` stops every wanted session that is alive (stop, never rm).
    pub started: bool,
    /// Names Foreman stopped wanting (`sm unwant`): a live row with one is stopped, never launched or
    /// resumed. A name wanted again leaves this list.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub retired: Vec<String>,
    #[serde(default)]
    pub sessions: Vec<WantedSession>,
}

/// One session the supervisor keeps alive while the workspace is started. `name` is the join key
/// against `claude agents` rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WantedSession {
    pub name: String,
    pub role: String,
    /// Chosen by the app, not derived: `claude --bg` refuses an untrusted workspace (m1/5), and a
    /// workspace root is usually not a git repo, so it is usually not trusted.
    pub cwd: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    pub prompt: String,
    /// `--permission-mode`; a role whose tools must park for the human uses `default`.
    #[serde(default = "default_permission_mode")]
    pub permission_mode: String,
    /// `--add-dir`: dirs outside `cwd` the session may write to (the relay's outbox).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub add_dirs: Vec<PathBuf>,
    /// The prompt a resume passes (`--bg --resume <id> -- <prompt>`). A resumed session gets no turn
    /// without one (m2/4), and a relay must re-arm its watcher.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_prompt: Option<String>,
    /// The full uuid of an adopted session: its launch passes `--resume <uuid>` with the supervised flags, a copy
    /// under a new id that carries the conversation (m5/61). Cleared once its row shows ([`DesiredState::forked`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_from: Option<String>,
}

impl WantedSession {
    /// The relay costs nothing while it waits on its watcher but may show `working` throughout (m0/4):
    /// it neither takes nor waits for a slot of the busy budget.
    pub fn exempt_from_budget(&self) -> bool {
        self.role == RELAY_ROLE
    }
}

pub const WORKSPACE_ROLE: &str = "workspace";
pub const RELAY_ROLE: &str = "relay";
/// A repo-controller: `fm-<workspace>-<repo slug>`, started on demand (`sm want`).
pub const REPO_ROLE: &str = "repo";
/// What a resumed relay is told: its `Monitor` died with the process.
pub const RELAY_RESUME_PROMPT: &str =
    "You were restarted and your Monitor is gone. Re-arm it now exactly as your instructions say (rewrite ready.json), then wait.";
pub const DEFAULT_PERMISSION_MODE: &str = "auto";
/// The newest retired names kept; an older one is no longer stopped when live.
pub const RETIRED_KEPT: usize = 100;

fn default_permission_mode() -> String {
    DEFAULT_PERMISSION_MODE.into()
}

pub fn workspace_session_name(workspace: &str) -> String {
    format!("{FM_PREFIX}{workspace}-{WORKSPACE_ROLE}")
}

pub fn relay_session_name(workspace: &str) -> String {
    format!("{FM_PREFIX}{workspace}-{RELAY_ROLE}")
}

/// An `fm-` session this home neither wants (`wanted`: the names in its desired states) nor names
/// (`fm-<workspace>-…` of one of `workspaces`): another `FOREMAN_HOME`'s, e.g. a selftest's.
pub fn is_foreign<'a>(name: &str, mut workspaces: impl Iterator<Item = &'a str>, mut wanted: impl Iterator<Item = &'a str>) -> bool {
    name.starts_with(FM_PREFIX) && !wanted.any(|w| w == name) && !workspaces.any(|w| name.starts_with(&format!("{FM_PREFIX}{w}-")))
}

/// `fm-<workspace>-relay` of one of `workspaces`: a relay row, by name alone (an unwanted one included).
pub fn is_relay<'a>(name: &str, mut workspaces: impl Iterator<Item = &'a str>) -> bool {
    workspaces.any(|w| name == relay_session_name(w))
}

/// `fm-<workspace>-relay`: cwd = the workspace root (trusted: the workspace session runs there),
/// `--add-dir` its relay dir (`<FOREMAN_HOME>/relay/<workspace>`, outside the cwd), auto mode, and
/// on resume told to re-arm.
pub fn relay_session(workspace: &str, cwd: impl Into<PathBuf>, relay_dir: impl Into<PathBuf>, prompt: impl Into<String>) -> WantedSession {
    WantedSession {
        name: relay_session_name(workspace),
        role: RELAY_ROLE.into(),
        cwd: cwd.into(),
        repo: None,
        prompt: prompt.into(),
        permission_mode: default_permission_mode(),
        add_dirs: vec![relay_dir.into()],
        resume_prompt: Some(RELAY_RESUME_PROMPT.into()),
        fork_from: None,
    }
}

/// `fm-<workspace>-<slug of repo>`. `None` when the slug is empty or would take the name of the
/// workspace's own sessions (`workspace`, `relay`) or of a scheduled run (`sched-…`).
pub fn repo_session_name(workspace: &str, repo: &str) -> Option<String> {
    let slug = supraman_core::slugify(repo);
    let reserved = slug == WORKSPACE_ROLE || slug == RELAY_ROLE || slug.starts_with(supraman_core::schedule::SCHED_SLUG_PREFIX);
    (!slug.is_empty() && !reserved).then(|| format!("{FM_PREFIX}{workspace}-{slug}"))
}

/// A repo-controller for `repo`: cwd = the repo's checkout (it merges there like a controller does, so the
/// launch config's `bgIsolation:"none"` applies; its nested agents work in worktrees), auto, role `repo`.
pub fn repo_session(workspace: &str, repo: &str, cwd: impl Into<PathBuf>, prompt: impl Into<String>) -> Option<WantedSession> {
    Some(WantedSession {
        name: repo_session_name(workspace, repo)?,
        role: REPO_ROLE.into(),
        cwd: cwd.into(),
        repo: Some(repo.into()),
        prompt: prompt.into(),
        permission_mode: default_permission_mode(),
        add_dirs: Vec::new(),
        resume_prompt: None,
        fork_from: None,
    })
}

/// [`repo_session`] that continues the conversation of `session_id` (a full uuid) instead of starting one.
pub fn adopted_session(workspace: &str, repo: &str, cwd: impl Into<PathBuf>, prompt: impl Into<String>, session_id: &str) -> Option<WantedSession> {
    Some(WantedSession {
        fork_from: Some(session_id.to_string()),
        ..repo_session(workspace, repo, cwd, prompt)?
    })
}

/// What the copy of an adopted session is told: the role itself comes with its start (`sm hook`'s FOREMAN-ROLE).
pub fn adopt_prompt(workspace: &str, w: &WantedSession) -> String {
    let what = match &w.repo {
        Some(repo) => format!("the {} session for {repo} in the Supraman workspace {workspace}", w.role),
        None => format!("the {} session of the Supraman workspace {workspace}", w.role),
    };
    format!(
        "Supraman adopted this conversation: from now on you are {}, {what}, supervised by Supraman. Follow the FOREMAN-ROLE instructions of this session's start as after a resume.",
        w.name
    )
}

/// What [`DesiredState::want`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wanted {
    Added,
    Updated,
    Unchanged,
}

impl DesiredState {
    pub fn new(workspace: impl Into<String>) -> Self {
        Self {
            workspace: workspace.into(),
            started: false,
            retired: Vec::new(),
            sessions: Vec::new(),
        }
    }

    /// Adds `w`, or replaces the wanted session of that name; either way it is no longer retired.
    pub fn want(&mut self, w: WantedSession) -> Wanted {
        self.retired.retain(|n| n != &w.name);
        match self.sessions.iter_mut().find(|s| s.name == w.name) {
            Some(s) if *s == w => Wanted::Unchanged,
            Some(s) => {
                *s = w;
                Wanted::Updated
            }
            None => {
                self.sessions.push(w);
                Wanted::Added
            }
        }
    }

    /// Removes the wanted session `name` and retires it, so a live one is stopped. False when it was
    /// not wanted (it is retired all the same).
    pub fn unwant(&mut self, name: &str) -> bool {
        let before = self.sessions.len();
        self.sessions.retain(|s| s.name != name);
        if !self.retired.iter().any(|n| n == name) {
            self.retired.push(name.to_string());
            self.retired.drain(..self.retired.len().saturating_sub(RETIRED_KEPT));
        }
        self.sessions.len() != before
    }

    /// The relay on demand: `Some` wants it (first, as the supervisor launches it before the
    /// others) unless it is wanted already; `None` retires a wanted one ([`DesiredState::unwant`], so R14
    /// stops it). True when this changed the state.
    pub fn set_relay(&mut self, relay: Option<WantedSession>) -> bool {
        let name = relay_session_name(&self.workspace);
        let wanted = self.sessions.iter().any(|w| w.name == name);
        match relay {
            Some(_) if wanted => false,
            Some(r) => {
                self.retired.retain(|n| n != &r.name);
                self.sessions.insert(0, r);
                true
            }
            None if wanted => self.unwant(&name),
            None => false,
        }
    }

    /// `name`'s row shows `session_id`, not the one it forks from: from now on it resumes as itself.
    pub fn forked(&mut self, name: &str, session_id: &str) -> bool {
        match self.sessions.iter_mut().find(|w| w.name == name) {
            Some(w) if w.fork_from.as_deref().is_some_and(|from| from != session_id) => {
                w.fork_from = None;
                true
            }
            _ => false,
        }
    }

    /// Start fresh after a failed adoption: `name` launches as a new session. False when it was not adopting.
    pub fn start_fresh(&mut self, name: &str) -> bool {
        self.sessions.iter_mut().find(|w| w.name == name).and_then(|w| w.fork_from.take()).is_some()
    }

    pub fn wants_relay(&self) -> bool {
        let name = relay_session_name(&self.workspace);
        self.sessions.iter().any(|w| w.name == name)
    }

    /// One `fm-<workspace>-workspace` session, not started.
    pub fn workspace_controller(workspace: &str, cwd: impl Into<PathBuf>, prompt: impl Into<String>) -> Self {
        Self {
            sessions: vec![WantedSession {
                name: workspace_session_name(workspace),
                role: WORKSPACE_ROLE.into(),
                cwd: cwd.into(),
                repo: None,
                prompt: prompt.into(),
                permission_mode: default_permission_mode(),
                add_dirs: Vec::new(),
                resume_prompt: None,
                fork_from: None,
            }],
            ..Self::new(workspace)
        }
    }

    /// The permission mode Foreman launches the wanted session `name` with, if it wants one: what
    /// `supraman_claude::human` needs to know before it writes into that session's pipe.
    pub fn launch_mode(home: &Path, workspace: &str, name: &str) -> Option<String> {
        let desired = Self::load(home, workspace).ok()??;
        desired.sessions.into_iter().find(|w| w.name == name).map(|w| w.permission_mode)
    }

    pub fn path(home: &Path, workspace: &str) -> Result<PathBuf> {
        supraman_core::validate_name(workspace)?;
        Ok(home.join("supervisor").join(format!("{workspace}.toml")))
    }

    /// `None` when the workspace has no supervisor file yet.
    pub fn load(home: &Path, workspace: &str) -> Result<Option<Self>> {
        let path = Self::path(home, workspace)?;
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(SupervisorError::Io { path, source }),
        };
        toml::from_str(&text)
            .map(Some)
            .map_err(|source| SupervisorError::Parse { path, source })
    }

    pub fn save(&self, home: &Path) -> Result<PathBuf> {
        let path = Self::path(home, &self.workspace)?;
        let text = toml::to_string(self).map_err(|source| SupervisorError::Serialize {
            workspace: self.workspace.clone(),
            source,
        })?;
        match supraman_core::fs::write_atomic(&path, text) {
            Ok(()) => Ok(path),
            Err(source) => Err(SupervisorError::Io { path, source }),
        }
    }

    /// `<FOREMAN_HOME>/supervisor/<workspace>.lock`, held by [`DesiredState::update`].
    pub fn lock_path(home: &Path, workspace: &str) -> Result<PathBuf> {
        Ok(Self::path(home, workspace)?.with_extension("lock"))
    }

    /// Read-modify-write against other writers (the app's Start/Stop, the poll thread, `sm want`), one at
    /// a time under [`DesiredState::lock_path`]: loads the file (or [`DesiredState::new`] when there is
    /// none), applies `edit` and saves.
    pub fn update<T>(home: &Path, workspace: &str, edit: impl FnOnce(&mut Self) -> T) -> Result<(Self, T)> {
        let lock = Self::lock_path(home, workspace)?;
        let _held = supraman_core::fs::lock_file(&lock).map_err(|source| SupervisorError::Io { path: lock, source })?;
        let mut desired = Self::load(home, workspace)?.unwrap_or_else(|| Self::new(workspace));
        let out = edit(&mut desired);
        desired.save(home)?;
        Ok((desired, out))
    }

    /// Unwants the repo session of each of `folders` that `workspace` wants, so the supervisor stops it (stop,
    /// never rm); returns their names. A workspace that wants none of them is left unwritten.
    pub fn release(home: &Path, workspace: &str, folders: &[&str]) -> Result<Vec<String>> {
        let names: Vec<String> = folders.iter().filter_map(|f| repo_session_name(workspace, f)).collect();
        let wanted = |d: &Self| names.iter().filter(|n| d.sessions.iter().any(|s| &s.name == *n)).cloned().collect::<Vec<_>>();
        if Self::load(home, workspace)?.is_none_or(|d| wanted(&d).is_empty()) {
            return Ok(Vec::new());
        }
        let (_, released) = Self::update(home, workspace, |d| {
            let released = wanted(d);
            for n in &released {
                d.unwant(n);
            }
            released
        })?;
        Ok(released)
    }
}

/// `<plugin_dir>/roles/<role>.md` with `{workspace}` filled in.
pub fn role_prompt(plugin_dir: &Path, role: &str, workspace: &str) -> Result<String> {
    let path = plugin_dir.join("roles").join(format!("{role}.md"));
    let template = fs::read_to_string(&path).map_err(|source| SupervisorError::Io { path, source })?;
    Ok(render_role(&template, workspace))
}

pub fn render_role(template: &str, workspace: &str) -> String {
    template.replace("{workspace}", workspace).trim().to_string()
}

/// [`render_role`] plus `{repo}`, for a repo session.
pub fn render_repo_role(template: &str, workspace: &str, repo: &str) -> String {
    render_role(&template.replace("{repo}", repo), workspace)
}

/// `{sm}` (this build's `sm.exe`) and `{home}` (`FOREMAN_HOME`) filled in, forward slashes: a role's
/// commands may run under Git Bash (m0/4). `{workspace}`/`{repo}` stay for [`render_role`] and the hook.
pub fn fill_paths(template: &str, sm: &Path, home: &Path) -> String {
    let forward = |p: &Path| p.display().to_string().replace('\\', "/");
    template.replace("{sm}", &forward(sm)).replace("{home}", &forward(home))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/supraman")
    }

    #[test]
    fn the_launch_mode_of_a_wanted_session_and_none_otherwise() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(DesiredState::launch_mode(home.path(), "demo", "fm-demo-workspace"), None);
        let mut desired = DesiredState::workspace_controller("demo", r"C:\demo", "p");
        let mut park = desired.sessions[0].clone();
        park.name = "fm-demo-park".into();
        park.permission_mode = "bypassPermissions".into();
        desired.sessions.push(park);
        desired.save(home.path()).unwrap();
        assert_eq!(DesiredState::launch_mode(home.path(), "demo", "fm-demo-workspace").as_deref(), Some("auto"));
        assert_eq!(DesiredState::launch_mode(home.path(), "demo", "fm-demo-park").as_deref(), Some("bypassPermissions"));
        assert_eq!(DesiredState::launch_mode(home.path(), "demo", "fm-demo-relay"), None, "not wanted: not known");
    }

    #[test]
    fn round_trips_through_toml_on_disk() {
        let home = tempfile::tempdir().unwrap();
        let mut desired = DesiredState::workspace_controller("demo", r"C:\demo\Alpha", "be the controller");
        desired.started = true;
        desired.sessions.push(WantedSession {
            name: "fm-demo-alpha".into(),
            role: "repo".into(),
            cwd: r"C:\demo\Alpha".into(),
            repo: Some("Alpha".into()),
            prompt: "line one\nline \"two\"".into(),
            permission_mode: "default".into(),
            add_dirs: Vec::new(),
            resume_prompt: None,
            fork_from: None,
        });
        desired.sessions.insert(0, relay_session("demo", r"C:\demo", r"C:\fm\relay\demo", "relay"));
        let path = desired.save(home.path()).unwrap();
        assert_eq!(path, home.path().join("supervisor").join("demo.toml"));
        assert_eq!(DesiredState::load(home.path(), "demo").unwrap(), Some(desired));
    }

    #[test]
    fn a_file_without_permission_mode_reads_as_auto() {
        let home = tempfile::tempdir().unwrap();
        let path = DesiredState::workspace_controller("demo", r"C:\d", "p").save(home.path()).unwrap();
        let text = fs::read_to_string(&path).unwrap().replace("permission_mode = \"auto\"\n", "");
        assert!(!text.contains("permission_mode"), "{text}");
        fs::write(&path, text).unwrap();
        let loaded = DesiredState::load(home.path(), "demo").unwrap().unwrap();
        assert_eq!(loaded.sessions[0].permission_mode, "auto");
    }

    #[test]
    fn absent_file_is_none_and_bad_names_are_refused() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(DesiredState::load(home.path(), "demo").unwrap(), None);
        assert!(DesiredState::load(home.path(), "../x").is_err());
        assert!(DesiredState::new("Bad Name").save(home.path()).is_err());
    }

    #[test]
    fn corrupt_file_names_the_path() {
        let home = tempfile::tempdir().unwrap();
        let path = DesiredState::new("demo").save(home.path()).unwrap();
        fs::write(&path, "started = \"yes\"").unwrap();
        match DesiredState::load(home.path(), "demo") {
            Err(SupervisorError::Parse { path: p, .. }) => assert_eq!(p, path),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn set_relay_wants_it_first_and_retires_it() {
        let mut d = DesiredState::workspace_controller("demo", r"C:\demo", "p");
        let relay = relay_session("demo", r"C:\demo", r"C:\fm\relay\demo", "r");
        assert!(!d.set_relay(None), "not wanted: nothing to retire");
        assert!(d.retired.is_empty());
        assert!(d.set_relay(Some(relay.clone())));
        assert_eq!(d.sessions[0].name, "fm-demo-relay");
        assert!(d.wants_relay());
        assert!(!d.set_relay(Some(relay.clone())), "wanted already");
        assert!(d.set_relay(None));
        assert!(!d.wants_relay());
        assert_eq!(d.retired, ["fm-demo-relay"]);
        assert_eq!(d.sessions.len(), 1, "the workspace session stays");
        assert!(d.set_relay(Some(relay)));
        assert!(d.retired.is_empty(), "wanted again: no longer retired");
    }

    #[test]
    fn retired_keeps_the_newest_names_only() {
        let mut d = DesiredState::workspace_controller("demo", r"C:\demo", "p");
        for i in 0..RETIRED_KEPT + 5 {
            d.unwant(&format!("fm-demo-r{i}"));
        }
        assert_eq!(d.retired.len(), RETIRED_KEPT);
        assert_eq!(d.retired[0], "fm-demo-r5");
        assert_eq!(d.retired.last().map(String::as_str), Some(format!("fm-demo-r{}", RETIRED_KEPT + 4).as_str()));
    }

    #[test]
    fn relay_is_named_by_convention_and_exempt_from_the_budget() {
        let r = relay_session("nomos", r"C:\n", r"C:\fm\relay\nomos", "p");
        assert_eq!(r.name, "fm-nomos-relay");
        assert_eq!(r.role, "relay");
        assert_eq!(r.permission_mode, "auto");
        assert_eq!(r.add_dirs, [PathBuf::from(r"C:\fm\relay\nomos")]);
        assert!(r.resume_prompt.as_deref().is_some_and(|p| p.contains("Re-arm")));
        assert!(r.exempt_from_budget());
        assert!(!DesiredState::workspace_controller("nomos", r"C:\n", "p").sessions[0].exempt_from_budget());
    }

    #[test]
    fn shipped_relay_role_renders_with_its_paths() {
        let template = fs::read_to_string(plugin_dir().join("roles").join("relay.md")).unwrap();
        for placeholder in ["{workspace}", "{sm}", "{home}"] {
            assert!(template.contains(placeholder), "{placeholder}");
        }
        let prompt = render_role(&template, "nomos");
        assert!(prompt.contains("relay/nomos/inbox"), "{prompt}");
        assert!(prompt.contains("SendMessage"), "{prompt}");
    }

    #[test]
    fn repo_sessions_are_named_by_slug_and_never_take_a_workspace_name() {
        assert_eq!(repo_session_name("nomos", "Nomos.Extract").as_deref(), Some("fm-nomos-nomos-extract"));
        assert_eq!(repo_session_name("nomos", "Relay"), None);
        assert_eq!(repo_session_name("nomos", "workspace"), None);
        assert_eq!(repo_session_name("nomos", "..."), None);
        assert_eq!(repo_session_name("nomos", "Sched.Nightly"), None, "fm-<ws>-sched-… is a scheduled run's");
        assert_eq!(repo_session_name("nomos", "scheduler").as_deref(), Some("fm-nomos-scheduler"));
        let r = repo_session("nomos", "Nomos.Extract", r"C:\n\Nomos.Extract", "p").unwrap();
        assert_eq!((r.role.as_str(), r.repo.as_deref(), r.permission_mode.as_str()), ("repo", Some("Nomos.Extract"), "auto"));
        assert!(!r.exempt_from_budget());
    }

    #[test]
    fn release_unwants_only_the_wanted_sessions_of_the_folders_and_writes_nothing_otherwise() {
        let home = tempfile::tempdir().unwrap();
        assert!(DesiredState::release(home.path(), "demo", &["Alpha"]).unwrap().is_empty());
        assert!(!DesiredState::path(home.path(), "demo").unwrap().exists(), "no desired state is made up");
        DesiredState::update(home.path(), "demo", |d| {
            d.want(repo_session("demo", "Alpha", r"C:\d\Alpha", "p").unwrap());
            d.want(repo_session("demo", "Beta", r"C:\d\Beta", "p").unwrap());
        })
        .unwrap();
        assert_eq!(DesiredState::release(home.path(), "demo", &["Alpha", "Gamma"]).unwrap(), ["fm-demo-alpha"]);
        let d = DesiredState::load(home.path(), "demo").unwrap().unwrap();
        assert_eq!(d.sessions.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["fm-demo-beta"]);
        assert_eq!(d.retired, ["fm-demo-alpha"], "retired, so the supervisor stops a live one");
        assert!(DesiredState::release(home.path(), "demo", &["Alpha"]).unwrap().is_empty());
    }

    #[test]
    fn want_is_idempotent_and_unwant_retires() {
        let mut d = DesiredState::new("demo");
        let r = repo_session("demo", "Alpha", r"C:\demo\Alpha", "p").unwrap();
        assert_eq!(d.want(r.clone()), Wanted::Added);
        assert_eq!(d.want(r.clone()), Wanted::Unchanged);
        assert_eq!(d.want(WantedSession { prompt: "p2".into(), ..r.clone() }), Wanted::Updated);
        assert_eq!(d.sessions.len(), 1);
        assert!(d.unwant("fm-demo-alpha"));
        assert!(!d.unwant("fm-demo-alpha"));
        assert_eq!((d.sessions.len(), d.retired.as_slice()), (0, ["fm-demo-alpha".to_string()].as_slice()));
        let home = tempfile::tempdir().unwrap();
        d.save(home.path()).unwrap();
        assert_eq!(DesiredState::load(home.path(), "demo").unwrap(), Some(d.clone()));
        assert_eq!(d.want(r), Wanted::Added);
        assert!(d.retired.is_empty());
    }

    #[test]
    fn concurrent_updates_lose_no_edit() {
        let home = tempfile::tempdir().unwrap();
        let writers: Vec<_> = (0..6)
            .map(|i| {
                let home = home.path().to_path_buf();
                std::thread::spawn(move || {
                    for n in 0..15 {
                        let repo = format!("r{i}x{n}");
                        DesiredState::update(&home, "demo", |d| d.want(repo_session("demo", &repo, r"C:\demo", "p").unwrap())).unwrap();
                    }
                })
            })
            .collect();
        for w in writers {
            w.join().unwrap();
        }
        let d = DesiredState::load(home.path(), "demo").unwrap().unwrap();
        assert_eq!(d.sessions.len(), 6 * 15, "every writer's want is kept");
    }

    #[test]
    fn update_creates_a_missing_file_and_leaves_only_it_and_its_lock() {
        let home = tempfile::tempdir().unwrap();
        let (d, added) = DesiredState::update(home.path(), "demo", |d| d.want(repo_session("demo", "Alpha", r"C:\demo\Alpha", "p").unwrap())).unwrap();
        assert_eq!(added, Wanted::Added);
        assert_eq!(DesiredState::load(home.path(), "demo").unwrap(), Some(d));
        let mut names: Vec<String> =
            fs::read_dir(home.path().join("supervisor")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, ["demo.lock", "demo.toml"]);
    }

    #[test]
    fn repo_role_fills_repo_and_paths() {
        let template = "Repo {repo} of {workspace}; run {sm} roster {workspace}; home {home}";
        let filled = fill_paths(template, Path::new(r"C:\fm\bin\sm.exe"), Path::new(r"C:\fm home"));
        assert_eq!(
            render_repo_role(&filled, "demo", "Alpha"),
            "Repo Alpha of demo; run C:/fm/bin/sm.exe roster demo; home C:/fm home"
        );
        let shipped = fs::read_to_string(plugin_dir().join("roles").join("repo.md")).unwrap();
        for placeholder in ["{workspace}", "{repo}"] {
            assert!(shipped.contains(placeholder), "{placeholder}");
        }
    }

    #[test]
    fn workspace_controller_is_named_by_convention() {
        let d = DesiredState::workspace_controller("nomos", r"C:\n", "p");
        assert!(!d.started);
        assert_eq!(d.sessions[0].name, "fm-nomos-workspace");
        assert_eq!(d.sessions[0].role, "workspace");
    }

    #[test]
    fn shipped_workspace_role_renders() {
        let prompt = role_prompt(&plugin_dir(), WORKSPACE_ROLE, "nomos").unwrap();
        assert!(prompt.starts_with("You are the workspace controller for nomos"), "{prompt}");
        assert!(!prompt.contains("{workspace}"));
        assert!(role_prompt(&plugin_dir(), "nope", "nomos").is_err());
    }

    #[test]
    fn an_adopted_session_forks_until_its_own_row_shows_and_start_fresh_drops_the_fork() {
        let home = tempfile::tempdir().unwrap();
        let from = "fc5bf027-48e3-4510-90b2-63a00eb76df8";
        let adopted = adopted_session("demo", "Alpha", r"C:\demo\Alpha", "p", from).unwrap();
        assert_eq!((adopted.name.as_str(), adopted.role.as_str(), adopted.fork_from.as_deref()), ("fm-demo-alpha", REPO_ROLE, Some(from)));
        let prompt = adopt_prompt("demo", &adopted);
        assert!(prompt.contains("fm-demo-alpha") && prompt.contains("Alpha") && prompt.contains("FOREMAN-ROLE"), "{prompt}");
        let (saved, _) = DesiredState::update(home.path(), "demo", |d| d.want(adopted.clone())).unwrap();
        assert_eq!(DesiredState::load(home.path(), "demo").unwrap(), Some(saved.clone()));
        let mut d = saved;
        assert!(!d.forked("fm-demo-alpha", from), "the row still shows the original's uuid");
        assert!(!d.forked("fm-demo-beta", "other"), "not wanted");
        assert!(d.forked("fm-demo-alpha", "f050c443-6aa7-47b2-a9e4-63a5b6e9d04d"));
        assert_eq!(d.sessions[0].fork_from, None);
        assert!(!d.forked("fm-demo-alpha", "f050c443-6aa7-47b2-a9e4-63a5b6e9d04d"), "once");
        d.want(adopted);
        assert!(d.start_fresh("fm-demo-alpha"));
        assert!(!d.start_fresh("fm-demo-alpha"));
        assert_eq!(d.sessions[0].fork_from, None);
    }
}
