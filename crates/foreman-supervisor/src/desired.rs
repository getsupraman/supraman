use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Result, SupervisorError};

/// What the human asked for on one workspace. Persisted at `<FOREMAN_HOME>/supervisor/<workspace>.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesiredState {
    /// `platform` in a file written before m2x/2 (`foreman_core::migrate_home` rewrites it).
    #[serde(alias = "platform")]
    pub workspace: String,
    /// Start/Stop. `false` stops every wanted session that is alive (stop, never rm).
    pub started: bool,
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
/// What a resumed relay is told: its `Monitor` died with the process.
pub const RELAY_RESUME_PROMPT: &str =
    "You were restarted and your Monitor is gone. Re-arm it now exactly as your instructions say (rewrite ready.json), then wait.";
pub const DEFAULT_PERMISSION_MODE: &str = "auto";

fn default_permission_mode() -> String {
    DEFAULT_PERMISSION_MODE.into()
}

pub fn workspace_session_name(workspace: &str) -> String {
    format!("fm-{workspace}-{WORKSPACE_ROLE}")
}

pub fn relay_session_name(workspace: &str) -> String {
    format!("fm-{workspace}-{RELAY_ROLE}")
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
    }
}

impl DesiredState {
    pub fn new(workspace: impl Into<String>) -> Self {
        Self {
            workspace: workspace.into(),
            started: false,
            sessions: Vec::new(),
        }
    }

    /// M2's desired state: one `fm-<workspace>-workspace` session, not started.
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
            }],
            ..Self::new(workspace)
        }
    }

    pub fn path(home: &Path, workspace: &str) -> Result<PathBuf> {
        foreman_core::validate_name(workspace)?;
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
        let io = |path: &Path| {
            let path = path.to_path_buf();
            move |source| SupervisorError::Io { path, source }
        };
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(io(dir))?;
        }
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, text).map_err(io(&tmp))?;
        fs::rename(&tmp, &path).map_err(io(&path))?;
        Ok(path)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/foreman")
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
    fn a_file_from_before_the_rename_loads_and_migrates_to_the_current_shape() {
        let home = tempfile::tempdir().unwrap();
        let mut old = DesiredState::workspace_controller("nomos", r"C:\n", "p");
        old.started = true;
        let path = old.save(home.path()).unwrap();
        let text = fs::read_to_string(&path)
            .unwrap()
            .replace("workspace = \"nomos\"", "platform = \"nomos\"")
            .replace("fm-nomos-workspace", "fm-nomos-platform")
            .replace("role = \"workspace\"", "role = \"platform\"");
        assert!(!text.contains("workspace"), "{text}");
        fs::write(&path, text).unwrap();
        let read = DesiredState::load(home.path(), "nomos").unwrap().unwrap();
        assert_eq!(read.workspace, "nomos", "serde alias");

        let notices = foreman_core::migrate_home(home.path()).unwrap();
        assert_eq!(notices.len(), 1, "{notices:?}");
        assert!(notices[0].contains("fm-nomos-platform"), "{}", notices[0]);
        old.started = false;
        assert_eq!(DesiredState::load(home.path(), "nomos").unwrap(), Some(old));
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
        for placeholder in ["{workspace}", "{fm}", "{home}"] {
            assert!(template.contains(placeholder), "{placeholder}");
        }
        let prompt = render_role(&template, "nomos");
        assert!(prompt.contains("relay/nomos/inbox"), "{prompt}");
        assert!(prompt.contains("SendMessage"), "{prompt}");
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
}
