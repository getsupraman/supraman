//! The access token is never logged and never reaches a file; only a session's minted `wss_` token does.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use supraman_workman::{Auth, AuthError};

use crate::{RoleToken, TokenProvider};

/// The [`TokenProvider`] of the app and of `sm`: Supraman's OAuth sign-in.
pub struct AppTokens {
    pub auth: Arc<Auth>,
}

impl TokenProvider for AppTokens {
    fn token(&self, role: &str) -> Result<Option<RoleToken>, String> {
        let Some(ws_role) = workman_role(role) else {
            return Ok(None);
        };
        Ok(bearer(&self.auth)?.map(|(url, token)| RoleToken { url, token, role: ws_role.into() }))
    }

    fn mcp_url(&self) -> Result<String, String> {
        mcp_url(&self.auth)
    }
}

/// The MCP URL of the server Supraman signs in to, signed in or not: the one source of every session config's URL.
/// `Err` is Workman not set up, in words that say to run `/workman:setup`.
pub fn mcp_url(auth: &Auth) -> Result<String, String> {
    auth.configured_origin().map(|origin| format!("{origin}/mcp")).map_err(|e| e.to_string())
}

static SIGN_INS: AtomicU64 = AtomicU64::new(0);

/// Called as a sign-in starts: a token minted from here on is newer than every sign-out queued before.
pub fn signing_in() {
    SIGN_INS.fetch_add(1, Ordering::SeqCst);
}

/// The sign-ins of this process so far; a mint and a queued sign-out each take it, read after the bearer.
pub fn sign_in_generation() -> u64 {
    SIGN_INS.load(Ordering::SeqCst)
}

/// `(MCP URL, access token)`.
pub fn bearer(auth: &Auth) -> Result<Option<(String, String)>, String> {
    match auth.current() {
        Ok((origin, token)) => Ok(Some((format!("{origin}/mcp"), token))),
        Err(AuthError::NotSignedIn { .. } | AuthError::NotSetUp(_)) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

pub fn workman_role(role: &str) -> Option<&'static str> {
    match role {
        "workspace" => Some("controller"),
        "repo" | "repo-controller" => Some("orchestrator"),
        _ => None,
    }
}
