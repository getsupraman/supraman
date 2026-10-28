use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{CoreError, Result};

pub const DEFAULT_MAX_WORKING: usize = 20;
/// `workstate_url` when unset: the local server, until the move to its hosted origin (m5/43).
pub const DEFAULT_WORKSTATE_URL: &str = "http://127.0.0.1:5290";
/// `terminal_font_size` when unset or invalid, and Ctrl+0's size (m5/41).
pub const DEFAULT_TERMINAL_FONT_SIZE: u32 = 14;
/// The bounds `terminal_font_size` is clamped to; `foreman_ui::term_element`'s MIN/MAX_FONT_SIZE.
pub const MIN_TERMINAL_FONT_SIZE: u32 = 6;
pub const MAX_TERMINAL_FONT_SIZE: u32 = 72;

/// `<FOREMAN_HOME>/settings.toml`, edited in the app and re-read by every supervisor tick.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    /// Most of this home's `fm-` sessions mid-turn at once, across its workspaces.
    #[serde(default = "default_max_working")]
    pub max_working: usize,
    /// The WorkState server Foreman signs in to (m5/43), an origin like `https://workstate.example`;
    /// an older file's `…/mcp` URL means its origin. Unset: [`DEFAULT_WORKSTATE_URL`]. No token is
    /// ever kept here: the sign-in's refresh token is in Credential Manager.
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

/// `http://127.0.0.1:5290/mcp` → `http://127.0.0.1:5290`.
pub fn url_origin(url: &str) -> String {
    let url = url.trim();
    let start = url.find("://").map_or(0, |i| i + 3);
    match url[start..].find(['/', '?', '#']) {
        Some(i) => url[..start + i].to_string(),
        None => url.trim_end_matches('/').to_string(),
    }
}

/// A server URL typed into Settings as the origin to keep: `http(s)://host[:port]`, any path dropped.
/// Plain `http` only for this machine (a token must not cross a network in the clear).
pub fn validate_workstate_url(text: &str) -> std::result::Result<String, String> {
    let text = text.trim();
    let (scheme, rest) = text.split_once("://").ok_or("a URL starts with https:// (or http:// for this machine)")?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return Err(format!("{scheme}:// is not http or https"));
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() || authority.contains('@') || authority.chars().any(char::is_whitespace) {
        return Err("the URL names no host".into());
    }
    let host = if let Some(v6) = authority.strip_prefix('[') {
        let (h, port) = v6.split_once(']').ok_or("an IPv6 host is closed with ]")?;
        if !port.is_empty() && port.strip_prefix(':').is_none_or(|p| p.parse::<u16>().is_err()) {
            return Err(format!("{port} is not a port"));
        }
        h.to_string()
    } else {
        let (h, port) = authority.rsplit_once(':').map_or((authority, None), |(h, p)| (h, Some(p)));
        if let Some(p) = port
            && p.parse::<u16>().is_err()
        {
            return Err(format!("{p} is not a port"));
        }
        if h.is_empty() || !h.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') {
            return Err(format!("{h} is not a host name"));
        }
        h.to_ascii_lowercase()
    };
    let local = matches!(host.as_str(), "127.0.0.1" | "localhost" | "::1");
    if scheme == "http" && !local {
        return Err("use https:// for a server that is not on this machine".into());
    }
    Ok(format!("{scheme}://{}", authority.to_ascii_lowercase()))
}

impl Settings {
    /// The server's origin: `workstate_url` without any path, else [`DEFAULT_WORKSTATE_URL`].
    pub fn workstate_origin(&self) -> String {
        url_origin(self.workstate_url.as_deref().unwrap_or(DEFAULT_WORKSTATE_URL))
    }

    /// The MCP endpoint a minted session's config names.
    pub fn workstate_mcp_url(&self) -> String {
        format!("{}/mcp", self.workstate_origin())
    }

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
            workstate_url: Some("https://workstate.example".into()),
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
    fn workstate_url_is_an_origin_and_an_old_env_line_is_ignored() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(Settings::default().workstate_origin(), "http://127.0.0.1:5290");
        assert_eq!(Settings::default().workstate_mcp_url(), "http://127.0.0.1:5290/mcp");
        fs::write(Settings::path(home.path()), "workstate_env = 'C:/x/.env'\nworkstate_url = \"http://127.0.0.1:5290/mcp\"\n").unwrap();
        let s = Settings::load(home.path()).unwrap();
        assert_eq!(s.workstate_origin(), "http://127.0.0.1:5290", "an older file's /mcp URL");
        s.save(home.path()).unwrap();
        assert!(!fs::read_to_string(Settings::path(home.path())).unwrap().contains("workstate_env"), "the .env line is gone");
    }

    #[test]
    fn validate_workstate_url_keeps_an_origin_and_https_off_this_machine() {
        assert_eq!(validate_workstate_url(" https://WorkState.dikaia.ai/ ").as_deref(), Ok("https://workstate.dikaia.ai"));
        assert_eq!(validate_workstate_url("https://workstate.dikaia.ai/mcp").as_deref(), Ok("https://workstate.dikaia.ai"));
        assert_eq!(validate_workstate_url("http://127.0.0.1:5290").as_deref(), Ok("http://127.0.0.1:5290"));
        assert_eq!(validate_workstate_url("http://localhost:9/api").as_deref(), Ok("http://localhost:9"));
        assert_eq!(validate_workstate_url("http://[::1]:9").as_deref(), Ok("http://[::1]:9"));
        for bad in ["workstate.dikaia.ai", "ftp://x", "http://workstate.dikaia.ai", "https://", "https://a b", "https://x:port", "https://u@x"] {
            assert!(validate_workstate_url(bad).is_err(), "{bad}");
        }
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
