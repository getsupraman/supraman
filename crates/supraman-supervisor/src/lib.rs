//! Keeps a workspace's wanted sessions alive: desired state, a pure [`reconcile`], restart with
//! backoff, a concurrency cap, and the Inbox rows ([`NeedsYou`]) for what only the human can fix.
//! No UI dependency: the app feeds [`Supervisor::tick`] one `claude agents --json --all` snapshot at a time.
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
//! | R2 | yes | 0, `fork_from` set (an Adopt) | | `Launch` with `--resume <fork_from>`: a copy under a new id that carries the conversation (claude cannot rename one); a live row of that uuid fails it; `fork_from` goes once the name's row shows another uuid; Remedy `Adopt` (Retry, Start fresh) | `an_adopted_session_is_launched_as_a_copy_and_a_failed_copy_offers_retry_or_start_fresh`, `an_adopted_session_launches_a_copy_of_its_uuid_with_the_supervised_flags_and_forks_once` |
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
//! One `max_working` for all workspaces of this home (the human's setting). [`tick_all`] ticks every workspace's
//! supervisor on one snapshot, one after another, with one [`Budget`]: it starts at `max_working`
//! minus this home's live `fm-` rows that are mid-turn ([`mid_turn`]: `status=busy`, not `state=working`), minus
//! every wanted session still settling after a start ([`Supervisor::starting`]); each launch/resume that succeeds takes a slot.
//! Another home's rows ([`is_foreign`]) take no slot: two Foremans do not share a budget. Tests
//! `two_started_workspaces_never_exceed_the_shared_budget_together`, `another_homes_busy_sessions_take_no_slot`.
//!
//! # Failures
//!
//! [`SupervisorMemory::record_result`] counts a failed launch/resume, and
//! [`SupervisorMemory::observe`] counts a start that is dead again after `settle` (20 s) but within
//! `stable_after` (60 s); 60 s alive clears the count. Backoff after failure 1..4: 5 s, 30 s, 2 min,
//! 10 min; the 5th gives up. A trust refusal is not a counted failure: it waits for Retry.
//! A resume whose output names no id is settled by the next snapshots: its own id live confirms it,
//! another live row of the name is a copy and a failure. A `claude stop` (R5, R14, Re-mint) that fails
//! [`STOP_FAILURES_SHOWN`] ticks in a row is a [`NeedsKind::StopFailed`] row until one succeeds;
//! a Re-mint whose stop fails stays due. Tests `a_resume_with_unrecognised_output_is_settled_by_the_next_snapshot`,
//! `a_stop_that_keeps_failing_is_an_inbox_row_until_it_stops`, `a_remint_whose_stop_fails_is_tried_again`.
//!
//! # Held
//!
//! [`Supervisor::hold`] (the app's check before launching), or a launch failing with
//! [`Failure::DaemonUnreachable`] that [`supraman_claude::daemon_mismatch`] confirms, holds every launch and
//! resume of the workspace behind one `LaunchFailed` Inbox row (Restart as administrator). Tests
//! `an_unreachable_service_foreman_cannot_reach_holds_with_one_row`,
//! `a_hold_from_the_apps_check_comes_before_any_launch`.

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
    DEFAULT_PERMISSION_MODE, DesiredState, RELAY_RESUME_PROMPT, RELAY_ROLE, REPO_ROLE, WORKSPACE_ROLE, WantedSession, Wanted,
    adopt_prompt, adopted_session, fill_paths, is_foreign, is_relay, relay_session, relay_session_name, render_repo_role, render_role, repo_session, repo_session_name, role_prompt,
    workspace_session_name,
};
pub use error::{Result, SupervisorError};
pub use executor::{LaunchConfig, Launcher, RoleToken, STOP_FAILURES_SHOWN, Supervisor, TokenProvider, session_identity};
pub use mint::{HttpMinter, MintError, MintRepo, MintRequest, Minted, MintedSession, MintedSessions, Minter, api_base, git_repo};
pub use memory::{Attempts, Failure, Millis, Policy, SECOND, SupervisorMemory};
pub use needs::{NeedsKind, NeedsYou, Remedy};
pub use reconcile::{Action, WaitReason, action_name, is_live, reconcile};
pub use tokens::{AppTokens, bearer, workstate_role};
