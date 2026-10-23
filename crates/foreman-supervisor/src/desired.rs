use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Result, SupervisorError};

/// What the human asked for on one platform. Persisted at `<FOREMAN_HOME>/supervisor/<platform>.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesiredState {
    pub platform: String,
    /// Start/Stop. `false` stops every wanted session that is alive (stop, never rm).
    pub started: bool,
    #[serde(default)]
    pub sessions: Vec<WantedSession>,
}

/// One session the supervisor keeps alive while the platform is started. `name` is the join key
/// against `claude agents` rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WantedSession {
    pub name: String,
    pub role: String,
    /// Chosen by the app, not derived: `claude --bg` refuses an untrusted workspace (m1/5), and a
    /// platform root is usually not a git repo, so it is usually not trusted.
    pub cwd: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    pub prompt: String,
    /// `--permission-mode`; a role whose tools must park for the human uses `default`.
    #[serde(default = "default_permission_mode")]
    pub permission_mode: String,
}

pub const PLATFORM_ROLE: &str = "platform";
pub const DEFAULT_PERMISSION_MODE: &str = "auto";

fn default_permission_mode() -> String {
    DEFAULT_PERMISSION_MODE.into()
}

pub fn platform_session_name(platform: &str) -> String {
    format!("fm-{platform}-{PLATFORM_ROLE}")
}

impl DesiredState {
    pub fn new(platform: impl Into<String>) -> Self {
        Self {
            platform: platform.into(),
            started: false,
            sessions: Vec::new(),
        }
    }

    /// M2's desired state: one `fm-<platform>-platform` session, not started.
    pub fn platform_controller(platform: &str, cwd: impl Into<PathBuf>, prompt: impl Into<String>) -> Self {
        Self {
            sessions: vec![WantedSession {
                name: platform_session_name(platform),
                role: PLATFORM_ROLE.into(),
                cwd: cwd.into(),
                repo: None,
                prompt: prompt.into(),
                permission_mode: default_permission_mode(),
            }],
            ..Self::new(platform)
        }
    }

    pub fn path(home: &Path, platform: &str) -> Result<PathBuf> {
        foreman_core::validate_name(platform)?;
        Ok(home.join("supervisor").join(format!("{platform}.toml")))
    }

    /// `None` when the platform has no supervisor file yet.
    pub fn load(home: &Path, platform: &str) -> Result<Option<Self>> {
        let path = Self::path(home, platform)?;
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
        let path = Self::path(home, &self.platform)?;
        let text = toml::to_string(self).map_err(|source| SupervisorError::Serialize {
            platform: self.platform.clone(),
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

/// `<plugin_dir>/roles/<role>.md` with `{platform}` filled in.
pub fn role_prompt(plugin_dir: &Path, role: &str, platform: &str) -> Result<String> {
    let path = plugin_dir.join("roles").join(format!("{role}.md"));
    let template = fs::read_to_string(&path).map_err(|source| SupervisorError::Io { path, source })?;
    Ok(render_role(&template, platform))
}

pub fn render_role(template: &str, platform: &str) -> String {
    template.replace("{platform}", platform).trim().to_string()
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
        let mut desired = DesiredState::platform_controller("demo", r"C:\demo\Alpha", "be the controller");
        desired.started = true;
        desired.sessions.push(WantedSession {
            name: "fm-demo-alpha".into(),
            role: "repo".into(),
            cwd: r"C:\demo\Alpha".into(),
            repo: Some("Alpha".into()),
            prompt: "line one\nline \"two\"".into(),
            permission_mode: "default".into(),
        });
        let path = desired.save(home.path()).unwrap();
        assert_eq!(path, home.path().join("supervisor").join("demo.toml"));
        assert_eq!(DesiredState::load(home.path(), "demo").unwrap(), Some(desired));
    }

    #[test]
    fn a_file_without_permission_mode_reads_as_auto() {
        let home = tempfile::tempdir().unwrap();
        let path = DesiredState::platform_controller("demo", r"C:\d", "p").save(home.path()).unwrap();
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
    fn platform_controller_is_named_by_convention() {
        let d = DesiredState::platform_controller("nomos", r"C:\n", "p");
        assert!(!d.started);
        assert_eq!(d.sessions[0].name, "fm-nomos-platform");
        assert_eq!(d.sessions[0].role, "platform");
    }

    #[test]
    fn shipped_platform_role_renders() {
        let prompt = role_prompt(&plugin_dir(), PLATFORM_ROLE, "nomos").unwrap();
        assert!(prompt.starts_with("You are the platform controller for nomos"), "{prompt}");
        assert!(!prompt.contains("{platform}"));
        assert!(role_prompt(&plugin_dir(), "nope", "nomos").is_err());
    }
}
