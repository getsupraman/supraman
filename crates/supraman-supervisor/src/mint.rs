//! Per-session Workman tokens (Skills `docs/workstate/14-agent-launch.md` §2): only the minted `wss_` token reaches a
//! session, kept in the secret store and printed by `sm mcp-header`; the bearer never leaves Supraman and `sm`.

use std::collections::BTreeMap;
use std::fmt;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use supraman_claude::Session;
use supraman_core::secret::SecretStore;

mod keeping;

pub use keeping::{CHECK_EVERY, Keeping, PING_TIMEOUT, RENEW_MARGIN};

/// `POST /api/agent-sessions`. `role` is the Workman role (never above the bearer's),
/// `external_role` Supraman's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MintRequest {
    pub name: String,
    pub role: String,
    pub launched_by: String,
    pub origin: String,
    pub cwd: String,
    pub external_role: String,
    /// One live token per name: `replace` revokes the old one and keeps the row and its `sessionId`.
    #[serde(default)]
    pub replace: bool,
    /// The repository a repo session works in: its bare `work_brief()` opens the project bound to it.
    /// `None` for the workspace session and the relay (the workspace root is not a repo).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<MintRepo>,
    /// The claude session uuid the token is for, when it is known before claude starts: a
    /// resume's, or the `--session-id` of a foreground `claude`. Never a guess: a `--bg` launch's id
    /// is known only from its row (the PATCH of [`Minter::bind`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_session_id: Option<String>,
    /// The row's current `launchedBy`, which this mint moves to `launched_by` ([`mint_as`]); never recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub takeover_from: Option<String>,
}

const LAUNCHER: &str = "supraman";
const FORMER_LAUNCHER: &str = "foreman";

/// The `launchedBy` (and `origin`) of every Supraman mint: `supraman/<path>`.
pub fn launched_by(path: &str) -> String {
    format!("{LAUNCHER}/{path}")
}

/// `label` under [`launched_by`]: a `foreman/…` label as its `supraman/…` one, any other as it is.
fn relabelled(label: &str) -> String {
    label.strip_prefix(FORMER_LAUNCHER).and_then(|rest| rest.strip_prefix('/')).map_or_else(|| label.to_string(), launched_by)
}

/// Mints `request`; a row recorded under another label (`former`) is moved to `request.launched_by` with
/// `takeoverFrom` (Workman 3.0.2), and minted without it when that 409s, as a row moved already does.
pub fn mint_as(minter: &dyn Minter, base: &str, bearer: &str, request: &MintRequest, former: Option<&str>) -> Result<Minted, MintError> {
    let Some(former) = former.filter(|f| *f != request.launched_by) else {
        return minter.mint(base, bearer, request);
    };
    let moving = MintRequest { takeover_from: Some(former.to_string()), ..request.clone() };
    match minter.mint(base, bearer, &moving) {
        Err(MintError::Refused { status: 409, .. }) => minter.mint(base, bearer, request),
        other => other,
    }
}

/// `repo:{root, remote?}` of a mint, as git prints them; Workman normalises both (Skills
/// `docs/workstate/02-data-model.md`, Identity).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MintRepo {
    /// The main checkout's root: the parent of `git rev-parse --path-format=absolute --git-common-dir`,
    /// the same for every worktree.
    pub root: String,
    /// `git remote get-url origin`; `None` without an origin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
}

/// The [`MintRepo`] of the git working tree `dir` (a worktree gives its main checkout's root); `None`
/// when `dir` is not in one or git cannot be run.
pub fn git_repo(dir: &Path) -> Option<MintRepo> {
    let common = git(dir, &["rev-parse", "--path-format=absolute", "--git-common-dir"])?;
    let root = Path::new(&common).parent()?.to_str()?.to_string();
    Some(MintRepo {
        root,
        remote: git(dir, &["remote", "get-url", "origin"]),
    })
}

/// `git -C <dir> <args>`: trimmed stdout when it succeeds and prints something.
fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = supraman_core::process::git(dir, args).ok()?;
    let text = out.stdout.trim().to_string();
    (out.success() && !text.is_empty()).then_some(text)
}

/// The `201` of a mint. `token` is shown once; `Debug` never prints it.
#[derive(Clone, PartialEq, Eq)]
pub struct Minted {
    pub session_id: String,
    pub token: String,
    pub expires_at: Option<String>,
}

impl fmt::Debug for Minted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Minted")
            .field("session_id", &self.session_id)
            .field("token", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// Never carries a token: the bearer is not part of any message, and a server reason is cut short.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MintError {
    #[error("Workman at {base} is unreachable ({reason})")]
    Unreachable { base: String, reason: String },
    #[error("Workman answered HTTP {status}{}", reason_suffix(.reason))]
    Refused { status: u16, reason: String },
    #[error("Workman's answer was not understood: {0}")]
    BadResponse(String),
}

fn reason_suffix(reason: &str) -> String {
    if reason.is_empty() { String::new() } else { format!(": {reason}") }
}

/// The three calls on `/api/agent-sessions`. `base` is the server's origin ([`supraman_core::url::origin`]).
/// Implemented by [`HttpMinter`]; tests use a fake.
pub trait Minter {
    fn mint(&self, base: &str, bearer: &str, request: &MintRequest) -> Result<Minted, MintError>;
    /// `PATCH /api/agent-sessions/{id}` `{claudeSessionId}`.
    fn bind(&self, base: &str, bearer: &str, session_id: &str, claude_session_id: &str) -> Result<(), MintError>;
    /// `DELETE /api/agent-sessions/{id}`; a `404` counts as revoked.
    fn revoke(&self, base: &str, bearer: &str, session_id: &str) -> Result<(), MintError>;
}

/// The server's origin, from its MCP URL; shared with the read API client.

/// Blocking HTTP/1.1 (TLS for an `https://` server), no proxy, 15 s per call.
pub struct HttpMinter {
    agent: ureq::Agent,
}

impl Default for HttpMinter {
    fn default() -> Self {
        Self { agent: supraman_workman::http_agent(Duration::from_secs(15)) }
    }
}

impl HttpMinter {
    fn send(&self, base: &str, req: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<(u16, String), MintError> {
        let reply = supraman_workman::read_reply(req).map_err(|reason| MintError::Unreachable { base: base.to_string(), reason })?;
        Ok((reply.status, reply.body))
    }
}

fn refused(status: u16, body: &str) -> MintError {
    MintError::Refused {
        status,
        reason: supraman_workman::server_reason(body),
    }
}

fn sessions_url(base: &str, session_id: &str) -> String {
    format!("{base}/api/agent-sessions/{}", supraman_workman::pct_encode(session_id))
}

impl Minter for HttpMinter {
    fn mint(&self, base: &str, bearer: &str, request: &MintRequest) -> Result<Minted, MintError> {
        let body = serde_json::to_string(request).map_err(|e| MintError::BadResponse(e.to_string()))?;
        let (status, text) = self.send(
            base,
            self.agent
                .post(format!("{base}/api/agent-sessions"))
                .header("Authorization", format!("Bearer {bearer}"))
                .content_type("application/json")
                .send(body.as_str()),
        )?;
        if !(200..300).contains(&status) {
            return Err(refused(status, &text));
        }
        let v: serde_json::Value = serde_json::from_str(&text).map_err(|_| MintError::BadResponse("not JSON".into()))?;
        let session_id = match v.get("sessionId") {
            Some(serde_json::Value::String(s)) if !s.is_empty() => s.clone(),
            Some(serde_json::Value::Number(n)) => n.to_string(),
            _ => return Err(MintError::BadResponse("no sessionId".into())),
        };
        let token = match v.get("token").and_then(|t| t.as_str()) {
            Some(t) if !t.is_empty() => t.to_string(),
            _ => return Err(MintError::BadResponse("no token".into())),
        };
        Ok(Minted {
            session_id,
            token,
            expires_at: v.get("expiresAt").and_then(|t| t.as_str()).map(str::to_string),
        })
    }

