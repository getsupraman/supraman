use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{CoreError, Result};

pub const DEFAULT_MAX_WORKING: usize = 3;
/// `workstate_url` when unset.
pub const DEFAULT_WORKSTATE_URL: &str = "http://127.0.0.1:5290/mcp";
/// `terminal_font_size` when unset or invalid, and Ctrl+0's size (m5/41).
pub const DEFAULT_TERMINAL_FONT_SIZE: u32 = 14;
/// The bounds `terminal_font_size` is clamped to; `foreman_ui::term_element`'s MIN/MAX_FONT_SIZE.
pub const MIN_TERMINAL_FONT_SIZE: u32 = 6;
pub const MAX_TERMINAL_FONT_SIZE: u32 = 72;

/// `<FOREMAN_HOME>/settings.toml`, edited in the app and re-read by every supervisor tick.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    /// Most `fm-` sessions with `state=working` at once, across all workspaces.
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
    /// More plugin dirs every launch gets, as `--plugin-dir` after Foreman's own (e.g. the rlarch plugin
    /// whose `/rlarch:controller` a repo-controller runs). Edited in the file; the app only shows them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_plugin_dirs: Vec<PathBuf>,
    /// Start elevated (one UAC prompt per start), so Foreman reaches a claude background service an
    /// elevated `claude --bg` started (m5/25).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub run_as_admin: bool,
    /// How long a workspace's relay stays wanted after its last fallback send (m5/32). Unset: 10 min.
    /// Edited in the file; the selftest shortens it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay_idle_secs: Option<u64>,
    /// The theme's name: a bundled one or one in `<home>/themes/*.json` (m5/31). Unset: VS Code Dark Modern.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    /// "campbell" paints ANSI 0-15 in Windows Terminal's Campbell. Unset: the theme's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_colors: Option<String>,
    /// Every terminal tab's font size in px (m5/41), as Ctrl+= / Ctrl+- / Ctrl+wheel and Settings →
    /// Appearance leave it. Unset or not a number: 14; read through [`Settings::terminal_font_size`].
    #[serde(default, deserialize_with = "lenient_size", skip_serializing_if = "Option::is_none")]
    pub terminal_font_size: Option<u32>,
}

/// A number (integer or float, rounded) or nothing: a bad value must not make the whole file unreadable.
fn lenient_size<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Option<u32>, D::Error> {
    Ok(match toml::Value::deserialize(d)? {
        toml::Value::Integer(n) => u32::try_from(n.max(0)).ok(),
        toml::Value::Float(f) if f.is_finite() => Some(f.round().clamp(0., u32::MAX as f64) as u32),
        _ => None,
    })
}

/// `size` within [`MIN_TERMINAL_FONT_SIZE`]..=[`MAX_TERMINAL_FONT_SIZE`].
pub fn clamp_terminal_font_size(size: u32) -> u32 {
    size.clamp(MIN_TERMINAL_FONT_SIZE, MAX_TERMINAL_FONT_SIZE)
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
            extra_plugin_dirs: Vec::new(),
            run_as_admin: false,
            relay_idle_secs: None,
            theme: None,
            terminal_colors: None,
            terminal_font_size: None,
        }
    }
}

impl Settings {
    pub fn relay_idle_ms(&self) -> i64 {
        self.relay_idle_secs.map_or(crate::DEFAULT_RELAY_IDLE_MS, |s| s.saturating_mul(1000).min(i64::MAX as u64) as i64)
    }

    pub fn terminal_font_size(&self) -> u32 {
        self.terminal_font_size.map_or(DEFAULT_TERMINAL_FONT_SIZE, clamp_terminal_font_size)
    }

    /// Stores `size` clamped; the default is stored as unset.
    pub fn set_terminal_font_size(&mut self, size: u32) {
        let size = clamp_terminal_font_size(size);
        self.terminal_font_size = (size != DEFAULT_TERMINAL_FONT_SIZE).then_some(size);
    }

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
            workstate_url: Some("http://127.0.0.1:5290/mcp".into()),
            left_width: Some(512),
            extra_plugin_dirs: vec![r"C:\x\plugins\rlarch".into()],
            run_as_admin: true,
            relay_idle_secs: Some(20),
            theme: Some("Foreman Dark".into()),
            terminal_colors: Some("campbell".into()),
            terminal_font_size: Some(20),
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
        assert!(s.extra_plugin_dirs.is_empty());
        assert_eq!(s.relay_idle_ms(), crate::DEFAULT_RELAY_IDLE_MS, "10 min unless set");
        fs::write(Settings::path(home.path()), "relay_idle_secs = 20\n").unwrap();
        assert_eq!(Settings::load(home.path()).unwrap().relay_idle_ms(), 20_000);
        fs::write(Settings::path(home.path()), "max_working = \"many\"").unwrap();
        assert!(matches!(Settings::load(home.path()), Err(CoreError::Parse { .. })));
    }

    #[test]
    fn terminal_font_size_is_14_unless_set_and_clamped() {
        let home = tempfile::tempdir().unwrap();
        let load = |text: &str| {
            fs::write(Settings::path(home.path()), text).unwrap();
            Settings::load(home.path()).unwrap()
        };
        assert_eq!(Settings::default().terminal_font_size(), DEFAULT_TERMINAL_FONT_SIZE);
        assert_eq!(load("max_working = 2
").terminal_font_size(), 14, "missing");
        assert_eq!(load("terminal_font_size = 18
").terminal_font_size(), 18);
        assert_eq!(load("terminal_font_size = 17.6
").terminal_font_size(), 18, "a float is rounded");
        assert_eq!(load("terminal_font_size = 1
").terminal_font_size(), MIN_TERMINAL_FONT_SIZE);
        assert_eq!(load("terminal_font_size = -3
").terminal_font_size(), MIN_TERMINAL_FONT_SIZE);
        assert_eq!(load("terminal_font_size = 500
").terminal_font_size(), MAX_TERMINAL_FONT_SIZE);
        for bad in ["\"big\"", "true", "[14]", "nan"] {
            let s = load(&format!("max_working = 2
terminal_font_size = {bad}
"));
            assert_eq!(s.terminal_font_size(), 14, "{bad} is invalid");
            assert_eq!(s.max_working, 2, "{bad} does not cost the rest of the file");
        }
    }

    #[test]
    fn terminal_font_size_saves_clamped_and_the_default_as_unset() {
        let home = tempfile::tempdir().unwrap();
        let mut s = Settings::default();
        s.set_terminal_font_size(19);
        s.save(home.path()).unwrap();
        assert!(fs::read_to_string(Settings::path(home.path())).unwrap().contains("terminal_font_size = 19"));
        assert_eq!(Settings::load(home.path()).unwrap().terminal_font_size(), 19);
        s.set_terminal_font_size(999);
        assert_eq!(s.terminal_font_size, Some(MAX_TERMINAL_FONT_SIZE));
        s.set_terminal_font_size(0);
        assert_eq!(s.terminal_font_size, Some(MIN_TERMINAL_FONT_SIZE));
        s.set_terminal_font_size(DEFAULT_TERMINAL_FONT_SIZE);
        s.save(home.path()).unwrap();
        assert!(!fs::read_to_string(Settings::path(home.path())).unwrap().contains("terminal_font_size"));
        assert_eq!(Settings::load(home.path()).unwrap().terminal_font_size(), 14);
    }
}
