//! `--mcp-config` without `--strict-mcp-config` deliberately overrides the user-scope registration named `supraman_claude::MCP_SERVER` (LEARNINGS mint/7).
//! A minted token's name is its credential's and cannot be renamed; see Skills `docs/workstate/11-sessions.md`, resolution order (a).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use supraman_core::home::home_env;
use supraman_core::paths::{path_key, within};
use supraman_core::{Workspace, owners};
use supraman_supervisor::{HAND_ROLE, HttpMinter, LaunchConfig, MintError, MintRepo, MintRequest, MintedSession, MintedSessions, Minter, api_base, git_repo};
use supraman_workman::Auth;

pub const EXTERNAL_ROLE: &str = "hand";
pub const ROLE: &str = "orchestrator";
const MAX_NAME: usize = 64;
const LAUNCHED_BY: &str = "foreman/hand";
/// Quoted for a pwsh '…' string.
const STALE: &str = "This tab''s Workman token was revoked when its claude exited; starting plain claude. Open a new + session for a fresh token.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Launch {
    Plain,
    /// `name` keys the file and `hands.json`; `session` is the Workman session's name.
    Minted { name: String, session: String, config: PathBuf, notice: Option<String>, new_session: Option<String> },
    Unminted(String),
}

impl Launch {
    #[cfg(test)]
    pub fn script(&self) -> String {
        self.script_with("claude", &Extras::default())
    }

    /// It deletes the config file once claude exits, however it exits, which is how [`Hands::ended`] sees it.
    pub fn script_with(&self, claude: &str, extras: &Extras) -> String {
        let mut flags: Vec<String> = extras.args.iter().map(|a| quote(a)).collect();
        let mut on_path = extras.path.as_ref().map(|d| format!("$env:Path = {} + $env:Path; ", quote(&format!("{};", d.display())))).unwrap_or_default();
        if let Some(file) = &extras.prompt {
            let file = quote(&file.to_string_lossy());
            on_path = format!("$supramanPrompt = [IO.File]::ReadAllText({file}); Remove-Item -LiteralPath {file} -ErrorAction SilentlyContinue; {on_path}");
            flags.extend(["'--'".into(), "$supramanPrompt".into()]);
        }
        let run = |head: Vec<String>| {
            let argv: Vec<String> = head.into_iter().chain(flags.iter().cloned()).collect();
            if argv.is_empty() { claude.to_string() } else { format!("supramanClaude @({})", argv.join(", ")) }
        };
        let script = match self {
            Launch::Minted { config, new_session, .. } => {
                let path = quote(&config.to_string_lossy());
                let mut head: Vec<String> = new_session.iter().flat_map(|id| ["'--session-id'".into(), quote(id)]).collect();
                head.extend(["'--mcp-config'".into(), path.clone()]);
                format!(
                    "{on_path}if (Test-Path -LiteralPath {path}) {{ try {{ {} }} finally {{ Remove-Item -LiteralPath {path} -ErrorAction SilentlyContinue }} }} else {{ Write-Host '{STALE}'; {} }}",
                    run(head),
                    run(Vec::new())
                )
            }
            Launch::Plain | Launch::Unminted(_) => format!("{on_path}{}", run(Vec::new())),
        };
        let passes_args = !flags.is_empty() || matches!(self, Launch::Minted { .. });
        if passes_args { format!("{}{script}", runner(claude)) } else { script }
    }

    #[cfg(test)]
    pub fn shell_args(&self) -> [String; 3] {
        self.shell_args_with("claude", &Extras::default())
    }

    pub fn shell_args_with(&self, claude: &str, extras: &Extras) -> [String; 3] {
        ["-NoExit".into(), "-EncodedCommand".into(), encoded_command(&self.script_with(claude, extras))]
    }
}

fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Under legacy arg passing (5.1, pwsh < 7.3) a native claude is started with its args as one MSVC-quoted command
/// line of the process's own, so the line never sits in claude's environment.
const RUNNER: &str = r#"function supramanClaude($supramanArgv) {
    $c = Get-Command -Name <command> -ErrorAction SilentlyContinue | Select-Object -First 1
    if ("$PSNativeCommandArgumentPassing" -in '', 'Legacy' -and $c.CommandType -eq 'Application' -and $c.Extension -notin '.cmd', '.bat') {
        $line = @(@(& { $args }<args>) + $supramanArgv | ForEach-Object { if ($_ -ne '' -and $_ -notmatch '[\s"]') { $_ } else { '"' + ($_ -replace '(\\*)"', '$1$1\"' -replace '(\\+)$', '$1$1') + '"' } }) -join ' '
        $p = [Diagnostics.Process]::Start((New-Object Diagnostics.ProcessStartInfo -Property @{ FileName = $c.Path; Arguments = $line; UseShellExecute = $false; WorkingDirectory = (Get-Location -PSProvider FileSystem).ProviderPath }))
        $p.WaitForExit(); $global:LASTEXITCODE = $p.ExitCode
    } else { <claude> @supramanArgv }
}
"#;

fn runner(claude: &str) -> String {
    let (word, args) = command_split(claude);
    RUNNER.replace("<command>", word).replace("<args>", args).replace("<claude>", claude)
}

/// The path of `& '<path>' …`, still quoted, else the first word; then the args after it, as PowerShell source.
fn command_split(claude: &str) -> (&str, &str) {
    let claude = claude.trim_start();
    if let Some(rest) = claude.strip_prefix("& '") {
        let mut chars = rest.char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            if c == '\'' {
                if chars.peek().is_some_and(|(_, n)| *n == '\'') {
                    chars.next();
                } else {
                    return (&claude[2..3 + i + 1], &claude[3 + i + 1..]);
                }
            }
        }
    }
    let end = claude.find(char::is_whitespace).unwrap_or(claude.len());
    (&claude[..end], &claude[end..])
}

/// `env` goes on the PTY, not into `--settings`: LEARNINGS sched2/1.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Extras {
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub path: Option<PathBuf>,
    /// A UTF-8 file holding claude's first prompt; the script reads it into a variable and deletes it, so the text
    /// reaches claude as one argument, never through the script's own quoting (LEARNINGS sched2/6).
    pub prompt: Option<PathBuf>,
}

