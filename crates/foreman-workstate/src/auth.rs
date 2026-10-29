//! Foreman's sign-in to WorkState (m5/43): an OAuth 2.1 native client for the resource `<origin>/api`
//! (Skills `docs/workstate/10-read-api.md` rule 5, `14-agent-launch.md` "Minting with OAuth").
//!
//! - **Discovery**: `<origin>/.well-known/oauth-protected-resource/api` names the authorization server;
//!   its `/.well-known/oauth-authorization-server` the endpoints (RFC 9728, RFC 8414).
//! - **Registration**: DCR as a public client (`token_endpoint_auth_method: none`), fresh at every
//!   interactive sign-in; its `client_id` is kept per issuer in `<home>/oauth/servers.json` (not a
//!   secret) beside the refresh token issued to it. A refresh refused with `invalid_client` or
//!   `invalid_grant` (e.g. the server's database restored) drops that pair: "Sign in again".
//! - **Authorize**: PKCE S256, a random `state` checked on return, exactly one `resource`, the loopback
//!   redirect `http://127.0.0.1:<ephemeral>/callback` (RFC 8252), 5 minutes at most.
//! - **Tokens**: the refresh token only in the [`SecretStore`] (a DPAPI-encrypted file), keyed by issuer;
//!   it rotates, so every refresh stores the new one, under a lock file shared with `fm`. The access
//!   token stays in memory and is refreshed 5 minutes before it expires. A refused refresh leaves
//!   "Sign in again" and is not retried until the next sign-in.
//! - **Sign out**: the refresh token to the revocation endpoint, which ends the whole authorization and
//!   every `wss_` token minted under it; then the stored and the held tokens go.
//!
//! No token is ever in a message, a log line, a plain-text file or a `Debug` output.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use foreman_core::Settings;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::client::{BearerSource, api_base};
use crate::http::pct_encode;
use crate::secret::{SecretStore, default_store};

pub const SCOPE: &str = "workstate.read workstate.sessions offline_access";
/// What DCR registers; the server ignores the port of a loopback redirect.
pub const REGISTERED_REDIRECT: &str = "http://127.0.0.1/callback";
pub const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// An access token with less left than this is refreshed before use.
const REFRESH_MARGIN: Duration = Duration::from_secs(5 * 60);
const HTTP_TIMEOUT: Duration = Duration::from_secs(15);
/// A revoke is a courtesy: a server that does not answer within this is left to lapse the grant.
const REVOKE_TIMEOUT: Duration = Duration::from_secs(4);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AuthError {
    #[error("not signed in to WorkState at {origin}")]
    NotSignedIn { origin: String },
    #[error("sign in to WorkState at {origin} again: {why}")]
    SignInAgain { origin: String, why: String },
    #[error("WorkState at {origin} is unreachable ({reason})")]
    Unreachable { origin: String, reason: String },
    #[error("{what} was refused: HTTP {status}{}", suffix(.error))]
    Refused { what: &'static str, status: u16, error: String },
    #[error("{0}")]
    BadResponse(String),
    #[error("the sign-in came back with a state that is not the one sent; nothing was signed in")]
    StateMismatch,
    #[error("the sign-in was refused: {0}")]
    Denied(String),
    #[error("the sign-in was not finished within {} minutes", SIGN_IN_TIMEOUT.as_secs() / 60)]
    TimedOut,
    #[error("the sign-in was cancelled")]
    Cancelled,
    #[error("cannot open the browser: {0}")]
    Browser(String),
    #[error("{0}")]
    Store(String),
}

fn suffix(error: &str) -> String {
    if error.is_empty() { String::new() } else { format!(" ({error})") }
}

/// What Settings → WorkState shows, known without a network call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthStatus {
    NotSignedIn,
    /// `who`: the token's `name` or subject, when the access token is readable. `unsaved`: why the
    /// refresh token is held in memory only, so the sign-in ends with this process.
    SignedIn { who: Option<String>, unsaved: Option<String> },
    SignInAgain(String),
}

