//! Which token Foreman reads with: `WORKSTATE_READER_TOKEN` from `settings.toml`'s `workstate_env`
//! when it has one, else `WORKSTATE_CONTROLLER_TOKEN`. The token stays in this process: it is the
//! bearer of [`Client`] and [`crate::Stream`] and is never printed.

use std::path::PathBuf;

use foreman_core::{DEFAULT_WORKSTATE_URL, Settings, read_env_value};

use crate::client::{Client, api_base};

pub const READER_TOKEN_KEY: &str = "WORKSTATE_READER_TOKEN";
pub const CONTROLLER_TOKEN_KEY: &str = "WORKSTATE_CONTROLLER_TOKEN";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CredentialsError {
    #[error("no WorkState .env in settings.toml (workstate_env)")]
    NoEnv,
    #[error("cannot read {}: {reason}", .path.display())]
    Unreadable { path: PathBuf, reason: String },
    #[error("{} has neither {READER_TOKEN_KEY} nor {CONTROLLER_TOKEN_KEY}", .path.display())]
    NoToken { path: PathBuf },
}

/// A [`Client`] for the server of `workstate_url` (default [`DEFAULT_WORKSTATE_URL`]), and the
/// `.env` key whose token it carries.
pub fn client_from_settings(settings: &Settings) -> Result<(Client, &'static str), CredentialsError> {
    let path = settings.workstate_env.as_deref().ok_or(CredentialsError::NoEnv)?;
    let unreadable = |e: std::io::Error| CredentialsError::Unreadable {
        path: path.to_path_buf(),
        reason: e.to_string(),
    };
    for key in [READER_TOKEN_KEY, CONTROLLER_TOKEN_KEY] {
        if let Some(token) = read_env_value(path, key).map_err(unreadable)? {
            let url = settings.workstate_url.as_deref().unwrap_or(DEFAULT_WORKSTATE_URL);
            return Ok((Client::new(api_base(url), token), key));
        }
    }
    Err(CredentialsError::NoToken { path: path.to_path_buf() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reader_token_wins_else_controller_else_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let env = dir.path().join(".env");
        let mut settings = Settings::default();
        assert_eq!(client_from_settings(&settings).unwrap_err(), CredentialsError::NoEnv);
        settings.workstate_env = Some(env.clone());
        assert!(matches!(client_from_settings(&settings).unwrap_err(), CredentialsError::Unreadable { .. }));

        std::fs::write(&env, "WORKSTATE_CONTROLLER_TOKEN=fake-controller\nWORKSTATE_READER_TOKEN=fake-reader\n").unwrap();
        let (client, key) = client_from_settings(&settings).unwrap();
        assert_eq!((key, client.token(), client.base()), (READER_TOKEN_KEY, "fake-reader", "http://127.0.0.1:5290"));

        std::fs::write(&env, "WORKSTATE_CONTROLLER_TOKEN=fake-controller\nWORKSTATE_READER_TOKEN=\n").unwrap();
        settings.workstate_url = Some("http://localhost:9/mcp".into());
        let (client, key) = client_from_settings(&settings).unwrap();
        assert_eq!((key, client.token(), client.base()), (CONTROLLER_TOKEN_KEY, "fake-controller", "http://localhost:9"));

        std::fs::write(&env, "WORKSTATE_WORKER_TOKEN=fake-worker\n").unwrap();
        let e = client_from_settings(&settings).unwrap_err();
        assert!(matches!(e, CredentialsError::NoToken { .. }));
        assert!(!e.to_string().contains("fake-worker"), "{e}");
    }
}
