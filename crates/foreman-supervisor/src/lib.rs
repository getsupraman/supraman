//! Keeps a workspace's wanted sessions alive: desired state, a pure [`reconcile`], restart with
//! backoff, a concurrency cap, and the Inbox rows ([`NeedsYou`]) for what only the human can fix.
//! No UI dependency: the app (or a later tray daemon) feeds [`Supervisor::tick`] one
//! `claude agents --json --all` snapshot at a time.
//!
//! # Reconcile rules
//!
//! Per wanted session, by name, top to bottom; the first matching row wins. "live" is
//! [`is_live`]: a pid, or the fresh pid-less row right after `--bg`. `state` is never consulted
//! for liveness (m1/7: `done` on a live pid is Idle).
//!
//! | # | started | rows with the name | memory | action | test |
//! |---|---|---|---|---|---|
//! | R6 | any | 2 or more | | `NeedsYou::Duplicate`, act on none | `duplicates_are_reported_and_left_alone` |
//! | R5 | no | 1, live | | `Stop` (never `rm`) | `not_started_stops_live_wanted_sessions_only` |
//! | R5 | no | 0, or 1 not live | | nothing | same |
//! | R4 | yes | 1, live, `Blocked` | | `NeedsYou::Blocked` (Attach), never restarted | `blocked_is_reported_and_never_restarted` |
//! | R1 | yes | 1, live | | nothing (Running/Idle) | `started_and_live_needs_nothing_running_or_idle`, `done_with_live_pid_is_idle_and_never_relaunched`, `fresh_row_without_pid_is_starting_not_dead` |
//! | R9 | yes | 0, or 1 not live | trust needed | `NeedsYou::TrustNeeded` (RunInTerminal); only Retry clears it | `trust_needed_shows_the_command_and_is_never_retried_automatically` |
//! | R8 | yes | 0, or 1 not live | gave up | `NeedsYou::GaveUp` (Retry) | `gave_up_is_reported_and_not_attempted_until_retry` |
//! | R10 | yes | 0, or 1 not live | started < `settle` ago | `Wait(Settling)` | `just_started_waits_to_settle_instead_of_launching_again` |
//! | R7 | yes | 0, or 1 not live | before `next_allowed` | `Wait(Backoff)` + `NeedsYou::LaunchFailed` (Retry) | `backoff_defers_and_reports_launch_failed` |
//! | R2 | yes | 0 | | `Launch` | `absent_is_launched` |
//! | R13 | yes | 1 not live | alive < `respawn_grace` (30 s) ago | `Wait(Respawn)`: the daemon respawns a session killed mid-turn, and a resume while it still counts a dead one running starts a copy (m2/4) | `a_session_that_just_died_is_left_to_the_daemon_for_the_respawn_grace` |
//! | R3 | yes | 1 not live (stopped, done, orphaned) | | `Resume` from the row's cwd | `stopped_or_orphaned_is_resumed_from_its_own_cwd` |
//! | R14 | any | 1 live, name in `retired` (not wanted) | | `Stop`, before the relay; never launched or resumed | `retired_sessions_are_stopped_before_the_relay_and_never_relaunched` |
//!
//! Then the cap (R11, R12): `reconcile` gets the free slots of the shared [`Budget`]; Resumes take
//! them before Launches, each in desired order; the rest become `Wait(Cap)`. Tests
//! `cap_counts_working_fm_sessions_and_defers_the_rest`, `resumes_take_slots_before_launches`.
//! A relay ([`WantedSession::exempt_from_budget`]) is outside the cap: its launch/resume goes first and
//! takes no slot, its `working` row counts for nothing, and on Stop it is stopped after the others. Tests
//! `the_relay_is_outside_the_cap_and_its_working_row_takes_no_slot`,
//! `the_relay_launches_first_with_its_add_dir_and_never_takes_a_budget_slot`,
//! `stop_stops_the_relay_after_the_other_sessions`.
//! Rows whose name is neither wanted nor retired are never touched.
//!
//! # The shared budget
//!
//! One `max_working` for all workspaces (the human's setting). [`tick_all`] ticks every workspace's
//! supervisor on one snapshot, one after another, with one [`Budget`]: it starts at `max_working`
//! minus the live `fm-` rows that are mid-turn ([`mid_turn`]: `status=busy`, not `state=working`), minus
//! every wanted session still settling after a start ([`Supervisor::starting`]); each launch/resume that succeeds takes a slot. Test
//! `two_started_workspaces_never_exceed_the_shared_budget_together`.
//!
//! # Failures
//!
//! [`SupervisorMemory::record_result`] counts a failed launch/resume, and
//! [`SupervisorMemory::observe`] counts a start that is dead again after `settle` (20 s) but within
//! `stable_after` (60 s); 60 s alive clears the count. Backoff after failure 1..4: 5 s, 30 s, 2 min,
//! 10 min; the 5th gives up. A trust refusal is not a counted failure: it waits for Retry.

mod budget;
mod desired;
mod error;
mod executor;
mod memory;
pub mod mint;
mod needs;
mod reconcile;

pub use budget::{Budget, mid_turn, tick_all, working};
pub use desired::{
    DEFAULT_PERMISSION_MODE, DesiredState, RELAY_RESUME_PROMPT, RELAY_ROLE, REPO_ROLE, WORKSPACE_ROLE, WantedSession, Wanted,
    fill_paths, relay_session, relay_session_name, render_repo_role, render_role, repo_session, repo_session_name, role_prompt,
    workspace_session_name,
};
pub use error::{Result, SupervisorError};
pub use executor::{LaunchConfig, Launcher, RoleToken, Supervisor, TokenProvider, session_identity};
pub use mint::{HttpMinter, MintError, MintRequest, Minted, MintedSession, MintedSessions, Minter, api_base};
pub use memory::{Attempts, Failure, Millis, Policy, SECOND, SupervisorMemory};
pub use needs::{NeedsKind, NeedsYou, Remedy};
pub use reconcile::{Action, WaitReason, action_name, is_live, reconcile};
