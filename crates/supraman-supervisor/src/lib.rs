//! Keeps a workspace's wanted sessions alive, fed one `claude agents --json --all` snapshot per [`Supervisor::tick`].
//!
//! # Reconcile rules
//!
//! Per wanted session, by name, top to bottom; the first matching row wins; "live" is [`is_live`].
//!
//! | # | started | rows with the name | memory | action | test |
//! |---|---|---|---|---|---|
//! | R6 | any | 2 or more | | `NeedsYou::Duplicate`, act on none | `duplicates_are_reported_and_left_alone` |
//! | R5 | no | 1, live | | `Stop` (never `rm`) | `not_started_stops_live_wanted_sessions_only` |
//! | R5 | no | 0, or 1 not live | | nothing | same |
//! | R4 | yes | 1, live, `Blocked` | | `NeedsYou::Blocked` (Attach), never restarted | `blocked_is_reported_and_never_restarted` |
//! | R1 | yes | 1, live | | nothing (Running/Idle) | `started_and_live_needs_nothing_running_or_idle`, `done_with_live_pid_is_idle_and_never_relaunched`, `fresh_row_without_pid_is_starting_not_dead` |
//! | R9 | yes | 0, or 1 not live | trust needed | `NeedsYou::TrustNeeded` (RunInTerminal, which runs `claude` there; the Inbox row also offers Retry); only Retry clears it | `trust_needed_shows_the_command_and_is_never_retried_automatically` |
//! | R8 | yes | 0, or 1 not live | gave up | `NeedsYou::GaveUp` (Retry) | `gave_up_is_reported_and_not_attempted_until_retry` |
//! | R10 | yes | 0, or 1 not live | started < `settle` ago | `Wait(Settling)` | `just_started_waits_to_settle_instead_of_launching_again` |
//! | R7 | yes | 0, or 1 not live | before `next_allowed` | `Wait(Backoff)` + `NeedsYou::LaunchFailed` (Retry) | `backoff_defers_and_reports_launch_failed` |
//! | R2 | yes | 0 | | `Launch` | `absent_is_launched` |
//! | R2 | yes | 0, `fork_from` set (an Adopt) | | `Launch` with `--resume <fork_from>`: a copy under a new id that carries the conversation (claude cannot rename one); a live row of that uuid fails it; `fork_from` goes once the name's row shows another uuid; Remedy `Adopt` (Retry, Start fresh) | `an_adopted_session_is_launched_as_a_copy_and_a_failed_copy_offers_retry_or_start_fresh`, `an_adopted_session_launches_a_copy_of_its_uuid_with_the_supervised_flags_and_forks_once` |
//! | R13 | yes | 1 not live | alive < `respawn_grace` (30 s) ago | `Wait(Respawn)`: the daemon respawns a session killed mid-turn, and a resume while it still counts a dead one running starts a copy | `a_session_that_just_died_is_left_to_the_daemon_for_the_respawn_grace` |
//! | R3 | yes | 1 not live (stopped, done, orphaned) | | `Resume` from the row's cwd | `stopped_or_orphaned_is_resumed_from_its_own_cwd` |
//! | R14 | any | 1 live, name in `retired` (not wanted) | | `Stop`, before the relay; never launched or resumed | `retired_sessions_are_stopped_before_the_relay_and_never_relaunched` |
//!
//! Then the cap (R11, R12): Resumes take the shared [`Budget`]'s free slots before Launches, the rest
//! `Wait(Cap)`; a relay ([`WantedSession::exempt_from_budget`]) takes no slot. Other names are never touched.

mod budget;
mod desired;
mod error;
mod executor;
mod memory;
pub mod mint;
mod needs;
mod reconcile;
mod tokens;

pub use budget::{Budget, mid_turn, tick_all, working};
pub use desired::{
    DEFAULT_PERMISSION_MODE, DesiredState, HAND_ROLE, RELAY_RESUME_PROMPT, RELAY_ROLE, REPO_ROLE, WORKSPACE_ROLE, WantedSession, Wanted,
    adopt_prompt, adopted_session, fill_paths, is_foreign, is_relay, relay_session, relay_session_name, remove_workspace, render_repo_role, render_role, repo_session, repo_session_name, role_prompt,
    workspace_session_name,
};
pub use error::{Result, SupervisorError};
pub use executor::{LaunchConfig, Launcher, RoleToken, STOP_FAILURES_SHOWN, Supervisor, TokenProvider, session_identity};
pub use mint::{
    Absent, Bearer, Gone, HttpMinter, Keeper, MintError, MintRepo, MintRequest, Minted, MintedSession, MintedSessions, Minter, Revoked, Unlisted, git_repo, runs_signed_out, CHECK_EVERY, Checked, Keeping, PING_TIMEOUT, RENEW_MARGIN, UNLISTED_LAPSE,
    former_holder, header, launched_by, mint_as, remove_mcp_config, revoke, revoke_deleted_runs,
};
pub use memory::{Attempts, Failure, Millis, Policy, SECOND, SupervisorMemory};
pub use needs::{NeedsKind, NeedsYou, Remedy};
pub use reconcile::{Action, WaitReason, action_name, is_live, reconcile};
pub use tokens::{AppTokens, bearer, mcp_url, sign_in_generation, signing_in, workman_role};
