//! Supraman's every Workman read: the read API v1 shapes ([`model`]), a blocking [`Client`] and the
//! `/api/stream` subscriber ([`Stream`]), and the OAuth sign-in whose access token they and every mint
//! carry ([`Auth`]). Read only (C21): the MCP tools stay the agents' write path.
//! Contract: Skills `docs/workstate/10-read-api.md`, frozen at v1, additive only.

pub mod auth;
mod client;
#[cfg(feature = "fake")]
pub mod fake;
mod health;
mod http;
pub mod model;
pub mod repo;
pub mod secret;
mod stream;
pub mod updates;

pub use auth::{Auth, AuthError, AuthStatus};
pub use client::{API_VERSION, BearerSource, BriefFor, Client, Error, JournalQuery, Result, SessionsQuery, api_base};
pub use health::{health, refuses};
pub use http::{Reply, agent as http_agent, agent_following as http_agent_following, pct_encode, read as read_reply, server_reason};
pub use model::*;
pub use secret::{MemoryStore, SecretStore};
pub use stream::{Stream, StreamConfig, StreamSignal};
