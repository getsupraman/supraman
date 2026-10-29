//! Whether the sessions Foreman launches will have WorkState (m5/3, m5/43), and if not, what is
//! missing and how to fix it — one wording for the notice above the tree, Settings → WorkState and the
//! Work tab. [`detect`] asks `GET /health` and then the sign-in for an access token (a refresh when one
//! is due), so it runs off the UI thread.

use std::path::Path;
use std::time::Duration;

use foreman_core::Settings;
use foreman_workstate::{Auth, AuthError, AuthStatus};

/// How often the status is checked again while the app runs.
pub const RECHECK_EVERY: Duration = Duration::from_secs(30);
const HEALTH_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WsStatus {
    Checking,
    /// Signed in, and the server answers; `who` is the token's name or subject when it says.
    Ready { url: String, who: Option<String> },
    NotSignedIn { url: String },
    /// The grant no longer works (a refused refresh, a wiped server).
    SignInAgain { url: String, why: String },
    /// `/health` did not answer 2xx, or the refresh could not reach the server.
    Unreachable { url: String, error: String },
}

/// `/health`, then an access token for the configured server. Blocks up to a few seconds.
pub fn detect(home: Option<&Path>, auth: Option<&Auth>) -> WsStatus {
    let settings = home.and_then(|h| Settings::load(h).ok()).unwrap_or_default();
    let url = settings.workstate_origin();
    if let Err(error) = foreman_workstate::health(&url, HEALTH_TIMEOUT) {
        return WsStatus::Unreachable { url, error };
    }
    let Some(auth) = auth else {
        return WsStatus::NotSignedIn { url };
    };
    match auth.access_token(&url) {
        Ok(_) => {
            let who = match auth.status(&url) {
                AuthStatus::SignedIn { who, .. } => who,
                _ => None,
            };
            WsStatus::Ready { url, who }
        }
        Err(AuthError::NotSignedIn { .. }) => WsStatus::NotSignedIn { url },
        Err(AuthError::SignInAgain { why, .. }) => WsStatus::SignInAgain { url, why },
        Err(AuthError::Unreachable { reason, .. }) => WsStatus::Unreachable { url, error: reason },
        Err(e) => WsStatus::SignInAgain { url, why: e.to_string() },
    }
}

impl WsStatus {
    /// What is wrong, in one sentence; `None` when nothing is (or not known yet).
    pub fn problem(&self) -> Option<String> {
        match self {
            WsStatus::Checking | WsStatus::Ready { .. } => None,
            WsStatus::NotSignedIn { url } => Some(format!("Not signed in to WorkState at {url}, so sessions run without WorkState.")),
            WsStatus::SignInAgain { url, why } => {
                Some(format!("Sign in to WorkState at {url} again ({why}); until then sessions with WorkState cannot start."))
            }
            WsStatus::Unreachable { url, error } => Some(format!("WorkState at {url} is unreachable ({error}).")),
        }
    }

    /// How to fix it, in one sentence.
    pub fn fix(&self) -> Option<String> {
        match self {
            WsStatus::Checking | WsStatus::Ready { .. } => None,
            WsStatus::NotSignedIn { .. } | WsStatus::SignInAgain { .. } => {
                Some("Sign in: your browser opens WorkState's sign-in page, and Foreman keeps the grant in the Foreman home, encrypted for your Windows account.".into())
            }
            WsStatus::Unreachable { .. } => Some(format!(
                "Check the server URL in Settings → WorkState. Checked again every {} s.",
                RECHECK_EVERY.as_secs()
            )),
        }
    }

    /// `problem` and `fix` as one paragraph: the Work tab's message.
    pub fn message(&self) -> Option<String> {
        Some(format!("{} {}", self.problem()?, self.fix()?))
    }

    /// The notice offers Sign in.
    pub fn signed_out(&self) -> bool {
        matches!(self, WsStatus::NotSignedIn { .. } | WsStatus::SignInAgain { .. })
    }

    pub fn unreachable(&self) -> bool {
        matches!(self, WsStatus::Unreachable { .. })
    }

    /// The line Start shows when `workspace` was started anyway: it never blocks.
    pub fn start_warning(&self, workspace: &str) -> Option<String> {
        match self {
            WsStatus::Checking | WsStatus::Ready { .. } => None,
            WsStatus::Unreachable { url, .. } => {
                Some(format!("Started {workspace}; WorkState at {url} is unreachable, so its sessions cannot start with it."))
            }
            WsStatus::SignInAgain { .. } => Some(format!("Started {workspace}; its sessions wait for a WorkState sign-in.")),
            WsStatus::NotSignedIn { .. } => Some(format!("Started {workspace} without WorkState: its sessions can talk, not plan.")),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use foreman_workstate::MemoryStore;
    use foreman_workstate::fake::{Answer, FakeWorkState};

    use super::*;

    #[test]
    fn signed_out_signed_in_and_broken_each_say_what_to_do() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let fake = FakeWorkState::start(vec![("/health".into(), Answer::text("Healthy"))]);
        let oauth = fake.enable_oauth();
        Settings { workstate_url: Some(fake.base().into()), ..Settings::default() }.save(&home).unwrap();
        let url = fake.base().to_string();

        let fresh = Auth::with_store(&home, Arc::new(MemoryStore::default()));
        let out = detect(Some(&home), Some(&fresh));
        assert_eq!(out, WsStatus::NotSignedIn { url: url.clone() });
        assert!(out.signed_out() && out.problem().unwrap().contains("Not signed in") && out.fix().unwrap().contains("Sign in"));
        assert!(out.start_warning("demo").unwrap().contains("without WorkState"));

        oauth.set_access_ttl(60);
        let auth = crate::tokens::signed_in(&home, fake.base());
        assert_eq!(detect(Some(&home), Some(&auth)), WsStatus::Ready { url: url.clone(), who: Some("Fake Operator".into()) });
        assert!(fake.seen().iter().filter(|s| s.target == "/health").all(|s| s.authorization.is_none()), "/health carries no token");

        oauth.fail_refresh(true);
        let broken = detect(Some(&home), Some(&auth));
        assert!(matches!(&broken, WsStatus::SignInAgain { why, .. } if why.contains("invalid_grant")), "{broken:?}");
        assert!(broken.signed_out() && broken.problem().unwrap().contains("again"));
        let text = format!("{} {broken:?}", broken.message().unwrap());
        for t in oauth.issued() {
            assert!(!text.contains(&t), "{text}");
        }
    }

    #[test]
    fn a_closed_port_is_unreachable_before_any_sign_in() {
        let dir = tempfile::tempdir().unwrap();
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let closed = format!("http://127.0.0.1:{port}");
        Settings { workstate_url: Some(closed.clone()), ..Settings::default() }.save(dir.path()).unwrap();
        let down = detect(Some(dir.path()), None);
        assert_eq!(down, WsStatus::Unreachable { url: closed.clone(), error: "connection refused".into() });
        assert!(down.problem().unwrap().contains(&closed));
        assert!(down.fix().unwrap().contains("Settings → WorkState"));
        assert!(down.start_warning("demo").unwrap().contains("unreachable"));
        assert_eq!(WsStatus::Ready { url: closed, who: None }.start_warning("demo"), None);
    }
}
