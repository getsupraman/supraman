use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{CoreError, Result};

pub const DEFAULT_MAX_WORKING: usize = 3;

/// `<FOREMAN_HOME>/settings.toml`, edited in the app and re-read by every supervisor tick.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    /// Most `fm-` sessions with `state=working` at once, across all platforms.
    #[serde(default = "default_max_working")]
    pub max_working: usize,
    /// A `.env` holding `WORKSTATE_<ROLE>_TOKEN=…` lines, read at launch time. Unset: sessions
    /// launch without a WorkState server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workstate_env: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workstate_url: Option<String>,
    /// The fleet panel's width in logical pixels, as the divider was last dragged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left_width: Option<u32>,
}

fn default_max_working() -> usize {
    DEFAULT_MAX_WORKING
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            max_working: DEFAULT_MAX_WORKING,
            workstate_env: None,
            workstate_url: None,
            left_width: None,
        }
    }
}

impl Settings {
    pub fn path(home: &Path) -> PathBuf {
        home.join("settings.toml")
    }

    /// The defaults when the file does not exist.
    pub fn load(home: &Path) -> Result<Self> {
        let path = Self::path(home);
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(CoreError::io(&path, e)),
        };
        toml::from_str(&text).map_err(|source| CoreError::Parse { path, source })
    }

    pub fn save(&self, home: &Path) -> Result<PathBuf> {
        let path = Self::path(home);
        let text = toml::to_string(self).map_err(|source| CoreError::Serialize {
            name: "settings".into(),
            source,
        })?;
        fs::create_dir_all(home).map_err(|e| CoreError::io(home, e))?;
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, text).map_err(|e| CoreError::io(&tmp, e))?;
        fs::rename(&tmp, &path).map_err(|e| CoreError::io(&path, e))?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_is_default_and_round_trips() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(Settings::load(home.path()).unwrap(), Settings::default());
        let s = Settings {
            max_working: 1,
            workstate_env: Some(r"C:\x\.env".into()),
            workstate_url: Some("http://localhost:5290/mcp".into()),
            left_width: Some(512),
        };
        s.save(home.path()).unwrap();
        assert_eq!(Settings::load(home.path()).unwrap(), s);
    }

    #[test]
    fn partial_file_keeps_defaults() {
        let home = tempfile::tempdir().unwrap();
        fs::write(Settings::path(home.path()), "workstate_url = \"http://x/mcp\"\n").unwrap();
        let s = Settings::load(home.path()).unwrap();
        assert_eq!(s.max_working, DEFAULT_MAX_WORKING);
        assert_eq!(s.workstate_url.as_deref(), Some("http://x/mcp"));
        fs::write(Settings::path(home.path()), "max_working = \"many\"").unwrap();
        assert!(matches!(Settings::load(home.path()), Err(CoreError::Parse { .. })));
    }
}
