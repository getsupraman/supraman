//! The blocking read API v1 client. `GET` only: Foreman never writes to WorkState (C21).

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use serde::de::DeserializeOwned;

use crate::http::{Reply, pct_encode, read, server_reason};
use crate::model::{Backlog, Board, Inbox, ItemPage, Journal, Overview, Resolved, Sessions};

pub const API_HEADER: &str = "X-WorkState-Api";
pub const API_VERSION: &str = "1";

/// Never carries the token: the bearer is part of no message, and a server reason is cut short.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("Workman answered HTTP {status}{}", reason_suffix(.reason))]
    Http { status: u16, reason: String },
    /// No bearer: not signed in, or the sign-in must be done again.
    #[error("{0}")]
    Auth(String),
    #[error("Workman at {base} is unreachable ({reason})")]
    Transport { base: String, reason: String },
    /// A 2xx without `X-WorkState-Api: 1`: not a v1 server (or not WorkState).
    #[error("{0}")]
    Api(String),
    #[error("Workman's answer was not understood: {0}")]
    Decode(String),
}

fn reason_suffix(reason: &str) -> String {
    if reason.is_empty() { String::new() } else { format!(": {reason}") }
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn api_base(mcp_url: &str) -> String {
    supraman_core::url::origin(mcp_url)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BriefFor {
    Resume,
    Worker,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JournalQuery<'a> {
    pub scope: Option<&'a str>,
    pub since: Option<i64>,
    pub limit: Option<u32>,
    /// `transition,verification,report,finding,note,claim,link,created,carried`.
    pub kinds: &'a [&'a str],
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionsQuery<'a> {
    /// The last 30 minutes only.
    pub active: bool,
    pub name: Option<&'a str>,
    pub claude_session_id: Option<&'a str>,
}

/// The OAuth sign-in ([`crate::Auth`]) in the app and `sm`; a fixed token in tests.
pub trait BearerSource: Send + Sync {
    /// The access token to send now; `Err` says why there is none, without any token in it.
    fn bearer(&self) -> std::result::Result<String, String>;
    /// The server refused `token` with a 401: the next [`Self::bearer`] must not return it again.
    fn rejected(&self, _token: &str) {}
}

struct Fixed(String);

impl BearerSource for Fixed {
    fn bearer(&self) -> std::result::Result<String, String> {
        Ok(self.0.clone())
    }
}

#[derive(Clone)]
pub struct Client {
    base: String,
    source: Arc<dyn BearerSource>,
    agent: ureq::Agent,
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client").field("base", &self.base).field("token", &"<redacted>").finish()
    }
}

impl Client {
    /// `base_url` is the server's origin (see [`api_base`]), `token` a fixed bearer (tests).
    pub fn new(base_url: impl Into<String>, token: impl Into<String>) -> Self {
        Self::with_source(base_url, Arc::new(Fixed(token.into())))
    }

    /// The bearer is asked of `source` at each call, so a refreshed access token is used at once.
    pub fn with_source(base_url: impl Into<String>, source: Arc<dyn BearerSource>) -> Self {
        Self {
            base: base_url.into().trim_end_matches('/').to_string(),
            source,
            agent: crate::http::agent(Duration::from_secs(15)),
        }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub(crate) fn bearer(&self) -> Result<String> {
        self.source.bearer().map_err(Error::Auth)
    }

    pub(crate) fn rejected(&self, token: &str) {
        self.source.rejected(token);
    }

    pub fn overview(&self) -> Result<Overview> {
        self.json("/api/v1/overview")
    }

    pub fn board(&self, run_id: &str) -> Result<Board> {
        self.json(&format!("/api/v1/runs/{}/board", segment(run_id)))
    }

    pub fn item(&self, id: &str) -> Result<ItemPage> {
        self.json(&format!("/api/v1/items/{}", segment(id)))
    }

    /// `text/plain`: the verbatim `work_brief`.
    pub fn brief(&self, id: &str, for_: BriefFor) -> Result<String> {
        let for_ = match for_ {
            BriefFor::Resume => "resume",
            BriefFor::Worker => "worker",
        };
        self.text(&format!("/api/v1/items/{}/brief?for={for_}", segment(id)))
    }

    pub fn inbox(&self, project_id: Option<&str>) -> Result<Inbox> {
        self.json(&with_query("/api/v1/inbox", &[("project", project_id)]))
    }

    pub fn journal(&self, q: &JournalQuery<'_>) -> Result<Journal> {
        let since = q.since.map(|s| s.to_string());
        let limit = q.limit.map(|l| l.to_string());
        let kinds = (!q.kinds.is_empty()).then(|| q.kinds.join(","));
        self.json(&with_query(
            "/api/v1/journal",
            &[("scope", q.scope), ("since", since.as_deref()), ("limit", limit.as_deref()), ("kinds", kinds.as_deref())],
        ))
    }

    pub fn backlog(&self, project_id: &str) -> Result<Backlog> {
        self.json(&with_query("/api/v1/backlog", &[("project", Some(project_id))]))
    }

    pub fn sessions(&self, q: &SessionsQuery<'_>) -> Result<Sessions> {
        self.json(&with_query(
            "/api/v1/sessions",
            &[("active", q.active.then_some("true")), ("name", q.name), ("claudeSessionId", q.claude_session_id)],
        ))
    }

    /// A ref a human typed → its items (zero, one or many).
    pub fn resolve(&self, ref_: &str) -> Result<Resolved> {
        self.json(&with_query("/api/v1/resolve", &[("ref", Some(ref_))]))
    }

    fn json<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let body = self.get(path)?;
        serde_json::from_str(&body).map_err(|e| Error::Decode(format!("{path}: {e}")))
    }

