//! First run (m5/3): whether the sessions Foreman launches will have WorkState, and if not, what is
//! missing and how to fix it — one wording for the notice above the tree, Settings → WorkState and the
//! Work tab. [`detect_local`] reads `settings.toml` and checks the `.env` for its keys (presence only,
//! never a value); [`detect`] adds one `GET /health`, so it runs off the UI thread.

use std::path::{Path, PathBuf};
use std::time::Duration;

use foreman_core::{DEFAULT_WORKSTATE_URL, Settings, has_env_key};

use crate::tokens::{token_key, workstate_role};

/// Test hook: the folder `rlarch:workstate-setup` installs WorkState into, instead of `~/.workstate`.
pub const INSTALL_DIR_VAR: &str = "FOREMAN_WORKSTATE_INSTALL";
pub const SETUP_SKILL: &str = "rlarch:workstate-setup";
/// How often the status is checked again while the app runs.
pub const RECHECK_EVERY: Duration = Duration::from_secs(30);
const HEALTH_TIMEOUT: Duration = Duration::from_secs(3);

/// The roles whose sessions want WorkState, each with its own key and the key of the WorkState role it acts as.
const ROLES: [&str; 2] = ["workspace", "repo"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WsStatus {
    Checking,
    Ready { url: String },
    /// (a) No `workstate_env`; `offer` is the setup skill's `.env` when it exists.
    NoEnv { offer: Option<PathBuf> },
    /// (b) The named `.env` cannot be read.
    EnvUnreadable { path: PathBuf, reason: String },
    /// (b) The `.env` has no line for some roles: their keys, e.g. `WORKSTATE_CONTROLLER_TOKEN`.
    MissingKeys { path: PathBuf, roles: Vec<&'static str>, keys: Vec<String> },
    /// (c) The `.env` is fine but `/health` did not answer 2xx.
    Unreachable { url: String, error: String },
}

/// `<FOREMAN_WORKSTATE_INSTALL or ~/.workstate>/.env`, where `rlarch:workstate-setup` writes it.
pub fn installed_env() -> Option<PathBuf> {
    let dir = match std::env::var_os(INSTALL_DIR_VAR) {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?).join(".workstate"),
    };
    Some(dir.join(".env"))
}

/// Everything but the server: (a) and (b), or `Ready` meaning "the files are fine".
pub fn detect_local(settings: &Settings, installed: Option<&Path>) -> WsStatus {
    let url = settings.workstate_url.clone().unwrap_or_else(|| DEFAULT_WORKSTATE_URL.into());
    let Some(path) = settings.workstate_env.clone() else {
        return WsStatus::NoEnv { offer: installed.filter(|p| p.is_file()).map(Path::to_path_buf) };
    };
    let (mut roles, mut keys) = (Vec::new(), Vec::new());
    for role in ROLES {
        let fallback = workstate_role(role).map(token_key);
        let found = match has_env_key(&path, &token_key(role)) {
            Ok(true) => true,
            Ok(false) => fallback.as_deref().is_some_and(|key| has_env_key(&path, key).unwrap_or(false)),
            Err(e) => {
                let reason = if e.kind() == std::io::ErrorKind::NotFound { "no such file".to_string() } else { e.to_string() };
                return WsStatus::EnvUnreadable { path, reason };
            }
        };
        if !found {
            roles.push(role);
            keys.push(fallback.unwrap_or_else(|| token_key(role)));
        }
    }
    if roles.is_empty() { WsStatus::Ready { url } } else { WsStatus::MissingKeys { path, roles, keys } }
}

/// [`detect_local`], then `GET /health` when the files are fine. Blocks up to a few seconds.
pub fn detect(home: Option<&Path>, installed: Option<&Path>) -> WsStatus {
    let settings = match home.map(Settings::load) {
        Some(Ok(settings)) => settings,
        // no home or an unreadable settings.toml is reported elsewhere (the setup notices, Settings)
        _ => Settings::default(),
    };
    match detect_local(&settings, installed) {
        WsStatus::Ready { url } => match foreman_workstate::health(&url, HEALTH_TIMEOUT) {
            Ok(()) => WsStatus::Ready { url },
            Err(error) => WsStatus::Unreachable { url, error },
        },
        other => other,
    }
}

/// `C:\Users\x\.workstate\.env` → `~/.workstate/.env`; any other path as it is.
pub fn tilde(path: &Path) -> String {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from);
    match home.as_deref().and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) => format!("~/{}", rest.display().to_string().replace('\\', "/")),
        None => path.display().to_string(),
    }
}

