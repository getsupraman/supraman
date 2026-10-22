//! The only code in Foreman that knows the `claude` CLI. This is the read side: list sessions,
//! derive their health, map them onto a platform's repos. Launching, stop and attach are M2.
//!
//! # Health
//!
//! [`Session::health`] walks this table top to bottom; the first matching row wins. "live state"
//! means `state` is `working` or `blocked`; "age" is `now - startedAt` (a missing `startedAt`
//! counts as old).
//!
//! | # | `state` | `kind` | `pid` | `status` | other | health |
//! |---|---|---|---|---|---|---|
//! | 1 | `stopped` or `done` | any | any | any | | `Stopped` |
//! | 2 | live state | `background` | absent | any | age > [`ORPHAN_AFTER`] | `Orphaned { last_state }` |
//! | 3 | `blocked` | any | any | any | | `Blocked { waiting_for }` |
//! | 4 | any | any | any | `waiting` | | `Blocked { waiting_for }` |
//! | 5 | any | any | any | any | `waitingFor` present | `Blocked { waiting_for }` |
//! | 6 | any | any | any | `busy` | | `Running` |
//! | 7 | any | any | any | `idle` | | `Idle` |
//! | 8 | `working` | any | any | absent | | `Running` (the pre-process row after `--bg`) |
//! | 9 | anything else | | | | unknown `state` or `status` value | `Unknown` |
//!
//! Row 2 exists because a dead process can leave its row marked `working`/`blocked`; such a
//! session must not reach the inbox, where Attach would target a process that is gone. Its
//! threshold keeps row 8 alive: m0/2 measured the legitimate no-pid `working` row for ~1.8 s.

mod assign;
mod cli;
mod error;
mod session;

pub use assign::{Assignment, assign, repo_of};
pub use cli::{
    ClaudeVersion, LIST_TIMEOUT, SUPPORTED_MAJOR, check_supported, check_version, cli_version,
    list_sessions, parse_version,
};
pub use error::{ClaudeError, Result};
pub use session::{
    Health, Kind, ORPHAN_AFTER, Session, SessionState, Status, now_ms, parse,
};