    fn bind(&self, base: &str, bearer: &str, session_id: &str, claude_session_id: &str) -> Result<(), MintError> {
        let body = serde_json::json!({ "claudeSessionId": claude_session_id }).to_string();
        let (status, text) = self.send(
            base,
            self.agent
                .patch(sessions_url(base, session_id))
                .header("Authorization", format!("Bearer {bearer}"))
                .content_type("application/json")
                .send(body.as_str()),
        )?;
        if (200..300).contains(&status) { Ok(()) } else { Err(refused(status, &text)) }
    }

    fn revoke(&self, base: &str, bearer: &str, session_id: &str) -> Result<(), MintError> {
        let (status, text) = self.send(
            base,
            self.agent
                .delete(sessions_url(base, session_id))
                .header("Authorization", format!("Bearer {bearer}"))
                .call(),
        )?;
        if (200..300).contains(&status) || status == 404 { Ok(()) } else { Err(refused(status, &text)) }
    }
}

/// What the supervisor remembers of one minted session. Not secret: no token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MintedSession {
    /// Workman's `sessionId`, for the PATCH and the DELETE.
    pub session_id: String,
    pub role: String,
    /// The claude session uuid sent with the mint or PATCHed since; `None` until then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_session_id: Option<String>,
    /// Epoch ms of the mint or of the last turn seen within the token's window: what slid its expiry
    /// last. `None` until the first tick sets it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_active: Option<crate::Millis>,
    /// The server the token is from ([`supraman_core::url::origin`] of its MCP URL); `None`: unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
}

/// A [`MintedSession`] with the mint that made its token, which a fresh one for the name repeats ([`header`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Entry {
    #[serde(flatten)]
    session: MintedSession,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mint: Option<MintRequest>,
}

/// Per session name, its [`MintedSession`]; persisted (a workspace's at [`Self::path`], the [+] sessions' at
/// [`Self::hands_path`], a scheduled run's at [`Self::run_path`]) so a restarted app still revokes what it minted and
/// [`Keeping`] can mint afresh. Without a path it lives in memory only.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MintedSessions {
    path: Option<PathBuf>,
    names: BTreeMap<String, Entry>,
}

impl MintedSessions {
    pub fn path(home: &Path, workspace: &str) -> PathBuf {
        home.join("supervisor").join(format!("{workspace}.sessions.json"))
    }

    pub fn hands_path(home: &Path) -> PathBuf {
        home.join("supervisor").join("hands.json")
    }

    pub fn run_path(home: &Path, name: &str) -> PathBuf {
        runs_dir(home).join(format!("{name}.json"))
    }

    /// The record of `name` in any of `home`'s files, with the mint to repeat for it (`replace`, its claude session).
    pub fn find(home: &Path, name: &str) -> Option<(MintedSession, MintRequest)> {
        record_files(home).into_iter().find_map(|path| {
            let entry = Self::load(Some(path)).ok()?.names.remove(name)?;
            let mut request = entry.mint?;
            request.replace = true;
            request.claude_session_id = entry.session.claude_session_id.clone();
            Some((entry.session, request))
        })
    }

    pub fn empty(path: Option<PathBuf>) -> Self {
        Self { path, names: BTreeMap::new() }
    }

    /// Moves a sessions file that cannot be read to `<file>.damaged`, so the next save does not overwrite the only
    /// record of the sessions it held; the path it went to.
    pub fn set_aside(path: &Path) -> std::io::Result<PathBuf> {
        let mut aside = path.as_os_str().to_owned();
        aside.push(".damaged");
        let aside = PathBuf::from(aside);
        std::fs::rename(path, &aside)?;
        Ok(aside)
    }

    /// A missing file is an empty set; a damaged one is an error, for [`Self::set_aside`].
    pub fn load(path: Option<PathBuf>) -> std::io::Result<Self> {
        let names = match &path {
            Some(p) => match std::fs::read_to_string(p) {
                Ok(text) => serde_json::from_str(&text).map_err(|e| std::io::Error::new(ErrorKind::InvalidData, e))?,
                Err(e) if e.kind() == ErrorKind::NotFound => BTreeMap::new(),
                Err(e) => return Err(e),
            },
            None => BTreeMap::new(),
        };
        Ok(Self { path, names })
    }

    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        supraman_core::fs::write_atomic(path, serde_json::to_string_pretty(&self.names).map_err(std::io::Error::other)?)
    }

    pub fn get(&self, name: &str) -> Option<&MintedSession> {
        self.names.get(name).map(|e| &e.session)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut MintedSession> {
        self.names.get_mut(name).map(|e| &mut e.session)
    }

    pub fn insert(&mut self, name: &str, session: MintedSession) {
        self.names.insert(name.to_string(), Entry { session, mint: None });
    }

    /// [`Self::insert`] with the mint that made its token.
    pub fn insert_minted(&mut self, name: &str, session: MintedSession, request: &MintRequest) {
        let mint = MintRequest { takeover_from: None, ..request.clone() };
        self.names.insert(name.to_string(), Entry { session, mint: Some(mint) });
    }

    /// The `launchedBy` recorded for the Workman session named `session_name`.
    pub fn launched_by(&self, session_name: &str) -> Option<&str> {
        self.names.values().filter_map(|e| e.mint.as_ref()).find(|m| m.name == session_name).map(|m| m.launched_by.as_str())
    }

    pub fn remove(&mut self, name: &str) -> Option<MintedSession> {
        self.names.remove(name).map(|e| e.session)
    }

    /// Each name bound to a claude session, with that session's id.
    pub fn bound(&self) -> Vec<(String, String)> {
        self.names.iter().filter_map(|(n, e)| Some((n.clone(), e.session.claude_session_id.clone()?))).collect()
    }

    pub fn names(&self) -> impl Iterator<Item = &String> {
        self.names.keys()
    }

    /// Gives each session of unknown origin the origin of the URL in its config file under `mcp_dir`, if
    /// that is still there; saved when one changed.
    pub fn adopt_origins(&mut self, mcp_dir: &Path) -> std::io::Result<()> {
        let mut changed = false;
        for (name, m) in self.names.iter_mut().map(|(n, e)| (n, &mut e.session)).filter(|(_, m)| m.origin.is_none()) {
            if let Some(url) = supraman_claude::mcp_config_url(mcp_dir, name) {
                m.origin = Some(supraman_core::url::origin(&url));
                changed = true;
            }
        }
        if changed { self.save() } else { Ok(()) }
    }

    /// Every session whose token is not from `origin`, with the origin it is from (`None`: unknown).
    pub fn elsewhere(&self, origin: &str) -> Vec<(String, Option<String>)> {
        let origin = supraman_core::url::origin(origin);
        self.names
            .iter()
            .filter(|(_, e)| e.session.origin.as_deref() != Some(origin.as_str()))
            .map(|(name, e)| (name.clone(), e.session.origin.clone()))
            .collect()
    }
}

