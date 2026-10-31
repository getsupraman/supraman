//! The only code in Foreman that knows the `claude` CLI. This is the read side: list sessions,
//! derive their health, map them onto a workspace's repos — and the write side ([`Claude`]):
//! launch, stop, rm, resume.
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
//! [`Session::supervised_health`] (the Inbox, the fleet panel, `sm roster`) reads B1/B3 without
//! `waitingFor` on an `fm-…` row as `Idle`: a Foreman session waits on peers, not on the human (m3/8).
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
mod daemon;
mod error;
pub mod forget;
pub mod human;
mod launch;
pub mod peer;
pub mod proc_tree;
mod session;
pub mod transcript;

pub use assign::{Assignment, assign, assign_with, main_repo, main_repo_walks};
pub use cli::{
    ClaudeVersion, LIST_TIMEOUT, MIN_VERSION, TESTED_VERSION, check_supported, check_version, cli_version,
    list_sessions, parse_version,
};
pub use daemon::{
    DaemonReach, UNREACHABLE, daemon_mismatch, daemon_status, elevation_mismatch, is_elevated, parse_daemon_status, pid_elevated, service_elevated, service_pid,
    task_run_level,
};
pub use error::{ClaudeError, Result};
pub use supraman_core::process::Output;
pub use forget::{ForgetError, Forgotten, find_transcript, forget_transcript, is_session_id};
pub use human::{Route, SendError, Sent, Sender, send_from_human};
pub use launch::{
    Claude, LaunchSpec, Removal, RmRefusal, SessionIdentity, WRITE_TIMEOUT, classify, name_taken, new_session_id, resume_argv, workstate_mcp_config,
    mcp_config_path, mcp_config_url, write_mcp_config,
};
pub use session::{
    Health, Kind, ORPHAN_AFTER, Session, SessionState, Status, now_ms, parse,
};
pub use transcript::{Launcher, Transcript, Transcripts, Usage, claude_dir, first_prompt};
