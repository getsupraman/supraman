//! Hand sessions (m5/20): the `+` on a repo row types `claude` into a shell. With WorkState set up
//! that claude gets its own minted `wss_` token like a supervised session ([`foreman_supervisor::mint`]):
//! role orchestrator (m5/35: a human drives it and runs queues, so it needs work_status/work_update),
//! `externalRole` `hand`, `repo` of its cwd, in an owner-only file passed as `--mcp-config <file>`
//! WITHOUT `--strict-mcp-config`, so the human keeps every other MCP server. The tab's pwsh runs the
//! launch as its `-EncodedCommand` (m5/22): nothing is typed or echoed, and only the path is in it.
//!
//! **The override is deliberate.** `--mcp-config` beats the user-scope `workstate` registration
//! (LEARNINGS mint/7), so a hand session talks to WorkState with Foreman's token, not the user's: it is
//! revoked with the tab and labelled `foreman/hand`. The tab card says so (`WorkState  Foreman token ·
//! orchestrator · <name>`).
//!
//! **Its name** is the session's own (m5/35): a resume mints with the name the tree shows for it
//! (registered name, else the transcript's title), a fresh `+` with `hand-<folder>-<n>`. It cannot be
//! renamed later: a minted token's name is its credential's, WorkState ignores `X-WorkState-Agent` and
//! refuses `work_session` for it (Skills `docs/workstate/11-sessions.md`, resolution order (a)). The
//! generated name stays the key of the config file and of `hands.json` either way.
//!
//! **Its claude session id** goes with the mint (`claudeSessionId`, m5/40): a resume's uuid, or for a
//! fresh `+` a new one that claude is started with (`--session-id`). WorkState lets a launcher take over
//! a name registered without one only when the ids match.
//!
//! The token is revoked when that claude exits (the launch deletes its file, [`Hands::ended`]),
//! when its tab closes, on quit, and on the next start for what an earlier run left
//! ([`Hands::release_all`]). The mint's bearer is the access token of the app's WorkState sign-in
//! (m5/43; the operator as controller, which caps the orchestrator mint). Not signed in: the tab opens
//! plain. A refused orchestrator mint opens the tab plain with the reason; it never falls back to a
//! worker token.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use foreman_supervisor::{HttpMinter, MintError, MintRepo, MintRequest, MintedSession, MintedSessions, Minter, api_base, git_repo};
use foreman_workstate::Auth;

pub const EXTERNAL_ROLE: &str = "hand";
/// The WorkState role of every hand session, `+` and resume alike.
pub const ROLE: &str = "orchestrator";
/// A session's own name is cut to this many characters for WorkState.
const MAX_NAME: usize = 64;
const LAUNCHED_BY: &str = "foreman/hand";
/// What the launch prints before it starts plain `claude` when its file is already gone, quoted for a
/// pwsh '…' string.
const STALE: &str = "This tab''s WorkState token was revoked when its claude exited; starting plain claude. Open a new + session for a fresh token.";

/// How the `+` session starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Launch {
    /// No WorkState: plain `claude`, as before m5/20.
    Plain,
    /// Its own token in `config`. `name` keys the file and `hands.json`; `session` is the WorkState
    /// session's name; `notice` says why it is not the one asked for. `new_session` is the
    /// `--session-id` a fresh `+` starts claude with, the one its mint carries; `None` for a resume.
    Minted { name: String, session: String, config: PathBuf, notice: Option<String>, new_session: Option<String> },
    /// WorkState is set up but the mint failed: plain `claude`, and this notice.
    Unminted(String),
}

impl Launch {
    #[cfg(test)]
    pub fn script(&self) -> String {
        self.script_with("claude")
    }

    /// What the tab's pwsh runs before its first prompt: `claude` (its program and arguments, e.g.
    /// `claude --resume '<id>'`, m5/27). It deletes the config file once claude exits, however it exits,
    /// which is how [`Hands::ended`] sees it. Should the file be gone already (the tab closed while it
    /// started), it starts plain `claude`, saying why (m5/18), without the `--session-id` its mint carried.
    pub fn script_with(&self, claude: &str) -> String {
        match self {
            Launch::Minted { config, new_session, .. } => {
                let path = config.to_string_lossy().replace('\'', "''");
                let id = new_session.as_ref().map(|id| format!(" --session-id '{}'", id.replace('\'', "''"))).unwrap_or_default();
                format!(
                    "if (Test-Path -LiteralPath '{path}') {{ try {{ {claude}{id} --mcp-config '{path}' }} finally {{ Remove-Item -LiteralPath '{path}' -ErrorAction SilentlyContinue }} }} else {{ Write-Host '{STALE}'; {claude} }}"
                )
            }
            Launch::Plain | Launch::Unminted(_) => claude.into(),
        }
    }

    #[cfg(test)]
    pub fn shell_args(&self) -> [String; 3] {
        self.shell_args_with("claude")
    }

    /// Appended to the tab's pwsh flags: [`Self::script_with`] runs unseen, and the shell stays when it ends.
    pub fn shell_args_with(&self, claude: &str) -> [String; 3] {
        ["-NoExit".into(), "-EncodedCommand".into(), encoded_command(&self.script_with(claude))]
    }
}

/// pwsh's `-EncodedCommand`: base64 of the UTF-16LE script, so no quoting reaches the command line.
pub fn encoded_command(script: &str) -> String {
    use base64::Engine as _;
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// The hand sessions this app minted a token for, by name, persisted at [`Hands::path`].
pub struct Hands {
    home: PathBuf,
    auth: Arc<Auth>,
    minter: Box<dyn Minter + Send>,
    repo_of: fn(&Path) -> Option<MintRepo>,
    minted: MintedSessions,
    /// The WorkState name of each key in `minted` launched by this run.
    sessions: std::collections::HashMap<String, String>,
    /// Servers of released tokens that were not revoked here: their grants go ([`Self::take_left`]).
    left: Vec<String>,
}

/// What the panel says when `n` open claude tabs hold tokens from another server than `to`.
pub fn restart_notice(n: usize, to: &str) -> Option<String> {
    (n > 0).then(|| {
        let (tabs, them) = if n == 1 { ("tab", "it") } else { ("tabs", "them") };
        format!("{n} open claude {tabs} still use another WorkState server: restart {them} to use {to}.")
    })
}

/// [`Hands::release_all`] on a thread of its own, waited on for at most `within`: its notices, or `None` when
/// time ran out. It goes on revoking until the process ends; what it does not reach stays in `hands.json`.
pub fn release_all_within(hands: Arc<Mutex<Hands>>, within: Duration) -> Option<Vec<String>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || tx.send(hands.lock().unwrap_or_else(|e| e.into_inner()).release_all()));
    rx.recv_timeout(within).ok()
}

