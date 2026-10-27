//! Per-session WorkState tokens (Skills `docs/workstate/14-agent-launch.md` §2). A session with a role
//! token gets a `wss_` token minted for its name with the static role token as bearer; only the minted
//! token reaches its config file. The static token stays in Foreman's process: it is the bearer of
//! the mint, the PATCH of the claude session uuid and the revoke, never a file or an argument.
//!
//! The minted token's expiry slides on use (72 h idle window). A session idle longer gets HTTP 401 on
//! `/mcp`: the supervisor shows it as [`crate::NeedsKind::TokenExpired`] (no turn seen within the
//! window), and its next launch or resume mints again with `replace`.

use std::collections::BTreeMap;
use std::fmt;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// `POST /api/agent-sessions`. `role` is the WorkState role (never above the bearer's),
/// `external_role` Foreman's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MintRequest {
    pub name: String,
    pub role: String,
    pub launched_by: String,
    pub origin: String,
    pub cwd: String,
    pub external_role: String,
    /// One live token per name: `replace` revokes the old one and keeps the row and its `sessionId`.
    pub replace: bool,
    /// The repository a repo session works in: its bare `work_brief()` opens the project bound to it
    /// (m5/8). `None` for the workspace session and the relay (the workspace root is not a repo).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo: Option<MintRepo>,
}

/// `repo:{root, remote?}` of a mint, as git prints them; WorkState normalises both (Skills
/// `docs/workstate/02-data-model.md`, Identity).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MintRepo {
    /// The main checkout's root: the parent of `git rev-parse --path-format=absolute --git-common-dir`,
    /// the same for every worktree.
    pub root: String,
    /// `git remote get-url origin`; `None` without an origin.
    #[serde(skip_serializing_if = "Option::is_none")]
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
    let out = foreman_claude::no_window(&mut std::process::Command::new("git"))
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !text.is_empty()).then_some(text)
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
    #[error("WorkState at {base} is unreachable ({reason})")]
    Unreachable { base: String, reason: String },
    #[error("WorkState answered HTTP {status}{}", reason_suffix(.reason))]
    Refused { status: u16, reason: String },
    #[error("WorkState's answer was not understood: {0}")]
    BadResponse(String),
}

fn reason_suffix(reason: &str) -> String {
    if reason.is_empty() { String::new() } else { format!(": {reason}") }
}

/// The three calls on `/api/agent-sessions`. `base` is the server's origin ([`api_base`]), `bearer`
/// the static role token. Implemented by [`HttpMinter`]; tests use a fake.
pub trait Minter {
    fn mint(&self, base: &str, bearer: &str, request: &MintRequest) -> Result<Minted, MintError>;
    /// `PATCH /api/agent-sessions/{id}` `{claudeSessionId}`.
    fn bind(&self, base: &str, bearer: &str, session_id: &str, claude_session_id: &str) -> Result<(), MintError>;
    /// `DELETE /api/agent-sessions/{id}`; a `404` counts as revoked.
    fn revoke(&self, base: &str, bearer: &str, session_id: &str) -> Result<(), MintError>;
}

/// The server's origin, from its MCP URL; shared with the read API client.
pub use foreman_workstate::api_base;

/// Blocking HTTP/1.1 without TLS (WorkState is on localhost), no proxy, 15 s per call.
pub struct HttpMinter {
    agent: ureq::Agent,
}

impl Default for HttpMinter {
    fn default() -> Self {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .proxy(None)
            .max_redirects(0)
            .timeout_global(Some(Duration::from_secs(15)))
            .build();
        Self { agent: config.into() }
    }
}