/// The server's endpoints, as discovered for one origin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerMeta {
    pub origin: String,
    pub resource: String,
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub registration_endpoint: Option<String>,
    pub revocation_endpoint: Option<String>,
}

/// `<home>/oauth/servers.json`: discovered servers by origin, registered clients by issuer. No token.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Known {
    #[serde(default)]
    servers: BTreeMap<String, ServerMeta>,
    #[serde(default)]
    clients: BTreeMap<String, String>,
}

struct Held {
    origin: String,
    issuer: String,
    access: String,
    expires: Instant,
    who: Option<String>,
}

#[derive(Default)]
struct Inner {
    held: Option<Held>,
    /// Issuer → why its grant no longer works; cleared by a sign-in.
    broken: BTreeMap<String, String>,
    /// Issuer → (refresh token, why) when the store refused it: the server has spent the previous one.
    unsaved: BTreeMap<String, (String, String)>,
}

impl Inner {
    fn unsaved_why(&self, issuer: &str) -> Option<String> {
        self.unsaved.get(issuer).map(|(_, why)| why.clone())
    }
}

pub struct Auth {
    home: PathBuf,
    store: Arc<dyn SecretStore>,
    agent: ureq::Agent,
    revoke_agent: ureq::Agent,
    inner: Mutex<Inner>,
    cancel: AtomicBool,
    timeout: Duration,
}

impl std::fmt::Debug for Auth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Auth").field("home", &self.home).finish_non_exhaustive()
    }
}

/// The token endpoint's answer.
struct Tokens {
    access: String,
    expires_in: u64,
    refresh: Option<String>,
}

impl Auth {
    /// Refresh tokens in `home`'s [`default_store`].
    pub fn open(home: &Path) -> Arc<Self> {
        Arc::new(Self::with_store(home, default_store(home)))
    }

    pub fn with_store(home: &Path, store: Arc<dyn SecretStore>) -> Self {
        Self {
            home: home.to_path_buf(),
            store,
            agent: crate::http::agent(HTTP_TIMEOUT),
            revoke_agent: crate::http::agent(REVOKE_TIMEOUT),
            inner: Mutex::default(),
            cancel: AtomicBool::new(false),
            timeout: SIGN_IN_TIMEOUT,
        }
    }

    /// Tests: a sign-in gives up after `timeout` instead of 5 minutes.
    pub fn with_sign_in_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    fn known_path(&self) -> PathBuf {
        self.home.join("oauth").join("servers.json")
    }

