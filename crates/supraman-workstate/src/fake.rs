//! A fake WorkState read API on 127.0.0.1 for tests, with its OAuth server behind [`FakeWorkState::enable_oauth`].

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

use crate::auth::form_decode;

const STREAM_TARGET: &str = "/api/stream";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seen {
    pub method: String,
    pub target: String,
    pub authorization: Option<String>,
    pub content_type: Option<String>,
    pub body: String,
    /// Every header as sent, names lowercased.
    pub headers: Vec<(String, String)>,
}

impl Seen {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub status: u16,
    pub content_type: &'static str,
    /// The value of `X-Workman-Api` and `X-WorkState-Api`, both sent as Workman 1.2 does; `None` sends neither.
    pub api: Option<&'static str>,
    pub body: String,
    pub headers: Vec<(String, String)>,
    /// A binary body sent instead of `body`.
    pub bytes: Option<Vec<u8>>,
    /// The body sent in chunks of this many bytes, this long apart: a download that takes a while.
    pub pace: Option<(usize, std::time::Duration)>,
}

impl Answer {
    pub fn json(body: impl Into<String>) -> Self {
        Self { status: 200, content_type: "application/json", api: Some("1"), body: body.into(), headers: Vec::new(), bytes: None, pace: None }
    }

    pub fn bytes(bytes: Vec<u8>) -> Self {
        Self { content_type: "application/octet-stream", api: None, bytes: Some(bytes), ..Self::json("") }
    }

    pub fn paced(self, chunk: usize, every: std::time::Duration) -> Self {
        Self { pace: Some((chunk.max(1), every)), ..self }
    }

    pub fn text(body: impl Into<String>) -> Self {
        Self { content_type: "text/plain; charset=utf-8", ..Self::json(body) }
    }

    pub fn status(status: u16, body: impl Into<String>) -> Self {
        Self { status, ..Self::json(body) }
    }

    pub fn api(self, api: Option<&'static str>) -> Self {
        Self { api, ..self }
    }

    pub fn header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// The head, then the body as [`Self::pace`] says; an error is the client gone.
    pub fn send(&self, stream: &mut impl Write) -> std::io::Result<()> {
        let Some(bytes) = &self.bytes else {
            if self.pace.is_none() {
                return stream.write_all(self.to_http().as_bytes());
            }
            return Self { bytes: Some(self.body.clone().into_bytes()), ..self.clone() }.send(stream);
        };
        let http = Self { body: String::new(), bytes: None, ..self.clone() }.to_http();
        let head = http.replacen("Content-Length: 0\r\n", &format!("Content-Length: {}\r\n", bytes.len()), 1);
        stream.write_all(head.as_bytes())?;
        match self.pace {
            None => stream.write_all(bytes),
            Some((chunk, every)) => {
                for part in bytes.chunks(chunk) {
                    stream.write_all(part)?;
                    stream.flush()?;
                    std::thread::sleep(every);
                }
                Ok(())
            }
        }
    }

    /// The bytes of the whole response, `Connection: close`.
    pub fn to_http(&self) -> String {
        let mut head = self.api.map(|v| format!("X-Workman-Api: {v}\r\nX-WorkState-Api: {v}\r\n")).unwrap_or_default();
        for (k, v) in &self.headers {
            head.push_str(&format!("{k}: {v}\r\n"));
        }
        format!(
            "HTTP/1.1 {} X\r\nContent-Type: {}\r\n{head}Content-Length: {}\r\nConnection: close\r\n\r\n{}",
            self.status,
            self.content_type,
            self.body.len(),
            self.body
        )
    }
}

type Routes = Arc<Mutex<Vec<(String, Answer)>>>;

pub struct FakeWorkState {
    base: String,
    seen: Arc<Mutex<Vec<Seen>>>,
    routes: Routes,
    /// Open `/api/stream` connections, past their response head.
    streams: Arc<Mutex<Vec<TcpStream>>>,
    oauth: Arc<Mutex<Option<Arc<FakeAuth>>>>,
}

/// One request off `stream`: (method, target, Authorization, body).
pub fn read_request(stream: &TcpStream) -> Option<(String, String, Option<String>, String)> {
    read_seen(stream).map(|s| (s.method, s.target, s.authorization, s.body))
}

