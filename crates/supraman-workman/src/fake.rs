//! A fake Workman on 127.0.0.1 for tests: the read API, plus OAuth, agent sessions and a journal once enabled.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
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
    /// The value of `X-Workman-Api`; `None` sends none.
    pub api: Option<&'static str>,
    pub body: String,
    pub headers: Vec<(String, String)>,
    /// A binary body sent instead of `body`.
    pub bytes: Option<Vec<u8>>,
    /// The body sent in chunks of this many bytes, this long apart: a download that takes a while.
    pub pace: Option<(usize, std::time::Duration)>,
    /// Read the request, then never answer while the client stays connected.
    pub silent: bool,
}

impl Answer {
    pub fn json(body: impl Into<String>) -> Self {
        Self { status: 200, content_type: "application/json", api: Some("1"), body: body.into(), headers: Vec::new(), bytes: None, pace: None, silent: false }
    }

    pub fn bytes(bytes: Vec<u8>) -> Self {
        Self { content_type: "application/octet-stream", api: None, bytes: Some(bytes), ..Self::json("") }
    }

    pub fn silent() -> Self {
        Self { silent: true, ..Self::json("") }
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
        let mut head = self.api.map(|v| format!("X-Workman-Api: {v}\r\n")).unwrap_or_default();
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
type Queued = Arc<Mutex<HashMap<String, VecDeque<Answer>>>>;
type Log = Arc<(Mutex<Vec<Seen>>, Condvar)>;


/// Every kind of row Workman's inbox serves.
pub const INBOX_KINDS: [&str; 7] = ["question", "blocked", "blocker_finding", "done_unverified", "stale_in_progress", "expired_claim", "assumption_due"];

/// One inbox row per [`INBOX_KINDS`] as Workman 3.0.3 serves it, in workspace `ws` and run `r1`, titled `Title of <kind>`.
pub fn inbox_rows(ws: &str) -> Vec<serde_json::Value> {
    let call = |kind: &str| format!("work_status(ref:\"{ws}/r1/{kind}\", to:\"in_progress\")");
    let task = |id: &str, ref_: &str, title: &str| serde_json::json!({ "id": id, "ref": ref_, "qualifiedRef": format!("r1/{ref_}"), "kind": "task", "title": title, "status": "open" });
    INBOX_KINDS
        .iter()
        .map(|kind| {
            serde_json::json!({
                "key": format!("{kind}:id-{kind}"), "kind": kind,
                "item": task(&format!("id-{kind}"), kind, &format!("Title of {kind}")),
                "run": { "id": "run1", "ref": "r1", "qualifiedRef": "r1", "kind": "run", "title": "The run", "status": "in_progress" },
                "what": call(kind), "since": "2026-11-06T11:00:00+00:00", "call": call(kind),
                "audience": if matches!(*kind, "question" | "blocked") { "human" } else { "controller" },
                "detail": null, "actor": "aw-1", "tree": null,
                "target": if *kind == "blocker_finding" { task("t", "9", "T") } else { serde_json::Value::Null },
                "count": 1, "url": format!("https://work.example/r/{ws}/r1/{kind}")
            })
        })
        .collect()
}

pub struct FakeWorkman {
    base: String,
    seen: Log,
    routes: Routes,
    queued: Queued,
    /// Open `/api/stream` connections, past their response head.
    streams: Arc<Mutex<Vec<TcpStream>>>,
    oauth: Arc<Mutex<Option<Arc<FakeAuth>>>>,
    sessions: Arc<Mutex<Option<Arc<FakeSessions>>>>,
    journal: Arc<Mutex<Option<Arc<FakeJournal>>>>,
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

impl FakeWorkman {
    /// `routes`: `<METHOD> <target>` or the exact target → answer; else a v1 404, but `/api/stream` stays open.
    pub fn start(routes: Vec<(String, Answer)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a fake Workman");
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen: Log = Arc::default();
        let routes: Routes = Arc::new(Mutex::new(routes));
        let queued: Queued = Arc::default();
        let streams: Arc<Mutex<Vec<TcpStream>>> = Arc::default();
        let oauth: Arc<Mutex<Option<Arc<FakeAuth>>>> = Arc::default();
        let sessions: Arc<Mutex<Option<Arc<FakeSessions>>>> = Arc::default();
        let journal: Arc<Mutex<Option<Arc<FakeJournal>>>> = Arc::default();
        let (table, waiting, log, open, guard, rows, events) =
            (routes.clone(), queued.clone(), seen.clone(), streams.clone(), oauth.clone(), sessions.clone(), journal.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let (routes, queued, log, open, guard, rows, events) =
                    (table.clone(), waiting.clone(), log.clone(), open.clone(), guard.clone(), rows.clone(), events.clone());
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
                    if answer.is_none() {
                        let rows = rows.lock().unwrap().clone();
                        answer = rows.and_then(|r| r.handle(method, target, authorization.as_deref(), body));
                    }
                    if answer.is_none() && method == "GET" {
                        let events = events.lock().unwrap().clone();
                        answer = events.and_then(|j| j.handle(target));
                    }
                    let keyed = format!("{method} {target}");
                    let answer = answer.or_else(|| {
                        let mut queued = queued.lock().unwrap();
                        let mut next = |key: &str| queued.get_mut(key).and_then(VecDeque::pop_front);
                        next(&keyed).or_else(|| next(target))
                    });
                    let answer = answer.or_else(|| {
                        let routes = routes.lock().unwrap();
                        let find = |key: &str| routes.iter().find(|(t, _)| t == key).map(|(_, a)| a.clone());
                        find(&keyed).or_else(|| find(target))
                    });
                    let stream_open = answer.is_none() && target == STREAM_TARGET;
                    let answer = answer.unwrap_or_else(|| Answer::status(404, format!(r#"{{"error":"no route {target}"}}"#)));
                    log.0.lock().unwrap().push(seen);
                    log.1.notify_all();
                    if stream_open {
                        let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nX-Workman-Api: 1\r\n\r\n: connected\n\n";
                        if stream.write_all(head.as_bytes()).is_ok() {
                            open.lock().unwrap().push(stream);
                        }
                        return;
                    }
                    if answer.silent {
                        let mut sink = [0u8; 64];
                        while matches!(stream.read(&mut sink), Ok(n) if n > 0) {}
                        return;
                    }
                    let _ = answer.send(&mut stream);
                });
            }
        });
        Self { base, seen, routes, queued, streams, oauth, sessions, journal }
    }

    /// Serves `GET /api/v1/journal` from [`FakeJournal`]'s events; queued answers for it are not used.
    pub fn enable_journal(&self) -> Arc<FakeJournal> {
        let journal = Arc::new(FakeJournal::default());
        *self.journal.lock().unwrap() = Some(journal.clone());
        journal
    }

    /// Mints and revokes agent sessions as Workman 2.3.0 does; queued answers for them are not used.
    pub fn enable_sessions(&self) -> Arc<FakeSessions> {
        let sessions = Arc::new(FakeSessions::default());
        *self.sessions.lock().unwrap() = Some(sessions.clone());
        sessions
    }

    /// Also the OAuth server of its origin: `/api/*` then takes only its live access tokens (else 401).
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

    /// `route` (a key as in [`Self::start`]) answers with the answers queued on it, in order, then as before.
    pub fn queue(&self, route: impl Into<String>, answer: Answer) {
        self.queued.lock().unwrap().entry(route.into()).or_default().push_back(answer);
    }

    /// One SSE frame `data: <data>` to every open stream; how many took it.
    pub fn push(&self, data: &str) -> usize {
        self.push_frame(&format!("data: {data}\n\n"))
    }

    /// `frame` as written, to every open stream; how many took it.
    pub fn push_frame(&self, frame: &str) -> usize {
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
        self.seen.0.lock().unwrap().clone()
    }

    /// [`Self::seen`] once `done` holds of it, or as it is when `timeout` runs out.
    pub fn seen_when(&self, timeout: Duration, done: impl Fn(&[Seen]) -> bool) -> Vec<Seen> {
        let (lock, cvar) = &*self.seen;
        let seen = cvar.wait_timeout_while(lock.lock().unwrap(), timeout, |s| !done(s)).unwrap().0;
        seen.clone()
    }
}

struct Code {
    client_id: String,
    redirect_uri: String,
    challenge: String,
    grant: u64,
    scope: String,
}

/// Workman 2.0's scope names; until 2.1 it also takes these old names for them.
const SCOPES: [&str; 3] = ["workman.read", "workman.sessions", "offline_access"];
const LEGACY_SCOPES: [&str; 2] = ["workstate.read", "workstate.sessions"];

/// `fake-<what>-<16 random hex>`: no id is guessable from another fake or another run.
fn fresh_id(what: &str) -> String {
    let mut r = [0u8; 8];
    getrandom::fill(&mut r).expect("the OS random source");
    format!("fake-{what}-{}", supraman_core::hex::encode(&r))
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
    /// Grant → the scope it was authorized with.
    scopes: HashMap<u64, String>,
    legacy_dropped: bool,
    next: u64,
    access_ttl: u64,
    refresh_len: usize,
    fail_refresh: bool,
    fail_revoke: bool,
    deny: bool,
    issued: Vec<String>,
    refusals: Vec<String>,
}

/// Shaped like Workman's (Skills `docs/workstate/10-read-api.md` rule 5); authorize redirects at once: the test is the browser.
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

    /// With a trailing slash, as Workman's.
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

    /// Refresh tokens issued from now on are padded to `bytes` (Workman 1.0.0's exceed 2560).
    pub fn set_refresh_len(&self, bytes: usize) {
        self.st().refresh_len = bytes;
    }

    pub fn fail_refresh(&self, fail: bool) {
        self.st().fail_refresh = fail;
    }

    /// The revoke endpoint refuses with `invalid_request`, the grant kept.
    pub fn fail_revoke(&self, fail: bool) {
        self.st().fail_revoke = fail;
    }

    /// As Workman 2.1: the old `workstate.*` scope names are no longer offered, and a grant made with them is refused.
    pub fn drop_legacy_scopes(&self) {
        self.st().legacy_dropped = true;
    }

    /// Every grant so far as one made with `scope`, as a Supraman before Workman 2.0 asked for.
    pub fn rescope_grants(&self, scope: &str) {
        for s in self.st().scopes.values_mut() {
            *s = scope.to_string();
        }
    }

    /// The scopes of the grants so far, in grant order.
    pub fn grant_scopes(&self) -> Vec<String> {
        let st = self.st();
        let mut grants: Vec<_> = st.scopes.iter().collect();
        grants.sort();
        grants.into_iter().map(|(_, s)| s.clone()).collect()
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
        st.access.get(token).is_some_and(|(grant, until)| *until > Instant::now() && !st.revoked.contains(grant) && !Self::stale_scope(&st, *grant))
    }

    fn offered(st: &AuthState, scope: &str) -> bool {
        SCOPES.contains(&scope) || (!st.legacy_dropped && LEGACY_SCOPES.contains(&scope))
    }

    fn stale_scope(st: &AuthState, grant: u64) -> bool {
        st.scopes.get(&grant).is_some_and(|s| s.split(' ').any(|s| !Self::offered(st, s)))
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
            "refresh_token": refresh, "scope": st.scopes.get(&grant).cloned().unwrap_or_default(),
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
            ("GET", "/.well-known/oauth-protected-resource/api") => {
                let st = self.st();
                let scopes: Vec<&str> = SCOPES.iter().chain(&LEGACY_SCOPES).copied().filter(|s| Self::offered(&st, s)).collect();
                Self::json(serde_json::json!({
                    "resource": format!("{base}/api"),
                    "authorization_servers": [base],
                    "scopes_supported": scopes,
                }))
            }
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
                let scope = get("scope");
                if !scope.split(' ').any(|s| s == "offline_access") || scope.split(' ').any(|s| !Self::offered(&st, s)) {
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
                    Code { client_id: get("client_id"), redirect_uri: redirect.clone(), challenge: get("code_challenge"), grant, scope },
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
                        st.scopes.insert(code.grant, code.scope);
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
                        if Self::stale_scope(&st, grant) {
                            return Some(Self::refuse(&mut st, &endpoint, "invalid_scope", "the grant's scope is no longer offered"));
                        }
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
                if st.fail_revoke {
                    return Some(Self::refuse(&mut st, "/connect/revoke", "invalid_request", "refused on request"));
                }
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

#[derive(Debug, Clone)]
struct Row {
    id: String,
    launched_by: String,
    /// The user that made the row ([`user_of`]).
    owner: Option<String>,
    token: Option<String>,
    revoked: bool,
    /// Epoch ms its token lapses at, under an idle window ([`FakeSessions::set_idle_window`]).
    expires: Option<i64>,
}

/// Agent-session rows, one per name, by Workman's rules: see `SessionStore.Tokens.cs` (2.3.0) and `takeoverFrom` (3.0.2).
#[derive(Default)]
pub struct FakeSessions {
    rows: Mutex<BTreeMap<String, Row>>,
    minted: Mutex<u64>,
    /// `(now, window)` in ms, once set.
    clock: Mutex<Option<(i64, i64)>>,
}

impl FakeSessions {
    /// From now on each token lapses `window_ms` after its mint or its last use; the clock reads `now_ms`.
    pub fn set_idle_window(&self, window_ms: i64, now_ms: i64) {
        *self.clock.lock().unwrap() = Some((now_ms, window_ms));
    }

    /// Sets the clock of [`Self::set_idle_window`].
    pub fn set_now(&self, now_ms: i64) {
        if let Some((now, _)) = self.clock.lock().unwrap().as_mut() {
            *now = now_ms;
        }
    }

    fn now(&self) -> Option<i64> {
        self.clock.lock().unwrap().map(|(now, _)| now)
    }

    fn alive(row: &Row, now: Option<i64>) -> bool {
        !row.revoked && row.token.is_some() && row.expires.zip(now).is_none_or(|(expires, now)| now < expires)
    }

    /// Whether `token` still answers: its row holds it, not revoked and not lapsed.
    pub fn live(&self, token: &str) -> bool {
        let now = self.now();
        self.rows.lock().unwrap().values().any(|r| Self::alive(r, now) && r.token.as_deref() == Some(token))
    }

    /// The live token of the session named `name`.
    pub fn token_of(&self, name: &str) -> Option<String> {
        let now = self.now();
        self.rows.lock().unwrap().get(name).filter(|r| Self::alive(r, now)).and_then(|r| r.token.clone())
    }

    /// Revokes the live token of the session named `name`, as a revoke elsewhere or its idle window would.
    pub fn revoke_name(&self, name: &str) {
        if let Some(r) = self.rows.lock().unwrap().get_mut(name) {
            r.revoked = true;
        }
    }

    /// The rows' names, revoked ones too.
    pub fn names(&self) -> Vec<String> {
        self.rows.lock().unwrap().keys().cloned().collect()
    }

    /// A use of `token`: whether it answers, its window slid when it does.
    fn used(&self, token: &str) -> bool {
        let clock = *self.clock.lock().unwrap();
        let now = clock.map(|(now, _)| now);
        let mut rows = self.rows.lock().unwrap();
        let Some(row) = rows.values_mut().find(|r| Self::alive(r, now) && r.token.as_deref() == Some(token)) else {
            return false;
        };
        if let Some((now, window)) = clock {
            row.expires = Some(now + window);
        }
        true
    }

    fn handle(&self, method: &str, target: &str, authorization: Option<&str>, body: &str) -> Option<Answer> {
        match (method, target) {
            ("POST", "/api/agent-sessions") => Some(self.mint(body, authorization)),
            ("POST", "/mcp") => Some(match authorization.and_then(|a| a.strip_prefix("Bearer ")) {
                Some(token) if self.used(token) => Answer::json(r#"{"jsonrpc":"2.0","id":0,"result":{}}"#),
                _ => Answer::status(401, r#"{"error":"invalid_token"}"#),
            }),
            ("DELETE", t) => {
                let id = t.strip_prefix("/api/agent-sessions/")?;
                let mut rows = self.rows.lock().unwrap();
                Some(match rows.values_mut().find(|r| r.id == id) {
                    Some(r) => {
                        r.revoked = true;
                        Answer::status(204, "")
                    }
                    None => Answer::status(404, r#"{"error":"no such session"}"#),
                })
            }
            _ => None,
        }
    }

    fn mint(&self, body: &str, authorization: Option<&str>) -> Answer {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else {
            return Answer::status(400, r#"{"error":"not JSON"}"#);
        };
        let (Some(name), Some(launched_by)) = (v["name"].as_str(), v["launchedBy"].as_str()) else {
            return Answer::status(400, r#"{"error":"name and launchedBy are required"}"#);
        };
        let replace = v["replace"].as_bool().unwrap_or(false);
        let clock = *self.clock.lock().unwrap();
        let mut rows = self.rows.lock().unwrap();
        let mut n = self.minted.lock().unwrap();
        *n += 1;
        let token = format!("wss_fake_session{n}");
        let existed = rows.contains_key(name);
        let row = rows
            .entry(name.to_string())
            .or_insert_with(|| Row { id: format!("s{n}"), launched_by: launched_by.to_string(), owner: user_of(authorization), token: None, revoked: false, expires: None });
        let takeover = v["takeoverFrom"].as_str().filter(|_| existed);
        if takeover.is_some_and(|from| from != row.launched_by || row.owner != user_of(authorization)) {
            return Answer::status(409, serde_json::json!({ "error": format!("the session \"{name}\" cannot be taken over from that launcher") }).to_string());
        }
        if row.launched_by != launched_by && takeover.is_none() {
            let error = format!(
                "the session name \"{name}\" is held by a session launched by {}, and this mint says {launched_by}; two launchers' sessions cannot share a name, and replace does not override that",
                row.launched_by
            );
            return Answer::status(409, serde_json::json!({ "error": error }).to_string());
        }
        if Self::alive(row, clock.map(|(now, _)| now)) && !replace {
            let error = format!("the session \"{name}\" already has a live token; pass replace: true to revoke it and mint a new one");
            return Answer::status(409, serde_json::json!({ "error": error }).to_string());
        }
        row.launched_by = launched_by.to_string();
        row.token = Some(token.clone());
        row.revoked = false;
        row.expires = clock.map(|(now, window)| now + window);
        let mut answer = serde_json::json!({ "sessionId": row.id, "token": token });
        if let Some(expires) = row.expires {
            answer["expiresAt"] = supraman_core::time::rfc3339_utc(expires).into();
        }
        Answer::status(201, answer.to_string())
    }
}

/// The `sub` of an access token [`FakeAuth`] issued; any other bearer is its own user.
fn user_of(authorization: Option<&str>) -> Option<String> {
    let bearer = authorization?.strip_prefix("Bearer ")?;
    let sub = bearer
        .split('.')
        .nth(1)
        .and_then(|claims| URL_SAFE_NO_PAD.decode(claims).ok())
        .and_then(|claims| serde_json::from_slice::<serde_json::Value>(&claims).ok())
        .and_then(|claims| claims["sub"].as_str().map(str::to_string));
    Some(sub.unwrap_or_else(|| bearer.to_string()))
}

/// A journal of `(seq, scope, kind)` events, paged as `GET /api/v1/journal` pages them.
#[derive(Default)]
pub struct FakeJournal {
    events: Mutex<Vec<(i64, String, String)>>,
}

impl FakeJournal {
    /// Appends events `seqs` under `scope`, of kind `transition`.
    pub fn add(&self, scope: &str, seqs: impl IntoIterator<Item = i64>) {
        let mut events = self.events.lock().unwrap();
        events.extend(seqs.into_iter().map(|seq| (seq, scope.to_string(), "transition".to_string())));
        events.sort_by_key(|e| e.0);
    }

    fn handle(&self, target: &str) -> Option<Answer> {
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        if path != "/api/v1/journal" {
            return None;
        }
        let q = form_decode(query);
        let num = |k: &str| q.get(k).and_then(|v| v.parse::<i64>().ok());
        let limit = num("limit").unwrap_or(50).clamp(1, 500) as usize;
        let kinds: Vec<&str> = q.get("kinds").map(|k| k.split(',').collect()).unwrap_or_default();
        let events = self.events.lock().unwrap();
        let newest = events.last().map_or(0, |e| e.0);
        let shown: Vec<&(i64, String, String)> = events
            .iter()
            .filter(|e| q.get("scope").is_none_or(|s| *s == e.1) && (kinds.is_empty() || kinds.contains(&e.2.as_str())))
            .collect();
        if q.contains_key("since") && q.contains_key("before") {
            return Some(Answer::status(400, r#"{"error":"since and before cannot be combined"}"#));
        }
        let (page, more, cursor) = match num("since") {
            Some(since) => {
                let after: Vec<_> = shown.into_iter().filter(|e| e.0 > since).collect();
                let page: Vec<_> = after.iter().take(limit).copied().collect();
                let cursor = page.last().map_or(since, |e| e.0);
                (page, after.len() > limit, cursor)
            }
            None => {
                let before = num("before");
                let below: Vec<_> = shown.into_iter().filter(|e| before.is_none_or(|b| e.0 < b)).collect();
                let page: Vec<_> = below[below.len().saturating_sub(limit)..].to_vec();
                // a before= page names its lowest seq, the next before=; the newest page its highest
                let cursor = match before {
                    Some(b) => page.first().map_or(b, |e| e.0),
                    None => page.last().map_or(newest, |e| e.0),
                };
                (page, below.len() > limit, cursor)
            }
        };
        let events: Vec<serde_json::Value> =
            page.iter().map(|e| serde_json::json!({ "seq": e.0, "at": "2026-11-03T12:00:00+00:00", "kind": e.2, "actor": "fake" })).collect();
        let body = serde_json::json!({ "asOf": "2026-11-03T12:00:00+00:00", "seq": newest, "cursor": cursor, "hasMore": more, "events": events });
        Some(Answer::json(body.to_string()))
    }
}