impl HttpMinter {
    fn send(&self, base: &str, req: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<(u16, String), MintError> {
        let mut res = req.map_err(|e| MintError::Unreachable {
            base: base.to_string(),
            reason: e.to_string(),
        })?;
        let status = res.status().as_u16();
        let body = res.body_mut().read_to_string().unwrap_or_default();
        Ok((status, body))
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

fn refused(status: u16, body: &str) -> MintError {
    MintError::Refused {
        status,
        reason: server_reason(body),
    }
}

fn sessions_url(base: &str, session_id: &str) -> String {
    let id: String = session_id
        .bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect();
    format!("{base}/api/agent-sessions/{id}")
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
    /// WorkState's `sessionId`, for the PATCH and the DELETE.
    pub session_id: String,
    /// Foreman's role, whose static token is the bearer of the PATCH and the DELETE.
    pub role: String,
    /// The claude session uuid PATCHed since the last mint; `None` until then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_session_id: Option<String>,
    /// Epoch ms of the mint or of the last turn seen within the token's window: what slid its expiry
    /// last. `None` for a file written before m5/7; the first tick sets it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_active: Option<crate::Millis>,
}

/// Per session name, its [`MintedSession`]; persisted at `<FOREMAN_HOME>/supervisor/<workspace>.sessions.json`
/// so a restarted app still revokes what it minted. Without a path it lives in memory only.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MintedSessions {
    path: Option<PathBuf>,
    names: BTreeMap<String, MintedSession>,
}

impl MintedSessions {
    pub fn path(home: &Path, workspace: &str) -> PathBuf {
        home.join("supervisor").join(format!("{workspace}.sessions.json"))
    }

    pub fn empty(path: Option<PathBuf>) -> Self {
        Self { path, names: BTreeMap::new() }
    }

    /// A missing file is an empty set.
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

    /// Written to a `.tmp` beside it, then renamed over it.
    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(&self.names).map_err(std::io::Error::other)?)?;
        std::fs::rename(&tmp, path)
    }

