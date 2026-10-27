//! Hand sessions (m5/20): the `+` on a repo row types `claude` into a shell. With WorkState set up
//! that claude gets its own minted `wss_` token like a supervised session ([`foreman_supervisor::mint`]):
//! role worker, `externalRole` `hand`, `repo` of its cwd, in an owner-only file passed as
//! `--mcp-config <file>` WITHOUT `--strict-mcp-config`, so the human keeps every other MCP server
//! (LEARNINGS: `--mcp-config` beats the user-scope `workstate` registration). The tab's pwsh runs the
//! launch as its `-EncodedCommand` (m5/22): nothing is typed or echoed, and only the path is in it.
//!
//! The token is revoked when that claude exits (the launch deletes its file, [`Hands::ended`]),
//! when its tab closes, on quit, and on the next start for what an earlier run left
//! ([`Hands::release_all`]). The mint's bearer is the repo role's static token, else the workspace's:
//! a worker token cannot mint.

use std::path::{Path, PathBuf};

use foreman_core::Settings;
use foreman_supervisor::{HttpMinter, MintRepo, MintRequest, MintedSession, MintedSessions, Minter, RoleToken, api_base, git_repo};

use crate::tokens::token_from;

pub const EXTERNAL_ROLE: &str = "hand";
const LAUNCHED_BY: &str = "foreman/hand";
/// What the launch prints before it starts plain `claude` when its file is already gone, quoted for a
/// pwsh '…' string.
const STALE: &str = "This tab''s WorkState token was revoked when its claude exited; starting plain claude. Open a new + session for a fresh token.";

/// How the `+` session starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Launch {
    /// No WorkState: plain `claude`, as before m5/20.
    Plain,
    /// Its own token in `config`.
    Minted { name: String, config: PathBuf },
    /// WorkState is set up but the mint failed: plain `claude`, and this notice.
    Unminted(String),
}

impl Launch {
    /// What the tab's pwsh runs before its first prompt. It deletes the config file once claude exits,
    /// however it exits, which is how [`Hands::ended`] sees it. Should the file be gone already (the
    /// tab closed while it started), it starts plain `claude`, saying why (m5/18).
    pub fn script(&self) -> String {
        match self {
            Launch::Minted { config, .. } => {
                let path = config.to_string_lossy().replace('\'', "''");
                format!(
                    "if (Test-Path -LiteralPath '{path}') {{ try {{ claude --mcp-config '{path}' }} finally {{ Remove-Item -LiteralPath '{path}' -ErrorAction SilentlyContinue }} }} else {{ Write-Host '{STALE}'; claude }}"
                )
            }
            Launch::Plain | Launch::Unminted(_) => "claude".into(),
        }
    }

    /// Appended to the tab's pwsh flags: [`Self::script`] runs unseen, and the shell stays when it ends.
    pub fn shell_args(&self) -> [String; 3] {
        ["-NoExit".into(), "-EncodedCommand".into(), encoded_command(&self.script())]
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
    minter: Box<dyn Minter + Send>,
    repo_of: fn(&Path) -> Option<MintRepo>,
    minted: MintedSessions,
}

impl Hands {
    pub fn path(home: &Path) -> PathBuf {
        home.join("supervisor").join("hands.json")
    }

    /// Over HTTP, with the repo from git. An unreadable file is forgotten, with a notice.
    pub fn open(home: &Path) -> (Self, Option<String>) {
        Self::with(home, HttpMinter::default(), git_repo)
    }

    pub fn with(home: &Path, minter: impl Minter + Send + 'static, repo_of: fn(&Path) -> Option<MintRepo>) -> (Self, Option<String>) {
        let path = Self::path(home);
        let (minted, notice) = match MintedSessions::load(Some(path.clone())) {
            Ok(m) => (m, None),
            Err(e) => (
                MintedSessions::empty(Some(path.clone())),
                Some(format!("Cannot read {} ({e}): hand-session tokens minted before lapse after their idle window", path.display())),
            ),
        };
        let hands = Self {
            home: home.to_path_buf(),
            minter: Box::new(minter),
            repo_of,
            minted,
        };
        (hands, notice)
    }

    fn mcp_dir(&self) -> PathBuf {
        self.home.join("mcp")
    }

    fn config_path(&self, name: &str) -> PathBuf {
        self.mcp_dir().join(format!("workstate-{name}.json"))
    }