    fn text(&self, path: &str) -> Result<String> {
        self.get(path)
    }

    /// One retry after a 401: an access token revoked or expired early is replaced by a fresh one.
    fn get(&self, path: &str) -> Result<String> {
        let token = self.bearer()?;
        match self.get_with(path, &token) {
            Err(Error::Http { status: 401, .. }) => {
                self.rejected(&token);
                let fresh = self.bearer()?;
                if fresh == token { self.get_with(path, &token) } else { self.get_with(path, &fresh) }
            }
            other => other,
        }
    }

    fn get_with(&self, path: &str, token: &str) -> Result<String> {
        let res = self.agent.get(format!("{}{path}", self.base)).header("Authorization", format!("Bearer {token}")).call();
        let Reply { status, headers, body } = read(res).map_err(|e| Error::Transport {
            base: self.base.clone(),
            reason: scrub(token, &e),
        })?;
        let api = headers.get(API_HEADER).and_then(|v| v.to_str().ok()).map(str::to_string);
        if !(200..300).contains(&status) {
            return Err(Error::Http {
                status,
                reason: scrub(token, &server_reason(&body)),
            });
        }
        match api.as_deref().map(str::trim) {
            Some(API_VERSION) => Ok(body),
            Some(v) => Err(Error::Api(format!("{} speaks read API {}, Supraman reads v{API_VERSION}", self.base, scrub(token, v)))),
            None => Err(Error::Api(format!("{} answered without {API_HEADER}: not a Workman read API v{API_VERSION}", self.base))),
        }
    }
}

pub(crate) fn scrub(token: &str, text: &str) -> String {
    if token.is_empty() { text.to_string() } else { text.replace(token, "<redacted>") }
}

fn segment(id: &str) -> String {
    pct_encode(id)
}

fn with_query(path: &str, pairs: &[(&str, Option<&str>)]) -> String {
    let query: Vec<String> = pairs.iter().filter_map(|(k, v)| v.map(|v| format!("{k}={}", pct_encode(v)))).collect();
    if query.is_empty() { path.to_string() } else { format!("{path}?{}", query.join("&")) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_base_is_the_mcp_urls_origin() {
        assert_eq!(api_base("http://127.0.0.1:5290/mcp"), "http://127.0.0.1:5290");
        assert_eq!(api_base("http://localhost:9/"), "http://localhost:9");
        assert_eq!(api_base("http://localhost:9"), "http://localhost:9");
        assert_eq!(api_base("https://ws.example?x=/y"), "https://ws.example", "a query is not part of the origin");
    }

    #[test]
    fn queries_encode_values_and_skip_absent_ones() {
        assert_eq!(with_query("/p", &[("a", None), ("ref", Some("m4/1 x"))]), "/p?ref=m4%2F1%20x");
        assert_eq!(with_query("/p", &[("a", None)]), "/p");
        assert_eq!(segment("a/b"), "a%2Fb");
    }

    #[test]
    fn debug_redacts_the_token() {
        let c = Client::new("http://127.0.0.1:1/", "fake-reader-secret");
        assert_eq!(c.base(), "http://127.0.0.1:1");
        let d = format!("{c:?}");
        assert!(!d.contains("fake-reader-secret") && d.contains("<redacted>"), "{d}");
    }
}
