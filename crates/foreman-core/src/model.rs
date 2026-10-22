use std::path::PathBuf;

use chrono::{DateTime, SubsecRound, Utc};
use serde::{Deserialize, Serialize};

// `repos` stays last: TOML emits it as an array of tables, which must follow the plain keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Platform {
    pub name: String,
    pub root: PathBuf,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub repos: Vec<Repo>,
}

impl Platform {
    pub fn new(name: impl Into<String>, root: impl Into<PathBuf>, repos: Vec<Repo>) -> Self {
        Self {
            name: name.into(),
            root: root.into(),
            created_at: Utc::now().trunc_subsecs(0),
            repos,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Repo {
    /// Path relative to the platform root, `/`-separated; the folder name at depth 1.
    pub name: String,
    pub path: PathBuf,
    pub has_claude_md: bool,
    pub has_agents_md: bool,
    /// `None` for a detached HEAD.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default = "enabled_default")]
    pub enabled: bool,
}

fn enabled_default() -> bool {
    true
}