impl WsStatus {
    /// What is wrong, in one sentence; `None` when nothing is (or not known yet).
    pub fn problem(&self) -> Option<String> {
        match self {
            WsStatus::Checking | WsStatus::Ready { .. } => None,
            WsStatus::NoEnv { .. } => {
                Some("WorkState is not set up: settings.toml names no WorkState .env, so sessions run without WorkState.".into())
            }
            WsStatus::EnvUnreadable { path, reason } => Some(format!(
                "WorkState's .env {} cannot be read ({reason}), so sessions run without WorkState.",
                path.display()
            )),
            WsStatus::MissingKeys { path, roles, keys } => Some(format!(
                "{} has no {} line, so {} sessions run without WorkState.",
                path.display(),
                keys.join(" or "),
                roles.join(" and ")
            )),
            WsStatus::Unreachable { url, error } => Some(format!("WorkState at {url} is unreachable ({error}).")),
        }
    }

    /// How to fix it, in one sentence.
    pub fn fix(&self) -> Option<String> {
        let choose = format!(
            "Choose the .env {SETUP_SKILL} wrote (~/.workstate/.env, or <clone>/mcp/.env) in Settings → WorkState, or install WorkState with the {SETUP_SKILL} skill."
        );
        match self {
            WsStatus::Checking | WsStatus::Ready { .. } => None,
            WsStatus::NoEnv { offer: Some(path) } => Some(format!("Found {}: use it, or choose another .env in Settings → WorkState.", tilde(path))),
            WsStatus::NoEnv { offer: None } => Some(format!(
                "Install WorkState with the {SETUP_SKILL} skill, or choose an existing .env in Settings → WorkState."
            )),
            WsStatus::EnvUnreadable { .. } | WsStatus::MissingKeys { .. } => Some(choose),
            WsStatus::Unreachable { .. } => Some(format!(
                "Start it with the {SETUP_SKILL} skill, or docker compose up in its install folder. Checked again every {} s.",
                RECHECK_EVERY.as_secs()
            )),
        }
    }

    /// `problem` and `fix` as one paragraph: the Work tab's message.
    pub fn message(&self) -> Option<String> {
        Some(format!("{} {}", self.problem()?, self.fix()?))
    }

    /// The `.env` the notice offers with one click.
    pub fn offer(&self) -> Option<&Path> {
        match self {
            WsStatus::NoEnv { offer } => offer.as_deref(),
            _ => None,
        }
    }

    pub fn unreachable(&self) -> bool {
        matches!(self, WsStatus::Unreachable { .. })
    }

    /// The line Start shows when `workspace` was started anyway: it never blocks.
    pub fn start_warning(&self, workspace: &str) -> Option<String> {
        match self {
            WsStatus::Checking | WsStatus::Ready { .. } => None,
            WsStatus::Unreachable { url, .. } => {
                Some(format!("Started {workspace}; WorkState at {url} is unreachable, so its sessions run without it."))
            }
            _ => Some(format!("Started {workspace} without WorkState: its sessions can talk, not plan.")),
        }
    }
}

#[cfg(test)]
mod tests {
    use foreman_workstate::fake::{Answer, FakeWorkState};

    use super::*;

    fn settings(env: Option<PathBuf>, url: Option<String>) -> Settings {
        Settings { workstate_env: env, workstate_url: url, ..Settings::default() }
    }

