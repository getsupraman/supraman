use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use supraman_core::Settings;
use supraman_core::process::{self, GIT_TIMEOUT, Kill};

/// The oldest git GitMan runs: `worktree list --porcelain -z` arrived in 2.36; the rest it uses
/// (`--diff-merges=first-parent` 2.31, `--end-of-options` 2.24, `--no-optional-locks` 2.15, porcelain v2 2.11) is older.
pub const FLOOR: Version = Version { major: 2, minor: 36, patch: 0 };

/// GitMan's table in `settings.toml`.
pub const SECTION: &str = "gitman";

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct GitmanSettings {
    /// A git to run in place of the first one on `PATH`.
    #[serde(default)]
    pub git: Option<PathBuf>,
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
}

/// The `[gitman] git` override, else the first git on `PATH`, checked once by running it.
pub fn locate(settings: &Settings) -> Located {
    let section: GitmanSettings = settings.section(SECTION).unwrap_or_default();
    match section.git.or_else(|| supraman_core::paths::executable_on_path("git")) {
        Some(path) => check(&path),
        None => Located::Missing { at: None },
    }
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
