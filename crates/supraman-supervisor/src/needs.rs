use std::path::PathBuf;

use crate::Millis;

/// One Inbox row: something on a workspace only the human can resolve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NeedsYou {
    pub kind: NeedsKind,
    pub workspace: String,
    pub name: String,
    /// When the supervisor first saw this need (kept across ticks by [`crate::Supervisor`]).
    pub since: Millis,
    pub detail: String,
    /// `None` for [`NeedsKind::Duplicate`]: which row to keep is the human's call, outside Foreman.
    pub action: Option<Remedy>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum NeedsKind {
    /// Informational: the session waits for an answer (`waitingFor`). Never restarted.
    Blocked,
    TrustNeeded,
    GaveUp,
    /// Failed, a retry is scheduled.
    LaunchFailed,
    /// Two or more rows carry one wanted name; the supervisor touches none of them.
    Duplicate,
    /// A live minted session had no turn for longer than its token's idle window: its next
    /// Workman call gets HTTP 401.
    TokenExpired,
    /// A live minted session's token is from another Workman server than the one Foreman uses.
    OtherServer,
    /// `claude stop` failed on a session that should stop: it keeps running.
    StopFailed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Remedy {
    /// `id` is the row's 8-hex `id`, what `claude attach` takes.
    Attach { id: String },
    /// Clears the name's failure memory; the next tick launches or resumes at once.
    Retry,
    /// Foreman never changes trust: the human runs `command` in `cwd`, accepts, then presses Retry.
    RunInTerminal { cwd: PathBuf, command: String },
    /// Goes through Retry ([`crate::Supervisor::retry`]): stops the live row `id`; the supervisor then
    /// resumes it with a freshly minted token in its config file.
    Remint { id: String },
    /// The background service runs as administrator and Foreman does not: turn Settings → Run
    /// Foreman as administrator on and restart elevated.
    RestartElevated,
    /// The copy of an adopted session (`from`, its full uuid) failed: Retry tries it again, Start fresh
    /// ([`crate::DesiredState::start_fresh`]) launches a new session instead.
    Adopt { from: String },
}