/// `name` is the session's `SUPRAMAN_SESSION`; a cwd outside every workspace gets the home and no role.
pub fn extras(config: &LaunchConfig, workspaces: &[Workspace], cwd: &Path, name: &str, sm_dir: Option<&Path>) -> Extras {
    let owner = owners(workspaces)
        .get(&path_key(cwd))
        .and_then(|n| workspaces.iter().find(|w| &w.name == n))
        .or_else(|| workspaces.iter().find(|w| w.contains(cwd)));
    let env = match (owner, &config.supraman_home) {
        (Some(w), _) => {
            let repo = w.folders.iter().find(|f| within(cwd, &f.path)).map(|f| f.name.as_str());
            config.session_env(&w.name, name, HAND_ROLE, repo, &[])
        }
        (None, Some(home)) => home_env(home).map(|(k, v)| (k.to_string(), v)).into(),
        (None, None) => Vec::new(),
    };
    Extras { args: config.interactive_args(), env, path: sm_dir.map(Path::to_path_buf), prompt: None }
}

/// `hand-<folder>`: the session name of a [+] session without a token of its own.
pub fn plain_name(cwd: &Path) -> String {
    format!("hand-{}", folder_slug(cwd))
}

fn folder_slug(cwd: &Path) -> String {
    let folder = supraman_core::slugify(&cwd.file_name().map(|f| f.to_string_lossy()).unwrap_or_default());
    if folder.is_empty() { "session".to_string() } else { folder }
}

pub fn encoded_command(script: &str) -> String {
    use base64::Engine as _;
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

pub struct Hands {
    home: PathBuf,
    auth: Arc<Auth>,
    minter: Box<dyn Minter + Send>,
    repo_of: fn(&Path) -> Option<MintRepo>,
    minted: MintedSessions,
    sessions: std::collections::HashMap<String, String>,
    /// Servers of released tokens that were not revoked here: their grants go ([`Self::take_left`]).
    left: Vec<String>,
}

pub fn restart_notice(n: usize, to: &str) -> Option<String> {
    (n > 0).then(|| {
        let (tabs, them) = if n == 1 { ("tab", "it") } else { ("tabs", "them") };
        format!("{n} open claude {tabs} still use another Workman server: restart {them} to use {to}.")
    })
}

/// It goes on revoking after `within` until the process ends; what it does not reach stays in `hands.json`.
pub fn release_all_within(hands: Arc<Mutex<Hands>>, within: Duration) -> Option<Vec<String>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || tx.send(hands.lock().unwrap_or_else(|e| e.into_inner()).release_all()));
    rx.recv_timeout(within).ok()
}

pub fn session_name(raw: &str) -> Option<String> {
    let line = supraman_ui::tabs::sanitize(raw)?;
    let cut: String = line.chars().take(MAX_NAME).collect();
    let cut = cut.trim_end().to_string();
    (!cut.is_empty()).then_some(cut)
}