fn read_seen(stream: &TcpStream) -> Option<Seen> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let (mut authorization, mut content_type, mut length, mut headers) = (None, None, 0, Vec::new());
    loop {
        let mut h = String::new();
        reader.read_line(&mut h).ok()?;
        let Some((k, v)) = h.trim_end().split_once(':') else { break };
        headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        if k.eq_ignore_ascii_case("authorization") {
            authorization = Some(v.trim().to_string());
        } else if k.eq_ignore_ascii_case("content-type") {
            content_type = Some(v.trim().to_string());
        } else if k.eq_ignore_ascii_case("content-length") {
            length = v.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).ok()?;
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("").to_string();
    Some(Seen { method, target, authorization, content_type, body: String::from_utf8_lossy(&body).into_owned(), headers })
}

impl FakeWorkState {
    /// `routes`: `<METHOD> <target>` or, for any method, the exact target (path and query) → answer.
    /// Anything else is a v1 404, except
    /// `/api/stream`, which stays open until [`Self::push`] fails on it or [`Self::drop_streams`].
    pub fn start(routes: Vec<(String, Answer)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a fake WorkState");
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let routes: Routes = Arc::new(Mutex::new(routes));
        let streams: Arc<Mutex<Vec<TcpStream>>> = Arc::default();
        let oauth: Arc<Mutex<Option<Arc<FakeAuth>>>> = Arc::default();
        let (table, log, open, guard) = (routes.clone(), seen.clone(), streams.clone(), oauth.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let (routes, log, open, guard) = (table.clone(), log.clone(), open.clone(), guard.clone());
                std::thread::spawn(move || {
                    let Some(seen) = read_seen(&stream) else { return };
                    let (method, target, authorization, body) = (&seen.method, &seen.target, &seen.authorization, &seen.body);
                    let oauth = guard.lock().unwrap().clone();
                    let mut answer = oauth.as_ref().and_then(|a| a.handle(method, target, body));
                    if answer.is_none()
                        && let Some(a) = &oauth
                        && target.starts_with("/api/")
                        && !a.accepts(authorization.as_deref())
                    {
                        answer = Some(a.unauthorized());
                    }
                    let keyed = format!("{method} {target}");
                    let answer = answer.or_else(|| {
                        let routes = routes.lock().unwrap();
                        let find = |key: &str| routes.iter().find(|(t, _)| t == key).map(|(_, a)| a.clone());
                        find(&keyed).or_else(|| find(target))
                    });
                    let stream_open = answer.is_none() && target == STREAM_TARGET;
                    let answer = answer.unwrap_or_else(|| Answer::status(404, format!(r#"{{"error":"no route {target}"}}"#)));
                    log.lock().unwrap().push(seen);
                    if stream_open {
                        let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nX-Workman-Api: 1\r\nX-WorkState-Api: 1\r\n\r\n: connected\n\n";
                        if stream.write_all(head.as_bytes()).is_ok() {
                            open.lock().unwrap().push(stream);
                        }
                        return;
                    }
                    let _ = answer.send(&mut stream);
                });
            }
        });
        Self { base, seen, routes, streams, oauth }
    }

    /// From now on the fake is also the OAuth authorization server of its origin, and `/api/*`
    /// takes only its live access tokens (else 401 with `WWW-Authenticate`).
    pub fn enable_oauth(&self) -> Arc<FakeAuth> {
        let auth = Arc::new(FakeAuth::new(&self.base));
        *self.oauth.lock().unwrap() = Some(auth.clone());
        auth
    }

    pub fn set(&self, target: impl Into<String>, answer: Answer) {
        let target = target.into();
        let mut routes = self.routes.lock().unwrap();
        match routes.iter_mut().find(|(t, _)| *t == target) {
            Some(route) => route.1 = answer,
            None => routes.push((target, answer)),
        }
    }

    /// One SSE frame `data: <data>` to every open stream; how many took it.
    pub fn push(&self, data: &str) -> usize {
        let frame = format!("data: {data}\n\n");
        let mut streams = self.streams.lock().unwrap();
        streams.retain_mut(|s| s.write_all(frame.as_bytes()).and_then(|()| s.flush()).is_ok());
        streams.len()
    }

    /// Open streams, as of the last [`Self::push`].
    pub fn streams(&self) -> usize {
        self.streams.lock().unwrap().len()
    }

    /// Closes every open stream: the subscriber sees the connection end.
    pub fn drop_streams(&self) {
        for s in self.streams.lock().unwrap().drain(..) {
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    /// The URL a `settings.toml` would name: `<base>/mcp`.
    pub fn mcp_url(&self) -> String {
        format!("{}/mcp", self.base)
    }

    pub fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

struct Code {
    client_id: String,
    redirect_uri: String,
    challenge: String,
    grant: u64,
}

/// `fake-<what>-<16 random hex>`: no id is guessable from another fake or another run.
fn fresh_id(what: &str) -> String {
    let mut r = [0u8; 8];
    getrandom::fill(&mut r).expect("the OS random source");
    format!("fake-{what}-{}", r.iter().map(|b| format!("{b:02x}")).collect::<String>())
}

#[derive(Default)]
struct AuthState {
    clients: Vec<String>,
    registered: Vec<String>,
    codes: HashMap<String, Code>,
    /// Access token → (grant, expiry).
    access: HashMap<String, (u64, Instant)>,
    /// Refresh token → (grant, already redeemed).
    refresh: HashMap<String, (u64, bool)>,
    revoked: HashSet<u64>,
    next: u64,
    access_ttl: u64,
    refresh_len: usize,
    fail_refresh: bool,
    deny: bool,
    issued: Vec<String>,
    refusals: Vec<String>,
}

/// Shaped like WorkState's (Skills `docs/workstate/10-read-api.md` rule 5); authorize redirects at once: the test is the browser.
pub struct FakeAuth {
    base: String,
    state: Mutex<AuthState>,
    /// (stalled, token requests waiting): see [`Self::stall_tokens`].
    stall: (Mutex<(bool, usize)>, Condvar),
}

impl FakeAuth {
    pub fn new(base: &str) -> Self {
        Self {
            base: base.trim_end_matches('/').to_string(),
            state: Mutex::new(AuthState { access_ttl: 3600, ..AuthState::default() }),
            stall: (Mutex::new((false, 0)), Condvar::new()),
        }
    }

    fn st(&self) -> std::sync::MutexGuard<'_, AuthState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// With a trailing slash, as WorkState's.
    pub fn issuer(&self) -> String {
        format!("{}/", self.base)
    }

    /// The server's database was wiped: no client and no grant is known any more.
    pub fn forget_clients(&self) {
        let mut st = self.st();
        st.clients.clear();
        st.refresh.clear();
        st.access.clear();
    }

    pub fn set_access_ttl(&self, secs: u64) {
        self.st().access_ttl = secs;
    }

    /// Refresh tokens issued from now on are padded to `bytes` (WorkState 1.0.0's exceed 2560).
    pub fn set_refresh_len(&self, bytes: usize) {
        self.st().refresh_len = bytes;
    }

    pub fn fail_refresh(&self, fail: bool) {
        self.st().fail_refresh = fail;
    }

    /// The operator refuses at the authorize step.
    pub fn deny(&self, deny: bool) {
        self.st().deny = deny;
    }

    pub fn registrations(&self) -> usize {
        self.st().registered.len()
    }

    pub fn registered(&self) -> Vec<String> {
        self.st().registered.clone()
    }

    /// While on, `/connect/token` requests wait unanswered; off releases them.
    pub fn stall_tokens(&self, on: bool) {
        let (lock, cvar) = &self.stall;
        lock.lock().unwrap_or_else(|e| e.into_inner()).0 = on;
        cvar.notify_all();
    }

    /// Token requests waiting in a stall.
    pub fn stalled(&self) -> usize {
        self.stall.0.lock().unwrap_or_else(|e| e.into_inner()).1
    }

    fn wait_out_stall(&self) {
        let (lock, cvar) = &self.stall;
        let mut s = lock.lock().unwrap_or_else(|e| e.into_inner());
        s.1 += 1;
        while s.0 {
            s = cvar.wait(s).unwrap_or_else(|e| e.into_inner());
        }
        s.1 -= 1;
    }

    /// Every OAuth error this fake answered: `<endpoint> <error>: <why>`. Never a token.
    pub fn refusals(&self) -> Vec<String> {
        self.st().refusals.clone()
    }

    /// Every token issued so far, access and refresh: what a test greps a home for.
    pub fn issued(&self) -> Vec<String> {
        self.st().issued.clone()
    }

    pub fn revoked(&self) -> usize {
        self.st().revoked.len()
    }

    /// `Authorization: Bearer <a live access token of a live grant>`.
    pub fn accepts(&self, authorization: Option<&str>) -> bool {
        let Some(token) = authorization.and_then(|a| a.strip_prefix("Bearer ")) else { return false };
        let st = self.st();
        st.access.get(token).is_some_and(|(grant, until)| *until > Instant::now() && !st.revoked.contains(grant))
    }

    pub fn unauthorized(&self) -> Answer {
        Answer { status: 401, content_type: "text/plain", ..Answer::json("") }
            .header("WWW-Authenticate", format!(r#"Bearer resource_metadata="{}/.well-known/oauth-protected-resource/api""#, self.base))
    }

    fn issue(&self, st: &mut AuthState, grant: u64) -> serde_json::Value {
        let jti = fresh_id("jti");
        let claims = URL_SAFE_NO_PAD.encode(format!(r#"{{"sub":"fake-operator","name":"Fake Operator","jti":"{jti}"}}"#));
        let access = format!("eyJhbGciOiJub25lIn0.{claims}.{}", fresh_id("at"));
        let mut refresh = fresh_id("rt");
        let pad = st.refresh_len.saturating_sub(refresh.len());
        refresh.push_str(&"r".repeat(pad));
        st.access.insert(access.clone(), (grant, Instant::now() + Duration::from_secs(st.access_ttl)));
        st.refresh.insert(refresh.clone(), (grant, false));
        st.issued.push(access.clone());
        st.issued.push(refresh.clone());
        serde_json::json!({
            "access_token": access, "token_type": "Bearer", "expires_in": st.access_ttl,
            "refresh_token": refresh, "scope": crate::auth::SCOPE,
        })
    }

    fn error(status: u16, error: &str) -> Answer {
        Answer { api: None, ..Answer::status(status, format!(r#"{{"error":"{error}"}}"#)) }
    }

    /// An OAuth error with an `error_description`, recorded in [`Self::refusals`].
    fn refuse(st: &mut AuthState, endpoint: &str, error: &str, why: &str) -> Answer {
        st.refusals.push(format!("{endpoint} {error}: {why}"));
        let body = serde_json::json!({ "error": error, "error_description": why });
        Answer { api: None, ..Answer::status(400, body.to_string()) }
    }

    fn json(v: serde_json::Value) -> Answer {
        Answer { api: None, ..Answer::json(v.to_string()) }
    }

    fn plain(status: u16, text: &str) -> Answer {
        Answer { status, api: None, ..Answer::text(text) }
    }

    pub fn handle(&self, method: &str, target: &str, body: &str) -> Option<Answer> {
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        let base = self.base.clone();
        Some(match (method, path) {
            ("GET", "/.well-known/oauth-protected-resource/api") => Self::json(serde_json::json!({
                "resource": format!("{base}/api"),
                "authorization_servers": [base],
                "scopes_supported": ["workstate.read", "workstate.sessions", "offline_access"],
            })),
            ("GET", "/.well-known/oauth-authorization-server") => Self::json(serde_json::json!({
                "issuer": self.issuer(),
                "authorization_endpoint": format!("{base}/connect/authorize"),
                "token_endpoint": format!("{base}/connect/token"),
                "registration_endpoint": format!("{base}/connect/register"),
                "revocation_endpoint": format!("{base}/connect/revoke"),
                "code_challenge_methods_supported": ["S256"],
                "grant_types_supported": ["authorization_code", "refresh_token"],
            })),
            ("POST", "/connect/register") => {
                let v: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
                let public = v["token_endpoint_auth_method"] == "none";
                let loopback = v["redirect_uris"]
                    .as_array()
                    .is_some_and(|r| !r.is_empty() && r.iter().all(|u| u.as_str() == Some("http://127.0.0.1/callback")));
                if !public || !loopback {
                    return Some(Self::error(400, "invalid_client_metadata"));
                }
                let mut st = self.st();
                let id = fresh_id("client");
                st.registered.push(id.clone());
                st.clients.push(id.clone());
                Answer { status: 201, ..Self::json(serde_json::json!({ "client_id": id, "token_endpoint_auth_method": "none" })) }
            }
            ("GET", "/connect/authorize") => {
                let q = form_decode(query);
                let resources = query.split('&').filter(|p| p.starts_with("resource=")).count();
                let get = |k: &str| q.get(k).cloned().unwrap_or_default();
                let mut st = self.st();
                if !st.clients.contains(&get("client_id")) {
                    return Some(Self::plain(400, "unknown client"));
                }
                let redirect = get("redirect_uri");
                if !redirect.starts_with("http://127.0.0.1:") || !redirect.ends_with("/callback") {
                    return Some(Self::plain(400, "bad redirect_uri"));
                }
                let state = get("state");
                let refuse = |error: &str| Self::plain(302, "").header("Location", format!("{redirect}?error={error}&state={state}"));
                if resources != 1 || get("resource") != format!("{base}/api") {
                    return Some(refuse("invalid_target"));
                }
                if get("response_type") != "code" || get("code_challenge_method") != "S256" || get("code_challenge").is_empty() {
                    return Some(refuse("invalid_request"));
                }
                if !get("scope").split(' ').any(|s| s == "offline_access") {
                    return Some(refuse("invalid_scope"));
                }
                if st.deny {
                    return Some(refuse("access_denied"));
                }
                st.next += 1;
                let grant = st.next;
                let code = fresh_id("code");
                st.codes.insert(
                    code.clone(),
                    Code { client_id: get("client_id"), redirect_uri: redirect.clone(), challenge: get("code_challenge"), grant },
                );
                Self::plain(302, "").header("Location", format!("{redirect}?code={code}&state={state}"))
            }
            ("POST", "/connect/token") => {
                self.wait_out_stall();
                let f = form_decode(body);
                let get = |k: &str| f.get(k).cloned().unwrap_or_default();
                let mut st = self.st();
                let grant_type = get("grant_type");
                let endpoint = format!("token ({grant_type})");
                if !st.clients.contains(&get("client_id")) {
                    return Some(Self::refuse(&mut st, &endpoint, "invalid_client", "unknown client"));
                }
                match grant_type.as_str() {
                    "authorization_code" => {
                        let Some(code) = st.codes.remove(&get("code")) else {
                            return Some(Self::refuse(&mut st, &endpoint, "invalid_grant", "unknown or used code"));
                        };
                        let pkce = URL_SAFE_NO_PAD.encode(Sha256::digest(get("code_verifier").as_bytes()));
                        if code.client_id != get("client_id") || code.redirect_uri != get("redirect_uri") || code.challenge != pkce {
                            return Some(Self::refuse(&mut st, &endpoint, "invalid_grant", "client, redirect or PKCE mismatch"));
                        }
                        Self::json(self.issue(&mut st, code.grant))
                    }
                    "refresh_token" => {
                        if st.fail_refresh {
                            return Some(Self::refuse(&mut st, &endpoint, "invalid_grant", "refused on request"));
                        }
                        let rt = get("refresh_token");
                        let grant = match st.refresh.get(&rt).copied() {
                            Some((grant, _)) if st.revoked.contains(&grant) => {
                                return Some(Self::refuse(&mut st, &endpoint, "invalid_grant", "revoked"));
                            }
                            Some((grant, false)) => grant,
                            Some(_) => return Some(Self::refuse(&mut st, &endpoint, "invalid_grant", "redeemed")),
                            None => return Some(Self::refuse(&mut st, &endpoint, "invalid_grant", "unknown")),
                        };
                        st.refresh.insert(rt, (grant, true));
                        Self::json(self.issue(&mut st, grant))
                    }
                    _ => Self::refuse(&mut st, &endpoint, "unsupported_grant_type", "unsupported grant_type"),
                }
            }
            ("POST", "/connect/revoke") => {
                let f = form_decode(body);
                let token = f.get("token").cloned().unwrap_or_default();
                let mut st = self.st();
                if let Some((grant, _)) = st.refresh.get(&token).copied() {
                    st.revoked.insert(grant);
                } else {
                    st.access.remove(&token);
                }
                Self::plain(200, "")
            }
            _ => return None,
        })
    }
}

/// The query of a redirect `Location`, for tests that play the browser.
pub fn location_query(location: &str) -> BTreeMap<String, String> {
    form_decode(location.split_once('?').map_or("", |(_, q)| q))
}