/// `raw` (a session's name as the tree shows it) as a WorkState session name: one line, no control or
/// bidi characters, at most [`MAX_NAME`] characters. `None` when nothing is left.
pub fn session_name(raw: &str) -> Option<String> {
    let line = foreman_ui::tabs::sanitize(raw)?;
    let cut: String = line.chars().take(MAX_NAME).collect();
    let cut = cut.trim_end().to_string();
    (!cut.is_empty()).then_some(cut)
}

impl Hands {
    pub fn path(home: &Path) -> PathBuf {
        home.join("supervisor").join("hands.json")
    }

    /// Over HTTP, with the repo from git. An unreadable file is forgotten, with a notice.
    pub fn open(home: &Path) -> (Self, Option<String>) {
        Self::with(home, crate::signin::auth(home), HttpMinter::default(), git_repo)
    }

    pub fn with(
        home: &Path,
        auth: Arc<Auth>,
        minter: impl Minter + Send + 'static,
        repo_of: fn(&Path) -> Option<MintRepo>,
    ) -> (Self, Option<String>) {
        let path = Self::path(home);
        let (mut minted, notice) = match MintedSessions::load(Some(path.clone())) {
            Ok(m) => (m, None),
            Err(e) => (
                MintedSessions::empty(Some(path.clone())),
                Some(format!("Cannot read {} ({e}): hand-session tokens minted before lapse after their idle window", path.display())),
            ),
        };
        let notice = notice.or_else(|| minted.adopt_origins(&home.join("mcp")).err().map(|e| format!("Cannot save {} ({e})", path.display())));
        let hands = Self {
            home: home.to_path_buf(),
            auth,
            minter: Box::new(minter),
            repo_of,
            minted,
            sessions: Default::default(),
            left: Vec::new(),
        };
        (hands, notice)
    }

    fn mcp_dir(&self) -> PathBuf {
        self.home.join("mcp")
    }

    fn config_path(&self, name: &str) -> std::io::Result<PathBuf> {
        foreman_claude::mcp_config_path(&self.mcp_dir(), name)
    }

    /// `Some` notice when the record of the minted tokens could not be saved: a token it lost is not
    /// revoked at the next start.
    fn save(&self) -> Option<String> {
        let path = Self::path(&self.home);
        self.minted.save().err().map(|e| format!("Cannot save {} ({e}): a hand token minted here is not revoked if Foreman ends without Quit", path.display()))
    }

    /// The bearer of the mint and the revoke: (the MCP URL, the sign-in's access token); `None` when
    /// not signed in.
    fn bearer(&self) -> Result<Option<(String, String)>, String> {
        crate::tokens::bearer(&self.auth)
    }