    pub fn get(&self, name: &str) -> Option<&MintedSession> {
        self.names.get(name)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut MintedSession> {
        self.names.get_mut(name)
    }

    pub fn insert(&mut self, name: &str, session: MintedSession) {
        self.names.insert(name.to_string(), session);
    }

    pub fn remove(&mut self, name: &str) -> Option<MintedSession> {
        self.names.remove(name)
    }

    pub fn names(&self) -> impl Iterator<Item = &String> {
        self.names.keys()
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;

    use super::*;

    /// One request as the fake server read it.
    #[derive(Debug)]
    struct Seen {
        line: String,
        headers: Vec<(String, String)>,
        body: String,
    }

    impl Seen {
        fn header(&self, name: &str) -> Option<&str> {
            self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
        }
    }

    /// A local HTTP server that answers each request with the next of `answers` and sends what it read.
    fn fake_server(answers: Vec<(u16, &'static str)>) -> (String, mpsc::Receiver<Seen>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for (status, body) in answers {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let mut headers = Vec::new();
                loop {
                    let mut h = String::new();
                    reader.read_line(&mut h).unwrap();
                    let h = h.trim_end();
                    if h.is_empty() {
                        break;
                    }
                    let (k, v) = h.split_once(':').unwrap();
                    headers.push((k.trim().to_string(), v.trim().to_string()));
                }
                let len = headers
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                    .map_or(0, |(_, v)| v.parse().unwrap());
                let mut buf = vec![0; len];
                reader.read_exact(&mut buf).unwrap();
                tx.send(Seen {
                    line: line.trim_end().to_string(),
                    headers,
                    body: String::from_utf8(buf).unwrap(),
                })
                .unwrap();
                let mut stream = stream;
                write!(
                    stream,
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        (base, rx)
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
        }
    }

    #[test]
    fn mint_http_client_posts_the_body_with_the_bearer_and_reads_the_201() {
        let (base, seen) = fake_server(vec![(
            201,
            r#"{"sessionId":"7f3c","name":"fm-demo-workspace","role":"controller","token":"wss_fake","expiresAt":"2026-10-29T00:00:00Z"}"#,
        )]);
        let minted = HttpMinter::default().mint(&base, "fake-static", &request()).unwrap();
        assert_eq!(minted.session_id, "7f3c");
        assert_eq!(minted.token, "wss_fake");
        assert_eq!(minted.expires_at.as_deref(), Some("2026-10-29T00:00:00Z"));
        assert!(!format!("{minted:?}").contains("wss_fake"), "Debug redacts the token");
        let s = seen.recv().unwrap();
        assert_eq!(s.line, "POST /api/agent-sessions HTTP/1.1");
        assert_eq!(s.header("Authorization"), Some("Bearer fake-static"));
        assert_eq!(s.header("Content-Type"), Some("application/json"));
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
        let (base, seen) = fake_server(vec![(201, r#"{"sessionId":"8","token":"wss_fake"}"#), (201, r#"{"sessionId":"9","token":"wss_fake"}"#)]);
        let mut req = request();
        req.external_role = "repo".into();
        req.repo = Some(MintRepo {
            root: "C:/demo/Alpha".into(),
            remote: Some("git@github.com:demo/alpha.git".into()),
        });
        HttpMinter::default().mint(&base, "fake-static", &req).unwrap();
        let body: serde_json::Value = serde_json::from_str(&seen.recv().unwrap().body).unwrap();
        assert_eq!(body["repo"], serde_json::json!({ "root": "C:/demo/Alpha", "remote": "git@github.com:demo/alpha.git" }));
        req.repo = Some(MintRepo { root: "C:/demo/Beta".into(), remote: None });
        HttpMinter::default().mint(&base, "fake-static", &req).unwrap();
        let body: serde_json::Value = serde_json::from_str(&seen.recv().unwrap().body).unwrap();
        assert_eq!(body["repo"], serde_json::json!({ "root": "C:/demo/Beta" }), "no origin, no remote");
    }

    #[test]
    fn mint_git_repo_of_a_worktree_is_its_main_checkout() {
        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join("main");
        std::fs::create_dir(&main).unwrap();
        let run = |cwd: &Path, args: &[&str]| {
            let ok = std::process::Command::new("git")
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
        // m5/26: a workspace root that is a repo, with a repo inside it: each sends its own root
        let child = main.join("child");
        std::fs::create_dir(&child).unwrap();
        run(&child, &["init", "-q"]);
        assert_eq!(git_repo(&main).unwrap().root, of_main.root, "the root repo");
        assert_eq!(canon(&git_repo(&child).unwrap().root), canon(child.to_str().unwrap()), "the repo inside it");
    }

    #[test]
    fn mint_http_client_patches_and_deletes_by_session_id() {
        let (base, seen) = fake_server(vec![(200, "{}"), (204, ""), (404, "")]);
        let m = HttpMinter::default();
        m.bind(&base, "fake-static", "42", "aaaa0001-0000-4000-8000-000000000001").unwrap();
        m.revoke(&base, "fake-static", "42").unwrap();
        m.revoke(&base, "fake-static", "a b").unwrap();
        let patch = seen.recv().unwrap();
        assert_eq!(patch.line, "PATCH /api/agent-sessions/42 HTTP/1.1");
        assert_eq!(patch.header("Authorization"), Some("Bearer fake-static"));
        assert_eq!(patch.body, r#"{"claudeSessionId":"aaaa0001-0000-4000-8000-000000000001"}"#);
        let delete = seen.recv().unwrap();
        assert_eq!(delete.line, "DELETE /api/agent-sessions/42 HTTP/1.1");
        assert_eq!(delete.header("Authorization"), Some("Bearer fake-static"));
        assert_eq!(seen.recv().unwrap().line, "DELETE /api/agent-sessions/a%20b HTTP/1.1", "404 counts as revoked");
    }

    #[test]
    fn mint_http_client_reports_status_and_reason_never_the_bearer() {
        let (base, _seen) = fake_server(vec![(409, r#"{"error":"a live token exists for fm-demo-workspace"}"#), (201, r#"{"token":"wss_x"}"#)]);
        let m = HttpMinter::default();
        let e = m.mint(&base, "fake-static", &request()).unwrap_err();
        assert_eq!(
            e,
            MintError::Refused {
                status: 409,
                reason: "a live token exists for fm-demo-workspace".into()
            }
        );
        assert_eq!(e.to_string(), "WorkState answered HTTP 409: a live token exists for fm-demo-workspace");
        assert_eq!(m.mint(&base, "fake-static", &request()).unwrap_err(), MintError::BadResponse("no sessionId".into()));
        // nothing listens on a port just freed
        let free = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
        let e = m.mint(&format!("http://{free}"), "fake-static", &request()).unwrap_err();
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
            MintedSession { session_id: "7".into(), role: "workspace".into(), claude_session_id: None, last_active: Some(42) },
        );
        s.save().unwrap();
        let back = MintedSessions::load(Some(path.clone())).unwrap();
        assert_eq!(back, s);
        assert!(!home.path().join("supervisor").join("demo.sessions.json.tmp").exists());
        std::fs::write(&path, "not json").unwrap();
        assert!(MintedSessions::load(Some(path)).is_err());
    }
}
