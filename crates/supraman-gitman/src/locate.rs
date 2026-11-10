use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use serde::{Deserialize, Serialize};
use supraman_core::Settings;
use supraman_core::process::{self, GIT_TIMEOUT, Kill};

/// The oldest git GitMan runs: `worktree list --porcelain -z` arrived in 2.36.
pub const FLOOR: Version = Version { major: 2, minor: 36, patch: 0 };

/// GitMan's table in `settings.toml`.
pub const SECTION: &str = "gitman";

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct GitmanSettings {
    /// A git to run in place of the first one on `PATH`.
    #[serde(default)]
    pub git: Option<PathBuf>,
}

/// The remote calls' keys of `[gitman]`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct RemoteSettings {
    /// What the toolbar's Pull does.
    #[serde(default)]
    pub pull: Option<crate::write::remote::PullChoice>,
    /// A fetch prunes the remote-tracking branches gone from their remote; on when unset.
    #[serde(default)]
    pub prune: Option<bool>,
}

/// `[gitman]`'s remote keys; their defaults when the table is absent or does not parse.
pub fn remote_settings(settings: &Settings) -> RemoteSettings {
    settings.section(SECTION).unwrap_or_default()
}

/// The background work's keys of `[gitman]`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct BackgroundSettings {
    /// Minutes between an open Git tab's fetches; 0 is off, unset [`FETCH_EVERY_MINUTES`].
    #[serde(default)]
    pub fetch_every_minutes: Option<u32>,
    /// `git maintenance run` in the background; on when unset.
    #[serde(default)]
    pub maintenance: Option<bool>,
    /// A WIP row in the graph for each other worktree with changes or a live session; on when unset.
    #[serde(default)]
    pub worktree_wip: Option<bool>,
}

pub const FETCH_EVERY_MINUTES: u32 = 1;

impl BackgroundSettings {
    pub fn fetch_every(&self) -> u32 {
        self.fetch_every_minutes.unwrap_or(FETCH_EVERY_MINUTES)
    }

    pub fn maintains(&self) -> bool {
        self.maintenance.unwrap_or(true)
    }

    pub fn worktree_wips(&self) -> bool {
        self.worktree_wip.unwrap_or(true)
    }
}

/// `[gitman]`'s background keys; their defaults when the table is absent or does not parse.
pub fn background_settings(settings: &Settings) -> BackgroundSettings {
    settings.section(SECTION).unwrap_or_default()
}

/// What a Git tab reads of `[gitman]`, once per tab: [`read_settings`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabSettings {
    pub git: Option<PathBuf>,
    /// `None` when its keys do not parse: the notice says why, and the tab does not take the defaults for them.
    pub remote: Option<RemoteSettings>,
    pub background: Option<BackgroundSettings>,
}

impl Default for TabSettings {
    /// A tab with no home: every key's default.
    fn default() -> TabSettings {
        TabSettings { git: None, remote: Some(RemoteSettings::default()), background: Some(BackgroundSettings::default()) }
    }
}

/// `[gitman]` of `home`'s `settings.toml` for a Git tab; a part whose keys do not parse is `None`, with the notice.
pub fn read_settings(home: &Path) -> Result<(TabSettings, Option<String>), String> {
    let settings = Settings::load(home).map_err(|e| format!("[gitman] git is not read: {e}"))?;
    let git = section(&settings)?.git;
    let remote = settings.section_or_notice::<RemoteSettings>(SECTION).map(Option::unwrap_or_default);
    let background = settings.section_or_notice::<BackgroundSettings>(SECTION).map(Option::unwrap_or_default);
    let notice = [remote.as_ref().err(), background.as_ref().err()].into_iter().flatten().next().cloned();
    Ok((TabSettings { git, remote: remote.ok(), background: background.ok() }, notice))
}