fn runs_dir(home: &Path) -> PathBuf {
    home.join("supervisor").join("runs")
}

/// `dir`'s files whose name ends with `suffix`, sorted; none when `dir` cannot be read.
fn files_ending(dir: &Path, suffix: &str) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.ends_with(suffix)))
        .collect();
    files.sort();
    files
}

/// Every record file of `home`: each workspace's, the [+] sessions', each scheduled run's.
fn record_files(home: &Path) -> Vec<PathBuf> {
    let mut files = files_ending(&home.join("supervisor"), ".sessions.json");
    files.push(MintedSessions::hands_path(home));
    files.extend(files_ending(&runs_dir(home), ".json"));
    files
}

/// Where a session's Workman token is kept: the secret store under its `mcp_token_key`, never a file; its config
/// under `mcp_dir` runs `sm mcp-header --home <home>` to print it.
#[derive(Clone)]
pub struct Keeper {
    home: PathBuf,
    mcp_dir: PathBuf,
    sm: PathBuf,
    store: Arc<dyn SecretStore>,
    /// Where a deleted run's transcript is looked for ([`still_has_transcript`]); `None`: the listing is trusted.
    claude_dir: Option<PathBuf>,
}

impl Keeper {
    pub fn new(home: &Path, mcp_dir: &Path, store: Arc<dyn SecretStore>) -> Self {
        let sm = supraman_core::install::sm_exe().unwrap_or_else(|_| PathBuf::from(supraman_core::HOOK_EXE));
        Self { home: home.to_path_buf(), mcp_dir: mcp_dir.to_path_buf(), sm, store, claude_dir: None }
    }

    pub fn with_claude_dir(self, claude_dir: Option<PathBuf>) -> Self {
        Self { claude_dir, ..self }
    }

    /// This keeper with `sm` as the `sm.exe` its configs run.
    pub fn with_sm(self, sm: PathBuf) -> Self {
        Self { sm, ..self }
    }

    /// `home`'s secret store, and its configs under `<home>/mcp`.
    pub fn open(home: &Path) -> Self {
        Self::new(home, &home.join("mcp"), supraman_core::secret::default_store(home))
    }

    pub fn mcp_dir(&self) -> &Path {
        &self.mcp_dir
    }

    pub fn store(&self) -> &dyn SecretStore {
        &*self.store
    }

    pub fn kept(&self, name: &str) -> Option<String> {
        self.store.get(&supraman_claude::mcp_token_key(name)).ok().flatten()
    }

    fn helper(&self, name: &str) -> String {
        supraman_claude::mcp_header_command(&self.sm, &self.home, name)
    }

    fn keep_token(&self, name: &str, token: &str) -> Result<(), String> {
        self.store.set(&supraman_claude::mcp_token_key(name), token).map_err(|e| format!("cannot keep the Workman token of {name}: {e}"))
    }

    /// `expires_at` (the mint's RFC 3339 `expiresAt`) and the `launched_by` of a re-mint as what [`Keeping`] knows of
    /// `name`'s token, while [`keeping::lock`] is held.
    fn note_locked(&self, name: &str, expires_at: Option<&str>, launched_by: Option<&str>) {
        let kept = keeping::Kept { expires_at: expires_at.and_then(supraman_core::time::rfc3339_ms), signed_out: false, launched_by: launched_by.map(str::to_string) };
        if let Err(e) = keeping::edit_locked(&self.home, |t| drop(t.insert(name.to_string(), kept))) {
            log::warn!("{name}: {e}; its token is renewed once Workman refuses it");
        }
    }

    fn keep(&self, name: &str, token: &str, expires_at: Option<&str>) -> Result<(), String> {
        let held = keeping::lock(&self.home);
        self.keep_token(name, token)?;
        match held {
            Ok(_held) => self.note_locked(name, expires_at, None),
            Err(e) => log::warn!("{name}: {e}; its token is renewed once Workman refuses it"),
        }
        Ok(())
    }

    /// Keeps `token` (minted with `expires_at`) for `name` and writes its helped config with `servers` beside Workman's.
    pub fn write(&self, name: &str, url: &str, token: &str, expires_at: Option<&str>, servers: &[(String, String)]) -> Result<PathBuf, String> {
        self.keep(name, token, expires_at)?;
        self.write_unminted(name, url, servers)
    }

    /// `name`'s helped config with no token kept: its helper prints the placeholder ([`header`]) until a mint keeps one.
    pub fn write_unminted(&self, name: &str, url: &str, servers: &[(String, String)]) -> Result<PathBuf, String> {
        supraman_claude::write_mcp_config_helped(&self.mcp_dir, name, url, &self.helper(name), servers)
            .map_err(|e| format!("cannot write the mcp config of {name}: {e}"))
    }

    /// `spec` with `token` (minted with `expires_at`) kept for its name and its helped config.
    pub fn launch(&self, spec: supraman_claude::LaunchSpec, url: &str, token: &str, expires_at: Option<&str>) -> Result<supraman_claude::LaunchSpec, String> {
        self.keep(&spec.name, token, expires_at)?;
        self.launch_unminted(spec, Some(url))
    }

    /// `spec` with its [`Self::write_unminted`] config; with no `url` (Workman not set up) a config without Workman.
    pub fn launch_unminted(&self, spec: supraman_claude::LaunchSpec, url: Option<&str>) -> Result<supraman_claude::LaunchSpec, String> {
        let name = spec.name.clone();
        match url {
            Some(url) => spec.with_helped_config(&self.mcp_dir, &name, url, &self.helper(&name)),
            None => spec.without_workman(&self.mcp_dir, &name),
        }
        .map_err(|e| format!("cannot write the mcp config of {name}: {e}"))
    }

    /// Deletes `name`'s config and kept token; what failed.
    fn forget(&self, name: &str) -> Vec<String> {
        let mut failed = Vec::new();
        if let Err(e) = remove_mcp_config(&self.mcp_dir, name) {
            failed.push(format!("{}{}", e[..1].to_uppercase(), &e[1..]));
        }
        if let Err(e) = self.store.delete(&supraman_claude::mcp_token_key(name)) {
            failed.push(format!("Cannot delete the kept Workman token of {name}: {e}"));
        }
        if let Err(e) = keeping::edit(&self.home, |t| drop(t.remove(name))) {
            failed.push(format!("{}{}", e[..1].to_uppercase(), &e[1..]));
        }
        failed
    }

    /// After a sign-out whose grant revoke took their tokens: `names` marked signed out, their kept tokens and configs
    /// left for their helpers, so [`Keeping`] mints them afresh after the next sign-in. What failed.
    pub fn signed_out(&self, names: &[String]) -> Vec<String> {
        if names.is_empty() {
            return Vec::new();
        }
        let marked = keeping::edit(&self.home, |t| {
            for name in names {
                t.entry(name.clone()).or_default().signed_out = true;
            }
        });
        marked.err().map(|e| format!("{}{}; they are renewed once Workman refuses them", e[..1].to_uppercase(), &e[1..])).into_iter().collect()
    }
}

