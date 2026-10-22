//! Role tokens for the sessions the app launches: `settings.toml` may name a WorkState `.env`
//! (`workstate_env`) and URL (`workstate_url`); the token of role `r` is its `WORKSTATE_<R>_TOKEN`
//! line, read when the session launches. No file, no line: the session launches without WorkState
//! (`--strict-mcp-config`, so never with the user-scope server). A token is never logged.

use std::io;
use std::path::{Path, PathBuf};

use foreman_core::Settings;
use foreman_supervisor::{RoleToken, TokenProvider};

pub const DEFAULT_WORKSTATE_URL: &str = "http://localhost:5290/mcp";

pub struct AppTokens {
    pub home: PathBuf,
}

impl TokenProvider for AppTokens {
    fn token(&self, role: &str) -> Option<RoleToken> {
        let settings = Settings::load(&self.home).ok()?;
        token_from(&settings, role)
    }
}

pub fn token_from(settings: &Settings, role: &str) -> Option<RoleToken> {
    let env = settings.workstate_env.as_deref()?;
    let token = read_env_value(env, &token_key(role)).ok()??;
    Some(RoleToken {
        url: settings.workstate_url.clone().unwrap_or_else(|| DEFAULT_WORKSTATE_URL.into()),
        token,
    })
}

pub fn token_key(role: &str) -> String {
    format!("WORKSTATE_{}_TOKEN", role.to_uppercase().replace('-', "_"))
}

/// The value of `key` in a dotenv file: `KEY=value`, optionally `export `-prefixed and quoted;
/// `#` lines and blank lines skipped. `None` when the key is absent or empty.
pub fn read_env_value(path: &Path, key: &str) -> io::Result<Option<String>> {
    let text = std::fs::read_to_string(path)?;
    for line in text.lines() {
        let line = line.trim();
        let line = line.strip_prefix("export ").unwrap_or(line);
        if line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        if k.trim() != key {
            continue;
        }
        let v = v.trim();
        let v = ['"', '\'']
            .iter()
            .find_map(|q| v.strip_prefix(*q).and_then(|v| v.strip_suffix(*q)))
            .unwrap_or(v);
        return Ok((!v.is_empty()).then(|| v.to_string()));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAKE: &str = "# fake WorkState env for tests\n\
WORKSTATE_CONTROLLER_TOKEN=fake-controller\n\
export WORKSTATE_PLATFORM_TOKEN = \"fake-platform\"\n\
WORKSTATE_REPO_CONTROLLER_TOKEN='fake-repo'\n\
WORKSTATE_EMPTY_TOKEN=\n\
not a line\n";

    #[test]
    fn reads_plain_exported_and_quoted_values() {
        let dir = tempfile::tempdir().unwrap();
        let env = dir.path().join(".env");
        std::fs::write(&env, FAKE).unwrap();
        let read = |k: &str| read_env_value(&env, k).unwrap();
        assert_eq!(read("WORKSTATE_CONTROLLER_TOKEN").as_deref(), Some("fake-controller"));
        assert_eq!(read(&token_key("platform")).as_deref(), Some("fake-platform"));
        assert_eq!(read(&token_key("repo-controller")).as_deref(), Some("fake-repo"));
        assert_eq!(read("WORKSTATE_EMPTY_TOKEN"), None);
        assert_eq!(read("WORKSTATE_WORKER_TOKEN"), None);
        assert!(read_env_value(&dir.path().join("missing.env"), "X").is_err());
    }

    #[test]
    fn provider_uses_settings_and_falls_back_to_none() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let tokens = AppTokens { home: home.clone() };
        assert_eq!(tokens.token("platform"), None, "no settings file");
        let env = dir.path().join(".env");
        std::fs::write(&env, FAKE).unwrap();
        let mut settings = Settings {
            workstate_env: Some(env),
            ..Settings::default()
        };
        settings.save(&home).unwrap();
        assert_eq!(
            tokens.token("platform"),
            Some(RoleToken {
                url: DEFAULT_WORKSTATE_URL.into(),
                token: "fake-platform".into()
            })
        );
        assert_eq!(tokens.token("worker"), None, "no line for the role");
        settings.workstate_url = Some("http://localhost:9/mcp".into());
        settings.save(&home).unwrap();
        assert_eq!(tokens.token("platform").unwrap().url, "http://localhost:9/mcp");
        settings.workstate_env = Some(dir.path().join("gone.env"));
        settings.save(&home).unwrap();
        assert_eq!(tokens.token("platform"), None, "unreadable file");
    }
}