    /// Empty only without a file; an unreadable one holds other origins' clients and is never saved over.
    fn load_known(&self) -> Result<Known, AuthError> {
        let path = self.known_path();
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Known::default()),
            Err(e) => return Err(AuthError::Store(format!("cannot read {}: {e}", path.display()))),
        };
        serde_json::from_str(&text)
            .map_err(|e| AuthError::Store(format!("{} is damaged ({e}); repair or delete it, then sign in again", path.display())))
    }

    fn save_known(&self, known: &Known) -> Result<(), AuthError> {
        let path = self.known_path();
        let write = || -> std::io::Result<()> {
            std::fs::create_dir_all(path.parent().expect("oauth dir"))?;
            let tmp = path.with_extension("json.tmp");
            std::fs::write(&tmp, serde_json::to_string_pretty(known).unwrap_or_default())?;
            std::fs::rename(&tmp, &path)
        };
        write().map_err(|e| AuthError::Store(format!("cannot write {}: {e}", path.display())))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The held access token for `origin`, while it has more than [`REFRESH_MARGIN`] left.
    fn fresh(&self, origin: &str) -> Option<String> {
        let inner = self.lock();
        let h = inner.held.as_ref().filter(|h| h.origin == origin)?;
        (h.expires.saturating_duration_since(Instant::now()) > REFRESH_MARGIN).then(|| h.access.clone())
    }

    fn not_broken(&self, origin: &str, issuer: &str) -> Result<(), AuthError> {
        match self.lock().broken.get(issuer) {
            Some(why) => Err(AuthError::SignInAgain { origin: origin.to_string(), why: why.clone() }),
            None => Ok(()),
        }
    }

    /// One refresh at a time across processes (the app and `fm`): a rotated token is read back fresh.
    fn refresh_lock(&self) -> Result<File, AuthError> {
        let path = self.home.join("oauth").join("refresh.lock");
        foreman_core::fs::lock_file(&path).map_err(|e| AuthError::Store(format!("cannot lock {}: {e}", path.display())))
    }

    /// The server URL `settings.toml` names, as an origin.
    pub fn configured_origin(&self) -> String {
        Settings::load(&self.home).unwrap_or_default().workstate.origin()
    }

    /// The issuer `servers.json` names for `origin`; `None` when it names none or cannot be read.
    pub fn issuer_of(&self, origin: &str) -> Option<String> {
        self.load_known().ok()?.servers.remove(&api_base(origin)).map(|m| m.issuer)
    }

    /// Without a network call: held, broken, or a refresh token stored for the origin's issuer.
    pub fn status(&self, origin: &str) -> AuthStatus {
        let origin = api_base(origin);
        {
            let inner = self.lock();
            if let Some(h) = inner.held.as_ref().filter(|h| h.origin == origin) {
                return AuthStatus::SignedIn { who: h.who.clone(), unsaved: inner.unsaved_why(&h.issuer) };
            }
        }
        let mut known = match self.load_known() {
            Ok(known) => known,
            Err(e) => return AuthStatus::SignInAgain(e.to_string()),
        };
        let Some(meta) = known.servers.remove(&origin) else {
            return AuthStatus::NotSignedIn;
        };
        {
            let inner = self.lock();
            if let Some(why) = inner.broken.get(&meta.issuer) {
                return AuthStatus::SignInAgain(why.clone());
            }
            if let Some(why) = inner.unsaved_why(&meta.issuer) {
                return AuthStatus::SignedIn { who: None, unsaved: Some(why) };
            }
        }
        match self.store.get(&meta.issuer) {
            Ok(Some(_)) => AuthStatus::SignedIn { who: None, unsaved: None },
            Ok(None) => AuthStatus::NotSignedIn,
            Err(e) => AuthStatus::SignInAgain(e),
        }
    }

    /// An access token for `origin`'s `/api`, refreshed when it has less than 5 minutes left.
    /// `inner` is never held across the refresh: [`Self::status`] answers the UI meanwhile.
    pub fn access_token(&self, origin: &str) -> Result<String, AuthError> {
        let origin = api_base(origin);
        if let Some(access) = self.fresh(&origin) {
            return Ok(access);
        }
        let known = self.load_known()?;
        let Some(meta) = known.servers.get(&origin).cloned() else {
            return Err(AuthError::NotSignedIn { origin });
        };
        self.not_broken(&origin, &meta.issuer)?;
        let Some(client_id) = known.clients.get(&meta.issuer).cloned() else {
            return Err(AuthError::NotSignedIn { origin });
        };
        let _lock = self.refresh_lock()?;
        // another thread may have refreshed, or been refused, while this one waited for the lock
        if let Some(access) = self.fresh(&origin) {
            return Ok(access);
        }
        self.not_broken(&origin, &meta.issuer)?;
        let unsaved = self.lock().unsaved.get(&meta.issuer).map(|(rt, _)| rt.clone());
        let refresh = match unsaved {
            Some(rt) => rt,
            None => match self.store.get(&meta.issuer).map_err(AuthError::Store)? {
                Some(rt) => rt,
                None => return Err(AuthError::NotSignedIn { origin }),
            },
        };
        let form = [("grant_type", "refresh_token"), ("refresh_token", refresh.as_str()), ("client_id", client_id.as_str())];
        match self.token_request(&meta, &form) {
            Ok(tokens) => {
                if let Some(rt) = &tokens.refresh {
                    self.keep_refresh(&meta.issuer, rt);
                }
                let who = who_of(&tokens.access);
                let access = tokens.access.clone();
                let issuer = meta.issuer.clone();
                self.lock().held = Some(Held { origin, issuer, access: tokens.access, expires: expiry(tokens.expires_in), who });
                Ok(access)
            }
            Err(e @ AuthError::Unreachable { .. }) => Err(e),
            Err(e) => {
                let why = match &e {
                    AuthError::Refused { error, .. } if !error.is_empty() => format!("the server refused the refresh ({error})"),
                    other => other.to_string(),
                };
                // the client or the grant is gone (a wiped server): the pair is useless, the next sign-in
                // registers afresh
                if let AuthError::Refused { error, .. } = &e
                    && matches!(error.as_str(), "invalid_client" | "invalid_grant")
                {
                    self.store.delete(&meta.issuer).map_err(AuthError::Store)?;
                    let mut known = self.load_known()?;
                    known.clients.remove(&meta.issuer);
                    self.save_known(&known)?;
                }
                let mut inner = self.lock();
                inner.unsaved.remove(&meta.issuer);
                inner.broken.insert(meta.issuer.clone(), why.clone());
                if inner.held.as_ref().is_some_and(|h| h.origin == origin) {
                    inner.held = None;
                }
                Err(AuthError::SignInAgain { origin, why })
            }
        }
    }

    /// One the store refuses stays in memory and [`Self::status`] says why: the server spent the old one.
    fn keep_refresh(&self, issuer: &str, refresh: &str) {
        let refused = self.store.set(issuer, refresh).err();
        let mut inner = self.lock();
        match refused {
            None => inner.unsaved.remove(issuer),
            Some(e) => {
                let why = format!("the renewed sign-in could not be stored ({e}) and lasts only until this process ends");
                inner.unsaved.insert(issuer.to_string(), (refresh.to_string(), why))
            }
        };
    }

    /// [`Self::access_token`] for the configured server: (origin, token).
    pub fn current(&self) -> Result<(String, String), AuthError> {
        let origin = self.configured_origin();
        let token = self.access_token(&origin)?;
        Ok((origin, token))
    }

    /// The server refused `token` (401): it is dropped, so the next call refreshes.
    pub fn forget_access(&self, token: &str) {
        let mut inner = self.lock();
        if inner.held.as_ref().is_some_and(|h| h.access == token) {
            inner.held = None;
        }
    }

    /// A [`BearerSource`] for a read client of `origin`.
    pub fn bearer_for(self: &Arc<Self>, origin: &str) -> Arc<dyn BearerSource> {
        Arc::new(AuthBearer { auth: self.clone(), origin: api_base(origin) })
    }

    /// Stops a running [`Self::sign_in`] at its next poll.
    pub fn cancel_sign_in(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    /// Discovers `origin`'s server, registers a fresh client (an unknown cached one would fail on the
    /// server's own page in the browser, where Foreman never sees `invalid_client`), opens `browser` on
    /// the authorize URL and waits on the loopback for the code; then the tokens, whose refresh token is
    /// kept with that client_id. Blocks up to 5 minutes: run it off the UI thread.
    pub fn sign_in(&self, origin: &str, browser: &dyn Fn(&str) -> Result<(), String>) -> Result<AuthStatus, AuthError> {
        self.cancel.store(false, Ordering::SeqCst);
        let origin = api_base(origin);
        let meta = self.discover(&origin)?;
        let mut known = self.load_known()?;
        known.servers.insert(origin.clone(), meta.clone());
        self.save_known(&known)?;
        let client_id = self.register(&meta)?;

        let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| AuthError::BadResponse(format!("cannot listen on the loopback: {e}")))?;
        let redirect = format!("http://127.0.0.1:{}/callback", listener.local_addr().map(|a| a.port()).unwrap_or_default());
        let verifier = random_b64(32);
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let state = random_b64(16);
        let resource = format!("{origin}/api");
        let url = format!(
            "{}?{}",
            meta.authorization_endpoint,
            form_encode(&[
                ("response_type", "code"),
                ("client_id", &client_id),
                ("redirect_uri", &redirect),
                ("scope", SCOPE),
                ("state", &state),
                ("code_challenge", &challenge),
                ("code_challenge_method", "S256"),
                ("resource", &resource),
            ])
        );
        browser(&url).map_err(AuthError::Browser)?;
        let params = self.wait_callback(&listener, &state)?;
        let code = params.get("code").cloned().ok_or_else(|| AuthError::BadResponse("the callback carried no code".into()))?;
        let form = [
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", redirect.as_str()),
            ("client_id", client_id.as_str()),
            ("code_verifier", verifier.as_str()),
        ];
        let tokens = self.token_request(&meta, &form)?;
        let refresh = tokens
            .refresh
            .ok_or_else(|| AuthError::BadResponse("the server gave no refresh token (offline_access)".into()))?;
        {
            let _lock = self.refresh_lock()?;
            self.keep_refresh(&meta.issuer, &refresh);
            let mut known = self.load_known()?;
            known.servers.insert(origin.clone(), meta.clone());
            known.clients.insert(meta.issuer.clone(), client_id);
            self.save_known(&known)?;
        }
        let who = who_of(&tokens.access);
        let mut inner = self.lock();
        inner.broken.remove(&meta.issuer);
        let unsaved = inner.unsaved_why(&meta.issuer);
        inner.held = Some(Held { origin, issuer: meta.issuer.clone(), access: tokens.access, expires: expiry(tokens.expires_in), who: who.clone() });
        Ok(AuthStatus::SignedIn { who, unsaved })
    }

    /// Revokes `origin`'s grant (the refresh token: the whole authorization and every `wss_` minted
    /// under it), after deleting the stored token and dropping held tokens, so no other call waits on the
    /// revoke. `Err` is a notice: a revoke that failed; everything local is gone anyway.
    pub fn sign_out(&self, origin: &str) -> Result<(), String> {
        let origin = api_base(origin);
        let known = self.load_known().map_err(|e| e.to_string())?;
        let refresh = {
            let mut inner = self.lock();
            if inner.held.as_ref().is_some_and(|h| h.origin == origin) {
                inner.held = None;
            }
            let Some(meta) = known.servers.get(&origin) else {
                return Ok(());
            };
            inner.broken.remove(&meta.issuer);
            let unsaved = inner.unsaved.remove(&meta.issuer).map(|(rt, _)| rt);
            drop(inner);
            let _lock = self.refresh_lock().map_err(|e| e.to_string())?;
            let stored = self.store.get(&meta.issuer)?;
            self.store.delete(&meta.issuer)?;
            unsaved.or(stored)
        };
        let meta = &known.servers[&origin];
        match (&refresh, &meta.revocation_endpoint, known.clients.get(&meta.issuer)) {
            (None, _, _) => Ok(()),
            (Some(_), None, _) => Err(format!("WorkState at {origin} names no revocation endpoint; the grant lapses after 30 days")),
            (Some(rt), Some(endpoint), client) => {
                let mut form = vec![("token", rt.as_str()), ("token_type_hint", "refresh_token")];
                if let Some(client) = client {
                    form.push(("client_id", client.as_str()));
                }
                let res = self
                    .revoke_agent
                    .post(endpoint)
                    .header("Accept", "application/json")
                    .content_type("application/x-www-form-urlencoded")
                    .send(form_encode(&form).as_str());
                match read(&origin, res) {
                    Ok((status, _)) if (200..300).contains(&status) => Ok(()),
                    Ok((status, body)) => Err(format!("WorkState at {origin} refused the sign-out: HTTP {status}{}", suffix(&oauth_error(&body)))),
                    Err(e) => Err(format!("{e}; the grant lapses after 30 days")),
                }
            }
        }
    }

    fn discover(&self, origin: &str) -> Result<ServerMeta, AuthError> {
        let prm = self.get_json(origin, &format!("{origin}/.well-known/oauth-protected-resource/api"))?;
        let resource = prm["resource"].as_str().unwrap_or_default().to_string();
        let Some(issuer) = prm["authorization_servers"].as_array().and_then(|a| a.first()).and_then(|v| v.as_str()) else {
            return Err(AuthError::BadResponse(format!("{origin} names no authorization server for /api")));
        };
        let issuer = issuer.to_string();
        let trimmed = issuer.trim_end_matches('/');
        let (scheme_host, path) = split_origin(trimmed);
        let as_url = format!("{scheme_host}/.well-known/oauth-authorization-server{path}");
        let asm = self.get_json(origin, &as_url)?;
        let field = |k: &str| asm[k].as_str().map(str::to_string);
        if field("issuer").as_deref().map(|i| i.trim_end_matches('/')) != Some(trimmed) {
            return Err(AuthError::BadResponse(format!("{as_url} is not the metadata of {issuer}")));
        }
        if let Some(methods) = asm["code_challenge_methods_supported"].as_array()
            && !methods.iter().any(|m| m.as_str() == Some("S256"))
        {
            return Err(AuthError::BadResponse(format!("{issuer} does not offer PKCE S256")));
        }
        let need = |k: &str| field(k).ok_or_else(|| AuthError::BadResponse(format!("{issuer} names no {k}")));
        let (authorization_endpoint, token_endpoint) = (need("authorization_endpoint")?, need("token_endpoint")?);
        Ok(ServerMeta {
            origin: origin.to_string(),
            resource,
            issuer: field("issuer").unwrap_or_default(),
            authorization_endpoint,
            token_endpoint,
            registration_endpoint: field("registration_endpoint"),
            revocation_endpoint: field("revocation_endpoint"),
        })
    }

    fn register(&self, meta: &ServerMeta) -> Result<String, AuthError> {
        let Some(endpoint) = &meta.registration_endpoint else {
            return Err(AuthError::BadResponse(format!("{} offers no client registration", meta.issuer)));
        };
        let body = serde_json::json!({
            "client_name": "Foreman",
            "redirect_uris": [REGISTERED_REDIRECT],
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none",
            "scope": SCOPE,
        })
        .to_string();
        let res = self
            .agent
            .post(endpoint)
            .header("Accept", "application/json")
            .content_type("application/json")
            .send(body.as_str());
        let (status, text) = read(&meta.origin, res)?;
        if !(200..300).contains(&status) {
            return Err(AuthError::Refused { what: "the client registration", status, error: oauth_error(&text) });
        }
        serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|v| v["client_id"].as_str().map(str::to_string))
            .ok_or_else(|| AuthError::BadResponse("the registration gave no client_id".into()))
    }

    fn token_request(&self, meta: &ServerMeta, form: &[(&str, &str)]) -> Result<Tokens, AuthError> {
        let (status, body) = self.post_form(&meta.origin, &meta.token_endpoint, form)?;
        if !(200..300).contains(&status) {
            return Err(AuthError::Refused { what: "the token request", status, error: oauth_error(&body) });
        }
        let v: serde_json::Value = serde_json::from_str(&body).map_err(|_| AuthError::BadResponse("the token answer is not JSON".into()))?;
        let access = v["access_token"].as_str().filter(|t| !t.is_empty()).map(str::to_string);
        let access = access.ok_or_else(|| AuthError::BadResponse("the token answer has no access_token".into()))?;
        Ok(Tokens {
            access,
            expires_in: v["expires_in"].as_u64().unwrap_or(3600),
            refresh: v["refresh_token"].as_str().filter(|t| !t.is_empty()).map(str::to_string),
        })
    }

    fn post_form(&self, origin: &str, url: &str, form: &[(&str, &str)]) -> Result<(u16, String), AuthError> {
        let body = form_encode(form);
        let res = self
            .agent
            .post(url)
            .header("Accept", "application/json")
            .content_type("application/x-www-form-urlencoded")
            .send(body.as_str());
        read(origin, res)
    }

    fn get_json(&self, origin: &str, url: &str) -> Result<serde_json::Value, AuthError> {
        let (status, text) = read(origin, self.agent.get(url).header("Accept", "application/json").call())?;
        if !(200..300).contains(&status) {
            return Err(AuthError::Refused { what: "the discovery", status, error: oauth_error(&text) });
        }
        serde_json::from_str(&text).map_err(|_| AuthError::BadResponse(format!("{url} is not JSON")))
    }

    /// The query of the first `GET /callback`, answered with a page the browser keeps.
    fn wait_callback(&self, listener: &TcpListener, state: &str) -> Result<BTreeMap<String, String>, AuthError> {
        let deadline = Instant::now() + self.timeout;
        listener.set_nonblocking(true).map_err(|e| AuthError::BadResponse(e.to_string()))?;
        loop {
            if self.cancel.swap(false, Ordering::SeqCst) {
                return Err(AuthError::Cancelled);
            }
            if Instant::now() >= deadline {
                return Err(AuthError::TimedOut);
            }
            let stream = match listener.accept() {
                Ok((stream, _)) => stream,
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(50));
                    continue;
                }
                Err(e) => return Err(AuthError::BadResponse(format!("the loopback failed: {e}"))),
            };
            let Some((path, params)) = read_request(&stream) else { continue };
            if path != "/callback" {
                answer(stream, "404 Not Found", "Not found.");
                continue;
            }
            if params.get("state").map(String::as_str) != Some(state) {
                answer(stream, "400 Bad Request", "Foreman did not sign in: this answer is not for the sign-in it started. You can close this tab.");
                return Err(AuthError::StateMismatch);
            }
            if let Some(error) = params.get("error") {
                let why = params.get("error_description").map_or(error.clone(), |d| format!("{error}: {d}"));
                answer(stream, "200 OK", &format!("Foreman did not sign in ({}). You can close this tab.", html(&why)));
                return Err(AuthError::Denied(why));
            }
            answer(stream, "200 OK", "Foreman is signed in to WorkState. You can close this tab.");
            return Ok(params);
        }
    }
}