/// The bearer a revoke goes with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Bearer {
    /// The MCP URL and the access token of the server signed in to.
    Signed { url: String, token: String },
    /// Signed out of the token's server: its grant revoke took every session token minted under it.
    SignedOut,
    /// No bearer now, and why.
    Unavailable(String),
}

impl Bearer {
    pub fn of(bearer: Result<Option<(String, String)>, String>) -> Self {
        match bearer {
            Ok(Some((url, token))) => Self::Signed { url, token },
            Ok(None) => Self::Unavailable("not signed in to Workman any more".into()),
            Err(e) => Self::Unavailable(e),
        }
    }
}

/// What [`revoke`] did besides the revoke.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Revoked {
    /// The token's server, when it is not the one signed in to: left to that server's grant revoke.
    pub left: Option<String>,
    /// What failed; an unused token lapses after its idle window.
    pub notices: Vec<String>,
}

/// The one end of a minted session, once it cannot come back (its claude session deleted): revokes `m`'s token with
/// `bearer`, unless the token is from another server than `bearer`'s, and deletes `name`'s config and kept token. A
/// sign-out is no end: [`Keeper::signed_out`].
pub fn revoke(keeper: &Keeper, minter: &dyn Minter, bearer: &Bearer, name: &str, m: &MintedSession) -> Revoked {
    let mut done = Revoked::default();
    let failed = match bearer {
        Bearer::SignedOut => None,
        Bearer::Unavailable(why) => Some(why.clone()),
        Bearer::Signed { url, .. } if m.origin.as_deref().is_some_and(|o| o != supraman_core::url::origin(url)) => {
            done.left = m.origin.clone();
            None
        }
        Bearer::Signed { url, token } => minter.revoke(&supraman_core::url::origin(url), token, &m.session_id).err().map(|e| e.to_string()),
    };
    if let Some(e) = failed {
        done.notices.push(format!("Cannot revoke the Workman token of {name} (session {}): {e}; unused, it lapses after its idle window", m.session_id));
    }
    done.notices.extend(keeper.forget(name));
    done
}

/// What `sm mcp-header` prints when no kept token can be read: a header Workman refuses, so claude fails this one
/// server instead of falling back to its own sign-in, which blocks every session.
const NO_TOKEN_HEADER: &str = r#"{"Authorization":"Bearer sm-no-token"}"#;

const HEADER_LOG_CAP: u64 = 1024 * 1024;

/// What `sm mcp-header` prints for `name` in `home`: its kept token's `Authorization` header, else
/// [`NO_TOKEN_HEADER`]; a read of the secret store only, [`Keeping`] keeps the token good. Each call appends a line
/// to `<home>/logs/mcp-header.log` (never the token), whose failure changes nothing.
pub fn header(home: Option<&Path>, name: &str) -> String {
    let Some(home) = home else {
        return NO_TOKEN_HEADER.to_string();
    };
    let (line, said) = match supraman_claude::mcp_header(&*supraman_core::secret::default_store(home), name) {
        Ok(line) => (line, "kept".to_string()),
        Err(why) => (NO_TOKEN_HEADER.to_string(), format!("placeholder: {why}")),
    };
    let logs = home.join("logs");
    let entry = format!("{} {} {}\n", supraman_core::time::rfc3339_utc(supraman_core::time::now_ms()), name.escape_debug(), said.replace(['\r', '\n'], " "));
    let _ = std::fs::create_dir_all(&logs).and_then(|()| supraman_core::fs::append_capped(&logs.join("mcp-header.log"), entry.as_bytes(), HEADER_LOG_CAP));
    line
}

/// The one re-mint of a kept token: `replace` on its record's mint under [`launched_by`]'s label, kept with its
/// `expiresAt` and that label under [`keeping::lock`]; the record is another owner's file and keeps its own.
fn remint(keeper: &Keeper, name: &str, bearer: Bearer, minter: &dyn Minter) -> Result<(), String> {
    let _held = keeping::lock(&keeper.home)?;
    let (m, request) = MintedSessions::find(&keeper.home, name).ok_or_else(|| format!("no record of {name}"))?;
    let Bearer::Signed { url, token } = bearer else {
        return Err("Supraman is not signed in to Workman".into());
    };
    if m.origin.as_deref().is_some_and(|o| o != supraman_core::url::origin(&url)) {
        return Err(format!("its token is from {}, not {}", m.origin.unwrap_or_default(), supraman_core::url::origin(&url)));
    }
    let moved = keeping::read(&keeper.home).ok().and_then(|mut t| t.remove(name)?.launched_by);
    let former = moved.unwrap_or_else(|| request.launched_by.clone());
    let request = MintRequest { launched_by: relabelled(&request.launched_by), origin: relabelled(&request.origin), ..request };
    let minted = mint_as(minter, &supraman_core::url::origin(&url), &token, &request, Some(&former)).map_err(|e| e.to_string())?;
    keeper.keep_token(name, &minted.token)?;
    keeper.note_locked(name, minted.expires_at.as_deref(), Some(&request.launched_by));
    Ok(())
}

/// The claude sessions of records that `claude agents --json --all` stopped listing: deleted once missing from two
/// non-empty snapshots in a row. An empty snapshot proves nothing and starts the count over.
#[derive(Debug, Default)]
pub struct Absent(BTreeMap<String, u32>);

impl Absent {
    /// The ids of `bound` (name, claude session id) deleted as of `observed`, a snapshot read without error.
    pub fn deleted(&mut self, bound: &[(String, String)], observed: &[Session]) -> Vec<String> {
        let mut missing = BTreeMap::new();
        let mut deleted = Vec::new();
        if !observed.is_empty() {
            for (_, sid) in bound.iter().filter(|(_, sid)| !observed.iter().any(|s| s.session_id == *sid)) {
                let times = self.0.get(sid).copied().unwrap_or(0) + 1;
                if times >= 2 {
                    deleted.push(sid.clone());
                } else {
                    missing.insert(sid.clone(), times);
                }
            }
        }
        self.0 = missing;
        deleted
    }
}

/// Whether `claude_session`'s transcript is still under `claude_dir`: a session a listing misses while its transcript
/// remains is not taken as deleted. Without a `claude_dir`, never.
pub fn still_has_transcript(claude_dir: Option<&Path>, claude_session: &str) -> bool {
    claude_dir.is_some_and(|dir| supraman_claude::find_transcript(dir, claude_session, None).is_some())
}

/// Each scheduled run's record whose claude session `absent` finds deleted in `observed` (read without error), and
/// whose transcript is gone: its name revoked ([`revoke`]) and taken out of its file, the file gone once empty. What
/// failed.
pub fn revoke_deleted_runs(keeper: &Keeper, minter: &dyn Minter, bearer: impl FnOnce() -> Bearer, absent: &mut Absent, observed: &[Session]) -> Vec<String> {
    let runs = runs_of(&keeper.home);
    let bound: Vec<(String, String)> = runs.iter().flat_map(|(_, r)| r.bound()).collect();
    let mut deleted = absent.deleted(&bound, observed);
    deleted.retain(|sid| !still_has_transcript(keeper.claude_dir.as_deref(), sid));
    if deleted.is_empty() {
        return Vec::new();
    }
    let bearer = bearer();
    let mut notices = Vec::new();
    for (path, mut records) in runs {
        let gone: Vec<(String, String)> = records.bound().into_iter().filter(|(_, sid)| deleted.contains(sid)).collect();
        for (name, sid) in &gone {
            log::info!("scheduled run {name}: its claude session {sid} is deleted, so its Workman token is revoked");
            if let Some(m) = records.remove(name) {
                notices.extend(revoke(keeper, minter, &bearer, name, &m).notices);
            }
        }
        if gone.is_empty() {
            continue;
        }
        let (verb, done) = if records.names().next().is_none() { ("delete", std::fs::remove_file(&path)) } else { ("save", records.save()) };
        if let Err(e) = done {
            notices.push(format!("Cannot {verb} {}: {e}", path.display()));
        }
    }
    notices
}

