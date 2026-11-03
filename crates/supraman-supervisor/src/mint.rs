//! Per-session Workman tokens (Skills `docs/workstate/14-agent-launch.md` §2): only the minted `wss_`
//! token reaches a session's config file; the bearer never leaves Foreman's process.

use std::collections::BTreeMap;
use std::fmt;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// `POST /api/agent-sessions`. `role` is the Workman role (never above the bearer's),
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
    /// The repository a repo session works in: its bare `work_brief()` opens the project bound to it.
    /// `None` for the workspace session and the relay (the workspace root is not a repo).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo: Option<MintRepo>,
    /// The claude session uuid the token is for, when it is known before claude starts: a
    /// resume's, or the `--session-id` of a foreground `claude`. Never a guess: a `--bg` launch's id
    /// is known only from its row (the PATCH of [`Minter::bind`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claude_session_id: Option<String>,
}

/// `repo:{root, remote?}` of a mint, as git prints them; Workman normalises both (Skills
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

/// The three calls on `/api/agent-sessions`. `base` is the server's origin ([`api_base`]).
/// Implemented by [`HttpMinter`]; tests use a fake.
pub trait Minter {
    fn mint(&self, base: &str, bearer: &str, request: &MintRequest) -> Result<Minted, MintError>;
    /// `PATCH /api/agent-sessions/{id}` `{claudeSessionId}`.
    fn bind(&self, base: &str, bearer: &str, session_id: &str, claude_session_id: &str) -> Result<(), MintError>;
    /// `DELETE /api/agent-sessions/{id}`; a `404` counts as revoked.
    fn revoke(&self, base: &str, bearer: &str, session_id: &str) -> Result<(), MintError>;
}

/// The server's origin, from its MCP URL; shared with the read API client.
pub use supraman_workman::api_base;

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
    /// The server the token is from ([`api_base`] of its MCP URL); `None`: unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
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

    /// Gives each session of unknown origin the origin of the URL in its config file under `mcp_dir`, if
    /// that is still there; saved when one changed.
    pub fn adopt_origins(&mut self, mcp_dir: &Path) -> std::io::Result<()> {
        let mut changed = false;
        for (name, m) in self.names.iter_mut().filter(|(_, m)| m.origin.is_none()) {
            if let Some(url) = supraman_claude::mcp_config_url(mcp_dir, name) {
                m.origin = Some(api_base(&url));
                changed = true;
            }
        }
        if changed { self.save() } else { Ok(()) }
    }

    /// Every session whose token is not from `origin`, with the origin it is from (`None`: unknown).
    pub fn elsewhere(&self, origin: &str) -> Vec<(String, Option<String>)> {
        let origin = api_base(origin);
        self.names
            .iter()
            .filter(|(_, m)| m.origin.as_deref() != Some(origin.as_str()))
            .map(|(name, m)| (name.clone(), m.origin.clone()))
            .collect()
    }
}

/// Deletes `name`'s Workman config under `mcp_dir` (written by `supraman_claude::write_mcp_config`); a file that is
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
                origin: Some("http://127.0.0.1:5290".into()),
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
        supraman_claude::write_mcp_config(&mcp, "fm-demo-workspace", "http://127.0.0.1:5290/mcp", "wss_fake", &[]).unwrap();
        let mut s = MintedSessions::load(Some(path.clone())).unwrap();
        assert_eq!(s.get("fm-demo-workspace").unwrap().origin, None, "a file from before m5/48");
        s.adopt_origins(&mcp).unwrap();
        let back = MintedSessions::load(Some(path)).unwrap();
        assert_eq!(back.get("fm-demo-workspace").unwrap().origin.as_deref(), Some("http://127.0.0.1:5290"), "saved");
        assert_eq!(back.get("fm-demo-alpha").unwrap().origin, None, "no config file: unknown");
        assert_eq!(
            back.elsewhere("https://b.example/"),
            [("fm-demo-alpha".to_string(), None), ("fm-demo-workspace".to_string(), Some("http://127.0.0.1:5290".to_string()))]
        );
        assert_eq!(back.elsewhere("http://127.0.0.1:5290").len(), 2, "alpha unknown, beta on b");
    }
}