struct AuthBearer {
    auth: Arc<Auth>,
    origin: String,
}

impl BearerSource for AuthBearer {
    fn bearer(&self) -> Result<String, String> {
        self.auth.access_token(&self.origin).map_err(|e| e.to_string())
    }

    fn rejected(&self, token: &str) {
        self.auth.forget_access(token);
    }
}

fn expiry(expires_in: u64) -> Instant {
    Instant::now() + Duration::from_secs(expires_in)
}

fn read(origin: &str, res: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<(u16, String), AuthError> {
    let mut res = res.map_err(|e| AuthError::Unreachable { origin: origin.to_string(), reason: e.to_string() })?;
    let status = res.status().as_u16();
    Ok((status, res.body_mut().read_to_string().unwrap_or_default()))
}

/// The `error` of an OAuth error answer; never a token (the answers carry none).
fn oauth_error(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v["error"].as_str().map(str::to_string))
        .unwrap_or_default()
}

/// `https://h:1/p` → (`https://h:1`, `/p`).
fn split_origin(url: &str) -> (String, &str) {
    let origin = foreman_core::url::origin(url);
    let path = url.strip_prefix(origin.as_str()).unwrap_or_default();
    (origin, path)
}

fn random_b64(n: usize) -> String {
    let mut bytes = vec![0u8; n];
    getrandom::fill(&mut bytes).expect("the OS random source");
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn form_encode(pairs: &[(&str, &str)]) -> String {
    pairs.iter().map(|(k, v)| format!("{}={}", pct_encode(k), pct_encode(v))).collect::<Vec<_>>().join("&")
}

/// `a=1&b=x%20y` → map; `+` is a space.
pub fn form_decode(query: &str) -> BTreeMap<String, String> {
    fn unpct(s: &str) -> String {
        let bytes = s.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'+' => out.push(b' '),
                b'%' => match s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    Some(b) => {
                        out.push(b);
                        i += 2;
                    }
                    None => out.push(b'%'),
                },
                b => out.push(b),
            }
            i += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    }
    query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| p.split_once('=').map_or((unpct(p), String::new()), |(k, v)| (unpct(k), unpct(v))))
        .collect()
}