impl Hands {
    pub fn path(home: &Path) -> PathBuf {
        home.join("supervisor").join("hands.json")
    }

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
        supraman_claude::mcp_config_path(&self.mcp_dir(), name)
    }

    fn save(&self) -> Option<String> {
        let path = Self::path(&self.home);
        self.minted.save().err().map(|e| format!("Cannot save {} ({e}): a hand token minted here is not revoked if Supraman ends without Quit", path.display()))
    }

    fn bearer(&self) -> Result<Option<(String, String)>, String> {
        supraman_supervisor::bearer(&self.auth)
    }

    fn next_name(&self, cwd: &Path) -> String {
        let folder = folder_slug(cwd);
        (1..).map(|n| format!("hand-{folder}-{n}")).find(|n| self.minted.get(n).is_none()).unwrap()
    }

    #[cfg(test)]
    pub fn launch(&mut self, cwd: &Path, own: Option<&str>) -> Launch {
        self.launch_for(cwd, own, None)
    }

    /// `claude_session` is its full uuid; anything else is not sent.
    pub fn launch_for(&mut self, cwd: &Path, own: Option<&str>, claude_session: Option<&str>) -> Launch {
        let (url, bearer) = match self.bearer() {
            Ok(Some(b)) => b,
            Ok(None) => return Launch::Plain,
            Err(e) => return Launch::Unminted(format!("This claude session runs without its own Workman token: {e}")),
        };
        let new_session = claude_session.is_none().then(supraman_claude::new_session_id);
        let claude_session = claude_session
            .filter(|s| supraman_claude::is_session_id(s))
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
            notice = Some(format!("Workman session {name}, not {}: another tab holds that name", request.name));
            request.name = name.clone();
        }
        let minted = match self.minter.mint(&base, &bearer, &request) {
            // `replace` covers a live token, so a 409 is another launcher's name (doc 14 §2)
            Err(MintError::Refused { status: 409, reason }) if request.name != name => {
                notice = Some(format!("Workman session {name}, not {}: {reason}", request.name));
                request.name = name.clone();
                self.minter.mint(&base, &bearer, &request)
            }
            other => other,
        };
        let minted = match minted {
            Ok(m) => m,
            Err(e) => return Launch::Unminted(format!("This claude session runs without its own Workman token: cannot mint an {ROLE} one ({e})")),
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
        match supraman_claude::write_mcp_config(&self.mcp_dir(), &name, &url, &minted.token, &[]) {
            Ok(config) => Launch::Minted { name, session: request.name, config, notice, new_session },
            Err(e) => {
                let revoked = self.release(&name);
                Launch::Unminted(format!(
                    "This claude session runs without its own Workman token: cannot write its mcp config ({e}){}",
                    revoked.map(|r| format!("; {r}")).unwrap_or_default()
                ))
            }
        }
    }

    pub fn ended(&self, name: &str) -> bool {
        self.minted.get(name).is_some() && self.config_path(name).is_ok_and(|p| !p.exists())
    }

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
            Ok(None) => Err("not signed in to Workman any more".into()),
            Err(e) => Err(e),
        };
        let revoke = result
            .err()
            .map(|e| format!("Cannot revoke the Workman token of {name} (session {}): {e}; unused, it lapses after its idle window", m.session_id));
        match (revoke, self.save()) {
            (Some(r), Some(s)) => Some(format!("{r}; {s}")),
            (r, s) => r.or(s),
        }
    }

    pub fn release_session(&mut self, claude_session_id: &str) -> Vec<String> {
        let names: Vec<String> = self
            .minted
            .names()
            .filter(|n| self.minted.get(n).is_some_and(|m| m.claude_session_id.as_deref() == Some(claude_session_id)))
            .cloned()
            .collect();
        names.iter().filter_map(|n| self.release(n)).collect()
    }

    pub fn release_all(&mut self) -> Vec<String> {
        let names: Vec<String> = self.minted.names().cloned().collect();
        names.iter().filter_map(|n| self.release(n)).collect()
    }

    pub fn elsewhere(&self, origin: &str) -> Vec<(String, Option<String>)> {
        self.minted.elsewhere(origin)
    }

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
    use std::cell::Cell;

    use supraman_workman::fake::{Answer, FakeAuth, FakeWorkman};

    use super::*;

    const WAIT: std::time::Duration = std::time::Duration::from_secs(10);
    const MINTS: usize = 12;

    /// The fake's requests past its OAuth server, one at a time as they arrive: (request line, Authorization, body).
    struct Requests {
        fake: FakeWorkman,
        taken: Cell<usize>,
    }

    impl Requests {
        fn recv_timeout(&self, timeout: std::time::Duration) -> Result<(String, String, String), ()> {
            let api = |seen: &[supraman_workman::fake::Seen]| -> Vec<supraman_workman::fake::Seen> {
                seen.iter().filter(|s| !s.target.starts_with("/connect/") && !s.target.starts_with("/.well-known/")).cloned().collect()
            };
            let taken = self.taken.get();
            let s = api(&self.fake.seen_when(timeout, |seen| api(seen).len() > taken)).into_iter().nth(taken).ok_or(())?;
            self.taken.set(taken + 1);
            Ok((format!("{} {} HTTP/1.1", s.method, s.target), s.authorization.unwrap_or_default(), s.body))
        }

        fn try_recv(&self) -> Result<(String, String, String), ()> {
            self.recv_timeout(std::time::Duration::ZERO)
        }
    }

    /// Each `POST` takes the next of `mints`, then mints `h<n>` with `wss_fake<n>`, n counting every `POST`.
    fn fake_workman(mints: Vec<(u16, &'static str)>) -> (String, Requests, Arc<FakeAuth>) {
        let ids = 1..=mints.len() + MINTS;
        let fake = FakeWorkman::start(ids.clone().map(|n| (format!("DELETE /api/agent-sessions/h{n}"), Answer::status(204, ""))).collect());
        let queued = mints.iter().map(|(status, body)| Answer::status(*status, *body));
        let minted = ids.skip(mints.len()).map(|n| Answer::status(201, format!(r#"{{"sessionId":"h{n}","token":"wss_fake{n}"}}"#)));
        for answer in queued.chain(minted) {
            fake.queue("POST /api/agent-sessions", answer);
        }
        let oauth = fake.enable_oauth();
        (fake.base().to_string(), Requests { fake, taken: Cell::new(0) }, oauth)
    }

    fn a_repo(cwd: &Path) -> Option<MintRepo> {
        cwd.ends_with("alpha").then(|| MintRepo {
            root: "C:/demo/alpha".into(),
            remote: Some("https://example.invalid/demo/alpha.git".into()),
        })
    }

    fn home_with(dir: &Path, url: Option<&str>) -> (PathBuf, Arc<Auth>) {
        let home = dir.join("home");
        match url {
            Some(url) => {
                let auth = crate::test_auth::signed_in(&home, url);
                (home, auth)
            }
            None => {
                supraman_core::Settings::default().save(&home).unwrap();
                let auth = Arc::new(Auth::with_store(&home, Arc::new(supraman_workman::MemoryStore::default())));
                (home, auth)
            }
        }
    }

    fn oauth(auth: &str) -> bool {
        auth.starts_with("Bearer eyJ")
    }

    #[test]
    fn a_hand_tab_minted_on_another_server_is_told_to_restart_and_left_to_its_grant() {
        let dir = tempfile::tempdir().unwrap();
        let (a, seen_a, _) = fake_workman(vec![]);
        let (b, seen_b, _) = fake_workman(vec![]);
        let (home, auth) = home_with(dir.path(), Some(&a));
        let (mut hands, _) = Hands::with(&home, auth.clone(), HttpMinter::default(), a_repo);
        let Launch::Minted { name, .. } = hands.launch(Path::new(r"C:\demo\alpha"), None) else { panic!() };
        seen_a.recv_timeout(WAIT).unwrap();
        assert!(hands.elsewhere(&a).is_empty(), "minted there");

        let mut settings = supraman_core::Settings::load(&home).unwrap();
        settings.workman.url = Some(b.clone());
        settings.save(&home).unwrap();
        auth.sign_in(&b, &supraman_workman::auth::http_browser()).unwrap();
        assert_eq!(hands.elsewhere(&b), [(name.clone(), Some(a.clone()))]);
        assert_eq!(
            restart_notice(1, &b).as_deref(),
            Some(format!("1 open claude tab still use another Workman server: restart it to use {b}.").as_str())
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
        supraman_claude::write_mcp_config(&home.join("mcp"), "hand-alpha-1", "http://127.0.0.1:1/mcp", "wss_old", &[]).unwrap();
        let (_, auth) = home_with(dir.path(), None);
        let (hands, notice) = Hands::with(&home, auth, HttpMinter::default(), a_repo);
        assert_eq!(notice, None);
        assert_eq!(hands.elsewhere("https://b.example"), [
            ("hand-alpha-1".to_string(), Some("http://127.0.0.1:1".to_string())),
            ("hand-beta-1".to_string(), None),
        ]);
        assert!(std::fs::read_to_string(Hands::path(&home)).unwrap().contains("http://127.0.0.1:1"), "saved");
    }

    #[test]
    fn hand_mint_carries_the_repo_and_none_outside_a_repo() {
        let dir = tempfile::tempdir().unwrap();
        let (url, seen, _oauth) = fake_workman(vec![]);
        let (home, auth) = home_with(dir.path(), Some(&url));
        let (mut hands, notice) = Hands::with(&home, auth.clone(), HttpMinter::default(), a_repo);
        assert_eq!(notice, None);

        let launch = hands.launch(Path::new(r"C:\demo\alpha"), None);
        let Launch::Minted { name, config, new_session: Some(sid), .. } = launch.clone() else { panic!("{launch:?}") };
        assert_eq!(name, "hand-alpha-1");
        assert!(supraman_claude::is_session_id(&sid), "{sid}");
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
        assert!(launch.script().contains(&format!("try {{ supramanClaude @('--session-id', '{sid}', '--mcp-config', '")), "{}", launch.script());
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
    fn hand_script_carries_the_plugin_flags_after_the_token_and_sm_first_on_path() {
        let extras = Extras {
            args: vec!["--plugin-dir".into(), r"C:\fm\plugins\supraman".into(), "--allowedTools".into(), "Bash(sm:*)".into(), "Bash(C:/o'neil/sm.exe:*)".into()],
            env: vec![("SUPRAMAN_ROLE".into(), "hand".into())],
            path: Some(PathBuf::from(r"C:\Tools\supraman")),
            prompt: None,
        };
        let flags = r"'--plugin-dir', 'C:\fm\plugins\supraman', '--allowedTools', 'Bash(sm:*)', 'Bash(C:/o''neil/sm.exe:*)'";
        let on_path = r"$env:Path = 'C:\Tools\supraman;' + $env:Path; ";
        let config = PathBuf::from(r"C:\fm\mcp\workstate-hand-alpha-1.json");
        let launch = Launch::Minted { name: "hand-alpha-1".into(), session: "hand-alpha-1".into(), config, notice: None, new_session: None };
        let resume = "claude --resume 'fc7b0272-bb70-4c4d-bf95-bbf3b3bc81ff'";
        let script = launch.script_with(resume, &extras);
        assert!(script.starts_with(&format!("{}{on_path}if (Test-Path", runner(resume))), "{script}");
        assert!(runner(resume).contains(&format!("{resume} @supramanArgv")) && runner(resume).contains("@(& { $args } --resume 'fc7b0272-bb70-4c4d-bf95-bbf3b3bc81ff')"), "{script}");
        assert!(script.contains(&format!("try {{ supramanClaude @('--mcp-config', 'C:\\fm\\mcp\\workstate-hand-alpha-1.json', {flags}) }} finally")), "{script}");
        assert!(script.ends_with(&format!("; supramanClaude @({flags}) }}")), "the stale branch gets them too: {script}");
        assert_eq!(Launch::Plain.script_with("claude", &extras), format!("{}{on_path}supramanClaude @({flags})", runner("claude")));
        assert_eq!(Launch::Unminted("x".into()).script_with("claude", &extras), format!("{}{on_path}supramanClaude @({flags})", runner("claude")));
        assert!(!script.contains("SUPRAMAN_ROLE"), "the env goes on the PTY, not into the script");
    }

    #[test]
    fn a_first_prompt_is_read_from_its_file_and_passed_last_after_the_option_end() {
        let extras = Extras { args: vec!["--allowedTools".into(), "Bash(sm:*)".into(), "--permission-mode".into(), "plan".into()], prompt: Some(PathBuf::from(r"C:\fm\prompts\o'neil.txt")), ..Extras::default() };
        let read = r"$supramanPrompt = [IO.File]::ReadAllText('C:\fm\prompts\o''neil.txt'); Remove-Item -LiteralPath 'C:\fm\prompts\o''neil.txt' -ErrorAction SilentlyContinue; ";
        let flags = "'--allowedTools', 'Bash(sm:*)', '--permission-mode', 'plan', '--', $supramanPrompt";
        assert_eq!(Launch::Plain.script_with("claude", &extras), format!("{}{read}supramanClaude @({flags})", runner("claude")));
        let launch = Launch::Minted { name: "hand-a-1".into(), session: "hand-a-1".into(), config: PathBuf::from(r"C:\fm\mcp\w.json"), notice: None, new_session: None };
        let script = launch.script_with("claude", &extras);
        assert!(script.starts_with(&format!("{}{read}", runner("claude"))), "{script}");
        assert!(script.contains(&format!(r"'--mcp-config', 'C:\fm\mcp\w.json', {flags}) }} finally")), "{script}");
        assert!(script.ends_with(&format!("; supramanClaude @({flags}) }}")), "the stale branch sends it too: {script}");
    }

    #[test]
    fn extras_give_a_workspace_folder_the_hand_role_and_the_shared_launch_pieces() {
        let home = PathBuf::from(r"C:\fm\home");
        let plugin = PathBuf::from(r"C:\fm\plugins\supraman");
        let config = crate::supervise::launch_config(Some(home.clone()), Some(&plugin), &supraman_core::Settings::default());
        let ws = Workspace::new("demo", r"C:\demo", vec![supraman_core::Folder::new("Alpha", r"C:\demo\Alpha")]);
        let sm = Path::new(r"C:\Tools\supraman");
        let extras = extras(&config, std::slice::from_ref(&ws), Path::new(r"C:\demo\Alpha"), "hand-Alpha-1", Some(sm));
        let mut want = vec!["--plugin-dir".to_string(), plugin.display().to_string(), "--allowedTools".into()];
        want.extend(supraman_core::sm_allowed_tools(&plugin));
        assert_eq!(extras.args, want);
        assert_eq!(extras.path.as_deref(), Some(sm));
        let env: std::collections::BTreeMap<&str, &str> = extras.env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        assert_eq!(
            env,
            [
                ("SUPRAMAN_REPO", "Alpha"),
                ("SUPRAMAN_ROLE", "hand"),
                ("SUPRAMAN_SESSION", "hand-Alpha-1"),
                ("SUPRAMAN_WORKSPACE", "demo"),
                ("SUPRAMAN_HOME", r"C:\fm\home"),
            ]
            .into()
        );
        assert_eq!(
            config.session_env("demo", "hand-Alpha-1", HAND_ROLE, Some("Alpha"), &[]),
            extras.env,
            "the same env a supervised session gets in its --settings"
        );

        let deeper = super::extras(&config, std::slice::from_ref(&ws), Path::new(r"C:\demo\Alpha\src"), "René's review", None);
        assert!(deeper.env.contains(&("SUPRAMAN_SESSION".into(), "René's review".into())) && deeper.env.contains(&("SUPRAMAN_REPO".into(), "Alpha".into())));
        assert_eq!(deeper.path, None);
        let home_only = super::extras(&config, std::slice::from_ref(&ws), Path::new(r"C:\demo"), "hand-demo", None);
        assert!(home_only.env.contains(&("SUPRAMAN_WORKSPACE".into(), "demo".into())) && !home_only.env.iter().any(|(k, _)| k == "SUPRAMAN_REPO"));
        let outside = super::extras(&config, std::slice::from_ref(&ws), Path::new(r"C:\elsewhere"), "hand-elsewhere", None);
        assert_eq!(outside.env, home_env(&home).map(|(k, v)| (k.to_string(), v)).to_vec(), "outside every workspace: the home only, no role");
        assert_eq!(outside.args, want);
        assert_eq!(plain_name(Path::new(r"C:\demo\my repo")), "hand-my-repo");
    }

    #[test]
    fn hand_script_passes_only_the_path_and_no_strict_flag() {
        let config = PathBuf::from(r"C:\Users\o'neil\Supraman\mcp\workstate-hand-alpha-1.json");
        let launch = Launch::Minted { name: "hand-alpha-1".into(), session: "hand-alpha-1".into(), config, notice: None, new_session: None };
        let script = launch.script();
        let path = "C:\\Users\\o''neil\\Supraman\\mcp\\workstate-hand-alpha-1.json";
        assert_eq!(
            script,
            format!(
                "{}if (Test-Path -LiteralPath '{path}') {{ try {{ supramanClaude @('--mcp-config', '{path}') }} finally {{ Remove-Item -LiteralPath '{path}' -ErrorAction SilentlyContinue }} }} else {{ Write-Host '{STALE}'; claude }}",
                runner("claude")
            )
        );
        assert!(!script.contains("--strict-mcp-config") && !script.contains("wss_"));
        assert_eq!(launch.shell_args(), ["-NoExit".to_string(), "-EncodedCommand".into(), encoded_command(&script)]);
        assert_eq!(Launch::Plain.script(), "claude", "no args: claude alone, no runner");
        let head = "claude --resume 'fc7b0272-bb70-4c4d-bf95-bbf3b3bc81ff'";
        let resume = launch.script_with(head, &Extras::default());
        assert!(resume.starts_with(&runner(head)) && runner(head).contains(&format!("{head} @supramanArgv")), "{resume}");
        assert!(resume.contains("try { supramanClaude @('--mcp-config', '"), "{resume}");
        assert!(resume.ends_with("; claude --resume 'fc7b0272-bb70-4c4d-bf95-bbf3b3bc81ff' }"), "the stale branch resumes too: {resume}");
        assert_eq!(Launch::Plain.script_with("claude --resume 'x'", &Extras::default()), "claude --resume 'x'");
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
                "{}if (Test-Path -LiteralPath 'C:\\fm\\mcp\\workstate-hand-alpha-1.json') {{ try {{ supramanClaude @('--session-id', 'aaaa0001-0000-4000-8000-000000000001', '--mcp-config', 'C:\\fm\\mcp\\workstate-hand-alpha-1.json') }} finally {{ Remove-Item -LiteralPath 'C:\\fm\\mcp\\workstate-hand-alpha-1.json' -ErrorAction SilentlyContinue }} }} else {{ Write-Host '{STALE}'; claude }}",
                runner("claude")
            )
        );
        assert_eq!(Launch::Unminted("x".into()).script(), "claude");
        // pwsh's own documented example
        assert_eq!(encoded_command("dir \"c:\\program files\" "), "ZABpAHIAIAAiAGMAOgBcAHAAcgBvAGcAcgBhAG0AIABmAGkAbABlAHMAIgAgAA==");
    }

    #[test]
    fn hand_config_file_is_owner_only() {
        let dir = tempfile::tempdir().unwrap();
        let (url, _seen, _oauth) = fake_workman(vec![]);
        let (home, auth) = home_with(dir.path(), Some(&url));
        let (mut hands, _) = Hands::with(&home, auth.clone(), HttpMinter::default(), a_repo);
        let Launch::Minted { config, .. } = hands.launch(Path::new(r"C:\demo\alpha"), None) else { panic!() };
        #[cfg(windows)]
        {
            let out = supraman_core::process::command("icacls").arg(&config).output().unwrap();
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
        let (url, seen, _oauth) = fake_workman(vec![]);
        let (home, auth) = home_with(dir.path(), Some(&url));
        let (mut hands, _) = Hands::with(&home, auth.clone(), HttpMinter::default(), a_repo);
        let alpha = Path::new(r"C:\demo\alpha");

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

        let Launch::Minted { name, config, .. } = hands.launch(alpha, None) else { panic!() };
        seen.recv_timeout(WAIT).unwrap();
        assert_eq!(hands.release(&name), None);
        assert_eq!(seen.recv_timeout(WAIT).unwrap().0, "DELETE /api/agent-sessions/h2 HTTP/1.1");
        assert!(!config.exists(), "close deletes the file");

        let Launch::Minted { config: c3, .. } = hands.launch(alpha, None) else { panic!() };
        let Launch::Minted { config: c4, .. } = hands.launch(Path::new(r"C:\demo"), None) else { panic!() };
        seen.recv_timeout(WAIT).unwrap();
        seen.recv_timeout(WAIT).unwrap();
        assert!(hands.release_all().is_empty());
        let mut deleted = [seen.recv_timeout(WAIT).unwrap().0, seen.recv_timeout(WAIT).unwrap().0];
        deleted.sort();
        assert_eq!(deleted, ["DELETE /api/agent-sessions/h3 HTTP/1.1", "DELETE /api/agent-sessions/h4 HTTP/1.1"]);
        assert!(!c3.exists() && !c4.exists() && hands.names().is_empty());

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

    struct SlowRevokes {
        revoke_takes: std::time::Duration,
        revoked: Arc<std::sync::atomic::AtomicUsize>,
        minted: std::sync::atomic::AtomicUsize,
    }

    impl Minter for SlowRevokes {
        fn mint(&self, _: &str, _: &str, _: &MintRequest) -> Result<supraman_supervisor::Minted, MintError> {
            let n = self.minted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(supraman_supervisor::Minted { session_id: format!("s{n}"), token: format!("wss_slow{n}"), expires_at: None })
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
        let (url, _seen, _oauth) = fake_workman(vec![]);
        let (home, auth) = home_with(dir, Some(&url));
        let revoked = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let minter = SlowRevokes { revoke_takes, revoked: revoked.clone(), minted: Default::default() };
        let (hands, _) = Hands::with(&home, auth.clone(), minter, a_repo);
        (home, Arc::new(Mutex::new(hands)), revoked, auth)
    }

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
        let (url, seen, _oauth) = fake_workman(vec![]);
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
    fn hand_without_workman_is_plain_and_mints_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (_url, seen, _oauth) = fake_workman(vec![]);
        let (home, auth) = home_with(dir.path(), None);
        let (mut hands, _) = Hands::with(&home, auth.clone(), HttpMinter::default(), a_repo);
        assert_eq!(hands.launch(Path::new(r"C:\demo\alpha"), None), Launch::Plain);
        assert!(seen.recv_timeout(std::time::Duration::from_millis(200)).is_err(), "no request");
        assert!(!home.join("mcp").exists());
        assert!(hands.release_all().is_empty());
    }

    #[test]
    fn hand_with_a_broken_sign_in_opens_it_plain_and_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let (url, seen, oauth) = fake_workman(vec![]);
        oauth.set_access_ttl(60);
        let (home, auth) = home_with(dir.path(), Some(&url));
        oauth.fail_refresh(true);
        let (mut hands, _) = Hands::with(&home, auth.clone(), HttpMinter::default(), a_repo);
        let Launch::Unminted(notice) = hands.launch(Path::new(r"C:\demo\alpha"), None) else { panic!() };
        assert!(notice.contains("sign in to Workman") && notice.contains("again"), "{notice}");
        assert!(seen.recv_timeout(std::time::Duration::from_millis(200)).is_err(), "no mint");
    }

    #[test]
    fn hand_mint_failure_opens_it_plain_with_one_notice() {
        let dir = tempfile::tempdir().unwrap();
        let (url, seen, _oauth) = fake_workman(vec![(409, r#"{"error":"name held by another launcher"}"#)]);
        let (home, auth) = home_with(dir.path(), Some(&url));
        let (mut hands, _) = Hands::with(&home, auth.clone(), HttpMinter::default(), a_repo);
        let launch = hands.launch(Path::new(r"C:\demo\alpha"), None);
        assert_eq!(
            launch,
            Launch::Unminted(
                "This claude session runs without its own Workman token: cannot mint an orchestrator one (Workman answered HTTP 409: name held by another launcher)".into()
            )
        );
        assert_eq!(launch.script(), "claude");
        assert!(oauth(&seen.recv_timeout(WAIT).unwrap().1));
        assert!(hands.names().is_empty());
    }

    #[test]
    fn a_saved_session_named_before_slug_names_resumes_under_its_own_name() {
        let dir = tempfile::tempdir().unwrap();
        let (url, seen, _oauth) = fake_workman(vec![]);
        let (home, auth) = home_with(dir.path(), Some(&url));
        let (mut hands, _) = Hands::with(&home, auth, HttpMinter::default(), a_repo);
        let folder = Path::new(r"C:\demo\Alpha");
        let sid = "fc7b0272-bb70-4c4d-bf95-bbf3b3bc81ff";
        let Launch::Minted { session, .. } = hands.launch_for(folder, Some("hand-Alpha-1"), Some(sid)) else { panic!() };
        assert_eq!(session, "hand-Alpha-1");
        let body: serde_json::Value = serde_json::from_str(&seen.recv_timeout(WAIT).unwrap().2).unwrap();
        assert_eq!(body["name"].as_str(), Some("hand-Alpha-1"));
        let Launch::Minted { session, .. } = hands.launch(folder, None) else { panic!() };
        assert_eq!(session, "hand-alpha-2", "a new session's name is slugify's, numbered after the resumed one's key");
    }

    #[test]
    fn hand_resume_mints_an_orchestrator_under_the_sessions_own_name() {
        let dir = tempfile::tempdir().unwrap();
        let (url, seen, _oauth) = fake_workman(vec![]);
        let (home, auth) = home_with(dir.path(), Some(&url));
        let (mut hands, _) = Hands::with(&home, auth.clone(), HttpMinter::default(), a_repo);
        let alpha = Path::new(r"C:\demo\alpha");

        let sid = "fc7b0272-bb70-4c4d-bf95-bbf3b3bc81ff";
        let Launch::Minted { name, session, config, notice, new_session } = hands.launch_for(alpha, Some("mw-workflow"), Some(sid)) else { panic!() };
        assert_eq!((name.as_str(), session.as_str(), notice), ("hand-alpha-1", "mw-workflow", None));
        assert_eq!(new_session, None, "a resume starts no new session");
        assert_eq!(hands.minted.get(&name).unwrap().claude_session_id.as_deref(), Some(sid));
        assert!(config.ends_with("workstate-hand-alpha-1.json"), "the file keeps Supraman's key: {config:?}");
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

        let Launch::Minted { session, notice, .. } = hands.launch(alpha, Some("mw-workflow")) else { panic!() };
        assert_eq!(session, "hand-alpha-2");
        assert!(notice.is_some_and(|n| n.contains("another tab holds that name")));
        let body: serde_json::Value = serde_json::from_str(&seen.recv_timeout(WAIT).unwrap().2).unwrap();
        assert_eq!(body["name"].as_str(), Some("hand-alpha-2"));

        let Launch::Minted { session, .. } = hands.launch(alpha, Some(&format!("  Fix the\tdeploy\n{}", "x".repeat(80)))) else { panic!() };
        assert_eq!(session, format!("Fix the deploy {}", "x".repeat(49)));
        seen.recv_timeout(WAIT).unwrap();
        let Launch::Minted { session, .. } = hands.launch(alpha, Some(" \u{200B} ")) else { panic!() };
        assert_eq!(session, "hand-alpha-4");
        seen.recv_timeout(WAIT).unwrap();

        assert!(hands.release_all().is_empty());
        let Launch::Minted { session, .. } = hands.launch(alpha, Some("mw-workflow")) else { panic!() };
        assert_eq!(session, "mw-workflow");
    }

    #[test]
    fn hand_name_held_by_another_launcher_falls_back_to_the_generated_one() {
        let dir = tempfile::tempdir().unwrap();
        let (url, seen, _oauth) = fake_workman(vec![(409, r#"{"error":"the session name \"nomos-0c\" is held by a session launched by no launcher"}"#)]);
        let (home, auth) = home_with(dir.path(), Some(&url));
        let (mut hands, _) = Hands::with(&home, auth.clone(), HttpMinter::default(), a_repo);
        let Launch::Minted { name, session, notice, .. } = hands.launch(Path::new(r"C:\demo\alpha"), Some("nomos-0c")) else { panic!() };
        assert_eq!((name.as_str(), session.as_str()), ("hand-alpha-1", "hand-alpha-1"));
        assert!(notice.unwrap().starts_with("Workman session hand-alpha-1, not nomos-0c: "));
        let names: Vec<String> = (0..2)
            .map(|_| serde_json::from_str::<serde_json::Value>(&seen.recv_timeout(WAIT).unwrap().2).unwrap()["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(names, ["nomos-0c", "hand-alpha-1"]);
    }

    #[test]
    fn hand_refused_orchestrator_is_not_minted_as_a_worker() {
        let dir = tempfile::tempdir().unwrap();
        let (url, seen, _oauth) = fake_workman(vec![(403, r#"{"error":"a worker cannot mint a orchestrator token; a role is never raised"}"#)]);
        let (home, auth) = home_with(dir.path(), Some(&url));
        let (mut hands, _) = Hands::with(&home, auth.clone(), HttpMinter::default(), a_repo);
        let Launch::Unminted(notice) = hands.launch(Path::new(r"C:\demo\alpha"), Some("mw-workflow")) else { panic!() };
        assert!(notice.contains("cannot mint an orchestrator one (Workman answered HTTP 403"), "{notice}");
        let body: serde_json::Value = serde_json::from_str(&seen.recv_timeout(WAIT).unwrap().2).unwrap();
        assert_eq!(body["role"].as_str(), Some("orchestrator"));
        assert!(seen.recv_timeout(std::time::Duration::from_millis(300)).is_err(), "no second mint");
        assert!(hands.names().is_empty());
    }

    #[test]
    fn the_runner_resolves_the_command_word_of_a_bare_or_quoted_claude() {
        assert_eq!(command_split("claude"), ("claude", ""));
        assert_eq!(command_split("claude --resume 'x'"), ("claude", " --resume 'x'"));
        assert_eq!(command_split(r"& 'C:\o''neil\claude.exe' --selftest-argv 'out'"), (r"'C:\o''neil\claude.exe'", " --selftest-argv 'out'"));
        assert!(runner(r"& 'C:\x\c.exe' -a").contains(r"Get-Command -Name 'C:\x\c.exe' "));
    }

    /// `powershell` always, `pwsh` too when a tab would find it.
    #[cfg(windows)]
    fn shells() -> Vec<&'static str> {
        let mut shells = vec!["powershell"];
        if supraman_core::paths::terminal_powershell().name == "pwsh" {
            shells.push("pwsh");
        }
        shells
    }

    /// A tab's PowerShell running `args` with `path` first on PATH; -NoExit's prompt reads stdin, and a closed one ends it.
    #[cfg(windows)]
    fn tab(shell: &str, args: Vec<String>, path: &Path, env: &[(&str, &Path)]) -> std::process::Output {
        let path = format!("{};{}", path.display(), std::env::var("PATH").unwrap_or_default());
        let mut command = supraman_core::process::command(shell);
        command.args(args).env("PATH", path).stdin(std::process::Stdio::null());
        env.iter().for_each(|(k, v)| {
            command.env(k, v);
        });
        command.output().unwrap()
    }

    #[cfg(windows)]
    fn tab_args(launch_args: impl IntoIterator<Item = String>) -> Vec<String> {
        crate::view::powershell_args(["-NoLogo".to_string(), "-NoProfile".into()].into_iter().chain(launch_args).collect())
    }

    #[cfg(windows)]
    #[test]
    fn hand_launch_deletes_the_config_when_claude_exits_and_shows_no_script() {
        for shell in shells() {
            let dir = tempfile::tempdir().unwrap();
            let config = dir.path().join("workstate-hand-alpha-1.json");
            std::fs::write(&config, "{}").unwrap();
            let seen = dir.path().join("argv.txt");
            let fake = format!("@echo %*> \"{}\"\r\n@echo fake claude ran\r\n@exit /b 3\r\n", seen.display());
            std::fs::write(dir.path().join("claude.cmd"), fake).unwrap();
            let launch = Launch::Minted { name: "hand-alpha-1".into(), session: "hand-alpha-1".into(), config: config.clone(), notice: None, new_session: None };
            let out = tab(shell, tab_args(launch.shell_args()), dir.path(), &[]);
            assert_eq!(std::fs::read_to_string(&seen).unwrap().trim(), format!("--mcp-config {}", config.display()), "{shell}");
            assert!(!config.exists(), "{shell}: deleted even though claude failed");
            let shown = String::from_utf8_lossy(&out.stdout);
            assert!(shown.contains("fake claude ran"), "{shell}: {out:?}");
            assert!(!shown.contains("Test-Path") && !shown.contains("mcp-config"), "{shell}: the script is not shown: {shown}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn hand_launch_runs_plain_claude_once_the_config_is_gone() {
        for shell in shells() {
            let dir = tempfile::tempdir().unwrap();
            let config = dir.path().join("workstate-hand-alpha-1.json");
            std::fs::write(&config, "{}").unwrap();
            let seen = dir.path().join("argv.txt");
            let line = Launch::Minted { name: "hand-alpha-1".into(), session: "hand-alpha-1".into(), config: config.clone(), notice: None, new_session: None }.script();
            let script = format!(
                "function claude {{ Add-Content -LiteralPath '{}' -Value ('[' + ($args -join ' ') + ']') }}; {line}; {line}",
                seen.display(),
            );
            let out = supraman_core::process::command(shell).args(["-NoProfile", "-Command", &script]).output().unwrap();
            let runs: Vec<String> = std::fs::read_to_string(&seen).unwrap().lines().map(str::to_string).collect();
            assert_eq!(runs, [format!("[--mcp-config {}]", config.display()), "[]".to_string()], "{shell}");
            assert!(String::from_utf8_lossy(&out.stdout).contains("Workman token was revoked"), "{shell}: {out:?}");
            assert!(!config.exists(), "{shell}");
        }
    }

    /// `examples/argv.rs`, which `cargo test` builds beside the test binary's `deps` folder.
    #[cfg(windows)]
    fn native_recorder() -> PathBuf {
        let exe = std::env::current_exe().unwrap();
        let built = exe.parent().and_then(Path::parent).unwrap().join("examples").join("argv.exe");
        assert!(built.is_file(), "{} is missing: cargo test builds examples/argv.rs", built.display());
        built
    }

    #[cfg(windows)]
    #[test]
    fn a_prompt_and_flags_reach_a_native_claude_and_a_claude_ps1_as_the_same_argv_under_every_powershell() {
        let prompt = "fix the \"voice menu\" bug in C:\\dir\\ now\nthen say \"done\" | 100% %PATH% `tick` $env:X é 中 C:\\end\\";
        let flags = ["--plugin-dir", r"C:\Program Files\x\", "--empty", "", r#"say \"hi\" and a\"b"#, r"Bash(C:/o'neil/sm.exe:*)"];
        let session = "aaaa0001-0000-4000-8000-000000000001";
        let dir = tempfile::tempdir().unwrap();
        let (native, script) = (dir.path().join("native"), dir.path().join("script"));
        std::fs::create_dir_all(&native).unwrap();
        std::fs::create_dir_all(&script).unwrap();
        std::fs::copy(native_recorder(), native.join("claude.exe")).unwrap();
        std::fs::write(script.join("claude.ps1"), "[IO.File]::AppendAllText($env:SUPRAMAN_ARGV_OUT, (ConvertTo-Json -InputObject @($args) -Compress) + \"`n\")\n").unwrap();
        for shell in shells() {
            for target in [&native, &script] {
                let config = dir.path().join("workstate-hand-alpha-1.json");
                std::fs::write(&config, "{}").unwrap();
                let prompt_file = dir.path().join("prompt.txt");
                std::fs::write(&prompt_file, prompt).unwrap();
                let out = dir.path().join("argv.json");
                let _ = std::fs::remove_file(&out);
                let _ = std::fs::remove_file(out.with_extension("env"));
                let extras = Extras { args: flags.iter().map(|f| f.to_string()).collect(), prompt: Some(prompt_file.clone()), ..Extras::default() };
                let launch = Launch::Minted { name: "hand-alpha-1".into(), session: "hand-alpha-1".into(), config: config.clone(), notice: None, new_session: Some(session.into()) };
                let ran = tab(shell, tab_args(launch.shell_args_with("claude", &extras)), target, &[("SUPRAMAN_ARGV_OUT", &out)]);
                let got = std::fs::read_to_string(&out).unwrap_or_else(|e| panic!("{shell} {}: no argv ({e}): {ran:?}", target.display()));
                let got: Vec<String> = serde_json::from_str(got.trim()).unwrap();
                let mut want: Vec<String> = ["--session-id", session, "--mcp-config"].map(String::from).to_vec();
                want.push(config.display().to_string());
                want.extend(flags.iter().map(|f| f.to_string()));
                want.extend(["--".to_string(), prompt.to_string()]);
                assert_eq!(got, want, "{shell} {}", target.display());
                assert!(!config.exists() && !prompt_file.exists(), "{shell}");
                if target == &native {
                    let env = std::fs::read_to_string(out.with_extension("env")).unwrap();
                    assert!(env.contains("SUPRAMAN_ARGV_OUT=") && !env.contains("voice menu"), "{shell}: the prompt is in claude's environment");
                }
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn a_claude_ps1_first_on_path_runs_under_a_restricted_policy_only_with_the_tabs_bypass() {
        let dir = tempfile::tempdir().unwrap();
        let ran = dir.path().join("ran.txt");
        std::fs::write(dir.path().join("claude.ps1"), format!("Set-Content -LiteralPath '{}' -Value ran\n", ran.display())).unwrap();
        for shell in shells() {
            let restricted: &[(&str, &Path)] = &[("PSExecutionPolicyPreference", Path::new("Restricted"))];
            let _ = std::fs::remove_file(&ran);
            let without = ["-NoLogo".to_string(), "-NoProfile".into()].into_iter().chain(Launch::Plain.shell_args()).collect();
            let out = tab(shell, without, dir.path(), restricted);
            assert!(!ran.exists(), "{shell}: Restricted refuses the script without -ExecutionPolicy Bypass: {out:?}");
            let out = tab(shell, tab_args(Launch::Plain.shell_args()), dir.path(), restricted);
            assert!(ran.exists(), "{shell}: a tab runs claude.ps1 under Restricted: {out:?}");
        }
    }
}