/// After a sign-out of `origin`: each scheduled run's record of it [`Keeper::signed_out`].
pub fn runs_signed_out(keeper: &Keeper, origin: &str) -> Vec<String> {
    let origin = supraman_core::url::origin(origin);
    let names: Vec<String> = runs_of(&keeper.home)
        .iter()
        .flat_map(|(_, records)| records.names().filter(|n| records.get(n).is_some_and(|m| m.origin.as_deref() == Some(origin.as_str()))).cloned().collect::<Vec<_>>())
        .collect();
    keeper.signed_out(&names)
}

fn runs_of(home: &Path) -> Vec<(PathBuf, MintedSessions)> {
    files_ending(&runs_dir(home), ".json").into_iter().filter_map(|p| Some((p.clone(), MintedSessions::load(Some(p)).ok()?))).collect()
}

/// Deletes `name`'s Workman config under `mcp_dir` (written by [`Keeper`]); a file that is
/// already gone is no error.
pub fn remove_mcp_config(mcp_dir: &Path, name: &str) -> Result<(), String> {
    let file = supraman_claude::mcp_config_path(mcp_dir, name).map_err(|e| format!("cannot delete the Workman config of {name}: {e}"))?;
    match std::fs::remove_file(&file) {
        Err(e) if e.kind() != ErrorKind::NotFound => Err(format!("cannot delete {}: {e}", file.display())),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use supraman_workman::fake::{Answer, FakeWorkman, closed_base};

    use super::*;

    const MINT: &str = "POST /api/agent-sessions";

    fn fake(routes: &[(&str, u16, &str)]) -> FakeWorkman {
        FakeWorkman::start(routes.iter().map(|(t, status, body)| (t.to_string(), Answer::status(*status, *body))).collect())
    }

    fn request() -> MintRequest {
        MintRequest {
            name: "fm-demo-workspace".into(),
            role: "controller".into(),
            launched_by: "foreman/demo".into(),
            origin: "foreman/demo".into(),
            cwd: r"C:\demo".into(),
            external_role: "workspace".into(),
            replace: true,
            repo: None,
            claude_session_id: None,
            takeover_from: None,
        }
    }

    #[test]
    fn mint_http_client_posts_the_body_with_the_bearer_and_reads_the_201() {
        let ws = fake(&[(
            MINT,
            201,
            r#"{"sessionId":"7f3c","name":"fm-demo-workspace","role":"controller","token":"wss_fake","expiresAt":"2026-10-29T00:00:00Z"}"#,
        )]);
        let minted = HttpMinter::default().mint(ws.base(), "fake-static", &request()).unwrap();
        assert_eq!(minted.session_id, "7f3c");
        assert_eq!(minted.token, "wss_fake");
        assert_eq!(minted.expires_at.as_deref(), Some("2026-10-29T00:00:00Z"));
        assert!(!format!("{minted:?}").contains("wss_fake"), "Debug redacts the token");
        let s = &ws.seen()[0];
        assert_eq!((s.method.as_str(), s.target.as_str()), ("POST", "/api/agent-sessions"));
        assert_eq!(s.authorization.as_deref(), Some("Bearer fake-static"));
        assert_eq!(s.content_type.as_deref(), Some("application/json"));
        let body: serde_json::Value = serde_json::from_str(&s.body).unwrap();
        assert_eq!(
            body,
            serde_json::json!({
                "name": "fm-demo-workspace", "role": "controller", "launchedBy": "foreman/demo",
                "origin": "foreman/demo", "cwd": r"C:\demo", "externalRole": "workspace", "replace": true
            })
        );
    }

    #[test]
    fn mint_http_client_sends_the_repo_of_a_repo_session() {
        let ws = fake(&[(MINT, 201, r#"{"sessionId":"8","token":"wss_fake"}"#)]);
        let mut req = request();
        req.external_role = "repo".into();
        req.repo = Some(MintRepo {
            root: "C:/demo/Alpha".into(),
            remote: Some("git@github.com:demo/alpha.git".into()),
        });
        HttpMinter::default().mint(ws.base(), "fake-static", &req).unwrap();
        let body: serde_json::Value = serde_json::from_str(&ws.seen()[0].body).unwrap();
        assert_eq!(body["repo"], serde_json::json!({ "root": "C:/demo/Alpha", "remote": "git@github.com:demo/alpha.git" }));
        req.repo = Some(MintRepo { root: "C:/demo/Beta".into(), remote: None });
        HttpMinter::default().mint(ws.base(), "fake-static", &req).unwrap();
        let body: serde_json::Value = serde_json::from_str(&ws.seen()[1].body).unwrap();
        assert_eq!(body["repo"], serde_json::json!({ "root": "C:/demo/Beta" }), "no origin, no remote");
    }

    #[test]
    fn mint_http_client_sends_the_claude_session_id_when_known() {
        let ws = fake(&[(MINT, 201, r#"{"sessionId":"8","token":"wss_fake"}"#)]);
        let mut req = request();
        req.claude_session_id = Some("aaaa0001-0000-4000-8000-000000000001".into());
        HttpMinter::default().mint(ws.base(), "fake-static", &req).unwrap();
        let body: serde_json::Value = serde_json::from_str(&ws.seen()[0].body).unwrap();
        assert_eq!(body["claudeSessionId"], "aaaa0001-0000-4000-8000-000000000001");
    }

    #[test]
    fn mint_git_repo_of_a_worktree_is_its_main_checkout() {
        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join("main");
        std::fs::create_dir(&main).unwrap();
        let run = |cwd: &Path, args: &[&str]| {
            let ok = supraman_core::process::command("git")
                .arg("-C")
                .arg(cwd)
                .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
                .args(args)
                .output()
                .unwrap()
                .status
                .success();
            assert!(ok, "git {args:?}");
        };
        run(&main, &["init", "-q"]);
        run(&main, &["commit", "-q", "--allow-empty", "-m", "x"]);
        let wt = dir.path().join("wt");
        run(&main, &["worktree", "add", "-q", wt.to_str().unwrap()]);
        let of_main = git_repo(&main).unwrap();
        assert_eq!(of_main, MintRepo { root: of_main.root.clone(), remote: None }, "no origin");
        let canon = |p: &str| std::fs::canonicalize(p).unwrap();
        assert_eq!(canon(&of_main.root), canon(main.to_str().unwrap()));
        assert_eq!(git_repo(&wt).unwrap().root, of_main.root, "a worktree sends its main checkout's root");
        run(&main, &["remote", "add", "origin", "https://example.invalid/demo/alpha.git"]);
        assert_eq!(git_repo(&wt).unwrap().remote.as_deref(), Some("https://example.invalid/demo/alpha.git"));
        let plain = tempfile::tempdir().unwrap();
        assert_eq!(git_repo(plain.path()), None, "not a repo");
        // a workspace root that is a repo, with a repo inside it: each sends its own root
        let child = main.join("child");
        std::fs::create_dir(&child).unwrap();
        run(&child, &["init", "-q"]);
        assert_eq!(git_repo(&main).unwrap().root, of_main.root, "the root repo");
        assert_eq!(canon(&git_repo(&child).unwrap().root), canon(child.to_str().unwrap()), "the repo inside it");
    }

    #[test]
    fn mint_http_client_patches_and_deletes_by_session_id() {
        let ws = fake(&[("PATCH /api/agent-sessions/42", 200, "{}"), ("DELETE /api/agent-sessions/42", 204, "")]);
        let m = HttpMinter::default();
        m.bind(ws.base(), "fake-static", "42", "aaaa0001-0000-4000-8000-000000000001").unwrap();
        m.revoke(ws.base(), "fake-static", "42").unwrap();
        m.revoke(ws.base(), "fake-static", "a b").unwrap();
        let seen = ws.seen();
        let line = |i: usize| format!("{} {}", seen[i].method, seen[i].target);
        assert_eq!(line(0), "PATCH /api/agent-sessions/42");
        assert_eq!(seen[0].authorization.as_deref(), Some("Bearer fake-static"));
        assert_eq!(seen[0].body, r#"{"claudeSessionId":"aaaa0001-0000-4000-8000-000000000001"}"#);
        assert_eq!(line(1), "DELETE /api/agent-sessions/42");
        assert_eq!(seen[1].authorization.as_deref(), Some("Bearer fake-static"));
        assert_eq!(line(2), "DELETE /api/agent-sessions/a%20b", "the fake's 404 counts as revoked");
    }

    #[test]
    fn mint_http_client_reports_status_and_reason_never_the_bearer() {
        let ws = fake(&[(MINT, 409, r#"{"error":"a live token exists for fm-demo-workspace"}"#)]);
        let m = HttpMinter::default();
        let e = m.mint(ws.base(), "fake-static", &request()).unwrap_err();
        assert_eq!(
            e,
            MintError::Refused {
                status: 409,
                reason: "a live token exists for fm-demo-workspace".into()
            }
        );
        assert_eq!(e.to_string(), "Workman answered HTTP 409: a live token exists for fm-demo-workspace");
        ws.set(MINT, Answer::status(201, r#"{"token":"wss_x"}"#));
        assert_eq!(m.mint(ws.base(), "fake-static", &request()).unwrap_err(), MintError::BadResponse("no sessionId".into()));
        let e = m.mint(&closed_base(), "fake-static", &request()).unwrap_err();
        assert!(matches!(e, MintError::Unreachable { .. }), "{e:?}");
        assert!(!e.to_string().contains("fake-static"), "{e}");
    }

    #[test]
    fn minted_sessions_round_trip_through_their_file() {
        let home = tempfile::tempdir().unwrap();
        let path = MintedSessions::path(home.path(), "demo");
        assert_eq!(path, home.path().join("supervisor").join("demo.sessions.json"));
        let mut s = MintedSessions::load(Some(path.clone())).unwrap();
        s.insert(
            "fm-demo-workspace",
            MintedSession {
                session_id: "7".into(),
                role: "workspace".into(),
                claude_session_id: None,
                last_active: Some(42),
                origin: Some("http://127.0.0.1:1".into()),
            },
        );
        s.save().unwrap();
        let back = MintedSessions::load(Some(path.clone())).unwrap();
        assert_eq!(back, s);
        assert!(!home.path().join("supervisor").join("demo.sessions.json.tmp").exists());
        std::fs::write(&path, "not json").unwrap();
        assert!(MintedSessions::load(Some(path)).is_err());
    }

    #[test]
    fn minted_sessions_of_unknown_origin_take_their_config_files_and_count_as_elsewhere_without_one() {
        let home = tempfile::tempdir().unwrap();
        let (path, mcp) = (MintedSessions::path(home.path(), "demo"), home.path().join("mcp"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"{"fm-demo-workspace":{"sessionId":"7","role":"workspace"},"fm-demo-alpha":{"sessionId":"8","role":"repo"},
                "fm-demo-beta":{"sessionId":"9","role":"repo","origin":"https://b.example"}}"#,
        )
        .unwrap();
        supraman_claude::write_mcp_config(&mcp, "fm-demo-workspace", "http://127.0.0.1:1/mcp", "wss_fake", &[]).unwrap();
        let mut s = MintedSessions::load(Some(path.clone())).unwrap();
        assert_eq!(s.get("fm-demo-workspace").unwrap().origin, None, "a file from before m5/48");
        s.adopt_origins(&mcp).unwrap();
        let back = MintedSessions::load(Some(path)).unwrap();
        assert_eq!(back.get("fm-demo-workspace").unwrap().origin.as_deref(), Some("http://127.0.0.1:1"), "saved");
        assert_eq!(back.get("fm-demo-alpha").unwrap().origin, None, "no config file: unknown");
        assert_eq!(
            back.elsewhere("https://b.example/"),
            [("fm-demo-alpha".to_string(), None), ("fm-demo-workspace".to_string(), Some("http://127.0.0.1:1".to_string()))]
        );
        assert_eq!(back.elsewhere("http://127.0.0.1:1").len(), 2, "alpha unknown, beta on b");
    }

    const SID: &str = "aaaa0001-0000-4000-8000-000000000001";
    const NAME: &str = "fm-demo-workspace";

    fn listed(session_id: &str) -> Session {
        Session {
            id: session_id[..8].into(),
            session_id: session_id.into(),
            name: "x".into(),
            cwd: PathBuf::new(),
            kind: supraman_claude::Kind::Background,
            state: Some(supraman_claude::SessionState::Stopped),
            status: None,
            waiting_for: None,
            pid: None,
            started_at: None,
        }
    }

    /// A keeper of `home` over a fake Workman: `NAME` minted there, recorded at `record`, its token kept and its
    /// config written.
    fn kept_at(home: &Path, fake: &FakeWorkman, record: PathBuf) -> (Keeper, String) {
        let keeper = Keeper::new(home, &home.join("mcp"), Arc::new(supraman_core::secret::MemoryStore::default()));
        let mut req = request();
        req.replace = false;
        let minted = HttpMinter::default().mint(fake.base(), "fake-static", &req).unwrap();
        let mut records = MintedSessions::empty(Some(record));
        let session = MintedSession {
            session_id: minted.session_id.clone(),
            role: "workspace".into(),
            claude_session_id: Some(SID.into()),
            last_active: None,
            origin: Some(fake.base().to_string()),
        };
        records.insert_minted(NAME, session, &req);
        records.save().unwrap();
        keeper.write(NAME, &fake.mcp_url(), &minted.token, None, &[]).unwrap();
        (keeper, minted.token)
    }

    fn header_of(token: &str) -> String {
        serde_json::json!({ "Authorization": format!("Bearer {token}") }).to_string()
    }

    fn mints(fake: &FakeWorkman) -> Vec<serde_json::Value> {
        fake.seen().iter().filter(|s| s.method == "POST" && s.target == "/api/agent-sessions").map(|s| serde_json::from_str(&s.body).unwrap()).collect()
    }

    #[test]
    fn remint_keeps_a_token_minted_with_replace_from_the_record() {
        let home = tempfile::tempdir().unwrap();
        let fake = FakeWorkman::start(Vec::new());
        let sessions = fake.enable_sessions();
        let (keeper, first) = kept_at(home.path(), &fake, MintedSessions::path(home.path(), "demo"));
        let signed = || Bearer::Signed { url: fake.mcp_url(), token: "fake-static".into() };

        sessions.revoke_name(NAME);
        remint(&keeper, NAME, signed(), &HttpMinter::default()).unwrap();
        let fresh = sessions.token_of(NAME).expect("a fresh token");
        assert_ne!(fresh, first);
        assert_eq!(keeper.kept(NAME).as_deref(), Some(fresh.as_str()), "it is kept");
        let posted = mints(&fake).pop().unwrap();
        assert_eq!((&posted["replace"], &posted["claudeSessionId"], &posted["name"]), (&true.into(), &SID.into(), &NAME.into()), "{posted}");
        assert_eq!((&posted["launchedBy"], &posted["role"]), (&"supraman/demo".into(), &"controller".into()), "the record's identity");
        assert!(!supraman_claude::mcp_config_url(keeper.mcp_dir(), NAME).is_none(), "the config stays");

        sessions.revoke_name(NAME);
        assert!(remint(&keeper, NAME, Bearer::SignedOut, &HttpMinter::default()).is_err(), "signed out: no mint");
        assert_eq!((keeper.kept(NAME).as_deref(), mints(&fake).len()), (Some(fresh.as_str()), 2), "the kept one stays");

        keeper.store().delete(&supraman_claude::mcp_token_key(NAME)).unwrap();
        remint(&keeper, NAME, signed(), &HttpMinter::default()).unwrap();
        assert_eq!(keeper.kept(NAME), sessions.token_of(NAME), "a missing kept token is minted too");
        assert_eq!(mints(&fake).len(), 3);
    }

    #[test]
    fn a_row_launched_as_foreman_is_reminted_under_supraman_without_a_409() {
        let home = tempfile::tempdir().unwrap();
        let fake = FakeWorkman::start(Vec::new());
        let sessions = fake.enable_sessions();
        let (keeper, _) = kept_at(home.path(), &fake, MintedSessions::path(home.path(), "demo"));
        let signed = || Bearer::Signed { url: fake.mcp_url(), token: "fake-static".into() };
        let remint = || super::remint(&keeper, NAME, signed(), &HttpMinter::default());

        remint().unwrap();
        let fresh = sessions.token_of(NAME).expect("minted");
        assert_eq!(keeper.kept(NAME), Some(fresh));
        let posts = mints(&fake);
        assert_eq!(posts.len(), 2, "one mint, no 409 before it: {posts:?}");
        let posted = &posts[1];
        assert_eq!((&posted["launchedBy"], &posted["origin"], &posted["takeoverFrom"]), (&"supraman/demo".into(), &"supraman/demo".into(), &"foreman/demo".into()), "{posted}");

        let (_, recorded) = MintedSessions::find(home.path(), NAME).unwrap();
        assert_eq!(recorded.launched_by, "foreman/demo", "the record is the workspace's, which relabels it on its own mint");
        assert!(remint().is_ok());
        let posts = mints(&fake);
        assert_eq!(posts.len(), 3, "kept.json holds the moved label: one plain mint");
        assert_eq!((&posts[2]["launchedBy"], posts[2].get("takeoverFrom")), (&"supraman/demo".into(), None));

        let token = sessions.token_of(NAME).unwrap();
        keeper.write(NAME, &fake.mcp_url(), &token, None, &[]).unwrap();
        assert!(remint().is_ok());
        let posts = mints(&fake);
        assert_eq!(posts.len(), 5, "the label forgotten, the takeover 409s on the moved row and the plain mint takes it");
        assert_eq!((&posts[3]["takeoverFrom"], &posts[4]["launchedBy"], posts[4].get("takeoverFrom")), (&"foreman/demo".into(), &"supraman/demo".into(), None));
        assert_ne!(sessions.token_of(NAME), Some(token), "a fresh token");
    }

    #[test]
    fn a_takeover_needs_the_rows_launcher_and_its_own_bearer_and_a_refused_one_keeps_the_row() {
        let fake = FakeWorkman::start(Vec::new());
        let sessions = fake.enable_sessions();
        let minter = HttpMinter::default();
        let first = minter.mint(fake.base(), "fake-static", &request()).unwrap();
        let moving = |from: &str| MintRequest { launched_by: "supraman/demo".into(), takeover_from: Some(from.into()), ..request() };
        let refused = |r: Result<Minted, MintError>| matches!(r, Err(MintError::Refused { status: 409, .. }));
        assert!(refused(minter.mint(fake.base(), "fake-static", &MintRequest { launched_by: "supraman/demo".into(), ..request() })), "no takeover");
        assert!(refused(minter.mint(fake.base(), "fake-static", &moving("foreman/other"))), "not the row's launcher");
        assert!(refused(minter.mint(fake.base(), "fake-other", &moving("foreman/demo"))), "another bearer");
        assert!(refused(minter.mint(fake.base(), "fake-static", &MintRequest { replace: false, ..moving("foreman/demo") })), "a live token without replace");
        assert!(sessions.live(&first.token), "nothing moved the row");
        assert!(minter.mint(fake.base(), "fake-static", &request()).is_ok(), "still foreman's");
        let moved = minter.mint(fake.base(), "fake-static", &moving("foreman/demo")).unwrap();
        assert_eq!(moved.session_id, first.session_id, "the same row");
        assert!(refused(minter.mint(fake.base(), "fake-static", &request())), "now supraman's");
    }

    #[test]
    fn a_mint_without_a_former_label_is_one_plain_call_and_its_409_stands() {
        let fake = FakeWorkman::start(Vec::new());
        fake.enable_sessions();
        let minter = HttpMinter::default();
        minter.mint(fake.base(), "fake-static", &request()).unwrap();
        let ours = MintRequest { launched_by: launched_by("demo"), replace: false, ..request() };
        let e = mint_as(&minter, fake.base(), "fake-static", &ours, None).unwrap_err();
        assert!(matches!(&e, MintError::Refused { status: 409, reason } if reason.contains("another launcher")), "{e:?}");
        let e = mint_as(&minter, fake.base(), "fake-static", &ours, Some("supraman/demo")).unwrap_err();
        assert!(matches!(&e, MintError::Refused { status: 409, .. }), "the same label is no takeover: {e:?}");
        assert!(mints(&fake).iter().all(|m| m.get("takeoverFrom").is_none()));
        assert_eq!(mints(&fake).len(), 3);
    }

    #[test]
    fn header_is_the_kept_token_or_the_placeholder_and_logs_why() {
        let home = tempfile::tempdir().unwrap();
        Keeper::open(home.path()).keep_token("hand-a-1", "fake-kept").unwrap();
        assert_eq!(header(Some(home.path()), "hand-a-1"), header_of("fake-kept"));
        assert_eq!(header(Some(home.path()), "hand-x-1"), NO_TOKEN_HEADER);
        assert_eq!(header(Some(home.path()), "../x"), NO_TOKEN_HEADER);
        assert_eq!(header(None, "hand-a-1"), NO_TOKEN_HEADER);
        let log = std::fs::read_to_string(home.path().join("logs").join("mcp-header.log")).unwrap();
        let said: Vec<&str> = log.lines().map(|l| l.split_once(' ').unwrap().1).collect();
        assert_eq!(said[..2], ["hand-a-1 kept", "hand-x-1 placeholder: no Workman token is kept for hand-x-1"]);
        assert!(said[2].starts_with(r#"../x placeholder: not a session name"#) && said.len() == 3 && !log.contains("fake-kept"), "{log}");
    }

    #[test]
    fn absent_counts_only_non_empty_snapshots_in_a_row() {
        let bound = [(NAME.to_string(), SID.to_string())];
        let other = listed("bbbb0002-0000-4000-8000-000000000002");
        let mut absent = Absent::default();
        assert!(absent.deleted(&bound, std::slice::from_ref(&other)).is_empty(), "once");
        assert!(absent.deleted(&bound, &[]).is_empty(), "an empty snapshot");
        assert!(absent.deleted(&bound, std::slice::from_ref(&other)).is_empty(), "the count started over");
        assert!(absent.deleted(&bound, &[listed(SID)]).is_empty(), "listed again");
        assert!(absent.deleted(&bound, std::slice::from_ref(&other)).is_empty());
        assert_eq!(absent.deleted(&bound, std::slice::from_ref(&other)), [SID.to_string()], "missing twice in a row");
    }

    #[test]
    fn a_deleted_runs_session_is_revoked_and_its_record_config_and_kept_token_go() {
        let home = tempfile::tempdir().unwrap();
        let fake = FakeWorkman::start(Vec::new());
        let sessions = fake.enable_sessions();
        let record = MintedSessions::run_path(home.path(), NAME);
        let (keeper, token) = kept_at(home.path(), &fake, record.clone());
        let signed = || Bearer::Signed { url: fake.mcp_url(), token: "fake-static".into() };
        let other = listed("bbbb0002-0000-4000-8000-000000000002");
        let mut absent = Absent::default();
        let mut look = |observed: &[Session]| revoke_deleted_runs(&keeper, &HttpMinter::default(), signed, &mut absent, observed);
        assert!(look(&[]).is_empty());
        assert!(look(std::slice::from_ref(&other)).is_empty());
        assert!(sessions.live(&token) && record.exists(), "one snapshot is not a delete");
        assert!(look(std::slice::from_ref(&other)).is_empty(), "no notice");
        assert!(!sessions.live(&token), "revoked");
        assert!(!record.exists() && keeper.kept(NAME).is_none(), "its record and kept token go");
        assert!(!supraman_claude::mcp_config_path(keeper.mcp_dir(), NAME).unwrap().exists(), "its config goes");
    }

    #[test]
    fn a_deleted_run_leaves_the_other_names_of_its_file() {
        let home = tempfile::tempdir().unwrap();
        let fake = FakeWorkman::start(Vec::new());
        let sessions = fake.enable_sessions();
        let record = MintedSessions::run_path(home.path(), NAME);
        let (keeper, token) = kept_at(home.path(), &fake, record.clone());
        let other = "bbbb0002-0000-4000-8000-000000000002";
        let mut records = MintedSessions::load(Some(record.clone())).unwrap();
        let session = MintedSession { session_id: "9".into(), role: "workspace".into(), claude_session_id: Some(other.into()), last_active: None, origin: Some(fake.base().to_string()) };
        records.insert_minted("fm-demo-other", session, &request());
        records.save().unwrap();
        let signed = || Bearer::Signed { url: fake.mcp_url(), token: "fake-static".into() };
        let mut absent = Absent::default();
        for _ in 0..2 {
            assert!(revoke_deleted_runs(&keeper, &HttpMinter::default(), signed, &mut absent, &[listed(other)]).is_empty());
        }
        assert!(!sessions.live(&token) && keeper.kept(NAME).is_none(), "the gone name is revoked");
        let left = MintedSessions::load(Some(record)).unwrap();
        assert_eq!(left.names().collect::<Vec<_>>(), ["fm-demo-other"], "the other name stays in its file");
    }

    #[test]
    fn a_run_missing_from_the_listing_whose_transcript_remains_is_not_revoked() {
        let home = tempfile::tempdir().unwrap();
        let fake = FakeWorkman::start(Vec::new());
        let sessions = fake.enable_sessions();
        let record = MintedSessions::run_path(home.path(), NAME);
        let (keeper, token) = kept_at(home.path(), &fake, record.clone());
        let claude = home.path().join("claude");
        let project = claude.join("projects").join("C--demo");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join(format!("{SID}.jsonl")), "{}
").unwrap();
        let keeper = keeper.with_claude_dir(Some(claude));
        let signed = || Bearer::Signed { url: fake.mcp_url(), token: "fake-static".into() };
        let other = listed("bbbb0002-0000-4000-8000-000000000002");
        let mut absent = Absent::default();
        for _ in 0..4 {
            assert!(revoke_deleted_runs(&keeper, &HttpMinter::default(), signed, &mut absent, std::slice::from_ref(&other)).is_empty());
        }
        assert!(sessions.live(&token) && record.exists() && keeper.kept(NAME).is_some(), "its transcript says it is not deleted");
    }

    #[test]
    fn a_sign_out_marks_the_runs_minted_there_and_keeps_their_files() {
        let home = tempfile::tempdir().unwrap();
        let fake = FakeWorkman::start(Vec::new());
        fake.enable_sessions();
        let record = MintedSessions::run_path(home.path(), NAME);
        let (keeper, token) = kept_at(home.path(), &fake, record.clone());
        let marked = || keeping::read(home.path()).unwrap().get(NAME).is_some_and(|k| k.signed_out);
        assert!(runs_signed_out(&keeper, "https://b.example").is_empty());
        assert!(record.exists() && !marked(), "another server's sign-out");
        assert!(runs_signed_out(&keeper, &format!("{}/", fake.base())).is_empty());
        assert!(record.exists() && keeper.kept(NAME).as_deref() == Some(token.as_str()) && marked());
        assert!(supraman_claude::mcp_config_url(keeper.mcp_dir(), NAME).is_some(), "its config stays");
        assert!(fake.seen().iter().all(|s| s.method != "DELETE"), "the grant revoke took it");
    }

    #[test]
    fn a_record_keeps_its_mint_and_is_found_in_any_of_the_homes_files() {
        let home = tempfile::tempdir().unwrap();
        let fake = FakeWorkman::start(Vec::new());
        fake.enable_sessions();
        kept_at(home.path(), &fake, MintedSessions::hands_path(home.path()));
        let (m, req) = MintedSessions::find(home.path(), NAME).expect("found in hands.json");
        assert_eq!((m.claude_session_id.as_deref(), req.claude_session_id.as_deref(), req.replace), (Some(SID), Some(SID), true));
        assert_eq!(MintedSessions::find(home.path(), "fm-demo-other"), None);
        let saved = std::fs::read_to_string(MintedSessions::hands_path(home.path())).unwrap();
        assert!(!saved.contains("wss_"), "no token in a record: {saved}");
    }
}