    #[test]
    fn no_env_offers_the_installed_env_only_when_it_exists() {
        let dir = tempfile::tempdir().unwrap();
        let installed = dir.path().join(".workstate").join(".env");
        let none = detect_local(&settings(None, None), Some(&installed));
        assert_eq!(none, WsStatus::NoEnv { offer: None });
        assert!(none.fix().unwrap().contains(SETUP_SKILL), "{:?}", none.fix());
        assert!(none.message().unwrap().contains("settings.toml names no WorkState .env"));

        std::fs::create_dir_all(installed.parent().unwrap()).unwrap();
        std::fs::write(&installed, "WORKSTATE_CONTROLLER_TOKEN=fake-c\n").unwrap();
        let offered = detect_local(&settings(None, None), Some(&installed));
        assert_eq!(offered.offer(), Some(installed.as_path()));
        assert!(offered.start_warning("demo").unwrap().contains("without WorkState"));
    }

    #[test]
    fn a_missing_or_keyless_env_names_what_is_missing_and_never_a_value() {
        let dir = tempfile::tempdir().unwrap();
        let env = dir.path().join("ws.env");
        let gone = detect_local(&settings(Some(env.clone()), None), None);
        assert_eq!(gone, WsStatus::EnvUnreadable { path: env.clone(), reason: "no such file".into() });

        std::fs::write(&env, "WORKSTATE_READER_TOKEN=fake-reader-secret\nWORKSTATE_CONTROLLER_TOKEN=\n").unwrap();
        let none = detect_local(&settings(Some(env.clone()), None), None);
        let WsStatus::MissingKeys { roles, keys, .. } = &none else { panic!("{none:?}") };
        assert_eq!((roles.as_slice(), keys.as_slice()), (&["workspace", "repo"][..], &["WORKSTATE_CONTROLLER_TOKEN".to_string(), "WORKSTATE_ORCHESTRATOR_TOKEN".into()][..]));
        let text = format!("{} {:?}", none.message().unwrap(), none);
        assert!(!text.contains("fake-reader-secret"), "{text}");

        std::fs::write(&env, "WORKSTATE_CONTROLLER_TOKEN=fake-c\n").unwrap();
        let half = detect_local(&settings(Some(env.clone()), None), None);
        assert!(matches!(&half, WsStatus::MissingKeys { roles, .. } if roles == &["repo"]), "{half:?}");
        assert!(half.problem().unwrap().contains("WORKSTATE_ORCHESTRATOR_TOKEN"));

        std::fs::write(&env, "WORKSTATE_WORKSPACE_TOKEN=fake-w\nWORKSTATE_ORCHESTRATOR_TOKEN=fake-o\n").unwrap();
        assert_eq!(detect_local(&settings(Some(env), None), None), WsStatus::Ready { url: DEFAULT_WORKSTATE_URL.into() });
    }

    #[test]
    fn a_complete_env_asks_the_server_a_closed_port_is_unreachable() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let env = dir.path().join("ws.env");
        std::fs::write(&env, "WORKSTATE_CONTROLLER_TOKEN=fake-c\nWORKSTATE_ORCHESTRATOR_TOKEN=fake-o\n").unwrap();
        let fake = FakeWorkState::start(vec![("/health".into(), Answer::text("Healthy"))]);
        settings(Some(env.clone()), Some(fake.mcp_url())).save(&home).unwrap();
        assert_eq!(detect(Some(&home), None), WsStatus::Ready { url: fake.mcp_url() });
        assert!(fake.seen().iter().all(|s| s.authorization.is_none()), "/health carries no token");

        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let closed = format!("http://127.0.0.1:{port}/mcp");
        settings(Some(env), Some(closed.clone())).save(&home).unwrap();
        let down = detect(Some(&home), None);
        assert_eq!(down, WsStatus::Unreachable { url: closed.clone(), error: "connection refused".into() });
        assert!(down.problem().unwrap().contains(&closed) && down.problem().unwrap().contains("connection refused"));
        assert!(down.fix().unwrap().contains("docker compose up"));
        assert!(down.start_warning("demo").unwrap().contains("unreachable"));
        assert_eq!(WsStatus::Ready { url: closed }.start_warning("demo"), None);
    }

    #[test]
    fn tilde_shortens_the_profile_only() {
        let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) else { return };
        assert_eq!(tilde(&PathBuf::from(home).join(".workstate").join(".env")), "~/.workstate/.env");
        assert_eq!(tilde(Path::new(r"Q:\elsewhere\.env")), r"Q:\elsewhere\.env");
    }
}
