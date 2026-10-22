use std::path::PathBuf;

use crate::Millis;

/// One Inbox row: something on a platform only the human can resolve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NeedsYou {
    pub kind: NeedsKind,
    pub platform: String,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Remedy {
    /// `id` is the row's 8-hex `id`, what `claude attach` takes.
    Attach { id: String },
    /// Clears the name's failure memory; the next tick launches or resumes at once.
    Retry,
    /// Foreman never changes trust: the human runs `command` in `cwd`, accepts, then presses Retry.
    RunInTerminal { cwd: PathBuf, command: String },
}