/// The access token's `name`, else `preferred_username`, else `sub`, when it is a readable JWT.
fn who_of(token: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    ["name", "preferred_username", "sub"].iter().find_map(|k| v[*k].as_str().filter(|s| !s.is_empty()).map(str::to_string))
}

fn html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// The request's path and query parameters; `None` for anything unreadable.
fn read_request(stream: &TcpStream) -> Option<(String, BTreeMap<String, String>)> {
    stream.set_nonblocking(false).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h).ok()? == 0 || h.trim_end().is_empty() {
            break;
        }
    }
    let target = line.split_whitespace().nth(1)?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    Some((path.to_string(), form_decode(query)))
}

fn answer(mut stream: TcpStream, status: &str, text: &str) {
    let page = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>Foreman</title></head><body style=\"font-family:sans-serif;margin:3em\"><p>{text}</p></body></html>"
    );
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}",
        page.len()
    );
    let _ = stream.flush();
}

/// A browser for tests and the selftest, never a real one: on a thread, `GET`s the authorize URL,
/// passes its redirect through `rewrite` and `GET`s that (the loopback callback).
pub fn http_browser_with(rewrite: fn(String) -> String) -> impl Fn(&str) -> Result<(), String> + Send + Sync {
    move |url: &str| {
        let url = url.to_string();
        std::thread::spawn(move || {
            let agent = crate::http::agent(HTTP_TIMEOUT);
            let Ok(res) = agent.get(&url).call() else { return };
            let Some(location) = res.headers().get("location").and_then(|v| v.to_str().ok()).map(str::to_string) else {
                return;
            };
            let _ = agent.get(rewrite(location)).call();
        });
        Ok(())
    }
}

