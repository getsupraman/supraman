//! The access token is never logged and never reaches a file; only a session's minted `wss_` token does.

use std::sync::Arc;

use supraman_workstate::{Auth, AuthError};

use crate::{RoleToken, TokenProvider};

/// The [`TokenProvider`] of the app and of `sm`: Foreman's OAuth sign-in.
pub struct AppTokens {
    pub auth: Arc<Auth>,
}

impl TokenProvider for AppTokens {
    fn token(&self, role: &str) -> Result<Option<RoleToken>, String> {
        let Some(ws_role) = workstate_role(role) else {
            return Ok(None);
        };
        Ok(bearer(&self.auth)?.map(|(url, token)| RoleToken { url, token, role: ws_role.into() }))
    }
}

/// `(MCP URL, access token)`.
pub fn bearer(auth: &Auth) -> Result<Option<(String, String)>, String> {
    match auth.current() {
        Ok((origin, token)) => Ok(Some((format!("{origin}/mcp"), token))),
        Err(AuthError::NotSignedIn { .. }) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

pub fn workstate_role(role: &str) -> Option<&'static str> {
    match role {
        "workspace" => Some("controller"),
        "repo" | "repo-controller" => Some("orchestrator"),
        _ => None,
    }
}