    /// The bearer of the mint and the revoke: (Foreman role, its static token).
    fn bearer(&self, role: Option<&str>) -> Option<(String, RoleToken)> {
        let settings = Settings::load(&self.home).ok()?;
        let roles: &[&str] = match role {
            Some(r) => &[r][..],
            None => &["repo", "workspace"],
        };
        roles.iter().find_map(|r| token_from(&settings, r).map(|t| (r.to_string(), t)))
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

    /// Mints the token of a new `+` session in `cwd` and writes its config file.
    pub fn launch(&mut self, cwd: &Path) -> Launch {
        let Some((role, bearer)) = self.bearer(None) else {
            return Launch::Plain;
        };
        let name = self.next_name(cwd);
        let folder = cwd.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
        let request = MintRequest {
            name: name.clone(),
            role: "worker".into(),
            launched_by: LAUNCHED_BY.into(),
            origin: format!("{LAUNCHED_BY}/{folder}"),
            cwd: cwd.to_string_lossy().into_owned(),
            external_role: EXTERNAL_ROLE.into(),
            replace: true,
            repo: (self.repo_of)(cwd),
        };
        let base = api_base(&bearer.url);
        let minted = match self.minter.mint(&base, &bearer.token, &request) {
            Ok(m) => m,
            Err(e) => return Launch::Unminted(format!("This claude session runs without its own WorkState token: cannot mint one ({e})")),
        };
        self.minted.insert(
            &name,
            MintedSession {
                session_id: minted.session_id.clone(),
                role,
                claude_session_id: None,
                last_active: None,
            },
        );
        let _ = self.minted.save();
        match foreman_claude::write_mcp_config(&self.mcp_dir(), &name, &bearer.url, &minted.token, &[]) {
            Ok(config) => Launch::Minted { name, config },
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
        self.minted.get(name).is_some() && !self.config_path(name).exists()
    }

    /// Revokes `name`'s token and deletes its config file. `Some` notice when the revoke failed.
    pub fn release(&mut self, name: &str) -> Option<String> {
        let m = self.minted.remove(name)?;
        let _ = std::fs::remove_file(self.config_path(name));
        let result = match self.bearer(Some(&m.role)) {
            Some((_, t)) => self.minter.revoke(&api_base(&t.url), &t.token, &m.session_id).map_err(|e| e.to_string()),
            None => Err(format!("no {} token to revoke it with", m.role)),
        };
        let _ = self.minted.save();
        result
            .err()
            .map(|e| format!("Cannot revoke the WorkState token of {name} (session {}): {e}; unused, it lapses after its idle window", m.session_id))
    }

    /// Every hand token: on quit, and on start for those an earlier run left.
    pub fn release_all(&mut self) -> Vec<String> {
        let names: Vec<String> = self.minted.names().cloned().collect();
        names.iter().filter_map(|n| self.release(n)).collect()
    }

    #[cfg(test)]
    fn names(&self) -> Vec<String> {
        self.minted.names().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::sync::mpsc;

    use super::*;

    /// A request the fake server should have seen: fail, never hang.
    const WAIT: std::time::Duration = std::time::Duration::from_secs(10);

    /// A WorkState stand-in on 127.0.0.1: answers POST with each of `mints` in turn (then 201s),
    /// DELETE with 204, and sends back (request line, Authorization, body).
    fn fake_workstate(mints: Vec<(u16, &'static str)>) -> (String, mpsc::Receiver<(String, String, String)>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut mints = mints.into_iter();
            let mut n = 0;
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let (mut line, mut auth, mut len) = (String::new(), String::new(), 0);
                reader.read_line(&mut line).unwrap();
                loop {
                    let mut h = String::new();
                    reader.read_line(&mut h).unwrap();
                    let Some((k, v)) = h.trim_end().split_once(':') else { break };
                    match k.to_ascii_lowercase().as_str() {
                        "authorization" => auth = v.trim().to_string(),
                        "content-length" => len = v.trim().parse().unwrap(),
                        _ => {}
                    }
                }
                let mut body = vec![0; len];
                reader.read_exact(&mut body).unwrap();
                let (status, answer) = if line.starts_with("POST") {
                    n += 1;
                    mints.next().map(|(s, b)| (s, b.to_string())).unwrap_or((201, format!(r#"{{"sessionId":"h{n}","token":"wss_fake{n}"}}"#)))
                } else {
                    (204, String::new())
                };
                write!(stream, "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}", answer.len()).unwrap();
                if tx.send((line.trim_end().to_string(), auth, String::from_utf8(body).unwrap())).is_err() {
                    break;
                }
            }
        });
        (url, rx)
    }

    fn a_repo(cwd: &Path) -> Option<MintRepo> {
        cwd.ends_with("alpha").then(|| MintRepo {
            root: "C:/demo/alpha".into(),
            remote: Some("https://example.invalid/demo/alpha.git".into()),
        })
    }

    /// A Foreman home whose settings name a fake `.env` and `url`.
    fn home_with(dir: &Path, url: Option<&str>, env: &str) -> PathBuf {
        let home = dir.join("home");
        let mut settings = Settings::default();
        if let Some(url) = url {
            let file = dir.join(".env");
            std::fs::write(&file, env).unwrap();
            settings.workstate_env = Some(file);
            settings.workstate_url = Some(url.into());
        }
        settings.save(&home).unwrap();
        home
    }

    const ENV: &str = "WORKSTATE_ORCHESTRATOR_TOKEN=fake-static-orch\nWORKSTATE_CONTROLLER_TOKEN=fake-static-ctl\n";

    #[test]
    fn hand_mint_carries_the_repo_and_none_outside_a_repo() {
        let dir = tempfile::tempdir().unwrap();
        let (url, seen) = fake_workstate(vec![]);
        let home = home_with(dir.path(), Some(&url), ENV);
        let (mut hands, notice) = Hands::with(&home, HttpMinter::default(), a_repo);
        assert_eq!(notice, None);

        let Launch::Minted { name, config } = hands.launch(Path::new(r"C:\demo\alpha")) else { panic!() };
        assert_eq!(name, "hand-alpha-1");
        let (line, auth, body) = seen.recv_timeout(WAIT).unwrap();
        assert_eq!(line, "POST /api/agent-sessions HTTP/1.1");
        assert_eq!(auth, "Bearer fake-static-orch", "the repo role's static token is the bearer");
        let body: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(
            body,
            serde_json::json!({
                "name": "hand-alpha-1", "role": "worker", "launchedBy": "foreman/hand", "origin": "foreman/hand/alpha",
                "cwd": r"C:\demo\alpha", "externalRole": "hand", "replace": true,
                "repo": { "root": "C:/demo/alpha", "remote": "https://example.invalid/demo/alpha.git" }
            })
        );
        let text = std::fs::read_to_string(&config).unwrap();
        assert!(text.contains("Bearer wss_fake1") && text.contains(&url), "the minted token, not the static one");
        assert!(!text.contains("fake-static"));

        let Launch::Minted { name, .. } = hands.launch(Path::new(r"C:\demo")) else { panic!() };
        assert_eq!(name, "hand-demo-1");
        let body: serde_json::Value = serde_json::from_str(&seen.recv_timeout(WAIT).unwrap().2).unwrap();
        assert_eq!(body.get("repo"), None, "not in a repo: no repo");
        let Launch::Minted { name, .. } = hands.launch(Path::new(r"C:\demo\alpha")) else { panic!() };
        assert_eq!(name, "hand-alpha-2", "a second + in the same repo");
    }

    #[test]
    fn hand_script_passes_only_the_path_and_no_strict_flag() {
        let config = PathBuf::from(r"C:\Users\o'neil\Foreman\mcp\workstate-hand-alpha-1.json");
        let launch = Launch::Minted { name: "hand-alpha-1".into(), config };
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
        assert_eq!(Launch::Unminted("x".into()).script(), "claude");
        // pwsh's own documented example
        assert_eq!(encoded_command("dir \"c:\\program files\" "), "ZABpAHIAIAAiAGMAOgBcAHAAcgBvAGcAcgBhAG0AIABmAGkAbABlAHMAIgAgAA==");
    }

    #[test]
    fn hand_config_file_is_owner_only() {
        let dir = tempfile::tempdir().unwrap();
        let (url, _seen) = fake_workstate(vec![]);
        let home = home_with(dir.path(), Some(&url), ENV);
        let (mut hands, _) = Hands::with(&home, HttpMinter::default(), a_repo);
        let Launch::Minted { config, .. } = hands.launch(Path::new(r"C:\demo\alpha")) else { panic!() };
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
        let (url, seen) = fake_workstate(vec![]);
        let home = home_with(dir.path(), Some(&url), ENV);
        let (mut hands, _) = Hands::with(&home, HttpMinter::default(), a_repo);
        let alpha = Path::new(r"C:\demo\alpha");

        // exit: the launch deleted the file
        let Launch::Minted { name, config } = hands.launch(alpha) else { panic!() };
        seen.recv_timeout(WAIT).unwrap();
        assert!(!hands.ended(&name));
        std::fs::remove_file(&config).unwrap();
        assert!(hands.ended(&name));
        assert_eq!(hands.release(&name), None);
        let (line, auth, _) = seen.recv_timeout(WAIT).unwrap();
        assert_eq!((line.as_str(), auth.as_str()), ("DELETE /api/agent-sessions/h1 HTTP/1.1", "Bearer fake-static-orch"));
        assert!(!hands.ended(&name), "forgotten");

        // close: the tab went while claude ran
        let Launch::Minted { name, config } = hands.launch(alpha) else { panic!() };
        seen.recv_timeout(WAIT).unwrap();
        assert_eq!(hands.release(&name), None);
        assert_eq!(seen.recv_timeout(WAIT).unwrap().0, "DELETE /api/agent-sessions/h2 HTTP/1.1");
        assert!(!config.exists(), "close deletes the file");

        // quit
        let Launch::Minted { config: c3, .. } = hands.launch(alpha) else { panic!() };
        let Launch::Minted { config: c4, .. } = hands.launch(Path::new(r"C:\demo")) else { panic!() };
        seen.recv_timeout(WAIT).unwrap();
        seen.recv_timeout(WAIT).unwrap();
        assert!(hands.release_all().is_empty());
        let mut deleted = [seen.recv_timeout(WAIT).unwrap().0, seen.recv_timeout(WAIT).unwrap().0];
        deleted.sort();
        assert_eq!(deleted, ["DELETE /api/agent-sessions/h3 HTTP/1.1", "DELETE /api/agent-sessions/h4 HTTP/1.1"]);
        assert!(!c3.exists() && !c4.exists() && hands.names().is_empty());

        // restart: a run that ended without quitting left one; the next start revokes it
        let Launch::Minted { config: c5, .. } = hands.launch(alpha) else { panic!() };
        seen.recv_timeout(WAIT).unwrap();
        drop(hands);
        let (mut again, _) = Hands::with(&home, HttpMinter::default(), a_repo);
        assert_eq!(again.names(), ["hand-alpha-1"]);
        assert!(again.release_all().is_empty());
        assert_eq!(seen.recv_timeout(WAIT).unwrap().0, "DELETE /api/agent-sessions/h5 HTTP/1.1");
        assert!(!c5.exists());
        let (third, _) = Hands::with(&home, HttpMinter::default(), a_repo);
        assert!(third.names().is_empty(), "the revoke was saved");
    }

    #[test]
    fn hand_without_workstate_is_plain_and_mints_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (_url, seen) = fake_workstate(vec![]);
        let home = home_with(dir.path(), None, "");
        let (mut hands, _) = Hands::with(&home, HttpMinter::default(), a_repo);
        assert_eq!(hands.launch(Path::new(r"C:\demo\alpha")), Launch::Plain);
        assert!(seen.recv_timeout(std::time::Duration::from_millis(200)).is_err(), "no request");
        assert!(!home.join("mcp").exists());
        assert!(hands.release_all().is_empty());
        // a .env with only a worker line cannot mint either
        let home = home_with(dir.path(), Some("http://127.0.0.1:9/mcp"), "WORKSTATE_WORKER_TOKEN=fake-worker\n");
        let (mut hands, _) = Hands::with(&home, HttpMinter::default(), a_repo);
        assert_eq!(hands.launch(Path::new(r"C:\demo\alpha")), Launch::Plain);
    }

    #[test]
    fn hand_mint_failure_opens_it_plain_with_one_notice() {
        let dir = tempfile::tempdir().unwrap();
        let (url, seen) = fake_workstate(vec![(409, r#"{"error":"name held by another launcher"}"#)]);
        let home = home_with(dir.path(), Some(&url), "WORKSTATE_CONTROLLER_TOKEN=fake-static-ctl\n");
        let (mut hands, _) = Hands::with(&home, HttpMinter::default(), a_repo);
        let launch = hands.launch(Path::new(r"C:\demo\alpha"));
        assert_eq!(
            launch,
            Launch::Unminted(
                "This claude session runs without its own WorkState token: cannot mint one (WorkState answered HTTP 409: name held by another launcher)".into()
            )
        );
        assert_eq!(launch.script(), "claude");
        assert_eq!(seen.recv_timeout(WAIT).unwrap().1, "Bearer fake-static-ctl", "the workspace token when there is no repo one");
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
        let launch = Launch::Minted { name: "hand-alpha-1".into(), config: config.clone() };
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
        let line = Launch::Minted { name: "hand-alpha-1".into(), config: config.clone() }.script();
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
