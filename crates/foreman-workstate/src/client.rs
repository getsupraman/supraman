//! The blocking read API v1 client. `GET` only: Foreman never writes to WorkState (C21).

use std::fmt;
use std::time::Duration;

use serde::de::DeserializeOwned;

use crate::model::{Backlog, Board, Inbox, ItemPage, Journal, Overview, Resolved, Sessions};

/// The response header every v1 answer carries.
pub const API_HEADER: &str = "X-WorkState-Api";
pub const API_VERSION: &str = "1";

/// Never carries the token: the bearer is part of no message, and a server reason is cut short.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("WorkState answered HTTP {status}{}", reason_suffix(.reason))]
    Http { status: u16, reason: String },
    #[error("WorkState at {base} is unreachable ({reason})")]
    Transport { base: String, reason: String },
    /// A 2xx without `X-WorkState-Api: 1`: not a v1 server (or not WorkState).
    #[error("{0}")]
    Api(String),
    #[error("WorkState's answer was not understood: {0}")]
    Decode(String),
}

fn reason_suffix(reason: &str) -> String {
    if reason.is_empty() { String::new() } else { format!(": {reason}") }
}

pub type Result<T> = std::result::Result<T, Error>;

/// `http://127.0.0.1:5290/mcp` → `http://127.0.0.1:5290`.
pub fn api_base(mcp_url: &str) -> String {
    let start = mcp_url.find("://").map_or(0, |i| i + 3);
    match mcp_url[start..].find('/') {
        Some(i) => mcp_url[..start + i].to_string(),
        None => mcp_url.trim_end_matches('/').to_string(),
    }
}

/// `?for=` of [`Client::brief`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BriefFor {
    Resume,
    Worker,
}

/// `GET /api/v1/journal` filters; all optional.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JournalQuery<'a> {
    /// An item id.
    pub scope: Option<&'a str>,
    pub since: Option<i64>,
    pub limit: Option<u32>,
    /// `transition,verification,report,finding,note,claim,link,created,carried`.
    pub kinds: &'a [&'a str],
}

/// `GET /api/v1/sessions` filters; all optional.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionsQuery<'a> {
    /// The last 30 minutes only.
    pub active: bool,
    pub name: Option<&'a str>,
    pub claude_session_id: Option<&'a str>,
}

/// Blocking HTTP/1.1 without TLS (WorkState is on localhost), no proxy, no redirects, 15 s per call.
/// `Debug` never prints the token.
#[derive(Clone)]
pub struct Client {
    base: String,
    token: String,
    agent: ureq::Agent,
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client").field("base", &self.base).field("token", &"<redacted>").finish()
    }
}

impl Client {
    /// `base_url` is the server's origin (see [`api_base`]), `token` a reader (or agent) token.
    pub fn new(base_url: impl Into<String>, token: impl Into<String>) -> Self {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .proxy(None)
            .max_redirects(0)
            .timeout_global(Some(Duration::from_secs(15)))
            .build();
        Self {
            base: base_url.into().trim_end_matches('/').to_string(),
            token: token.into(),
            agent: config.into(),
        }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub(crate) fn token(&self) -> &str {
        &self.token
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

    fn get(&self, path: &str) -> Result<String> {
        let mut res = self
            .agent
            .get(format!("{}{path}", self.base))
            .header("Authorization", format!("Bearer {}", self.token))
            .call()
            .map_err(|e| Error::Transport {
                base: self.base.clone(),
                reason: self.scrub(&e.to_string()),
            })?;
        let status = res.status().as_u16();
        let api = res.headers().get(API_HEADER).and_then(|v| v.to_str().ok()).map(str::to_string);
        let body = res.body_mut().read_to_string().map_err(|e| Error::Transport {
            base: self.base.clone(),
            reason: self.scrub(&e.to_string()),
        })?;
        if !(200..300).contains(&status) {
            return Err(Error::Http {
                status,
                reason: self.scrub(&server_reason(&body)),
            });
        }
        match api.as_deref().map(str::trim) {
            Some(API_VERSION) => Ok(body),
            Some(v) => Err(Error::Api(format!("{} speaks read API {}, Foreman reads v{API_VERSION}", self.base, self.scrub(v)))),
            None => Err(Error::Api(format!("{} answered without {API_HEADER}: not a WorkState read API v{API_VERSION}", self.base))),
        }
    }

    /// Whatever a server or transport says, the token never leaves in it.
    fn scrub(&self, text: &str) -> String {
        if self.token.is_empty() { text.to_string() } else { text.replace(&self.token, "<redacted>") }
    }
}

/// What a refusal says: `error`/`title`/`message`/`detail` of a JSON body, else its start.
fn server_reason(body: &str) -> String {
    let reason = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| ["error", "title", "message", "detail"].iter().find_map(|k| v.get(*k)?.as_str().map(str::to_string)))
        .unwrap_or_else(|| body.trim().to_string());
    reason.chars().take(200).collect()
}

fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

fn segment(id: &str) -> String {
    encode(id)
}

fn with_query(path: &str, pairs: &[(&str, Option<&str>)]) -> String {
    let query: Vec<String> = pairs.iter().filter_map(|(k, v)| v.map(|v| format!("{k}={}", encode(v)))).collect();
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
