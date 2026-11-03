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
    home_from(std::env::var_os(HOME_VAR), std::env::var_os("APPDATA"))
}

/// For a session's or a script's environment.
pub fn home_env(home: &Path) -> [(&'static str, String); 1] {
    [(HOME_VAR, home.display().to_string())]
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
}
