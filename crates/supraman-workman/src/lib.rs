//! Supraman's every Workman read, never a write (C21); contract: Skills `docs/workstate/10-read-api.md`, v1, additive only.

pub mod auth;
mod client;
#[cfg(feature = "fake")]
pub mod fake;
mod health;
mod http;
pub mod model;
pub mod repo;
pub mod status;
mod stream;
pub mod updates;

pub use auth::{Auth, AuthError, AuthStatus, SignedOut};
pub use client::{API_VERSION, BearerSource, BriefFor, Client, Error, JournalQuery, Result, SessionsQuery};
pub use health::{health, refuses};
pub use http::{Reply, agent as http_agent, agent_following as http_agent_following, pct_encode, read as read_reply, server_reason};
pub use model::*;
pub use stream::{Stream, StreamConfig, StreamSignal};

/// `http://127.0.0.1:<port>` of a port just freed: nothing listens there.
pub fn closed_base() -> String {
    let port = std::net::TcpListener::bind("127.0.0.1:0").and_then(|l| l.local_addr()).expect("bind a loopback port").port();
    format!("http://127.0.0.1:{port}")
}