    /// `hand-<folder>-<n>`, the lowest `n` not held.
    fn next_name(&self, cwd: &Path) -> String {
        let folder: String = cwd
            .file_name()
            .map(|f| f.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' }).collect())
            .unwrap_or_default();
        let folder = if folder.trim_matches('-').is_empty() { "session".to_string() } else { folder };
        (1..).map(|n| format!("hand-{folder}-{n}")).find(|n| self.minted.get(n).is_none()).unwrap()
    }

    /// Mints the token of a `+` session in `cwd` and writes its config file. `own` is the session's
    /// own name when it has one (a resume); else, or when another launcher holds that name, it is
    /// `hand-<folder>-<n>`.
    #[cfg(test)]
    pub fn launch(&mut self, cwd: &Path, own: Option<&str>) -> Launch {
        self.launch_for(cwd, own, None)
    }

    /// [`Self::launch`] for a resume of `claude_session` (its full uuid; anything else is not sent).
    /// That uuid, or a fresh `+`'s new one, goes with the mint and is kept with the token so Delete can
    /// revoke it ([`Self::release_session`]).
    pub fn launch_for(&mut self, cwd: &Path, own: Option<&str>, claude_session: Option<&str>) -> Launch {
        let (url, bearer) = match self.bearer() {
            Ok(Some(b)) => b,
            Ok(None) => return Launch::Plain,
            Err(e) => return Launch::Unminted(format!("This claude session runs without its own WorkState token: {e}")),
        };
        let new_session = claude_session.is_none().then(foreman_claude::new_session_id);
        let claude_session = claude_session
            .filter(|s| foreman_claude::is_session_id(s))
            .map(str::to_string)
            .or_else(|| new_session.clone());
        let name = self.next_name(cwd);
        let folder = cwd.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
        let base = api_base(&url);
        let mut request = MintRequest {
            name: own.and_then(session_name).unwrap_or_else(|| name.clone()),
            role: ROLE.into(),
            launched_by: LAUNCHED_BY.into(),
            origin: format!("{LAUNCHED_BY}/{folder}"),
            cwd: cwd.to_string_lossy().into_owned(),
            external_role: EXTERNAL_ROLE.into(),
            replace: true,
            repo: (self.repo_of)(cwd),
            claude_session_id: claude_session.clone(),
        };
        let mut notice = None;
        // `replace` would take the token of another tab's session of that name
        if self.sessions.values().any(|s| *s == request.name) {
            notice = Some(format!("WorkState session {name}, not {}: another tab holds that name", request.name));
            request.name = name.clone();
        }
        let minted = match self.minter.mint(&base, &bearer, &request) {
            // `replace` covers a live token, so a 409 is another launcher's name (doc 14 §2)
            Err(MintError::Refused { status: 409, reason }) if request.name != name => {
                notice = Some(format!("WorkState session {name}, not {}: {reason}", request.name));
                request.name = name.clone();
                self.minter.mint(&base, &bearer, &request)
            }
            other => other,
        };
        let minted = match minted {
            Ok(m) => m,
            Err(e) => return Launch::Unminted(format!("This claude session runs without its own WorkState token: cannot mint an {ROLE} one ({e})")),
        };
        self.minted.insert(
            &name,
            MintedSession {
                session_id: minted.session_id.clone(),
                role: EXTERNAL_ROLE.into(),
                claude_session_id: claude_session,
                last_active: None,
                origin: Some(base),
            },
        );
        if let Some(unsaved) = self.save() {
            notice = Some(notice.map_or(unsaved.clone(), |n| format!("{n}; {unsaved}")));
        }
        self.sessions.insert(name.clone(), request.name.clone());
        match foreman_claude::write_mcp_config(&self.mcp_dir(), &name, &url, &minted.token, &[]) {
            Ok(config) => Launch::Minted { name, session: request.name, config, notice, new_session },
            Err(e) => {
                let revoked = self.release(&name);
                Launch::Unminted(format!(
                    "This claude session runs without its own WorkState token: cannot write its mcp config ({e}){}",
                    revoked.map(|r| format!("; {r}")).unwrap_or_default()
                ))
            }
        }
    }

    /// The session's claude has exited: its config file is gone.
    pub fn ended(&self, name: &str) -> bool {
        self.minted.get(name).is_some() && self.config_path(name).is_ok_and(|p| !p.exists())
    }

    /// Revokes `name`'s token and deletes its config file. `Some` notice when the revoke failed.
    pub fn release(&mut self, name: &str) -> Option<String> {
        let m = self.minted.remove(name)?;
        self.sessions.remove(name);
        if let Ok(config) = self.config_path(name) {
            let _ = std::fs::remove_file(config);
        }
        let bearer = self.bearer();
        let other = match (&bearer, &m.origin) {
            (Ok(Some((url, _))), Some(o)) if *o != api_base(url) => Some(o.clone()),
            _ => None,
        };
        if let Some(other) = &other
            && !self.left.contains(other)
        {
            self.left.push(other.clone());
        }
        let result = match bearer {
            Ok(Some(_)) if other.is_some() => Ok(()),
            Ok(Some((url, token))) => self.minter.revoke(&api_base(&url), &token, &m.session_id).map_err(|e| e.to_string()),
            Ok(None) => Err("not signed in to WorkState any more".into()),
            Err(e) => Err(e),
        };
        let revoke = result
            .err()
            .map(|e| format!("Cannot revoke the WorkState token of {name} (session {}): {e}; unused, it lapses after its idle window", m.session_id));
        match (revoke, self.save()) {
            (Some(r), Some(s)) => Some(format!("{r}; {s}")),
            (r, s) => r.or(s),
        }
    }

    /// Every token minted for claude session `claude_session_id` (m5/38: its Delete), as [`Self::release`].
    pub fn release_session(&mut self, claude_session_id: &str) -> Vec<String> {
        let names: Vec<String> = self
            .minted
            .names()
            .filter(|n| self.minted.get(n).is_some_and(|m| m.claude_session_id.as_deref() == Some(claude_session_id)))
            .cloned()
            .collect();
        names.iter().filter_map(|n| self.release(n)).collect()
    }

    /// Every hand token: on quit, and on start for those an earlier run left.
    pub fn release_all(&mut self) -> Vec<String> {
        let names: Vec<String> = self.minted.names().cloned().collect();
        names.iter().filter_map(|n| self.release(n)).collect()
    }

    /// The open sessions whose token is not from `origin`, with the server it is from (`None`: unknown).
    pub fn elsewhere(&self, origin: &str) -> Vec<(String, Option<String>)> {
        self.minted.elsewhere(origin)
    }

    /// The servers of tokens [`Self::release`] left to their server's grant revoke, since the last call.
    pub fn take_left(&mut self) -> Vec<String> {
        std::mem::take(&mut self.left)
    }

    #[cfg(test)]
    fn names(&self) -> Vec<String> {
        self.minted.names().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::mpsc;

    use foreman_workstate::fake::{FakeAuth, read_request};

    use super::*;

    /// A request the fake server should have seen: fail, never hang.
    const WAIT: std::time::Duration = std::time::Duration::from_secs(10);

    /// A WorkState stand-in on 127.0.0.1 that is also its fake OAuth server: answers a mint with each of
    /// `mints` in turn (then 201s), DELETE with 204, and sends back (request line, Authorization, body)
    /// of each `/api/agent-sessions` request.
    fn fake_workstate(mints: Vec<(u16, &'static str)>) -> (String, mpsc::Receiver<(String, String, String)>, Arc<FakeAuth>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let oauth = Arc::new(FakeAuth::new(&url));
        let as_server = oauth.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut mints = mints.into_iter();
            let mut n = 0;
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let (method, target, auth, body) = read_request(&stream).unwrap();
                if let Some(answer) = as_server.handle(&method, &target, &body) {
                    stream.write_all(answer.to_http().as_bytes()).unwrap();
                    continue;
                }
                let (line, auth) = (format!("{method} {target} HTTP/1.1"), auth.unwrap_or_default());
                let (status, answer) = if line.starts_with("POST") {
                    n += 1;
                    mints.next().map(|(s, b)| (s, b.to_string())).unwrap_or((201, format!(r#"{{"sessionId":"h{n}","token":"wss_fake{n}"}}"#)))
                } else {
                    (204, String::new())
                };
                write!(stream, "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}", answer.len()).unwrap();
                if tx.send((line, auth, body)).is_err() {
                    break;
                }
            }
        });
        (url, rx, oauth)
    }

    fn a_repo(cwd: &Path) -> Option<MintRepo> {
        cwd.ends_with("alpha").then(|| MintRepo {
            root: "C:/demo/alpha".into(),
            remote: Some("https://example.invalid/demo/alpha.git".into()),
        })
    }

    /// A Foreman home signed in to the fake at `url`; not signed in without one.
    fn home_with(dir: &Path, url: Option<&str>) -> (PathBuf, Arc<Auth>) {
        let home = dir.join("home");
        match url {
            Some(url) => {
                let auth = crate::tokens::signed_in(&home, url);
                (home, auth)
            }
            None => {
                foreman_core::Settings::default().save(&home).unwrap();
                let auth = Arc::new(Auth::with_store(&home, Arc::new(foreman_workstate::MemoryStore::default())));
                (home, auth)
            }
        }
    }

    /// The sign-in's access token is the bearer.
    fn oauth(auth: &str) -> bool {
        auth.starts_with("Bearer eyJ")
    }

    #[test]
    fn a_hand_tab_minted_on_another_server_is_told_to_restart_and_left_to_its_grant() {
        let dir = tempfile::tempdir().unwrap();
        let (a, seen_a, _) = fake_workstate(vec![]);
        let (b, seen_b, _) = fake_workstate(vec![]);
        let (home, auth) = home_with(dir.path(), Some(&a));
        let (mut hands, _) = Hands::with(&home, auth.clone(), HttpMinter::default(), a_repo);
        let Launch::Minted { name, .. } = hands.launch(Path::new(r"C:\demolpha"), None) else { panic!() };
        seen_a.recv_timeout(WAIT).unwrap();
        assert!(hands.elsewhere(&a).is_empty(), "minted there");

        let mut settings = foreman_core::Settings::load(&home).unwrap();
        settings.workstate.url = Some(b.clone());
        settings.save(&home).unwrap();
        auth.sign_in(&b, &foreman_workstate::auth::http_browser()).unwrap();
        assert_eq!(hands.elsewhere(&b), [(name.clone(), Some(a.clone()))]);
        assert_eq!(
            restart_notice(1, &b).as_deref(),
            Some(format!("1 open claude tab still use another WorkState server: restart it to use {b}.").as_str())
        );
        assert_eq!(restart_notice(0, &b), None);

        assert_eq!(hands.release(&name), None, "no notice");
        assert_eq!(hands.take_left(), [a.clone()], "A's grant revoke covers it");
        assert!(hands.take_left().is_empty());
        assert!(seen_b.recv_timeout(std::time::Duration::from_millis(300)).is_err(), "no DELETE of A's session id on B");
        assert!(seen_a.try_recv().is_err());
    }

    #[test]
    fn a_hands_file_from_before_m5_48_takes_its_config_files_origin() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(home.join("supervisor")).unwrap();
        std::fs::write(Hands::path(&home), r#"{"hand-alpha-1":{"sessionId":"h1","role":"hand"},"hand-beta-1":{"sessionId":"h2","role":"hand"}}"#).unwrap();
        foreman_claude::write_mcp_config(&home.join("mcp"), "hand-alpha-1", "http://127.0.0.1:5290/mcp", "wss_old", &[]).unwrap();
        let (_, auth) = home_with(dir.path(), None);
        let (hands, notice) = Hands::with(&home, auth, HttpMinter::default(), a_repo);
        assert_eq!(notice, None);
        assert_eq!(hands.elsewhere("https://b.example"), [
            ("hand-alpha-1".to_string(), Some("http://127.0.0.1:5290".to_string())),
            ("hand-beta-1".to_string(), None),
        ]);
        assert!(std::fs::read_to_string(Hands::path(&home)).unwrap().contains("http://127.0.0.1:5290"), "saved");
    }

    #[test]
    fn hand_mint_carries_the_repo_and_none_outside_a_repo() {
        let dir = tempfile::tempdir().unwrap();
        let (url, seen, _oauth) = fake_workstate(vec![]);
        let (home, auth) = home_with(dir.path(), Some(&url));
        let (mut hands, notice) = Hands::with(&home, auth.clone(), HttpMinter::default(), a_repo);
        assert_eq!(notice, None);

        let launch = hands.launch(Path::new(r"C:\demo\alpha"), None);
        let Launch::Minted { name, config, new_session: Some(sid), .. } = launch.clone() else { panic!("{launch:?}") };
        assert_eq!(name, "hand-alpha-1");
        assert!(foreman_claude::is_session_id(&sid), "{sid}");
        let (line, bearer, body) = seen.recv_timeout(WAIT).unwrap();
        assert_eq!(line, "POST /api/agent-sessions HTTP/1.1");
        assert!(oauth(&bearer), "the sign-in's access token is the bearer: {bearer}");
        let body: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(
            body,
            serde_json::json!({
                "name": "hand-alpha-1", "role": "orchestrator", "launchedBy": "foreman/hand", "origin": "foreman/hand/alpha",
                "cwd": r"C:\demo\alpha", "externalRole": "hand", "replace": true,
                "repo": { "root": "C:/demo/alpha", "remote": "https://example.invalid/demo/alpha.git" },
                "claudeSessionId": sid
            })
        );
        // m5/40: claude starts with the uuid its mint carried, and the token keeps it
        assert!(launch.script().contains(&format!("try {{ claude --session-id '{sid}' --mcp-config '")), "{}", launch.script());
        assert_eq!(hands.minted.get(&name).unwrap().claude_session_id.as_deref(), Some(sid.as_str()));
        let text = std::fs::read_to_string(&config).unwrap();
        assert!(text.contains("Bearer wss_fake1") && text.contains(&format!("{url}/mcp")), "the minted token, not the sign-in's");
        assert!(!text.contains("eyJ"));

        let Launch::Minted { name, new_session, .. } = hands.launch(Path::new(r"C:\demo"), None) else { panic!() };
        assert_eq!(name, "hand-demo-1");
        assert_ne!(new_session.as_deref(), Some(sid.as_str()), "each + gets its own uuid");
        let body: serde_json::Value = serde_json::from_str(&seen.recv_timeout(WAIT).unwrap().2).unwrap();
        assert_eq!(body.get("repo"), None, "not in a repo: no repo");
        assert_eq!(body["claudeSessionId"].as_str(), new_session.as_deref());
        let Launch::Minted { name, .. } = hands.launch(Path::new(r"C:\demo\alpha"), None) else { panic!() };
        assert_eq!(name, "hand-alpha-2", "a second + in the same repo");
    }

    #[test]
    fn hand_script_passes_only_the_path_and_no_strict_flag() {
        let config = PathBuf::from(r"C:\Users\o'neil\Foreman\mcp\workstate-hand-alpha-1.json");
        let launch = Launch::Minted { name: "hand-alpha-1".into(), session: "hand-alpha-1".into(), config, notice: None, new_session: None };
        let script = launch.script();
        let path = "C:\\Users\\o''neil\\Foreman\\mcp\\workstate-hand-alpha-1.json";
        assert_eq!(
            script,
            format!(
                "if (Test-Path -LiteralPath '{path}') {{ try {{ claude --mcp-config '{path}' }} finally {{ Remove-Item -LiteralPath '{path}' -ErrorAction SilentlyContinue }} }} else {{ Write-Host '{STALE}'; claude }}"
            )
        );
        assert!(!script.contains("--strict-mcp-config") && !script.contains("wss_"));
        assert_eq!(launch.shell_args(), ["-NoExit".to_string(), "-EncodedCommand".into(), encoded_command(&script)]);
        assert_eq!(Launch::Plain.script(), "claude");
        // m5/27: a closed session resumed through the same launch keeps its own token
        let resume = launch.script_with("claude --resume 'fc7b0272-bb70-4c4d-bf95-bbf3b3bc81ff'");
        assert!(resume.contains("try { claude --resume 'fc7b0272-bb70-4c4d-bf95-bbf3b3bc81ff' --mcp-config '"), "{resume}");
        assert!(resume.ends_with("; claude --resume 'fc7b0272-bb70-4c4d-bf95-bbf3b3bc81ff' }"), "the stale branch resumes too: {resume}");
        assert_eq!(Launch::Plain.script_with("claude --resume 'x'"), "claude --resume 'x'");
        // m5/40: a fresh + starts with its uuid; the stale branch starts a session of its own
        let fresh = Launch::Minted {
            name: "hand-alpha-1".into(),
            session: "hand-alpha-1".into(),
            config: PathBuf::from(r"C:\fm\mcp\workstate-hand-alpha-1.json"),
            notice: None,
            new_session: Some("aaaa0001-0000-4000-8000-000000000001".into()),
        };
        assert_eq!(
            fresh.script(),
            format!(
                "if (Test-Path -LiteralPath 'C:\\fm\\mcp\\workstate-hand-alpha-1.json') {{ try {{ claude --session-id 'aaaa0001-0000-4000-8000-000000000001' --mcp-config 'C:\\fm\\mcp\\workstate-hand-alpha-1.json' }} finally {{ Remove-Item -LiteralPath 'C:\\fm\\mcp\\workstate-hand-alpha-1.json' -ErrorAction SilentlyContinue }} }} else {{ Write-Host '{STALE}'; claude }}"
            )
        );
        assert_eq!(Launch::Unminted("x".into()).script(), "claude");
        // pwsh's own documented example
        assert_eq!(encoded_command("dir \"c:\\program files\" "), "ZABpAHIAIAAiAGMAOgBcAHAAcgBvAGcAcgBhAG0AIABmAGkAbABlAHMAIgAgAA==");
    }

    #[test]
    fn hand_config_file_is_owner_only() {
        let dir = tempfile::tempdir().unwrap();
        let (url, _seen, _oauth) = fake_workstate(vec![]);
        let (home, auth) = home_with(dir.path(), Some(&url));
        let (mut hands, _) = Hands::with(&home, auth.clone(), HttpMinter::default(), a_repo);
        let Launch::Minted { config, .. } = hands.launch(Path::new(r"C:\demo\alpha"), None) else { panic!() };
        #[cfg(windows)]
        {
            let out = std::process::Command::new("icacls").arg(&config).output().unwrap();
            let acl = String::from_utf8_lossy(&out.stdout);
            let user = std::env::var("USERNAME").unwrap();
            let aces: Vec<&str> = acl.lines().filter(|l| l.contains(":(")).collect();
            assert_eq!(aces.len(), 1, "{acl}");
            assert!(aces[0].contains(&user) && aces[0].contains("(F)") && !aces[0].contains("(I)"), "{acl}");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&config).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn hand_token_is_revoked_on_exit_close_quit_and_restart() {
        let dir = tempfile::tempdir().unwrap();
        let (url, seen, _oauth) = fake_workstate(vec![]);
        let (home, auth) = home_with(dir.path(), Some(&url));
        let (mut hands, _) = Hands::with(&home, auth.clone(), HttpMinter::default(), a_repo);
        let alpha = Path::new(r"C:\demo\alpha");

        // exit: the launch deleted the file
        let Launch::Minted { name, config, .. } = hands.launch(alpha, None) else { panic!() };
        seen.recv_timeout(WAIT).unwrap();
        assert!(!hands.ended(&name));
        std::fs::remove_file(&config).unwrap();
        assert!(hands.ended(&name));
        assert_eq!(hands.release(&name), None);
        let (line, bearer, _) = seen.recv_timeout(WAIT).unwrap();
        assert_eq!(line, "DELETE /api/agent-sessions/h1 HTTP/1.1");
        assert!(oauth(&bearer));
        assert!(!hands.ended(&name), "forgotten");

        // close: the tab went while claude ran
        let Launch::Minted { name, config, .. } = hands.launch(alpha, None) else { panic!() };
        seen.recv_timeout(WAIT).unwrap();
        assert_eq!(hands.release(&name), None);
        assert_eq!(seen.recv_timeout(WAIT).unwrap().0, "DELETE /api/agent-sessions/h2 HTTP/1.1");
        assert!(!config.exists(), "close deletes the file");

        // quit
        let Launch::Minted { config: c3, .. } = hands.launch(alpha, None) else { panic!() };
        let Launch::Minted { config: c4, .. } = hands.launch(Path::new(r"C:\demo"), None) else { panic!() };
        seen.recv_timeout(WAIT).unwrap();
        seen.recv_timeout(WAIT).unwrap();
        assert!(hands.release_all().is_empty());
        let mut deleted = [seen.recv_timeout(WAIT).unwrap().0, seen.recv_timeout(WAIT).unwrap().0];
        deleted.sort();
        assert_eq!(deleted, ["DELETE /api/agent-sessions/h3 HTTP/1.1", "DELETE /api/agent-sessions/h4 HTTP/1.1"]);
        assert!(!c3.exists() && !c4.exists() && hands.names().is_empty());

        // restart: a run that ended without quitting left one; the next start revokes it
        let Launch::Minted { config: c5, .. } = hands.launch(alpha, None) else { panic!() };
        seen.recv_timeout(WAIT).unwrap();
        drop(hands);
        let (mut again, _) = Hands::with(&home, auth.clone(), HttpMinter::default(), a_repo);
        assert_eq!(again.names(), ["hand-alpha-1"]);
        assert!(again.release_all().is_empty());
        assert_eq!(seen.recv_timeout(WAIT).unwrap().0, "DELETE /api/agent-sessions/h5 HTTP/1.1");
        assert!(!c5.exists());
        let (third, _) = Hands::with(&home, auth.clone(), HttpMinter::default(), a_repo);
        assert!(third.names().is_empty(), "the revoke was saved");
    }

    /// Mints at once; each revoke takes `revoke_takes`, and is counted once it is done.
    struct SlowRevokes {
        revoke_takes: std::time::Duration,
        revoked: Arc<std::sync::atomic::AtomicUsize>,
        minted: std::sync::atomic::AtomicUsize,
    }

    impl Minter for SlowRevokes {
        fn mint(&self, _: &str, _: &str, _: &MintRequest) -> Result<foreman_supervisor::Minted, MintError> {
            let n = self.minted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(foreman_supervisor::Minted { session_id: format!("s{n}"), token: format!("wss_slow{n}"), expires_at: None })
        }

        fn bind(&self, _: &str, _: &str, _: &str, _: &str) -> Result<(), MintError> {
            Ok(())
        }

        fn revoke(&self, _: &str, _: &str, _: &str) -> Result<(), MintError> {
            std::thread::sleep(self.revoke_takes);
            self.revoked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
    }

    fn slow_hands(dir: &Path, revoke_takes: std::time::Duration) -> (PathBuf, Arc<Mutex<Hands>>, Arc<std::sync::atomic::AtomicUsize>, Arc<Auth>) {
        let (url, _seen, _oauth) = fake_workstate(vec![]);
        let (home, auth) = home_with(dir, Some(&url));
        let revoked = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let minter = SlowRevokes { revoke_takes, revoked: revoked.clone(), minted: Default::default() };
        let (hands, _) = Hands::with(&home, auth.clone(), minter, a_repo);
        (home, Arc::new(Mutex::new(hands)), revoked, auth)
    }

    /// A-29: quit waits for the revokes at most `within`; an unreachable server cannot hold it N × 15 s.
    #[test]
    fn quit_release_returns_by_its_deadline_and_still_revokes_a_server_that_answers() {
        let dir = tempfile::tempdir().unwrap();
        let (_, hands, revoked, _) = slow_hands(dir.path(), std::time::Duration::from_millis(10));
        for cwd in [r"C:\demo\alpha", r"C:\demo"] {
            assert!(matches!(hands.lock().unwrap().launch(Path::new(cwd), None), Launch::Minted { .. }));
        }
        assert_eq!(release_all_within(hands.clone(), WAIT), Some(Vec::new()));
        assert_eq!(revoked.load(std::sync::atomic::Ordering::SeqCst), 2, "a server that answers gets every revoke");

        let dir = tempfile::tempdir().unwrap();
        let (home, hands, revoked, auth) = slow_hands(dir.path(), std::time::Duration::from_secs(3));
        for cwd in [r"C:\demo\alpha", r"C:\demo", r"C:\x\alpha"] {
            assert!(matches!(hands.lock().unwrap().launch(Path::new(cwd), None), Launch::Minted { .. }));
        }
        let started = std::time::Instant::now();
        assert_eq!(release_all_within(hands, std::time::Duration::from_millis(300)), None);
        assert!(started.elapsed() < std::time::Duration::from_secs(2), "{:?}", started.elapsed());
        assert_eq!(revoked.load(std::sync::atomic::Ordering::SeqCst), 0);
        let (left, _) = Hands::with(&home, auth, HttpMinter::default(), a_repo);
        assert_eq!(left.names().len(), 3, "what quit did not reach stays for the next start");
    }

    /// A-30: a mint whose record cannot be saved says so, and so does a release.
    #[test]
    fn a_hands_json_that_cannot_be_saved_is_a_notice() {
        let dir = tempfile::tempdir().unwrap();
        let (home, hands, _, _) = slow_hands(dir.path(), std::time::Duration::ZERO);
        std::fs::create_dir_all(Hands::path(&home)).unwrap();
        let mut hands = hands.lock().unwrap();
        let Launch::Minted { name, notice, .. } = hands.launch(Path::new(r"C:\demo\alpha"), None) else { panic!() };
        let notice = notice.expect("the failed save is reported");
        assert!(notice.starts_with("Cannot save ") && notice.contains("hands.json"), "{notice}");
        let released = hands.release(&name).expect("the failed save is reported");
        assert!(released.starts_with("Cannot save ") && !released.contains("Cannot revoke"), "{released}");
    }

    #[test]
    fn deleting_a_resumed_session_revokes_only_its_own_token() {
        let dir = tempfile::tempdir().unwrap();
        let (url, seen, _oauth) = fake_workstate(vec![]);
        let (home, auth) = home_with(dir.path(), Some(&url));
        let (mut hands, _) = Hands::with(&home, auth.clone(), HttpMinter::default(), a_repo);
        let alpha = Path::new(r"C:\demo\alpha");
        let sid = "5e1f7e57-0000-4000-8000-00000000c105";
        let Launch::Minted { name, config, .. } = hands.launch_for(alpha, Some("resumed"), Some(sid)) else { panic!() };
        let Launch::Minted { name: other, .. } = hands.launch(alpha, None) else { panic!() };
        seen.recv_timeout(WAIT).unwrap();
        let other_sid = hands.minted.get(&other).unwrap().claude_session_id.clone().unwrap();
        assert_ne!(other_sid, sid);
        assert_eq!(serde_json::from_str::<serde_json::Value>(&seen.recv_timeout(WAIT).unwrap().2).unwrap()["claudeSessionId"], other_sid);
        assert!(hands.release_session("ffffffff-0000-4000-8000-000000000001").is_empty());
        assert!(seen.recv_timeout(std::time::Duration::from_millis(300)).is_err(), "another session's delete revokes nothing");
        assert!(hands.release_session(sid).is_empty());
        assert_eq!(seen.recv_timeout(WAIT).unwrap().0, "DELETE /api/agent-sessions/h1 HTTP/1.1");
        assert!(!config.exists());
        assert_eq!(hands.names(), [other.clone()]);
        assert_ne!(name, other);
        assert!(hands.release_session(sid).is_empty(), "once");
        assert!(seen.recv_timeout(std::time::Duration::from_millis(300)).is_err());
    }

    #[test]
    fn hand_without_workstate_is_plain_and_mints_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (_url, seen, _oauth) = fake_workstate(vec![]);
        let (home, auth) = home_with(dir.path(), None);
        let (mut hands, _) = Hands::with(&home, auth.clone(), HttpMinter::default(), a_repo);
        assert_eq!(hands.launch(Path::new(r"C:\demo\alpha"), None), Launch::Plain);
        assert!(seen.recv_timeout(std::time::Duration::from_millis(200)).is_err(), "no request");
        assert!(!home.join("mcp").exists());
        assert!(hands.release_all().is_empty());
    }

    /// Signed in, but the grant no longer refreshes: plain, saying to sign in again; no mint.
    #[test]
    fn hand_with_a_broken_sign_in_opens_it_plain_and_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let (url, seen, oauth) = fake_workstate(vec![]);
        oauth.set_access_ttl(60);
        let (home, auth) = home_with(dir.path(), Some(&url));
        oauth.fail_refresh(true);
        let (mut hands, _) = Hands::with(&home, auth.clone(), HttpMinter::default(), a_repo);
        let Launch::Unminted(notice) = hands.launch(Path::new(r"C:\demo\alpha"), None) else { panic!() };
        assert!(notice.contains("sign in to WorkState") && notice.contains("again"), "{notice}");
        assert!(seen.recv_timeout(std::time::Duration::from_millis(200)).is_err(), "no mint");
    }

    #[test]
    fn hand_mint_failure_opens_it_plain_with_one_notice() {
        let dir = tempfile::tempdir().unwrap();
        let (url, seen, _oauth) = fake_workstate(vec![(409, r#"{"error":"name held by another launcher"}"#)]);
        let (home, auth) = home_with(dir.path(), Some(&url));
        let (mut hands, _) = Hands::with(&home, auth.clone(), HttpMinter::default(), a_repo);
        let launch = hands.launch(Path::new(r"C:\demo\alpha"), None);
        assert_eq!(
            launch,
            Launch::Unminted(
                "This claude session runs without its own WorkState token: cannot mint an orchestrator one (WorkState answered HTTP 409: name held by another launcher)".into()
            )
        );
        assert_eq!(launch.script(), "claude");
        assert!(oauth(&seen.recv_timeout(WAIT).unwrap().1));
        assert!(hands.names().is_empty());
    }

    /// m5/35: a resume mints an orchestrator under the session's own name, on the same bearer; the
    /// file key stays Foreman's.
    #[test]
    fn hand_resume_mints_an_orchestrator_under_the_sessions_own_name() {
        let dir = tempfile::tempdir().unwrap();
        let (url, seen, _oauth) = fake_workstate(vec![]);
        let (home, auth) = home_with(dir.path(), Some(&url));
        let (mut hands, _) = Hands::with(&home, auth.clone(), HttpMinter::default(), a_repo);
        let alpha = Path::new(r"C:\demo\alpha");

        let sid = "fc7b0272-bb70-4c4d-bf95-bbf3b3bc81ff";
        let Launch::Minted { name, session, config, notice, new_session } = hands.launch_for(alpha, Some("mw-workflow"), Some(sid)) else { panic!() };
        assert_eq!((name.as_str(), session.as_str(), notice), ("hand-alpha-1", "mw-workflow", None));
        assert_eq!(new_session, None, "a resume starts no new session");
        assert_eq!(hands.minted.get(&name).unwrap().claude_session_id.as_deref(), Some(sid));
        assert!(config.ends_with("workstate-hand-alpha-1.json"), "the file keeps Foreman's key: {config:?}");
        let (_, bearer, body) = seen.recv_timeout(WAIT).unwrap();
        assert!(oauth(&bearer));
        let body: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!((body["name"].as_str(), body["role"].as_str()), (Some("mw-workflow"), Some("orchestrator")));
        assert_eq!((body["launchedBy"].as_str(), body["externalRole"].as_str()), (Some("foreman/hand"), Some("hand")));
        assert_eq!(body["claudeSessionId"].as_str(), Some(sid), "the resumed uuid (m5/40)");

        // never a short id: claude --resume <8 hex> starts a new session
        let Launch::Minted { name: short, new_session, .. } = hands.launch_for(alpha, Some("other"), Some("fc7b0272")) else { panic!() };
        assert_eq!(new_session, None);
        let body: serde_json::Value = serde_json::from_str(&seen.recv_timeout(WAIT).unwrap().2).unwrap();
        assert_eq!(body.get("claudeSessionId"), None, "{body}");
        assert_eq!(hands.release(&short), None);
        seen.recv_timeout(WAIT).unwrap();

        // the same session in a second tab would take the first one's token: it gets the generated name
        let Launch::Minted { session, notice, .. } = hands.launch(alpha, Some("mw-workflow")) else { panic!() };
        assert_eq!(session, "hand-alpha-2");
        assert!(notice.is_some_and(|n| n.contains("another tab holds that name")));
        let body: serde_json::Value = serde_json::from_str(&seen.recv_timeout(WAIT).unwrap().2).unwrap();
        assert_eq!(body["name"].as_str(), Some("hand-alpha-2"));

        // a title is cut to one line of 64 characters; a blank one is no name
        let Launch::Minted { session, .. } = hands.launch(alpha, Some(&format!("  Fix the\tdeploy\n{}", "x".repeat(80)))) else { panic!() };
        assert_eq!(session, format!("Fix the deploy {}", "x".repeat(49)));
        seen.recv_timeout(WAIT).unwrap();
        let Launch::Minted { session, .. } = hands.launch(alpha, Some(" \u{200B} ")) else { panic!() };
        assert_eq!(session, "hand-alpha-4");
        seen.recv_timeout(WAIT).unwrap();

        // released, the name is free again
        assert!(hands.release_all().is_empty());
        let Launch::Minted { session, .. } = hands.launch(alpha, Some("mw-workflow")) else { panic!() };
        assert_eq!(session, "mw-workflow");
    }

    #[test]
    fn hand_name_held_by_another_launcher_falls_back_to_the_generated_one() {
        let dir = tempfile::tempdir().unwrap();
        let (url, seen, _oauth) = fake_workstate(vec![(409, r#"{"error":"the session name \"nomos-0c\" is held by a session launched by no launcher"}"#)]);
        let (home, auth) = home_with(dir.path(), Some(&url));
        let (mut hands, _) = Hands::with(&home, auth.clone(), HttpMinter::default(), a_repo);
        let Launch::Minted { name, session, notice, .. } = hands.launch(Path::new(r"C:\demo\alpha"), Some("nomos-0c")) else { panic!() };
        assert_eq!((name.as_str(), session.as_str()), ("hand-alpha-1", "hand-alpha-1"));
        assert!(notice.unwrap().starts_with("WorkState session hand-alpha-1, not nomos-0c: "));
        let names: Vec<String> = (0..2)
            .map(|_| serde_json::from_str::<serde_json::Value>(&seen.recv_timeout(WAIT).unwrap().2).unwrap()["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(names, ["nomos-0c", "hand-alpha-1"]);
    }

    /// A bearer whose role cannot mint an orchestrator: plain, with the reason, and no worker mint.
    #[test]
    fn hand_refused_orchestrator_is_not_minted_as_a_worker() {
        let dir = tempfile::tempdir().unwrap();
        let (url, seen, _oauth) = fake_workstate(vec![(403, r#"{"error":"a worker cannot mint a orchestrator token; a role is never raised"}"#)]);
        let (home, auth) = home_with(dir.path(), Some(&url));
        let (mut hands, _) = Hands::with(&home, auth.clone(), HttpMinter::default(), a_repo);
        let Launch::Unminted(notice) = hands.launch(Path::new(r"C:\demo\alpha"), Some("mw-workflow")) else { panic!() };
        assert!(notice.contains("cannot mint an orchestrator one (WorkState answered HTTP 403"), "{notice}");
        let body: serde_json::Value = serde_json::from_str(&seen.recv_timeout(WAIT).unwrap().2).unwrap();
        assert_eq!(body["role"].as_str(), Some("orchestrator"));
        assert!(seen.recv_timeout(std::time::Duration::from_millis(300)).is_err(), "no second mint");
        assert!(hands.names().is_empty());
    }

    /// The tab's pwsh as Foreman starts it, with a stand-in `claude.cmd` first on PATH that fails: it
    /// gets only the path, the file is gone after it, and the terminal shows none of the script.
    #[cfg(windows)]
    #[test]
    fn hand_launch_deletes_the_config_when_claude_exits_and_shows_no_script() {
        let Ok(probe) = std::process::Command::new("pwsh").args(["-NoProfile", "-Command", "exit 0"]).output() else {
            return;
        };
        assert!(probe.status.success());
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("workstate-hand-alpha-1.json");
        std::fs::write(&config, "{}").unwrap();
        let seen = dir.path().join("argv.txt");
        let fake = format!("@echo %*> \"{}\"\r\n@echo fake claude ran\r\n@exit /b 3\r\n", seen.display());
        std::fs::write(dir.path().join("claude.cmd"), fake).unwrap();
        let path = format!("{};{}", dir.path().display(), std::env::var("PATH").unwrap_or_default());
        let launch = Launch::Minted { name: "hand-alpha-1".into(), session: "hand-alpha-1".into(), config: config.clone(), notice: None, new_session: None };
        // -NoExit's prompt reads stdin; a closed one ends it like a human typing `exit`.
        let out = std::process::Command::new("pwsh")
            .args(["-NoLogo", "-NoProfile"])
            .args(launch.shell_args())
            .env("PATH", path)
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap();
        assert_eq!(std::fs::read_to_string(&seen).unwrap().trim(), format!("--mcp-config {}", config.display()));
        assert!(!config.exists(), "deleted even though claude failed");
        let shown = String::from_utf8_lossy(&out.stdout);
        assert!(shown.contains("fake claude ran"), "{out:?}");
        assert!(!shown.contains("Test-Path") && !shown.contains("mcp-config"), "the script is not shown: {shown}");
    }

    /// The launch run again after claude exited (m5/18 c5): no `--mcp-config` to the deleted file,
    /// plain claude after the notice.
    #[test]
    fn hand_launch_runs_plain_claude_once_the_config_is_gone() {
        let Ok(probe) = std::process::Command::new("pwsh").args(["-NoProfile", "-Command", "exit 0"]).output() else {
            return;
        };
        assert!(probe.status.success());
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("workstate-hand-alpha-1.json");
        std::fs::write(&config, "{}").unwrap();
        let seen = dir.path().join("argv.txt");
        let line = Launch::Minted { name: "hand-alpha-1".into(), session: "hand-alpha-1".into(), config: config.clone(), notice: None, new_session: None }.script();
        let script = format!(
            "function claude {{ Add-Content -LiteralPath '{}' -Value ('[' + ($args -join ' ') + ']') }}; {line}; {line}",
            seen.display(),
        );
        let out = std::process::Command::new("pwsh").args(["-NoProfile", "-Command", &script]).output().unwrap();
        let runs: Vec<String> = std::fs::read_to_string(&seen).unwrap().lines().map(str::to_string).collect();
        assert_eq!(runs, [format!("[--mcp-config {}]", config.display()), "[]".to_string()]);
        assert!(String::from_utf8_lossy(&out.stdout).contains("WorkState token was revoked"), "{out:?}");
        assert!(!config.exists());
    }
}
