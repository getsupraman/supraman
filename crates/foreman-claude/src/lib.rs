//! The only code in Foreman that knows the `claude` CLI. This is the read side: list sessions,
//! derive their health, map them onto a platform's repos. Launching, stop and attach are M2.
//!
//! # Health
//!
//! [`Session::health`] walks the table for the row's `kind` top to bottom; the first matching row
//! wins. "live state" means `state` is `working` or `blocked`; "age" is `now - startedAt` (a missing
//! `startedAt` counts as old).
//!
//! `background` rows. `state` describes the turn, not the process: a session whose turn ended is
//! `state=done, status=idle` on a live pid (m1/7). So liveness comes from `pid` first.
//!
//! | # | `pid` | `state` | `status` | other | health |
//! |---|---|---|---|---|---|
//! | B1 | present | any | `waiting` | | `Blocked { waiting_for }` |
//! | B2 | present | any | any | `waitingFor` present | `Blocked { waiting_for }` |
//! | B3 | present | `blocked` | absent | | `Blocked { waiting_for }` |
//! | B4 | present | any | `busy` | | `Running` |
//! | B5 | present | any | `idle` | | `Idle` |
//! | B6 | present | `done` or `stopped` | absent | | `Idle` |
//! | B7 | present | `working` | absent | | `Running` (pid up, status not yet written) |
//! | B8 | present | anything else | | | `Unknown` |
//! | B9 | absent | `done` or `stopped` | any | | `Stopped` |
//! | B10 | absent | live state | any | age > [`ORPHAN_AFTER`] | `Orphaned { last_state }` |
//! | B11 | absent | `working` | any | | `Running` (the pre-process row after `--bg`) |
//! | B12 | absent | `blocked` | any | | `Blocked { waiting_for }` |
//! | B13 | absent | anything else | | | `Unknown` |
//!
//! B10 exists because a dead process can leave its row marked `working`/`blocked`; such a
//! session must not reach the inbox, where Attach would target a process that is gone. Its
//! threshold keeps B11 alive: m0/2 measured the legitimate no-pid `working` row for ~1.8 s.
//!
//! Every other `kind` (interactive rows carry `status` and `pid`, no `state`):
//!
//! | # | `state` | `status` | other | health |
//! |---|---|---|---|---|
//! | I1 | `stopped` or `done` | any | | `Stopped` |
//! | I2 | `blocked` | any | | `Blocked { waiting_for }` |
//! | I3 | any | `waiting` | | `Blocked { waiting_for }` |
//! | I4 | any | any | `waitingFor` present | `Blocked { waiting_for }` |
//! | I5 | any | `busy` | | `Running` |
//! | I6 | any | `idle` | | `Idle` |
//! | I7 | `working` | absent | | `Running` |
//! | I8 | anything else | | unknown `state` or `status` value | `Unknown` |

mod assign;
mod cli;
mod error;
mod session;

pub use assign::{Assignment, assign, main_repo, repo_of};
pub use cli::{
    ClaudeVersion, LIST_TIMEOUT, SUPPORTED_MAJOR, check_supported, check_version, cli_version,
    list_sessions, parse_version,
};
pub use error::{ClaudeError, Result};
pub use session::{
    Health, Kind, ORPHAN_AFTER, Session, SessionState, Status, now_ms, parse,
};
