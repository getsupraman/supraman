//! Foreman's every WorkState read: the read API v1 shapes ([`model`]), a blocking [`Client`] and the
//! `/api/stream` subscriber ([`Stream`]). Read only (C21): the MCP tools stay the agents' write path.
//! Contract: Skills `docs/workstate/10-read-api.md`, frozen at v1, additive only.

mod client;
mod credentials;
mod health;
#[cfg(feature = "fake")]
pub mod fake;
pub mod model;
mod stream;

pub use client::{API_HEADER, API_VERSION, BriefFor, Client, Error, JournalQuery, Result, SessionsQuery, api_base};
pub use credentials::{CONTROLLER_TOKEN_KEY, CredentialsError, READER_TOKEN_KEY, client_from_settings};
pub use health::health;
pub use model::*;
pub use stream::{Stream, StreamConfig, StreamSignal};