/// [`http_browser_with`] that changes nothing.
pub fn http_browser() -> impl Fn(&str) -> Result<(), String> + Send + Sync {
    http_browser_with(|l| l)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_round_trip_and_origin_split() {
        let e = form_encode(&[("redirect_uri", "http://127.0.0.1:5/callback"), ("scope", "a b")]);
        assert_eq!(e, "redirect_uri=http%3A%2F%2F127.0.0.1%3A5%2Fcallback&scope=a%20b");
        let d = form_decode(&e);
        assert_eq!(d["redirect_uri"], "http://127.0.0.1:5/callback");
        assert_eq!(d["scope"], "a b");
        assert_eq!(form_decode("a=x+y&b=%zz&c")["a"], "x y");
        assert_eq!(form_decode("b=%zz")["b"], "%zz");
        for (issuer, origin, path) in [
            ("https://ws.example/tenant", "https://ws.example", "/tenant"),
            ("https://ws.example", "https://ws.example", ""),
            ("http://127.0.0.1:5290/a/b", "http://127.0.0.1:5290", "/a/b"),
            ("ws.example/x", "ws.example", "/x"),
        ] {
            assert_eq!(split_origin(issuer), (origin.to_string(), path), "{issuer}");
        }
    }

    #[test]
    fn who_reads_a_jwt_and_nothing_else() {
        let payload = URL_SAFE_NO_PAD.encode(br#"{"sub":"u1","name":"Operator"}"#);
        assert_eq!(who_of(&format!("h.{payload}.s")).as_deref(), Some("Operator"));
        let payload = URL_SAFE_NO_PAD.encode(br#"{"sub":"u1"}"#);
        assert_eq!(who_of(&format!("h.{payload}.s")).as_deref(), Some("u1"));
        assert_eq!(who_of("opaque-token"), None);
    }
}
