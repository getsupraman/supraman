//! The only code in Supraman that knows the `claude` CLI: list sessions, derive their health, map them onto repos, and [`Claude`]'s writes.
//! A `background` row's `state` is the turn, not the process (`done` on a live pid is idle), so liveness comes from `pid` first;
//! a turn that ends asking in plain text is `state=blocked, status=idle` with no `waitingFor` (claude 2.1.316).

mod assign;
mod cli;
mod daemon;
mod error;
pub mod forget;
mod launch;
pub mod peer;
mod plugin_server;
pub mod proc_tree;
mod session;
pub mod transcript;

pub use assign::{Holding, Placement, holding, main_repo, main_repo_walks, place, place_with};
pub use cli::{
    ClaudeVersion, LIST_TIMEOUT, MIN_VERSION, TESTED_VERSION, check_supported, check_version, cli_version,
    list_sessions, parse_version,
};
pub use daemon::{
    DaemonReach, UNREACHABLE, daemon_mismatch, daemon_status, elevation_mismatch, parse_daemon_status, service_elevated, service_pid,
    task_run_level,
};
pub use error::{ClaudeError, Result};
pub use forget::{ForgetError, Forgotten, find_transcript, forget_transcript, is_session_id};
pub use launch::{
    Claude, LaunchSpec, Removal, RmRefusal, SessionIdentity, WRITE_TIMEOUT, classify, name_taken, new_session_id, resume_argv,
    MCP_SERVER, mcp_config_names, mcp_config_path, mcp_config_url, mcp_header, mcp_header_command, mcp_header_home, mcp_token_key, write_mcp_config_helped, write_mcp_config_servers,
};
pub use plugin_server::plugin_server;
pub use session::{
    Health, Kind, ORPHAN_AFTER, Session, SessionState, Status, now_ms, parse,
};
pub use transcript::{Launcher, Transcript, Transcripts, Usage, chains, continued_in, first_prompt, newest, newest_in};
