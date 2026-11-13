//! Workman's sign-in states as Supraman says them: the notice above the tree, Settings → Workman and the Work tab.

use std::time::Duration;

pub const RECHECK_EVERY: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkmanStatus {
    Checking,
    /// `who` is the token's name or subject when it says; `warning` is [`supraman_core::workman_server::ServerJson::warning`].
    Ready { url: String, who: Option<String>, warning: Option<String> },
    NotSignedIn { url: String },
    SignInAgain { url: String, why: String },
    Unreachable { url: String, error: String },
    /// No usable `~/.workman/server.json`: Supraman and its sessions run without Workman.
    NotSetUp { why: String },
}

impl WorkmanStatus {
    /// The server checked; `None` while checking or not set up.
    pub fn checked_url(&self) -> Option<&str> {
        match self {
            WorkmanStatus::Ready { url, .. } | WorkmanStatus::NotSignedIn { url } | WorkmanStatus::SignInAgain { url, .. } | WorkmanStatus::Unreachable { url, .. } => {
                Some(url)
            }
            WorkmanStatus::Checking | WorkmanStatus::NotSetUp { .. } => None,
        }
    }

    pub fn problem(&self) -> Option<String> {
        match self {
            WorkmanStatus::Checking => None,
            WorkmanStatus::Ready { warning, .. } => warning.clone(),
            WorkmanStatus::NotSetUp { why } => Some(why.clone()),
            WorkmanStatus::NotSignedIn { url } => Some(format!("Not signed in to Workman at {url}, so sessions run without Workman.")),
            WorkmanStatus::SignInAgain { url, why } => {
                Some(format!("Sign in to Workman at {url} again ({why}); until then sessions with Workman cannot start."))
            }
            WorkmanStatus::Unreachable { url, error } => Some(format!("Workman at {url} is unreachable ({error}).")),
        }
    }

    pub fn fix(&self) -> Option<String> {
        match self {
            WorkmanStatus::Checking | WorkmanStatus::Ready { .. } => None,
            WorkmanStatus::NotSignedIn { .. } | WorkmanStatus::SignInAgain { .. } => {
                Some("Sign in: your browser opens Workman's sign-in page, and Supraman keeps the grant in the Supraman home, encrypted for your Windows account.".into())
            }
            WorkmanStatus::Unreachable { .. } => Some(format!(
                "Check the server URL in Settings → Workman. Checked again every {} s.",
                RECHECK_EVERY.as_secs()
            )),
            WorkmanStatus::NotSetUp { .. } => Some(format!(
                "Supraman's sessions run without Workman until then. Checked again every {} s.",
                RECHECK_EVERY.as_secs()
            )),
        }
    }

    pub fn message(&self) -> Option<String> {
        Some(format!("{} {}", self.problem()?, self.fix()?))
    }

    pub fn signed_out(&self) -> bool {
        matches!(self, WorkmanStatus::NotSignedIn { .. } | WorkmanStatus::SignInAgain { .. })
    }

    pub fn unreachable(&self) -> bool {
        matches!(self, WorkmanStatus::Unreachable { .. })
    }
}