/// `[gitman] <key> = value`, the table's other keys kept; the caller saves it (`Settings::update`).
pub fn set_setting(settings: &mut Settings, key: &str, value: impl Serialize) -> Result<(), String> {
    let mut table: serde_json::Map<String, serde_json::Value> = settings.section(SECTION).unwrap_or_default();
    table.insert(key.to_string(), serde_json::to_value(value).map_err(|e| e.to_string())?);
    settings.set_section(SECTION, &table).map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    /// `git --version`'s output: `git version 2.44.0.windows.1`, `git version 2.39.3 (Apple Git-145)`.
    pub fn parse(output: &str) -> Option<Version> {
        let rest = output.trim().strip_prefix("git version ")?;
        let mut parts = rest.split(|c: char| !c.is_ascii_digit()).take(3).map(str::parse::<u32>);
        let major = parts.next()?.ok()?;
        let minor = parts.next()?.ok()?;
        let patch = parts.next().and_then(Result::ok).unwrap_or(0);
        Some(Version { major, minor, patch })
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// A git that answered `--version` at or above [`FLOOR`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Git {
    pub path: PathBuf,
    pub version: Version,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Located {
    Found(Git),
    /// `at` is the configured path that holds no file; `None` when no git is on `PATH`.
    Missing { at: Option<PathBuf> },
    TooOld { path: PathBuf, version: Version, floor: Version },
    /// It did not run, or did not answer like git.
    Unusable { path: PathBuf, reason: String },
    /// `[gitman]` in `settings.toml` is not GitMan's table: its notice.
    BadSetting(String),
}

fn section(settings: &Settings) -> Result<GitmanSettings, String> {
    settings.section_or_notice(SECTION).map(Option::unwrap_or_default)
}

/// The `[gitman] git` override, else the first git on `PATH`, checked once by running it.
pub fn locate(settings: &Settings) -> Located {
    match section(settings) {
        Ok(section) => locate_at(section.git),
        Err(notice) => Located::BadSetting(notice),
    }
}

fn locate_at(setting: Option<PathBuf>) -> Located {
    match setting.or_else(|| supraman_core::paths::executable_on_path("git")) {
        Some(path) => check(&path),
        None => Located::Missing { at: None },
    }
}

/// [`locate`], a git it found kept for the process per `[gitman] git` setting while its file is there.
pub fn located(settings: &Settings) -> Located {
    match section(settings) {
        Ok(section) => located_at(section.git),
        Err(notice) => Located::BadSetting(notice),
    }
}

/// [`located`] of a `[gitman] git` already read ([`TabSettings::git`]).
pub fn located_at(setting: Option<PathBuf>) -> Located {
    static FOUND: Mutex<Vec<(Option<PathBuf>, Git)>> = Mutex::new(Vec::new());
    let kept = FOUND.lock().unwrap_or_else(PoisonError::into_inner).iter().find(|(s, _)| *s == setting).map(|(_, git)| git.clone());
    if let Some(git) = kept.filter(|g| g.path.is_file()) {
        return Located::Found(git);
    }
    let located = locate_at(setting.clone());
    let mut found = FOUND.lock().unwrap_or_else(PoisonError::into_inner);
    found.retain(|(s, _)| *s != setting);
    if let Located::Found(git) = &located {
        found.push((setting, git.clone()));
    }
    located
}

/// Runs `path --version` and judges the answer.
pub fn check(path: &Path) -> Located {
    if !path.is_file() {
        return Located::Missing { at: Some(path.to_path_buf()) };
    }
    match process::run(process::command(path).arg("--version"), GIT_TIMEOUT, Kill::Tree) {
        Ok(out) if out.success() => judge(path, &out.stdout),
        Ok(out) => Located::Unusable { path: path.to_path_buf(), reason: format!("exited with {:?}: {}", out.code, out.stderr.trim()) },
        Err(e) => Located::Unusable { path: path.to_path_buf(), reason: e.to_string() },
    }
}

/// What `version_output` of the git at `path` means.
pub fn judge(path: &Path, version_output: &str) -> Located {
    let path = path.to_path_buf();
    match Version::parse(version_output) {
        Some(version) if version >= FLOOR => Located::Found(Git { path, version }),
        Some(version) => Located::TooOld { path, version, floor: FLOOR },
        None => Located::Unusable { path, reason: format!("not a git version: {}", version_output.trim()) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::write::remote::PullChoice;

    #[test]
    fn a_remote_key_is_set_with_the_tables_other_keys_kept() {
        let mut settings = Settings::default();
        settings.set_section(SECTION, &GitmanSettings { git: Some("C:/git.exe".into()) }).unwrap();
        set_setting(&mut settings, "pull", PullChoice::Rebase).unwrap();
        assert_eq!(remote_settings(&settings), RemoteSettings { pull: Some(PullChoice::Rebase), prune: None });
        assert_eq!(section(&settings).unwrap().git.as_deref(), Some(Path::new("C:/git.exe")));
    }

    fn read(toml: &str) -> Result<(TabSettings, Option<String>), String> {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(Settings::path(home.path()), toml).unwrap();
        read_settings(home.path())
    }

    #[test]
    fn a_tabs_settings_are_read_whole_and_a_key_that_does_not_parse_is_said_never_defaulted() {
        let (settings, notice) = read("[gitman]\nfetch_every_minutes = 0\npull = \"rebase\"\n").unwrap();
        assert!(notice.is_none());
        assert_eq!(settings.background.map(|b| b.fetch_every()), Some(0));
        assert_eq!(settings.remote.and_then(|r| r.pull), Some(PullChoice::Rebase));
        let (settings, notice) = read("[gitman]\nfetch_every_minutes = \"often\"\n").unwrap();
        assert!(notice.is_some_and(|n| n.starts_with("[gitman] in settings.toml is ignored")));
        assert_eq!(settings.background, None, "never the default of a fetch every minute");
        assert_eq!(settings.remote, Some(RemoteSettings::default()));
        assert!(read("[gitman]\ngit = 5\n").is_err_and(|e| e.starts_with("[gitman] in settings.toml is ignored")));
        assert!(read("not toml [").is_err_and(|e| e.starts_with("[gitman] git is not read")));
    }
}
