//! The launch of Supraman's own sessions ([+] tabs and scheduled runs) and the Workman tokens they are minted.

mod launch;
mod leftover;
pub mod mint;
mod tokens;

pub use launch::{BgSession, HAND_ROLE, LaunchConfig, Launcher, Millis, RoleToken, TokenProvider, is_foreign, is_live, mid_turn, working};
pub use leftover::end_controllers;
pub use mint::{
    Absent, Bearer, Gone, HttpMinter, Keeper, MintError, MintRepo, MintRequest, Minted, MintedSession, MintedSessions, Minter, Revoked, Unlisted, git_repo, runs_signed_out, CHECK_EVERY, Checked, Keeping, PING_TIMEOUT, RENEW_MARGIN, UNLISTED_LAPSE,
    former_holder, header, launched_by, mint_as, remove_mcp_config, revoke, revoke_deleted_runs,
};
pub use tokens::{AppTokens, bearer, mcp_url, sign_in_generation, signing_in, workman_role};
