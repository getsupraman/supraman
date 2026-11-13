use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub const HOME_VAR: &str = "SUPRAMAN_HOME";
pub const HOME_DIR: &str = "Supraman";

fn non_empty(v: Option<OsString>) -> Option<OsString> {
    v.filter(|v| !v.is_empty())
}

/// `SUPRAMAN_HOME`, else `%APPDATA%\Supraman`; an empty value counts as unset.
pub fn home_from(supraman_home: Option<OsString>, appdata: Option<OsString>) -> Option<PathBuf> {
    non_empty(supraman_home).map(PathBuf::from).or_else(|| default_home(appdata))
}

pub fn default_home(appdata: Option<OsString>) -> Option<PathBuf> {
    non_empty(appdata).map(|a| PathBuf::from(a).join(HOME_DIR))
}

/// The home an env var names, without the `%APPDATA%` default: a hook's or a session's.
pub fn env_home() -> Option<PathBuf> {
    non_empty(std::env::var_os(HOME_VAR)).map(PathBuf::from)
}

pub fn home_from_env() -> Option<PathBuf> {
    home_from(std::env::var_os(HOME_VAR), crate::platform::paths::data_dir())
}

/// `%APPDATA%\Supraman`, whatever `SUPRAMAN_HOME` says.
pub fn default_home_from_env() -> Option<PathBuf> {
    default_home(crate::platform::paths::data_dir())
}

/// For a session's or a script's environment.
pub fn home_env(home: &Path) -> [(&'static str, String); 1] {
    [(HOME_VAR, home.display().to_string())]
}

pub const ROLE_VAR: &str = "SUPRAMAN_ROLE";
pub const SESSION_VAR: &str = "SUPRAMAN_SESSION";
pub const REPO_VAR: &str = "SUPRAMAN_REPO";

/// A Supraman session's `SUPRAMAN_*` env beside its home: the one writer (`vars`) and reader (`from_env`) of these four.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionEnv {
    pub role: Option<String>,
    /// The session's name, what `sm tell` signs with.
    pub session: Option<String>,
    pub repo: Option<String>,
}

impl SessionEnv {
    /// This process's; a var unset or not Unicode is `None`, an empty one kept.
    pub fn from_env() -> Self {
        let var = |k| std::env::var(k).ok();
        Self { role: var(ROLE_VAR), session: var(SESSION_VAR), repo: var(REPO_VAR) }
    }

    /// This process's `SUPRAMAN_ROLE` as the OS holds it, Unicode or not.
    pub fn role_os() -> Option<OsString> {
        std::env::var_os(ROLE_VAR)
    }

    /// The vars to set for a child, in the order role, session, repo; a `None` is left out.
    pub fn vars(&self) -> Vec<(&'static str, String)> {
        [(ROLE_VAR, &self.role), (SESSION_VAR, &self.session), (REPO_VAR, &self.repo)]
            .into_iter()
            .filter_map(|(k, v)| v.clone().map(|v| (k, v)))
            .collect()
    }
}

/// Where a Send's prompt file waits for its launch to read it; `prune::prune` takes what is left.
pub fn prompts_dir(home: &Path) -> PathBuf {
    home.join("prompts")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supraman_home_wins_then_appdata() {
        let appdata = || Some(OsString::from(r"C:\Users\x\AppData\Roaming"));
        assert_eq!(home_from(Some(OsString::from(r"D:\s")), appdata()), Some(PathBuf::from(r"D:\s")));
        assert_eq!(home_from(Some(OsString::new()), appdata()), Some(PathBuf::from(r"C:\Users\x\AppData\Roaming\Supraman")), "empty counts as unset");
        assert_eq!(home_from(None, None), None);
        assert_eq!(home_env(Path::new(r"D:\s")), [("SUPRAMAN_HOME", r"D:\s".to_string())]);
    }

    #[test]
    fn session_env_vars_leave_out_what_is_unset() {
        let env = SessionEnv { role: Some("repo".into()), session: None, repo: Some("api".into()) };
        assert_eq!(env.vars(), [(ROLE_VAR, "repo".to_string()), (REPO_VAR, "api".to_string())]);
        assert!(SessionEnv::default().vars().is_empty());
    }
}
