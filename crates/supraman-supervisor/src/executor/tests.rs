use std::cell::RefCell;

use supraman_claude::{ClaudeError, Kind, SessionState, Status};

use super::*;
use crate::{MintError, Minted, NeedsKind, Remedy, SECOND, is_foreign, tick_all};
use supraman_core::FM_PREFIX;

const NOW: Millis = 1_800_000_000_000;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Call {
    Launch { name: String, argv: Vec<String> },
    Resume { session_id: String, cwd: PathBuf, prompt: Option<String> },
    Stop { id: String },
}

#[derive(Default)]
struct FakeLauncher {
    calls: RefCell<Vec<Call>>,
    /// Popped per launch/resume; empty means success.
    errors: RefCell<Vec<ClaudeError>>,
    /// What a successful launch/resume prints; `None`: `backgrounded · aaaa0001 · x`.
    output: RefCell<Option<String>>,
    /// Popped per stop; empty means success.
    stop_errors: RefCell<Vec<ClaudeError>>,
}

impl FakeLauncher {
    fn result(&self) -> supraman_claude::Result<String> {
        match self.errors.borrow_mut().pop() {
            Some(e) => Err(e),
            None => Ok(self.output.borrow().clone().unwrap_or_else(|| "backgrounded · aaaa0001 · x".into())),
        }
    }

    fn take(&self) -> Vec<Call> {
        std::mem::take(&mut self.calls.borrow_mut())
    }
}

impl Launcher for FakeLauncher {
    fn launch(&self, spec: &LaunchSpec) -> supraman_claude::Result<String> {
        let argv = spec.argv().iter().map(|a| a.to_string_lossy().into_owned()).collect();
        self.calls.borrow_mut().push(Call::Launch { name: spec.name.clone(), argv });
        self.result()
    }

    fn resume(&self, session_id: &str, cwd: &Path, prompt: Option<&str>) -> supraman_claude::Result<String> {
        self.calls.borrow_mut().push(Call::Resume {
            session_id: session_id.into(),
            cwd: cwd.into(),
            prompt: prompt.map(str::to_string),
        });
        self.result()
    }

    fn stop(&self, id: &str) -> supraman_claude::Result<String> {
        self.calls.borrow_mut().push(Call::Stop { id: id.into() });
        self.stop_errors.borrow_mut().pop().map_or(Ok(String::new()), Err)
    }
}

/// The same bearer for every role, with the Workman role the app maps it to; `.1` an error instead.
struct FakeTokens(Option<RoleToken>, Option<String>, Result<String, String>);

impl TokenProvider for FakeTokens {
    fn token(&self, role: &str) -> Result<Option<RoleToken>, String> {
        if let Some(e) = &self.1 {
            return Err(e.clone());
        }
        let Some(mut t) = self.0.clone() else { return Ok(None) };
        t.role = match role {
            "workspace" => "controller".into(),
            "repo" => "orchestrator".into(),
            other => other.into(),
        };
        Ok(Some(t))
    }

    fn mcp_url(&self) -> Result<String, String> {
        self.2.clone()
    }
}

const SIGNED_OUT_URL: &str = "http://127.0.0.1:2/mcp";
const STATIC: &str = "fake-static-role-token";

fn static_token() -> Option<RoleToken> {
    Some(RoleToken {
        url: "http://127.0.0.1:1/mcp".into(),
        token: STATIC.into(),
        role: String::new(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MintCall {
    Mint { base: String, bearer: String, request: MintRequest },
    Bind { bearer: String, session_id: String, claude_session_id: String },
    Revoke { bearer: String, session_id: String },
}

#[derive(Default)]
struct MintLog {
    calls: Vec<MintCall>,
    /// Popped per mint; empty means success.
    mint_errors: Vec<MintError>,
    revoke_error: Option<MintError>,
    /// One Workman row per name, as `replace` keeps it.
    rows: Vec<String>,
    minted: u32,
}

/// Never touches the network: every supervisor in these tests has one.
#[derive(Clone, Default)]
struct FakeMinter(std::sync::Arc<std::sync::Mutex<MintLog>>);

impl FakeMinter {
    fn take(&self) -> Vec<MintCall> {
        std::mem::take(&mut self.0.lock().unwrap().calls)
    }
}

impl Minter for FakeMinter {
    fn mint(&self, base: &str, bearer: &str, request: &MintRequest) -> Result<Minted, MintError> {
        let mut log = self.0.lock().unwrap();
        log.calls.push(MintCall::Mint { base: base.into(), bearer: bearer.into(), request: request.clone() });
        if let Some(e) = log.mint_errors.pop() {
            return Err(e);
        }
        let row = match log.rows.iter().position(|n| n == &request.name) {
            Some(i) => i,
            None => {
                log.rows.push(request.name.clone());
                log.rows.len() - 1
            }
        };
        log.minted += 1;
        Ok(Minted {
            session_id: format!("ws-{row}"),
            token: format!("wss_fake{}", log.minted),
            expires_at: None,
        })
    }

    fn bind(&self, _base: &str, bearer: &str, session_id: &str, claude_session_id: &str) -> Result<(), MintError> {
        self.0.lock().unwrap().calls.push(MintCall::Bind {
            bearer: bearer.into(),
            session_id: session_id.into(),
            claude_session_id: claude_session_id.into(),
        });
        Ok(())
    }

    fn revoke(&self, _base: &str, bearer: &str, session_id: &str) -> Result<(), MintError> {
        let mut log = self.0.lock().unwrap();
        log.calls.push(MintCall::Revoke { bearer: bearer.into(), session_id: session_id.into() });
        log.revoke_error.clone().map_or(Ok(()), Err)
    }
}

fn supervisor(started: bool, tokens: Option<RoleToken>, mcp_dir: &Path) -> Supervisor<FakeLauncher, FakeTokens> {
    let mut desired = DesiredState::workspace_controller("demo", r"C:\demo\cwd", "be the controller");
    desired.started = started;
    let config = LaunchConfig {
        mcp_dir: mcp_dir.into(),
        plugin_dirs: vec![r"C:\fm\plugins\supraman".into()],
        ..LaunchConfig::default()
    };
    Supervisor::new(desired, mcp_dir, config, FakeLauncher::default(), FakeTokens(tokens, None, Ok(SIGNED_OUT_URL.into())))
        .with_minter(FakeMinter::default())
        .with_repo_of(fake_repo_of)
        .with_daemon_check(|| None)
}

/// Alpha's checkout has an origin, Beta's none; any other cwd is not a repo.
fn fake_repo_of(dir: &Path) -> Option<MintRepo> {
    match dir.file_name()?.to_str()? {
        "Alpha" => Some(MintRepo { root: "C:/demo/Alpha".into(), remote: Some("https://example.invalid/demo/alpha.git".into()) }),
        "Beta" => Some(MintRepo { root: "C:/demo/Beta".into(), remote: None }),
        _ => None,
    }
}

/// [`supervisor`] with the static role token, and a handle on its fake minter.
fn minting(started: bool, mcp_dir: &Path) -> (Supervisor<FakeLauncher, FakeTokens>, FakeMinter) {
    let minter = FakeMinter::default();
    (supervisor(started, static_token(), mcp_dir).with_minter(minter.clone()), minter)
}

fn config_text(mcp_dir: &Path, name: &str) -> String {
    std::fs::read_to_string(mcp_dir.join(format!("workstate-{name}.json"))).unwrap()
}

/// `name`'s config runs `sm mcp-header` for it and holds no token.
fn assert_helped(mcp_dir: &Path, name: &str) {
    let text = config_text(mcp_dir, name);
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let helper = v["mcpServers"]["workman"]["headersHelper"].as_str().unwrap_or_default();
    assert!(helper.contains(" mcp-header --home ") && helper.ends_with(&format!(" {name}")), "{text}");
    assert!(!text.contains("Bearer") && !text.contains("wss_") && !text.contains(STATIC), "no token in the config: {text}");
}

/// The token `name`'s session gets: the one kept for it.
fn kept(sup: &Supervisor<FakeLauncher, FakeTokens>, name: &str) -> String {
    sup.keeper().kept(name).unwrap_or_default()
}

fn launch_argv(calls: &[Call]) -> String {
    match calls {
        [Call::Launch { argv, .. }] => argv.join(" "),
        other => panic!("{other:?}"),
    }
}

fn free() -> Budget {
    Budget { max: 3, free: 3 }
}

fn row(state: SessionState, status: Option<Status>, pid: Option<u32>) -> Session {
    Session {
        id: "aaaa0001".into(),
        session_id: "aaaa0001-0000-4000-8000-000000000001".into(),
        name: "fm-demo-workspace".into(),
        cwd: r"C:\demo\cwd".into(),
        kind: Kind::Background,
        state: Some(state),
        status,
        waiting_for: None,
        pid,
        started_at: Some(NOW - 3_600_000),
    }
}

#[test]
fn mint_launch_writes_only_the_minted_token_and_settles() {
    let dir = tempfile::tempdir().unwrap();
    let mcp = dir.path().join("mcp");
    let (mut sup, minter) = minting(true, &mcp);
    assert_eq!(sup.tick(&[], &mut free(), NOW), []);
    match sup.launcher().take().as_slice() {
        [Call::Launch { name, argv }] => {
            assert_eq!(name, "fm-demo-workspace");
            let argv = argv.join(" ");
            assert!(argv.contains("--strict-mcp-config"), "{argv}");
            assert!(argv.contains("workstate-fm-demo-workspace.json"), "{argv}");
            assert!(argv.contains(r"--plugin-dir C:\fm\plugins\supraman"), "{argv}");
            assert!(argv.ends_with("-- be the controller"), "{argv}");
            assert!(!argv.contains(STATIC) && !argv.contains("wss_"), "no token in argv: {argv}");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        minter.take(),
        [MintCall::Mint {
            base: "http://127.0.0.1:1".into(),
            bearer: STATIC.into(),
            request: MintRequest {
                name: "fm-demo-workspace".into(),
                role: "controller".into(),
                launched_by: "supraman/demo".into(),
                origin: "supraman/demo".into(),
                cwd: r"C:\demo\cwd".into(),
                external_role: "workspace".into(),
                replace: true,
                repo: None,
                claude_session_id: None,
                takeover_from: None,
            },
        }]
    );
    let text = config_text(&mcp, "fm-demo-workspace");
    assert!(!text.contains(STATIC), "the static token never reaches the file: {text}");
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["mcpServers"]["workman"]["url"], "http://127.0.0.1:1/mcp");
    assert_eq!(v["mcpServers"]["workman"]["headers"], serde_json::json!({}));
    assert_helped(&mcp, "fm-demo-workspace");
    assert_eq!(kept(&sup, "fm-demo-workspace"), "wss_fake1");
    assert_eq!(sup.minted().get("fm-demo-workspace").unwrap().session_id, "ws-0");
    assert_eq!(sup.tick(&[], &mut free(), NOW + 5 * SECOND), []);
    assert_eq!(sup.launcher().take(), [], "the row has not shown yet: no second launch");
    let live = row(SessionState::Working, Some(Status::Busy), Some(4242));
    sup.tick(std::slice::from_ref(&live), &mut free(), NOW + 61 * SECOND);
    assert_eq!(sup.launcher().take(), []);
    assert_eq!(sup.memory.get("fm-demo-workspace").unwrap().started_at, None, "stable");
}

#[test]
fn mint_of_a_name_recorded_as_foreman_takes_its_row_over_and_records_supraman() {
    let dir = tempfile::tempdir().unwrap();
    let (mut sup, minter) = minting(true, &dir.path().join("mcp"));
    let former = MintRequest {
        name: "fm-demo-workspace".into(),
        role: "controller".into(),
        launched_by: "foreman/demo".into(),
        origin: "foreman/demo".into(),
        cwd: r"C:\demo\cwd".into(),
        external_role: "workspace".into(),
        replace: true,
        repo: None,
        claude_session_id: None,
        takeover_from: None,
    };
    let session = MintedSession { session_id: "ws-0".into(), role: "workspace".into(), claude_session_id: None, origin: None };
    sup.minted.insert_minted("fm-demo-workspace", session, &former);
    sup.tick(&[], &mut free(), NOW);
    let calls = minter.take();
    let [MintCall::Mint { request, .. }] = calls.as_slice() else { panic!("{calls:?}") };
    assert_eq!((request.launched_by.as_str(), request.takeover_from.as_deref()), ("supraman/demo", Some("foreman/demo")));
    assert_eq!(sup.minted.launched_by("fm-demo-workspace"), Some("supraman/demo"), "the next mint is a plain one");
}

fn alpha() -> WantedSession {
    WantedSession {
        name: "fm-demo-alpha".into(),
        role: "repo".into(),
        cwd: r"C:\demo\Alpha".into(),
        repo: Some("Alpha".into()),
        prompt: "run the queue".into(),
        permission_mode: "auto".into(),
        add_dirs: Vec::new(),
        resume_prompt: None,
        fork_from: None,
    }
}

#[test]
fn mint_each_session_gets_its_own_token_with_its_identity_in_the_mint() {
    let dir = tempfile::tempdir().unwrap();
    let mcp = dir.path().join("mcp");
    let (mut sup, minter) = minting(true, &mcp);
    sup.desired.sessions.push(alpha());
    sup.tick(&[], &mut free(), NOW);
    assert_eq!(sup.launcher().take().len(), 2);
    let requests: Vec<MintRequest> = minter
        .take()
        .into_iter()
        .map(|c| match c {
            MintCall::Mint { request, .. } => request,
            other => panic!("{other:?}"),
        })
        .collect();
    let repo = &requests[1];
    assert_eq!(
        (repo.name.as_str(), repo.role.as_str(), repo.external_role.as_str()),
        ("fm-demo-alpha", "orchestrator", "repo")
    );
    assert_eq!((repo.launched_by.as_str(), repo.origin.as_str(), repo.cwd.as_str()), ("supraman/demo", "supraman/demo/Alpha", r"C:\demo\Alpha"));
    assert_eq!((requests[0].role.as_str(), requests[0].origin.as_str()), ("controller", "supraman/demo"));
    assert_eq!(
        repo.repo,
        Some(MintRepo { root: "C:/demo/Alpha".into(), remote: Some("https://example.invalid/demo/alpha.git".into()) }),
        "a repo session is minted with its repo"
    );
    assert_eq!(requests[0].repo, None, "the workspace root is not a repo");

    let ws = config_text(&mcp, "fm-demo-workspace");
    let repo = config_text(&mcp, "fm-demo-alpha");
    assert!(kept(&sup, "fm-demo-workspace") == "wss_fake1" && kept(&sup, "fm-demo-alpha") == "wss_fake2", "{ws} {repo}");
    assert_helped(&mcp, "fm-demo-workspace");
    assert_helped(&mcp, "fm-demo-alpha");
    assert!(["X-WorkState-", "X-Workman-"].iter().all(|h| !ws.contains(h) && !repo.contains(h)), "identity is on the row, not in headers: {repo}");
}

/// A Claude config dir with the workman plugin installed under `dir`, declaring its `agents` server.
fn claude_with_agents(dir: &Path) -> (PathBuf, String) {
    let root = dir.join("cache").join("workman").join("2.3.0");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join(".mcp.json"), r#"{"mcpServers":{"agents":{"command":"node","args":["${CLAUDE_PLUGIN_ROOT}/agents.mjs"]}}}"#).unwrap();
    let claude = dir.join("claude");
    std::fs::create_dir_all(claude.join("plugins")).unwrap();
    let installed = serde_json::json!({ "version": 2, "plugins": { "workman@supraman": [{ "installPath": root }] } });
    std::fs::write(claude.join("plugins").join("installed_plugins.json"), installed.to_string()).unwrap();
    (claude, format!("{}/agents.mjs", root.display()))
}

#[test]
fn a_launch_and_a_resume_put_the_workman_plugins_agents_server_beside_workman() {
    let dir = tempfile::tempdir().unwrap();
    let mcp = dir.path().join("mcp");
    let (claude, script) = claude_with_agents(dir.path());
    let (mut sup, _minter) = minting(true, &mcp);
    sup.config.claude_dir = Some(claude);
    sup.tick(&[], &mut free(), NOW);
    assert!(launch_argv(&sup.launcher().take()).contains("--strict-mcp-config"));
    let servers = |text: String| serde_json::from_str::<serde_json::Value>(&text).unwrap()["mcpServers"].clone();
    let want = serde_json::json!({ "command": "node", "args": [script] });
    let launched = servers(config_text(&mcp, "fm-demo-workspace"));
    assert_eq!((&launched["agents"], &launched["workman"]["url"]), (&want, &serde_json::json!("http://127.0.0.1:1/mcp")));

    sup.tick(&[row(SessionState::Working, Some(Status::Busy), Some(1))], &mut free(), NOW + 61 * SECOND);
    sup.tick(&[row(SessionState::Stopped, None, None)], &mut free(), NOW + 100 * SECOND);
    assert!(matches!(sup.launcher().take().as_slice(), [Call::Resume { .. }]));
    let resumed = servers(config_text(&mcp, "fm-demo-workspace"));
    assert_eq!(resumed["agents"], want, "the re-mint keeps it");
    assert_helped(&mcp, "fm-demo-workspace");
    assert_eq!(kept(&sup, "fm-demo-workspace"), "wss_fake2");

    let (mut plain, _) = minting(true, &dir.path().join("plain"));
    plain.tick(&[], &mut free(), NOW);
    let only = servers(config_text(&dir.path().join("plain"), "fm-demo-workspace"));
    assert_eq!(only.as_object().unwrap().keys().collect::<Vec<_>>(), ["workman"], "no claude dir, no plugin server");
}

#[test]
fn workman_not_set_up_launches_strict_with_the_agents_server_and_no_workman_entry_and_no_notice() {
    let dir = tempfile::tempdir().unwrap();
    let mcp = dir.path().join("mcp");
    let (claude, script) = claude_with_agents(dir.path());
    let mut sup = supervisor(true, None, &mcp);
    sup.config.claude_dir = Some(claude);
    *sup.tokens_mut() = FakeTokens(None, None, Err("Workman is not set up: run /workman:setup".into()));
    sup.tick(&[], &mut free(), NOW);
    assert!(launch_argv(&sup.launcher().take()).contains("--strict-mcp-config"));
    let config: serde_json::Value = serde_json::from_str(&config_text(&mcp, "fm-demo-workspace")).unwrap();
    assert_eq!(config["mcpServers"].as_object().unwrap().keys().collect::<Vec<_>>(), ["agents"], "{config}");
    assert_eq!(config["mcpServers"]["agents"], serde_json::json!({ "command": "node", "args": [script] }));
    assert!(sup.take_notices().is_empty() && sup.needs_you().is_empty(), "not a failure: the app says it once");
}

#[test]
fn mint_relaunch_and_resume_replace_the_token_on_the_same_session() {
    let dir = tempfile::tempdir().unwrap();
    let mcp = dir.path().join("mcp");
    let (mut sup, minter) = minting(true, &mcp);
    sup.tick(&[], &mut free(), NOW);
    sup.launcher().take();
    minter.take();
    sup.tick(&[row(SessionState::Working, Some(Status::Busy), Some(1))], &mut free(), NOW + 61 * SECOND);
    minter.take();
    // the session died and its row is stopped: resumed with a fresh token in the same file
    let stopped = row(SessionState::Stopped, None, None);
    sup.tick(std::slice::from_ref(&stopped), &mut free(), NOW + 100 * SECOND);
    assert!(matches!(sup.launcher().take().as_slice(), [Call::Resume { .. }]));
    match minter.take().as_slice() {
        [MintCall::Mint { request, bearer, .. }] => {
            assert!(request.replace);
            assert_eq!(bearer, STATIC);
            assert_eq!(request.claude_session_id, Some(stopped.session_id.clone()), "the uuid --bg --resume keeps");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(sup.minted().get("fm-demo-workspace").unwrap().session_id, "ws-0", "same Workman row");
    let token = kept(&sup, "fm-demo-workspace");
    assert!(token == "wss_fake2" && token != "wss_fake1" && token != STATIC, "{token}");
    assert_helped(&mcp, "fm-demo-workspace");
}

#[test]
fn mint_on_a_new_server_stops_and_resumes_every_minted_session_with_its_new_url() {
    let dir = tempfile::tempdir().unwrap();
    let mcp = dir.path().join("mcp");
    let (mut sup, minter) = minting(true, &mcp);
    sup.tick(&[], &mut free(), NOW);
    sup.launcher().take();
    minter.take();
    let live = row(SessionState::Working, Some(Status::Busy), Some(1));
    sup.tick(std::slice::from_ref(&live), &mut free(), NOW + 61 * SECOND);
    sup.launcher().take();
    minter.take();

    assert!(sup.server_changed(Some("http://127.0.0.1:1")).is_empty(), "minted there");
    sup.tokens_mut().0.as_mut().unwrap().url = "https://workman.example/mcp".into();
    assert!(sup.server_changed(None).is_empty(), "not signed in: nothing to mint with");
    assert_eq!(
        sup.server_changed(Some("https://workman.example")),
        [("fm-demo-workspace".to_string(), Some("http://127.0.0.1:1".to_string()))]
    );
    assert!(sup.server_changed(Some("https://workman.example")).is_empty(), "once");
    sup.tick(std::slice::from_ref(&live), &mut free(), NOW + 70 * SECOND);
    assert!(matches!(sup.launcher().take().as_slice(), [Call::Stop { .. }]), "the live row is stopped");
    let stopped = row(SessionState::Stopped, None, None);
    sup.tick(std::slice::from_ref(&stopped), &mut free(), NOW + 130 * SECOND);
    let calls = sup.launcher().take();
    assert!(matches!(calls.as_slice(), [Call::Resume { session_id, .. }] if *session_id == stopped.session_id), "{calls:?}");
    match minter.take().as_slice() {
        [MintCall::Mint { base, request, .. }] => {
            assert_eq!(base, "https://workman.example");
            assert!(request.replace);
            assert_eq!(request.claude_session_id, Some(stopped.session_id.clone()));
        }
        other => panic!("{other:?}"),
    }
    let text = config_text(&mcp, "fm-demo-workspace");
    assert!(text.contains("https://workman.example/mcp") && kept(&sup, "fm-demo-workspace") == "wss_fake2", "{text}");
    assert_helped(&mcp, "fm-demo-workspace");
}

/// A supervisor of `home` whose sessions file holds `json`, with the static role token.
fn minting_from(home: &Path, json: &str) -> (Supervisor<FakeLauncher, FakeTokens>, FakeMinter) {
    let path = MintedSessions::path(home, "demo");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, json).unwrap();
    let mut desired = DesiredState::workspace_controller("demo", r"C:\demo\cwd", "be the controller");
    desired.started = true;
    let config = LaunchConfig { mcp_dir: home.join("mcp"), supraman_home: Some(home.into()), ..LaunchConfig::default() };
    let minter = FakeMinter::default();
    let sup = Supervisor::new(desired, home, config, FakeLauncher::default(), FakeTokens(static_token(), None, Ok(SIGNED_OUT_URL.into())))
        .with_minter(minter.clone())
        .with_repo_of(fake_repo_of)
        .with_daemon_check(|| None);
    (sup, minter)
}

#[test]
fn a_damaged_sessions_file_is_set_aside_not_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let (mut sup, _) = minting_from(dir.path(), "{not json");
    let notices = sup.take_notices();
    assert!(notices.iter().any(|n| n.contains("demo.sessions.json.damaged")), "{notices:?}");
    let path = MintedSessions::path(dir.path(), "demo");
    let mut aside = path.as_os_str().to_owned();
    aside.push(".damaged");
    assert_eq!(std::fs::read_to_string(&aside).unwrap(), "{not json");
    let live = row(SessionState::Working, Some(Status::Busy), Some(1));
    sup.tick(std::slice::from_ref(&live), &mut free(), NOW + 10 * SECOND);
    assert_eq!(std::fs::read_to_string(&aside).unwrap(), "{not json", "a later save leaves it");
}

#[test]
fn mint_of_unknown_origin_is_reminted_once_unless_its_config_names_the_server() {
    const OLD: &str = r#"{"fm-demo-workspace":{"sessionId":"ws-0","role":"workspace","lastActive":1800000000000}}"#;
    let live = row(SessionState::Working, Some(Status::Busy), Some(1));
    let dir = tempfile::tempdir().unwrap();
    let (mut sup, minter) = minting_from(dir.path(), OLD);
    assert_eq!(sup.server_changed(Some("http://127.0.0.1:1")), [("fm-demo-workspace".to_string(), None)], "no config file: unknown");
    assert!(sup.server_changed(Some("http://127.0.0.1:1")).is_empty(), "once");
    sup.tick(std::slice::from_ref(&live), &mut free(), NOW + 10 * SECOND);
    assert!(matches!(sup.launcher().take().as_slice(), [Call::Stop { .. }]));
    minter.take();
    let stopped = row(SessionState::Stopped, None, None);
    sup.tick(std::slice::from_ref(&stopped), &mut free(), NOW + 70 * SECOND);
    assert!(matches!(sup.launcher().take().as_slice(), [Call::Resume { .. }]));
    let calls = minter.take();
    assert!(matches!(calls.as_slice(), [MintCall::Mint { .. }]), "{calls:?}");
    assert_eq!(sup.minted().get("fm-demo-workspace").unwrap().origin.as_deref(), Some("http://127.0.0.1:1"), "recorded now");

    let dir = tempfile::tempdir().unwrap();
    supraman_claude::write_mcp_config_helped(&dir.path().join("mcp"), "fm-demo-workspace", "http://127.0.0.1:1/mcp", "sm", &[]).unwrap();
    let (mut sup, _) = minting_from(dir.path(), OLD);
    assert!(sup.server_changed(Some("http://127.0.0.1:1")).is_empty(), "its config file already names the server");
    sup.tick(std::slice::from_ref(&live), &mut free(), NOW + 10 * SECOND);
    assert_eq!(sup.launcher().take(), []);
    let saved = std::fs::read_to_string(MintedSessions::path(dir.path(), "demo")).unwrap();
    assert!(saved.contains(r#""origin": "http://127.0.0.1:1""#), "{saved}");
}

#[test]
fn mint_on_another_server_offers_re_mint_and_is_never_revoked_or_patched_on_this_one() {
    let dir = tempfile::tempdir().unwrap();
    let mcp = dir.path().join("mcp");
    let (mut sup, minter) = minting(true, &mcp);
    sup.tick(&[], &mut free(), NOW);
    sup.launcher().take();
    minter.take();
    sup.tokens_mut().0.as_mut().unwrap().url = "https://workman.example/mcp".into();
    // checked while its row was not live: nothing to stop, and then it shows live with the old token
    assert_eq!(sup.server_changed(Some("https://workman.example")).len(), 1);
    sup.tick(&[], &mut free(), NOW + SECOND);
    sup.launcher().take();
    let live = row(SessionState::Working, Some(Status::Busy), Some(1));
    let needs = sup.tick(std::slice::from_ref(&live), &mut free(), NOW + 61 * SECOND);
    let other: Vec<_> = needs.iter().filter(|n| n.kind == NeedsKind::OtherServer).collect();
    let [need] = other.as_slice() else { panic!("{needs:?}") };
    assert_eq!(need.action, Some(Remedy::Remint { id: live.id.clone() }));
    assert!(need.detail.contains("http://127.0.0.1:1") && need.detail.contains("https://workman.example"), "{}", need.detail);
    assert!(minter.take().is_empty(), "no PATCH of the old server's session on the new one");

    sup.retry("fm-demo-workspace");
    sup.tick(std::slice::from_ref(&live), &mut free(), NOW + 62 * SECOND);
    assert!(matches!(sup.launcher().take().as_slice(), [Call::Stop { .. }]), "Re-mint stops it");

    sup.desired.started = false;
    for t in [63, 64] {
        sup.tick(&[another()], &mut free(), NOW + t * SECOND);
    }
    assert!(minter.take().is_empty(), "deleted, its server's grant revoke covers it, not a DELETE here");
    assert!(sup.minted().get("fm-demo-workspace").is_none());
    assert!(!mcp.join("workstate-fm-demo-workspace.json").exists());
}

#[test]
fn mint_without_a_bearer_now_fails_the_launch_with_why() {
    let dir = tempfile::tempdir().unwrap();
    let (mut sup, minter) = minting(true, dir.path());
    sup.tokens_mut().1 = Some("sign in to Workman at http://127.0.0.1:1 again: the server refused the refresh (invalid_grant)".into());
    let needs = sup.tick(&[], &mut free(), NOW);
    assert!(sup.launcher().take().is_empty(), "nothing launched without Workman");
    assert!(minter.take().is_empty());
    assert!(
        needs.iter().any(|n| n.kind == NeedsKind::LaunchFailed && n.detail.contains("sign in to Workman") && n.detail.contains("again")),
        "{needs:?}"
    );
}

#[test]
fn mint_patches_the_claude_session_once_per_mint() {
    let dir = tempfile::tempdir().unwrap();
    let (mut sup, minter) = minting(true, dir.path());
    sup.tick(&[], &mut free(), NOW);
    minter.take();
    let live = row(SessionState::Working, Some(Status::Busy), Some(4242));
    sup.tick(std::slice::from_ref(&live), &mut free(), NOW + 5 * SECOND);
    assert_eq!(
        minter.take(),
        [MintCall::Bind {
            bearer: STATIC.into(),
            session_id: "ws-0".into(),
            claude_session_id: "aaaa0001-0000-4000-8000-000000000001".into(),
        }]
    );
    sup.tick(std::slice::from_ref(&live), &mut free(), NOW + 10 * SECOND);
    sup.tick(std::slice::from_ref(&live), &mut free(), NOW + 70 * SECOND);
    assert_eq!(minter.take(), [], "once");
    assert_eq!(
        sup.minted().get("fm-demo-workspace").unwrap().claude_session_id.as_deref(),
        Some("aaaa0001-0000-4000-8000-000000000001")
    );
    // a resume mints again with the uuid in its body, so there is nothing left to PATCH
    sup.tick(&[row(SessionState::Stopped, None, None)], &mut free(), NOW + 200 * SECOND);
    sup.tick(std::slice::from_ref(&live), &mut free(), NOW + 205 * SECOND);
    let calls = minter.take();
    match calls.as_slice() {
        [MintCall::Mint { request, .. }] => assert_eq!(request.claude_session_id.as_deref(), Some("aaaa0001-0000-4000-8000-000000000001")),
        other => panic!("{other:?}"),
    }
}

#[test]
fn mint_stop_keeps_the_token_and_the_config() {
    let dir = tempfile::tempdir().unwrap();
    let (mut sup, minter) = minting(true, dir.path());
    sup.tick(&[], &mut free(), NOW);
    minter.take();
    let live = row(SessionState::Working, Some(Status::Busy), Some(1));
    sup.tick(std::slice::from_ref(&live), &mut free(), NOW + SECOND);
    minter.take();
    assert!(dir.path().join("workstate-fm-demo-workspace.json").is_file());
    sup.desired.started = false;
    sup.tick(std::slice::from_ref(&live), &mut free(), NOW + 2 * SECOND);
    assert_eq!(sup.launcher().take().last(), Some(&Call::Stop { id: "aaaa0001".into() }));
    assert_eq!(minter.take(), [], "a Stop revokes nothing");
    assert_helped(dir.path(), "fm-demo-workspace");
    assert_eq!(kept(&sup, "fm-demo-workspace"), "wss_fake1");
    assert_eq!(sup.minted().get("fm-demo-workspace").map(|m| m.session_id.as_str()), Some("ws-0"));
    sup.tick(&[row(SessionState::Stopped, None, None)], &mut free(), NOW + 3 * SECOND);
    assert_eq!(minter.take(), [], "nor later, while its claude session is there");
    assert!(sup.take_notices().is_empty());
}

#[test]
fn mint_unwant_and_give_up_keep_the_token_too() {
    let dir = tempfile::tempdir().unwrap();
    let (mut sup, minter) = minting(true, dir.path());
    sup.desired.sessions.push(alpha());
    sup.tick(&[], &mut free(), NOW);
    minter.take();
    // `sm unwant` (R14): retired while the workspace runs
    sup.desired.unwant("fm-demo-alpha");
    sup.tick(&[], &mut free(), NOW + SECOND);
    assert_eq!(minter.take(), [], "an unwant revokes nothing");
    assert_helped(dir.path(), "fm-demo-alpha");
    assert_eq!(kept(&sup, "fm-demo-alpha"), "wss_fake2");
    assert!(dir.path().join("workstate-fm-demo-workspace.json").exists(), "the others keep theirs");
    // the workspace session dies within 20 s of each start until the supervisor gives up
    let mut now = NOW + 30 * SECOND;
    while !sup.memory.get("fm-demo-workspace").is_some_and(|a| a.gave_up) {
        sup.tick(&[], &mut free(), now);
        now += 30 * SECOND;
    }
    let revokes: Vec<MintCall> = minter.take().into_iter().filter(|c| matches!(c, MintCall::Revoke { .. })).collect();
    assert_eq!(revokes, [], "a give-up revokes nothing");
    assert_helped(dir.path(), "fm-demo-workspace");
    assert!(sup.minted().get("fm-demo-workspace").is_some());
}

/// A session of another name, so a snapshot without the minted one is not empty.
fn another() -> Session {
    Session {
        id: "bbbb0002".into(),
        session_id: "bbbb0002-0000-4000-8000-000000000002".into(),
        name: "fm-demo-other".into(),
        ..row(SessionState::Stopped, None, None)
    }
}

/// Launched, bound to its claude session, then stopped: what a delete finds.
fn minted_and_stopped(mcp_dir: &Path) -> (Supervisor<FakeLauncher, FakeTokens>, FakeMinter) {
    let (mut sup, minter) = minting(true, mcp_dir);
    sup.tick(&[], &mut free(), NOW);
    sup.tick(&[row(SessionState::Working, Some(Status::Busy), Some(1))], &mut free(), NOW + SECOND);
    sup.desired.started = false;
    sup.tick(&[row(SessionState::Working, Some(Status::Busy), Some(1))], &mut free(), NOW + 2 * SECOND);
    sup.launcher().take();
    minter.take();
    (sup, minter)
}

#[test]
fn a_deleted_claude_session_revokes_once_two_snapshots_miss_it() {
    let dir = tempfile::tempdir().unwrap();
    let (mut sup, minter) = minted_and_stopped(dir.path());
    sup.tick(&[another()], &mut free(), NOW + 3 * SECOND);
    assert_eq!(minter.take(), [], "one snapshot without it is not yet a delete");
    sup.tick(&[another()], &mut free(), NOW + 4 * SECOND);
    assert_eq!(minter.take(), [MintCall::Revoke { bearer: STATIC.into(), session_id: "ws-0".into() }]);
    assert!(!dir.path().join("workstate-fm-demo-workspace.json").exists(), "the config goes");
    assert_eq!(sup.keeper().kept("fm-demo-workspace"), None, "the kept token goes");
    assert_eq!(sup.minted().get("fm-demo-workspace"), None);
    sup.tick(&[another()], &mut free(), NOW + 5 * SECOND);
    assert_eq!(minter.take(), [], "revoked once");
    assert!(sup.take_notices().is_empty());
}

#[test]
fn a_claude_session_missing_from_the_listing_whose_transcript_remains_is_not_revoked_or_renewed_and_lapses() {
    let dir = tempfile::tempdir().unwrap();
    let (mut sup, minter) = minted_and_stopped(dir.path());
    let claude = dir.path().join("claude");
    let project = claude.join("projects").join("C--demo-cwd");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("aaaa0001-0000-4000-8000-000000000001.jsonl"), "{}
").unwrap();
    sup.config.claude_dir = Some(claude);
    for t in 3..7 {
        sup.tick(&[another()], &mut free(), NOW + t * SECOND);
    }
    assert_eq!(minter.take(), [], "its transcript says it is not deleted");
    assert_helped(dir.path(), "fm-demo-workspace");
    assert_eq!(kept(&sup, "fm-demo-workspace"), "wss_fake1");

    let mut keeping = crate::Keeping::default();
    keeping.signed_in();
    let signed = || crate::Bearer::Signed { url: static_token().unwrap().url, token: STATIC.into() };
    let never = |_: &str, _: &str| -> bool { panic!("no ping of an unlisted token") };
    let checked = keeping.check(sup.keeper(), &minter, signed, NOW + 7 * SECOND, never);
    assert!(checked.renewed.is_empty() && checked.notices.is_empty(), "{checked:?}");
    assert_eq!(minter.take(), [], "an unlisted token is never minted afresh");

    let marked = NOW + 4 * SECOND;
    sup.tick(&[another()], &mut free(), marked + crate::UNLISTED_LAPSE - 1);
    assert!(sup.minted().get("fm-demo-workspace").is_some(), "its expiresAt unknown: kept until UNLISTED_LAPSE after its mark");
    sup.tick(&[another()], &mut free(), marked + crate::UNLISTED_LAPSE);
    assert_eq!(minter.take(), [], "no revoke: it lapsed");
    assert_eq!(sup.minted().get("fm-demo-workspace"), None, "its record goes");
    assert_eq!(sup.keeper().kept("fm-demo-workspace"), None, "its kept token goes");
    assert!(!dir.path().join("workstate-fm-demo-workspace.json").exists(), "its config goes");
    assert!(sup.take_notices().is_empty());
}

#[test]
fn a_record_never_bound_ends_once_its_name_is_unwanted_and_unlisted_long_enough() {
    let dir = tempfile::tempdir().unwrap();
    let (mut sup, minter) = minting(true, dir.path());
    sup.desired.sessions.push(alpha());
    sup.tick(&[], &mut free(), NOW);
    minter.take();
    sup.desired.unwant("fm-demo-alpha");
    let live = row(SessionState::Working, Some(Status::Busy), Some(1));
    let mut now = NOW + SECOND;
    let mut tick = |sup: &mut Supervisor<FakeLauncher, FakeTokens>, observed: &[Session]| {
        sup.tick(observed, &mut free(), now);
        now += SECOND;
    };
    for _ in 1..UNWANTED_UNBOUND_TICKS {
        tick(&mut sup, std::slice::from_ref(&live));
    }
    tick(&mut sup, &[]);
    for _ in 1..UNWANTED_UNBOUND_TICKS {
        tick(&mut sup, std::slice::from_ref(&live));
    }
    assert!(minter.take().iter().all(|c| !matches!(c, MintCall::Revoke { .. })), "an empty snapshot started the count over");
    assert_helped(dir.path(), "fm-demo-alpha");
    tick(&mut sup, std::slice::from_ref(&live));
    let revokes: Vec<MintCall> = minter.take().into_iter().filter(|c| matches!(c, MintCall::Revoke { .. })).collect();
    assert_eq!(revokes, [MintCall::Revoke { bearer: STATIC.into(), session_id: "ws-1".into() }]);
    assert_eq!(sup.minted().get("fm-demo-alpha"), None);
    assert_eq!(sup.keeper().kept("fm-demo-alpha"), None);
    assert!(!dir.path().join("workstate-fm-demo-alpha.json").exists(), "its config goes");
    assert!(sup.minted().get("fm-demo-workspace").is_some(), "the bound one stays");
}

#[test]
fn an_unwanted_unbound_name_a_snapshot_still_lists_keeps_its_token() {
    let dir = tempfile::tempdir().unwrap();
    let (mut sup, minter) = minting(true, dir.path());
    sup.desired.sessions.push(alpha());
    sup.tick(&[], &mut free(), NOW);
    minter.take();
    sup.desired.unwant("fm-demo-alpha");
    let listed = Session { name: "fm-demo-alpha".into(), id: "cccc0003".into(), session_id: "cccc0003-0000-4000-8000-000000000003".into(), ..row(SessionState::Stopped, None, None) };
    for t in 1..=2 * UNWANTED_UNBOUND_TICKS as i64 {
        sup.tick(std::slice::from_ref(&listed), &mut free(), NOW + t * SECOND);
    }
    assert!(minter.take().iter().all(|c| !matches!(c, MintCall::Revoke { .. })));
    assert_eq!(kept(&sup, "fm-demo-alpha"), "wss_fake2");
}

#[test]
fn an_empty_snapshot_revokes_nothing_and_starts_the_count_over() {
    let dir = tempfile::tempdir().unwrap();
    let (mut sup, minter) = minted_and_stopped(dir.path());
    for t in 3..6 {
        sup.tick(&[], &mut free(), NOW + t * SECOND);
    }
    sup.tick(&[another()], &mut free(), NOW + 6 * SECOND);
    sup.tick(&[], &mut free(), NOW + 7 * SECOND);
    sup.tick(&[another()], &mut free(), NOW + 8 * SECOND);
    assert_eq!(minter.take(), [], "never two non-empty snapshots in a row without it");
    assert_helped(dir.path(), "fm-demo-workspace");
    assert_eq!(kept(&sup, "fm-demo-workspace"), "wss_fake1");
}

/// The names `kept.json` under `home`'s `supervisor` marks signed out.
fn signed_out_marks(home: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(home.join("supervisor").join("kept.json")).unwrap_or_default();
    let table: BTreeMap<String, serde_json::Value> = serde_json::from_str(&text).unwrap_or_default();
    table.into_iter().filter(|(_, k)| k["signedOut"] == true).map(|(n, _)| n).collect()
}

#[test]
fn a_sign_out_marks_every_session_minted_on_that_server_and_keeps_its_files() {
    let dir = tempfile::tempdir().unwrap();
    let (mut sup, minter) = minting(true, dir.path());
    sup.desired.sessions.push(alpha());
    sup.tick(&[], &mut free(), NOW);
    minter.take();
    sup.minted.get_mut("fm-demo-alpha").unwrap().origin = Some("https://b.example".into());
    sup.signed_out("http://127.0.0.1:1/");
    assert_eq!(minter.take(), [], "the grant revoke took the tokens");
    assert!(sup.minted().get("fm-demo-workspace").is_some(), "the record stays for the re-mint after the next sign-in");
    assert_helped(dir.path(), "fm-demo-workspace");
    assert_eq!(kept(&sup, "fm-demo-workspace"), "wss_fake1", "the kept token stays for its helper");
    assert_eq!(signed_out_marks(dir.path()), ["fm-demo-workspace"]);
    assert!(sup.minted().get("fm-demo-alpha").is_some(), "another server's stays");
    assert_helped(dir.path(), "fm-demo-alpha");
    assert!(sup.take_notices().is_empty());
}

#[test]
fn a_sign_out_queued_before_a_sign_in_keeps_what_the_new_grant_minted() {
    let dir = tempfile::tempdir().unwrap();
    let (mut sup, minter) = minting(true, dir.path());
    sup.tick(&[], &mut free(), NOW);
    minter.take();
    let queued = crate::sign_in_generation();
    crate::signing_in();
    sup.desired.sessions.push(alpha());
    sup.tick(&[], &mut free(), NOW + SECOND);
    assert!(sup.minted().get("fm-demo-alpha").is_some(), "minted on the new grant");
    minter.take();
    sup.signed_out_as_of("http://127.0.0.1:1/", queued);
    assert_eq!(minter.take(), [], "the grant revoke took the old tokens");
    assert!(sup.minted().get("fm-demo-workspace").is_some(), "the old grant's record stays");
    assert_eq!(kept(&sup, "fm-demo-workspace"), "wss_fake1", "with its kept token");
    assert_eq!(signed_out_marks(dir.path()), ["fm-demo-workspace"], "marked signed out: only the old grant's");
    assert!(sup.minted().get("fm-demo-alpha").is_some(), "the new grant's record stays");
    assert_eq!(kept(&sup, "fm-demo-alpha"), "wss_fake2", "with its token");
    assert_helped(dir.path(), "fm-demo-alpha");
}

#[test]
fn mint_failure_fails_the_launch_without_falling_back_to_the_static_token() {
    let dir = tempfile::tempdir().unwrap();
    let (mut sup, minter) = minting(true, dir.path());
    minter.0.lock().unwrap().mint_errors.push(MintError::Refused { status: 401, reason: "unknown token".into() });
    let needs = sup.tick(&[], &mut free(), NOW);
    assert_eq!(sup.launcher().take(), [], "nothing launched");
    assert_eq!(needs.len(), 1);
    assert_eq!(needs[0].kind, NeedsKind::LaunchFailed);
    assert_eq!(needs[0].action, Some(Remedy::Retry));
    assert!(
        needs[0].detail.contains("cannot mint a Workman token for fm-demo-workspace: Workman answered HTTP 401: unknown token"),
        "{}",
        needs[0].detail
    );
    assert!(!needs[0].detail.contains(STATIC));
    assert!(!dir.path().join("workstate-fm-demo-workspace.json").exists(), "no file");
    // the server is back: Retry launches with a minted token
    sup.retry("fm-demo-workspace");
    sup.tick(&[], &mut free(), NOW + SECOND);
    assert!(!launch_argv(&sup.launcher().take()).contains(STATIC));
    assert_eq!(kept(&sup, "fm-demo-workspace"), "wss_fake1");
    assert_helped(dir.path(), "fm-demo-workspace");
}

#[test]
fn mint_a_failed_revoke_is_logged_not_a_notice_and_the_file_still_goes() {
    let dir = tempfile::tempdir().unwrap();
    let (mut sup, minter) = minted_and_stopped(dir.path());
    minter.0.lock().unwrap().revoke_error = Some(MintError::Unreachable { base: "http://127.0.0.1:1".into(), reason: "refused".into() });
    sup.take_notices();
    for t in [3, 4] {
        sup.tick(&[another()], &mut free(), NOW + t * SECOND);
    }
    let notices = sup.take_notices();
    assert!(notices.is_empty(), "{notices:?}");
    assert!(!dir.path().join("workstate-fm-demo-workspace.json").exists());
    assert_eq!(sup.keeper().kept("fm-demo-workspace"), None);
}

#[test]
fn mint_sessions_survive_a_restart_and_are_revoked_once_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let (mut sup, minter) = minting(true, &dir.path().join("mcp"));
    sup.config.supraman_home = Some(home.clone());
    let config = sup.config.clone();
    let mut sup = Supervisor::new(sup.desired.clone(), &home, config.clone(), FakeLauncher::default(), FakeTokens(static_token(), None, Ok(SIGNED_OUT_URL.into())))
        .with_minter(minter.clone());
    assert_eq!(sup.minted().get("fm-demo-workspace"), None, "nothing saved yet");
    sup.tick(&[], &mut free(), NOW);
    minter.take();
    let saved = std::fs::read_to_string(MintedSessions::path(&home, "demo")).unwrap();
    assert!(saved.contains("ws-0") && !saved.contains("wss_") && !saved.contains(STATIC), "{saved}");
    let live = row(SessionState::Working, Some(Status::Busy), Some(1));
    sup.tick(std::slice::from_ref(&live), &mut free(), NOW + SECOND);
    minter.take();
    // the app restarts, then the workspace is stopped, and later its session deleted
    let mut desired = sup.desired.clone();
    desired.started = false;
    let mut sup = Supervisor::new(desired, &home, config, FakeLauncher::default(), FakeTokens(static_token(), None, Ok(SIGNED_OUT_URL.into()))).with_minter(minter.clone());
    sup.tick(std::slice::from_ref(&live), &mut free(), NOW + 2 * SECOND);
    assert_eq!(minter.take(), [], "the Stop keeps it");
    for t in [3, 4] {
        sup.tick(&[another()], &mut free(), NOW + t * SECOND);
    }
    assert_eq!(minter.take(), [MintCall::Revoke { bearer: STATIC.into(), session_id: "ws-0".into() }]);
    assert_eq!(std::fs::read_to_string(MintedSessions::path(&home, "demo")).unwrap().trim(), "{}");
}

#[test]
fn a_live_supervised_session_keeps_its_token_across_quit_and_start() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let (mut sup, minter) = minting(true, &dir.path().join("mcp"));
    sup.config.supraman_home = Some(home.clone());
    let config = sup.config.clone();
    let started = || Supervisor::new(sup.desired.clone(), &home, config.clone(), FakeLauncher::default(), FakeTokens(static_token(), None, Ok(SIGNED_OUT_URL.into()))).with_minter(minter.clone());
    let mut before = started();
    before.tick(&[], &mut free(), NOW);
    let live = row(SessionState::Working, Some(Status::Busy), Some(7));
    before.tick(std::slice::from_ref(&live), &mut free(), NOW + 61 * SECOND);
    let minted = before.minted().get("fm-demo-workspace").cloned().expect("minted at launch");
    minter.take();
    drop(before);

    let mut after = started();
    after.tick(std::slice::from_ref(&live), &mut free(), NOW + 120 * SECOND);
    let calls: Vec<MintCall> = minter.take().into_iter().filter(|c| !matches!(c, MintCall::Bind { .. })).collect();
    assert_eq!(calls, [], "the quit and the start neither revoke nor mint a live session's token");
    assert_eq!(after.minted().get("fm-demo-workspace"), Some(&minted));
}

#[test]
fn mint_a_live_session_idle_for_days_raises_no_row_and_is_never_restarted() {
    const HOUR: Millis = 3_600 * SECOND;
    let dir = tempfile::tempdir().unwrap();
    let (mut sup, minter) = minting(true, dir.path());
    sup.tick(&[], &mut free(), NOW);
    let busy = row(SessionState::Working, Some(Status::Busy), Some(7));
    let idle = row(SessionState::Working, Some(Status::Idle), Some(7));
    sup.tick(std::slice::from_ref(&busy), &mut free(), NOW + 61 * SECOND);
    sup.launcher().take();
    minter.take();
    for at in [73 * HOUR, 200 * HOUR] {
        assert_eq!(sup.tick(std::slice::from_ref(&idle), &mut free(), NOW + at), [], "the app's Keeping keeps its token fresh");
    }
    sup.retry("fm-demo-workspace");
    sup.tick(std::slice::from_ref(&idle), &mut free(), NOW + 201 * HOUR);
    assert_eq!(sup.launcher().take(), [], "a plain Retry never stops a session");
    assert_eq!(minter.take(), []);
}

#[test]
fn mint_nothing_without_a_role_token() {
    let dir = tempfile::tempdir().unwrap();
    let minter = FakeMinter::default();
    let mut sup = supervisor(true, None, dir.path()).with_minter(minter.clone());
    sup.tick(&[], &mut free(), NOW);
    assert_eq!(sup.launcher().take().len(), 1);
    assert_eq!(minter.take(), []);
    assert_helped(dir.path(), "fm-demo-workspace");
    assert_eq!(kept(&sup, "fm-demo-workspace"), "", "no token is kept");
}

#[test]
fn signed_out_launches_with_the_helped_config_under_strict() {
    let dir = tempfile::tempdir().unwrap();
    let mut sup = supervisor(true, None, dir.path());
    sup.tick(&[], &mut free(), NOW);
    let argv = match sup.launcher().take().as_slice() {
        [Call::Launch { argv, .. }] => argv.clone(),
        other => panic!("{other:?}"),
    };
    let config = argv.iter().position(|a| a == "--mcp-config").map(|i| argv[i + 1].clone());
    assert_eq!(config.as_deref().map(Path::new), Some(dir.path().join("workstate-fm-demo-workspace.json").as_path()), "{argv:?}");
    assert!(argv.iter().any(|a| a == "--strict-mcp-config"), "never the user-scope server: {argv:?}");
    assert_eq!(supraman_claude::mcp_config_url(dir.path(), "fm-demo-workspace").as_deref(), Some(SIGNED_OUT_URL));
    assert_helped(dir.path(), "fm-demo-workspace");
}

#[test]
fn a_sign_in_keeps_a_token_for_a_live_session_launched_signed_out_without_stopping_it() {
    let dir = tempfile::tempdir().unwrap();
    let minter = FakeMinter::default();
    let mut sup = supervisor(true, None, dir.path()).with_minter(minter.clone());
    sup.tick(&[], &mut free(), NOW);
    sup.launcher().take();
    let live = row(SessionState::Working, Some(Status::Busy), Some(1));
    sup.server_changed(None);
    sup.tick(std::slice::from_ref(&live), &mut free(), NOW + SECOND);
    assert_eq!(minter.take(), [], "signed out: nothing to mint with");

    *sup.tokens_mut() = FakeTokens(static_token(), None, Ok(SIGNED_OUT_URL.into()));
    assert!(sup.server_changed(Some("http://127.0.0.1:1")).is_empty());
    sup.tick(std::slice::from_ref(&live), &mut free(), NOW + 2 * SECOND);
    assert_eq!(sup.launcher().take(), [], "the live session is not stopped");
    match minter.take().as_slice() {
        [MintCall::Mint { request, .. }] => {
            assert_eq!(request.claude_session_id.as_deref(), Some(live.session_id.as_str()));
            assert!(request.replace);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(kept(&sup, "fm-demo-workspace"), "wss_fake1", "its helper prints the minted token");
    assert!(sup.minted().get("fm-demo-workspace").is_some(), "recorded, so a deletion revokes it");
    assert_helped(dir.path(), "fm-demo-workspace");

    sup.tick(std::slice::from_ref(&live), &mut free(), NOW + 3 * SECOND);
    assert!(sup.server_changed(Some("http://127.0.0.1:1")).is_empty());
    sup.tick(std::slice::from_ref(&live), &mut free(), NOW + 4 * SECOND);
    assert_eq!(minter.take(), [], "once per sign-in");
}

#[test]
fn session_gets_its_env_isolation_and_permission_mode_through_settings() {
    let dir = tempfile::tempdir().unwrap();
    let mut sup = supervisor(true, None, dir.path());
    sup.config.supraman_home = Some(r"C:\fm\home".into());
    sup.config.bg_isolation_none = true;
    sup.config.allowed_tools = vec!["Bash(sm:*)".into(), "Bash(C:/fm/bin/sm.exe:*)".into()];
    sup.desired.sessions[0].permission_mode = "default".into();
    sup.tick(&[], &mut free(), NOW);
    let argv = match sup.launcher().take().as_slice() {
        [Call::Launch { argv, .. }] => argv.clone(),
        other => panic!("{other:?}"),
    };
    let at = |flag: &str| argv.iter().position(|a| a == flag).map(|i| argv[i + 1].clone());
    assert_eq!(at("--permission-mode").as_deref(), Some("default"));
    let tools = argv.iter().position(|a| a == "--allowedTools").unwrap();
    assert_eq!(argv[tools + 1..tools + 4], ["Bash(sm:*)", "Bash(C:/fm/bin/sm.exe:*)", "--"]);
    let mut settings: serde_json::Value = serde_json::from_str(&at("--settings").unwrap()).unwrap();
    let path = settings["env"].as_object_mut().unwrap().remove("PATH").expect("the session's PATH");
    let first = path.as_str().unwrap().split(';').next().unwrap().to_string();
    assert_eq!(first, supraman_core::install::exe_dir().unwrap().display().to_string(), "the sm beside this exe comes first");
    assert_eq!(
        settings,
        serde_json::json!({
            "env": {
                "SUPRAMAN_HOME": r"C:\fm\home", "SUPRAMAN_ROLE": "workspace", "SUPRAMAN_WORKSPACE": "demo",
                "SUPRAMAN_SESSION": "fm-demo-workspace",
            },
            "worktree": { "bgIsolation": "none" }
        })
    );
    assert_eq!(LaunchConfig::default().settings_with("demo", "fm-demo-workspace", "workspace", None, &[]), None);
}

fn workspace(name: &str, mcp_dir: &Path) -> Supervisor<FakeLauncher, FakeTokens> {
    let mut desired = DesiredState::workspace_controller(name, r"C:\demo\cwd", "be the controller");
    desired.started = true;
    let config = LaunchConfig {
        mcp_dir: mcp_dir.into(),
        ..LaunchConfig::default()
    };
    Supervisor::new(desired, mcp_dir, config, FakeLauncher::default(), FakeTokens(None, None, Ok(SIGNED_OUT_URL.into())))
}

fn live(name: &str, n: u32, state: SessionState, status: Status) -> Session {
    Session {
        id: format!("bbbb000{n}"),
        session_id: format!("bbbb000{n}-0000-4000-8000-000000000000"),
        name: name.into(),
        cwd: r"C:\demo\cwd".into(),
        kind: Kind::Background,
        state: Some(state),
        status: Some(status),
        waiting_for: None,
        pid: Some(100 + n),
        started_at: Some(NOW),
    }
}

#[test]
fn two_started_workspaces_never_exceed_the_shared_budget_together() {
    let dir = tempfile::tempdir().unwrap();
    let launched = |sups: &[Supervisor<FakeLauncher, FakeTokens>]| -> Vec<usize> {
        sups.iter().map(|s| s.launcher().take().len()).collect()
    };
    let mut sups = [workspace("alpha", dir.path()), workspace("beta", dir.path())];
    // budget 1, both need a launch: only alpha goes
    tick_all(sups.iter_mut(), &[], &[], 1, NOW);
    assert_eq!(launched(&sups), [1, 0]);
    // alpha's row has not shown yet: it still holds the slot, beta waits
    tick_all(sups.iter_mut(), &[], &[], 1, NOW + 2 * SECOND);
    assert_eq!(launched(&sups), [0, 0]);
    // alpha is working: beta waits
    let alpha_busy = live("fm-alpha-workspace", 1, SessionState::Working, Status::Busy);
    tick_all(sups.iter_mut(), &[], std::slice::from_ref(&alpha_busy), 1, NOW + 4 * SECOND);
    assert_eq!(launched(&sups), [0, 0]);
    // alpha's turn ended (done + live pid = idle): the slot frees, beta goes
    let alpha_idle = live("fm-alpha-workspace", 1, SessionState::Done, Status::Idle);
    tick_all(sups.iter_mut(), &[], std::slice::from_ref(&alpha_idle), 1, NOW + 30 * SECOND);
    assert_eq!(launched(&sups), [0, 1]);
    // the same snapshot again: beta, settling, holds the slot
    tick_all(sups.iter_mut(), &[], std::slice::from_ref(&alpha_idle), 1, NOW + 32 * SECOND);
    assert_eq!(launched(&sups), [0, 0]);

    // budget 2: both in one tick
    let mut sups = [workspace("alpha", dir.path()), workspace("beta", dir.path())];
    tick_all(sups.iter_mut(), &[], &[], 2, NOW);
    assert_eq!(launched(&sups), [1, 1]);

    // budget 0 pauses every launch; a working session of a registered workspace nobody here supervises counts too
    let mut sups = [workspace("alpha", dir.path()), workspace("beta", dir.path())];
    tick_all(sups.iter_mut(), &["alpha", "beta", "gamma"], &[], 0, NOW);
    assert_eq!(launched(&sups), [0, 0]);
    let other = live("fm-gamma-workspace", 3, SessionState::Working, Status::Busy);
    tick_all(sups.iter_mut(), &["alpha", "beta", "gamma"], std::slice::from_ref(&other), 1, NOW + SECOND);
    assert_eq!(launched(&sups), [0, 0]);
    let needs = tick_all(sups.iter_mut(), &["alpha", "beta", "gamma"], std::slice::from_ref(&other), 2, NOW + 2 * SECOND);
    assert_eq!(launched(&sups), [1, 0]);
    assert!(needs.is_empty(), "a cap wait is not an Inbox row: {needs:?}");
}

#[test]
fn another_homes_busy_sessions_take_no_slot() {
    let dir = tempfile::tempdir().unwrap();
    let launched = |sups: &[Supervisor<FakeLauncher, FakeTokens>]| -> Vec<usize> {
        sups.iter().map(|s| s.launcher().take().len()).collect()
    };
    let foreign = [
        live("fm-nomos-workspace", 7, SessionState::Working, Status::Busy),
        live("fm-nomos-nomos", 8, SessionState::Working, Status::Busy),
    ];
    assert_eq!(Budget::new(1, &foreign, &[], NOW).free, 0, "unfiltered, they would fill the budget");

    // the limit at 1 while another home is mid-turn: alpha goes, then beta
    let mut sups = [workspace("alpha", dir.path()), workspace("beta", dir.path())];
    tick_all(sups.iter_mut(), &["alpha", "beta"], &foreign, 1, NOW);
    assert_eq!(launched(&sups), [1, 0]);
    let mut rows = foreign.to_vec();
    rows.push(live("fm-alpha-workspace", 1, SessionState::Working, Status::Busy));
    tick_all(sups.iter_mut(), &["alpha", "beta"], &rows, 1, NOW + 30 * SECOND);
    assert_eq!(launched(&sups), [0, 0], "alpha's own busy row holds the slot");
    rows.pop();
    rows.push(live("fm-alpha-workspace", 1, SessionState::Done, Status::Idle));
    tick_all(sups.iter_mut(), &["alpha", "beta"], &rows, 1, NOW + 60 * SECOND);
    assert_eq!(launched(&sups), [0, 1]);

    // a session this home wants under another workspace's name is its own
    let mut sups = [workspace("alpha", dir.path()), workspace("beta", dir.path())];
    sups[1].desired.sessions[0].name = "fm-nomos-workspace".into();
    tick_all(sups.iter_mut(), &["alpha", "beta"], &foreign[..1], 1, NOW);
    assert_eq!(launched(&sups), [0, 0]);
}

#[test]
fn hand_sessions_are_never_wanted_respawned_stopped_or_counted() {
    let dir = tempfile::tempdir().unwrap();
    let hand = |name: &str, n: u32, pid: Option<u32>| Session {
        kind: Kind::Interactive,
        pid,
        ..live(name, n, SessionState::Working, Status::Busy)
    };
    let busy = [hand("hand-cwd-1", 1, Some(101)), hand("René's review", 2, Some(102))];
    assert_eq!(Budget::new(1, &busy, &["demo"], NOW).free, 1, "a busy hand session takes no slot");
    let gone = [hand("hand-cwd-1", 1, None), hand("René's review", 2, None)];

    let mut sup = workspace("demo", dir.path());
    let controller = live("fm-demo-workspace", 9, SessionState::Done, Status::Idle);
    for (i, hands) in [&busy, &gone, &busy, &gone].into_iter().enumerate() {
        let rows: Vec<Session> = hands.iter().cloned().chain([controller.clone()]).collect();
        let needs = tick_all(std::iter::once(&mut sup), &["demo"], &rows, 1, NOW + i as i64 * 60 * SECOND);
        assert!(needs.is_empty(), "tick {i}: {needs:?}");
        assert_eq!(sup.launcher().take(), [], "tick {i}: no hand session is launched, stopped or resumed");
    }
    assert!(sup.desired.sessions.iter().all(|w| w.name.starts_with(FM_PREFIX)), "{:?}", sup.desired.sessions);
    assert!(!is_foreign("hand-cwd-1", ["demo"].into_iter(), std::iter::empty()), "not another home's session either");
}

#[test]
fn a_workspace_session_idling_on_the_relay_frees_its_slot() {
    let dir = tempfile::tempdir().unwrap();
    let mut sups = [workspace("alpha", dir.path()), workspace("beta", dir.path())];
    tick_all(sups.iter_mut(), &[], &[], 1, NOW);
    assert_eq!(sups.iter().map(|s| s.launcher().take().len()).collect::<Vec<_>>(), [1, 0]);
    // just started, no prompt taken yet: `status=idle` on a live pid still holds the slot while settling
    let fresh = live("fm-alpha-workspace", 1, SessionState::Working, Status::Idle);
    tick_all(sups.iter_mut(), &[], std::slice::from_ref(&fresh), 1, NOW + 2 * SECOND);
    assert_eq!(sups.iter().map(|s| s.launcher().take().len()).collect::<Vec<_>>(), [0, 0]);
    // its turn ended, `state` stayed `working`: the slot is free once it settled
    tick_all(sups.iter_mut(), &[], std::slice::from_ref(&fresh), 1, NOW + 30 * SECOND);
    assert_eq!(sups.iter().map(|s| s.launcher().take().len()).collect::<Vec<_>>(), [0, 1]);
}

#[test]
fn stopped_row_is_resumed_from_its_cwd_with_its_session_id() {
    let dir = tempfile::tempdir().unwrap();
    let mut sup = supervisor(true, None, dir.path());
    sup.tick(&[row(SessionState::Stopped, None, None)], &mut free(), NOW);
    assert_eq!(
        sup.launcher().take(),
        [Call::Resume {
            session_id: "aaaa0001-0000-4000-8000-000000000001".into(),
            cwd: r"C:\demo\cwd".into(),
            prompt: None,
        }]
    );
}

fn with_relay(sup: &mut Supervisor<FakeLauncher, FakeTokens>) {
    let relay = crate::relay_session("demo", r"C:\demo\cwd", r"C:\fm\home\relay\demo", "be the relay");
    sup.desired.sessions.insert(0, relay);
}

#[test]
fn the_relay_launches_first_with_its_add_dir_and_never_takes_a_budget_slot() {
    let dir = tempfile::tempdir().unwrap();
    let mut sup = supervisor(true, None, dir.path());
    with_relay(&mut sup);
    let mut budget = Budget { max: 1, free: 1 };
    sup.tick(&[], &mut budget, NOW);
    let calls = sup.launcher().take();
    let names: Vec<&str> = calls
        .iter()
        .map(|c| match c {
            Call::Launch { name, .. } => name.as_str(),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(names, ["fm-demo-relay", "fm-demo-workspace"], "relay first");
    let Call::Launch { argv, .. } = &calls[0] else { unreachable!() };
    let at = argv.iter().position(|a| a == "--add-dir").expect("--add-dir");
    assert_eq!(argv[at + 1], r"C:\fm\home\relay\demo");
    assert_eq!(budget.free, 0, "only the workspace session took the slot");
    assert_eq!(sup.starting(&[], NOW + SECOND), 1, "a settling relay holds no slot");

    // budget 0: the relay still goes, the workspace waits
    let mut sup = supervisor(true, None, dir.path());
    with_relay(&mut sup);
    sup.tick(&[], &mut Budget { max: 0, free: 0 }, NOW);
    assert!(matches!(sup.launcher().take().as_slice(), [Call::Launch { name, .. }] if name == "fm-demo-relay"));
}

#[test]
fn armed_relays_leave_the_budget_to_the_workspace_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let mut sups = [workspace("alpha", dir.path()), workspace("beta", dir.path())];
    for (s, name) in sups.iter_mut().zip(["alpha", "beta"]) {
        let relay = crate::relay_session(name, r"C:\demo\cwd", r"C:\fm\relay", "be the relay");
        s.desired.sessions.insert(0, relay);
    }
    tick_all(sups.iter_mut(), &[], &[], 1, NOW);
    let first: Vec<Vec<Call>> = sups.iter().map(|s| s.launcher().take()).collect();
    assert_eq!(first[0].len(), 2, "alpha: relay and workspace {first:?}");
    assert_eq!(first[1].len(), 1, "beta: relay only {first:?}");
    let rows = [
        live("fm-alpha-relay", 1, SessionState::Working, Status::Busy),
        live("fm-beta-relay", 2, SessionState::Working, Status::Busy),
        live("fm-alpha-workspace", 3, SessionState::Done, Status::Idle),
    ];
    tick_all(sups.iter_mut(), &[], &rows, 1, NOW + 30 * SECOND);
    let second: Vec<usize> = sups.iter().map(|s| s.launcher().take().len()).collect();
    assert_eq!(second, [0, 1], "beta's workspace session goes once alpha's is idle");
}

#[test]
fn a_resumed_relay_is_told_to_rearm() {
    let dir = tempfile::tempdir().unwrap();
    let mut sup = supervisor(true, None, dir.path());
    with_relay(&mut sup);
    let mut relay = row(SessionState::Stopped, None, None);
    relay.name = "fm-demo-relay".into();
    let live = row(SessionState::Done, Some(Status::Idle), Some(5));
    sup.tick(&[relay, live], &mut Budget { max: 0, free: 0 }, NOW);
    match sup.launcher().take().as_slice() {
        [Call::Resume { prompt: Some(p), .. }] => assert!(p.contains("Re-arm"), "{p}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn stop_stops_the_relay_after_the_other_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let mut sup = supervisor(false, None, dir.path());
    with_relay(&mut sup);
    let mut relay = row(SessionState::Working, Some(Status::Busy), Some(2));
    relay.name = "fm-demo-relay".into();
    relay.id = "aaaa0002".into();
    sup.tick(&[relay, row(SessionState::Done, Some(Status::Idle), Some(1))], &mut free(), NOW);
    assert_eq!(
        sup.launcher().take(),
        [Call::Stop { id: "aaaa0001".into() }, Call::Stop { id: "aaaa0002".into() }]
    );
}

#[test]
fn idle_done_with_pid_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let mut sup = supervisor(true, None, dir.path());
    assert_eq!(sup.tick(&[row(SessionState::Done, Some(Status::Idle), Some(27304))], &mut free(), NOW), []);
    assert_eq!(sup.launcher().take(), []);
}

#[test]
fn stop_stops_live_and_never_removes() {
    let dir = tempfile::tempdir().unwrap();
    let mut sup = supervisor(false, None, dir.path());
    sup.tick(&[row(SessionState::Working, Some(Status::Busy), Some(1))], &mut free(), NOW);
    assert_eq!(sup.launcher().take(), [Call::Stop { id: "aaaa0001".into() }]);
    sup.tick(&[row(SessionState::Stopped, None, None)], &mut free(), NOW + SECOND);
    assert_eq!(sup.launcher().take(), []);
    for later in [3, 60, 3_600] {
        sup.tick(&[row(SessionState::Stopped, None, None)], &mut free(), NOW + later * SECOND);
        assert_eq!(sup.launcher().take(), [], "{later} s after the stop nothing restarts it");
    }
}

#[test]
fn trust_refusal_shows_in_the_same_tick_and_waits_for_retry() {
    let dir = tempfile::tempdir().unwrap();
    let mut sup = supervisor(true, None, dir.path());
    sup.launcher.errors.borrow_mut().push(ClaudeError::TrustNeeded { cwd: r"C:\demo\cwd".into() });
    let needs = sup.tick(&[], &mut free(), NOW);
    assert_eq!(needs.len(), 1);
    assert_eq!(needs[0].kind, NeedsKind::TrustNeeded);
    assert_eq!(needs[0].since, NOW);
    assert_eq!(
        needs[0].action,
        Some(Remedy::RunInTerminal { cwd: r"C:\demo\cwd".into(), command: "claude".into() })
    );
    sup.launcher().take();
    let later = sup.tick(&[], &mut free(), NOW + 3_600 * SECOND);
    assert_eq!(later[0].since, NOW, "since is kept");
    assert_eq!(sup.launcher().take(), [], "never retried automatically");
    sup.retry("fm-demo-workspace");
    assert_eq!(sup.tick(&[], &mut free(), NOW + 3_601 * SECOND), []);
    assert_eq!(sup.launcher().take().len(), 1);
}

fn unreachable() -> ClaudeError {
    ClaudeError::DaemonUnreachable {
        code: Some(1),
        stderr: "Starting background service… Couldn't reach the background service (background service did not become reachable within 45s) — run 'claude daemon status'".into(),
    }
}

/// An unelevated Supraman facing an elevated service gets one row saying why, at once, not five 45 s failures.
#[test]
fn an_unreachable_service_supraman_cannot_reach_holds_with_one_row() {
    let dir = tempfile::tempdir().unwrap();
    let mut sup = supervisor(true, None, dir.path()).with_daemon_check(|| Some("runs as administrator".into()));
    sup.launcher.errors.borrow_mut().push(unreachable());
    let needs = sup.tick(&[], &mut free(), NOW);
    assert_eq!(sup.launcher().take().len(), 1);
    assert_eq!(needs.len(), 1, "{needs:?}");
    assert_eq!((needs[0].kind, needs[0].detail.as_str(), needs[0].since), (NeedsKind::LaunchFailed, "runs as administrator", NOW));
    assert_eq!(needs[0].action, Some(Remedy::RestartElevated));
    assert_eq!(sup.memory.get("fm-demo-workspace").map(|a| a.failures).unwrap_or(0), 0, "not a counted failure");
    for t in 1..600 {
        assert_eq!(sup.tick(&[], &mut free(), NOW + t * SECOND).len(), 1);
    }
    assert_eq!(sup.launcher().take(), [], "held: nothing launched");
    // the app's check finds it reachable again
    sup.hold(None, NOW + 600 * SECOND);
    assert_eq!(sup.tick(&[], &mut free(), NOW + 601 * SECOND), []);
    assert_eq!(sup.launcher().take().len(), 1);
}

/// The app's check before any launch: held from the start, never launched.
#[test]
fn a_hold_from_the_apps_check_comes_before_any_launch() {
    let dir = tempfile::tempdir().unwrap();
    let mut sup = supervisor(true, None, dir.path());
    sup.hold(Some("runs as administrator".into()), NOW);
    let needs = sup.tick(&[], &mut free(), NOW);
    assert_eq!(sup.launcher().take(), []);
    assert_eq!(needs.len(), 1);
    assert_eq!(needs[0].detail, "runs as administrator");
    sup.retry(&needs[0].name);
    assert_eq!(sup.held(), None, "Retry tries again");
    sup.tick(&[], &mut free(), NOW + SECOND);
    assert_eq!(sup.launcher().take().len(), 1);
}

/// Unreachable for another reason (the check says elevation is not why): counted and backed off.
#[test]
fn an_unreachable_service_with_no_elevation_mismatch_backs_off_as_before() {
    let dir = tempfile::tempdir().unwrap();
    let mut sup = supervisor(true, None, dir.path()).with_daemon_check(|| None);
    sup.launcher.errors.borrow_mut().push(unreachable());
    let needs = sup.tick(&[], &mut free(), NOW);
    assert_eq!(sup.held(), None);
    assert_eq!(sup.memory.get("fm-demo-workspace").unwrap().failures, 1);
    assert!(needs[0].detail.starts_with("failure 1 of 5: claude failed (exit 1): Starting background service…"), "{needs:?}");
}

#[test]
fn repeated_failures_back_off_then_give_up_until_retry() {
    let dir = tempfile::tempdir().unwrap();
    let mut sup = supervisor(true, None, dir.path());
    let fail = || ClaudeError::CliFailed { code: Some(1), stderr: "boom".into() };
    let mut now = NOW;
    let mut attempts = Vec::new();
    for _ in 0..5 {
        sup.launcher.errors.borrow_mut().push(fail());
    }
    // Tick every second for an hour: launches only at 0, +5 s, +35 s, +155 s, +755 s.
    while now < NOW + 3_600 * SECOND {
        sup.tick(&[], &mut free(), now);
        if !sup.launcher().take().is_empty() {
            attempts.push((now - NOW) / SECOND);
        }
        now += SECOND;
    }
    assert_eq!(attempts, [0, 5, 35, 155, 755]);
    let needs = sup.needs_you();
    assert_eq!(needs.len(), 1);
    assert_eq!(needs[0].kind, NeedsKind::GaveUp);
    assert_eq!(needs[0].since, NOW + 755 * SECOND);
    assert_eq!(needs[0].detail, "gave up after 5 failures; last: claude failed (exit 1): boom");
    sup.retry("fm-demo-workspace");
    sup.tick(&[], &mut free(), now);
    assert_eq!(sup.launcher().take().len(), 1);
}

#[test]
fn a_resume_that_starts_a_copy_is_a_failure() {
    let dir = tempfile::tempdir().unwrap();
    let mut sup = supervisor(true, None, dir.path());
    *sup.launcher.output.borrow_mut() = Some("backgrounded · bbbb0002 (idle — send a prompt to start)".into());
    let needs = sup.tick(&[row(SessionState::Stopped, None, None)], &mut free(), NOW);
    assert_eq!(sup.launcher().take().len(), 1);
    assert_eq!(needs[0].kind, NeedsKind::LaunchFailed);
    assert!(needs[0].detail.contains("started a copy bbbb0002"), "{}", needs[0].detail);
    assert_eq!(resumed("backgrounded · aaaa0001 · fm-demo-workspace", "aaaa0001"), Resumed::InPlace);
    assert_eq!(resumed("backgrounded · bbbb0002 · fm-demo-workspace", "aaaa0001"), Resumed::Copy("bbbb0002".into()));
    assert_eq!(resumed("", "aaaa0001"), Resumed::Unknown);
    assert_eq!(resumed("backgrounded · resumed fm-demo-workspace", "aaaa0001"), Resumed::Unknown);
    assert_eq!(resumed("Session resumed in the background", "aaaa0001"), Resumed::Unknown);
}

#[test]
fn a_resume_with_unrecognised_output_is_settled_by_the_next_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let name = "fm-demo-workspace";
    let stopped = row(SessionState::Stopped, None, None);
    let mut copy = row(SessionState::Working, Some(Status::Busy), Some(9));
    copy.id = "bbbb0002".into();
    let woke = row(SessionState::Working, Some(Status::Busy), Some(9));

    let mut sup = supervisor(true, None, dir.path());
    *sup.launcher.output.borrow_mut() = Some("Resumed in the background".into());
    assert_eq!(sup.tick(std::slice::from_ref(&stopped), &mut free(), NOW), []);
    assert!(matches!(sup.launcher().take().as_slice(), [Call::Resume { .. }]));
    assert!(sup.take_notices().is_empty(), "an unknown resume output is logged");
    sup.tick(std::slice::from_ref(&copy), &mut free(), NOW + 2 * SECOND);
    let a = sup.memory.get(name).unwrap();
    assert_eq!((a.failures, a.started_at), (1, None));
    assert_eq!(a.last_error.as_deref(), Some("the resume of aaaa0001 started a copy bbbb0002 (the daemon still counted it running)"));

    let mut sup = supervisor(true, None, dir.path());
    *sup.launcher.output.borrow_mut() = Some(String::new());
    sup.tick(std::slice::from_ref(&stopped), &mut free(), NOW);
    sup.tick(&[], &mut free(), NOW + SECOND);
    sup.tick(std::slice::from_ref(&woke), &mut free(), NOW + 2 * SECOND);
    sup.tick(std::slice::from_ref(&copy), &mut free(), NOW + 3 * SECOND);
    assert_eq!(sup.memory.get(name).unwrap().failures, 0, "confirmed by its own id: a later row is not this resume's");
}

#[test]
fn a_stop_that_keeps_failing_is_an_inbox_row_until_it_stops() {
    let dir = tempfile::tempdir().unwrap();
    let mut sup = supervisor(false, None, dir.path());
    let live = row(SessionState::Working, Some(Status::Busy), Some(1));
    let fail = || ClaudeError::CliFailed { code: Some(1), stderr: "boom".into() };
    sup.launcher.stop_errors.borrow_mut().extend([fail(), fail(), fail(), fail()]);
    for t in 0..2 {
        assert_eq!(sup.tick(std::slice::from_ref(&live), &mut free(), NOW + t * SECOND), [], "not after {}", t + 1);
    }
    let needs = sup.tick(std::slice::from_ref(&live), &mut free(), NOW + 2 * SECOND);
    assert_eq!(needs.len(), 1, "{needs:?}");
    assert_eq!((needs[0].kind, needs[0].since), (NeedsKind::StopFailed, NOW));
    assert_eq!(needs[0].action, Some(Remedy::Attach { id: "aaaa0001".into() }));
    assert_eq!(needs[0].detail, "it keeps running: claude stop failed 3 times; last: claude failed (exit 1): boom");
    let needs = sup.tick(std::slice::from_ref(&live), &mut free(), NOW + 3 * SECOND);
    assert_eq!((needs.len(), needs[0].since), (1, NOW), "since kept");
    assert_eq!(sup.launcher().take(), vec![Call::Stop { id: "aaaa0001".into() }; 4], "stop, never rm, every tick");
    assert_eq!(sup.tick(std::slice::from_ref(&live), &mut free(), NOW + 4 * SECOND), [], "stopped");
    assert_eq!(sup.tick(&[row(SessionState::Stopped, None, None)], &mut free(), NOW + 5 * SECOND), []);
}

#[test]
fn a_remint_whose_stop_fails_is_tried_again() {
    let dir = tempfile::tempdir().unwrap();
    let mcp = dir.path().join("mcp");
    let (mut sup, minter) = minting(true, &mcp);
    sup.tick(&[], &mut free(), NOW);
    let live = row(SessionState::Working, Some(Status::Busy), Some(1));
    sup.tick(std::slice::from_ref(&live), &mut free(), NOW + 61 * SECOND);
    sup.launcher().take();
    minter.take();
    sup.tokens_mut().0.as_mut().unwrap().url = "https://workman.example/mcp".into();
    assert_eq!(sup.server_changed(Some("https://workman.example")).len(), 1);
    sup.launcher.stop_errors.borrow_mut().push(ClaudeError::CliFailed { code: Some(1), stderr: "busy".into() });
    let needs = sup.tick(std::slice::from_ref(&live), &mut free(), NOW + 70 * SECOND);
    assert_eq!(sup.launcher().take(), [Call::Stop { id: "aaaa0001".into() }]);
    assert!(needs.iter().any(|n| n.kind == NeedsKind::OtherServer), "still on the other server: {needs:?}");
    assert!(sup.server_changed(Some("https://workman.example")).is_empty(), "not scheduled twice");
    sup.tick(std::slice::from_ref(&live), &mut free(), NOW + 71 * SECOND);
    assert_eq!(sup.launcher().take(), [Call::Stop { id: "aaaa0001".into() }], "tried again");
    sup.tick(std::slice::from_ref(&live), &mut free(), NOW + 72 * SECOND);
    assert_eq!(sup.launcher().take(), [], "stopped: done");
    let stopped = row(SessionState::Stopped, None, None);
    sup.tick(std::slice::from_ref(&stopped), &mut free(), NOW + 130 * SECOND);
    assert!(matches!(sup.launcher().take().as_slice(), [Call::Resume { .. }]));
    assert!(matches!(minter.take().as_slice(), [MintCall::Mint { base, .. }] if base == "https://workman.example"));
}

#[test]
fn a_remint_without_a_live_row_is_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let (mut sup, _minter) = minting(true, dir.path());
    sup.remint_due.push("fm-demo-workspace".into());
    sup.remint_due.push("fm-demo-gone".into());
    let mut gone = row(SessionState::Working, Some(Status::Busy), Some(2));
    gone.name = "fm-demo-gone".into();
    sup.tick(&[row(SessionState::Stopped, None, None), gone], &mut free(), NOW);
    assert!(sup.remint_due.is_empty(), "{:?}", sup.remint_due);
    assert!(!sup.launcher().take().iter().any(|c| matches!(c, Call::Stop { .. })));
}

#[test]
fn starting_counts_only_a_started_workspace_and_ignores_a_hold() {
    let dir = tempfile::tempdir().unwrap();
    let mut sup = supervisor(true, None, dir.path());
    sup.tick(&[], &mut free(), NOW);
    assert_eq!(sup.starting(&[], NOW + SECOND), 1);
    sup.hold(Some("runs as administrator".into()), NOW + SECOND);
    assert_eq!(sup.starting(&[], NOW + 2 * SECOND), 1, "launched before the hold: still settling");
    assert_eq!(sup.starting(&[], NOW + 20 * SECOND), 0, "settled");
    sup.desired.started = false;
    assert_eq!(sup.starting(&[], NOW + 2 * SECOND), 0);
    let mut held = supervisor(true, None, dir.path());
    held.hold(Some("runs as administrator".into()), NOW);
    held.tick(&[], &mut free(), NOW);
    assert_eq!(held.starting(&[], NOW + SECOND), 0, "held: nothing launched");
}

#[test]
fn a_trust_refusal_on_resume_waits_for_retry() {
    let dir = tempfile::tempdir().unwrap();
    let mut sup = supervisor(true, None, dir.path());
    let stopped = row(SessionState::Stopped, None, None);
    sup.launcher.errors.borrow_mut().push(ClaudeError::TrustNeeded { cwd: r"C:\demo\cwd".into() });
    let needs = sup.tick(std::slice::from_ref(&stopped), &mut free(), NOW);
    assert!(matches!(sup.launcher().take().as_slice(), [Call::Resume { .. }]));
    assert_eq!(needs.len(), 1);
    assert_eq!(needs[0].kind, NeedsKind::TrustNeeded);
    sup.tick(std::slice::from_ref(&stopped), &mut free(), NOW + 3_600 * SECOND);
    assert_eq!(sup.launcher().take(), [], "never retried automatically");
    sup.retry("fm-demo-workspace");
    sup.tick(std::slice::from_ref(&stopped), &mut free(), NOW + 3_601 * SECOND);
    assert!(matches!(sup.launcher().take().as_slice(), [Call::Resume { .. }]));
}

#[test]
fn a_row_whose_cwd_is_gone_backs_off_and_is_never_launched_afresh() {
    let dir = tempfile::tempdir().unwrap();
    let mut sup = supervisor(true, None, dir.path());
    let mut stopped = row(SessionState::Stopped, None, None);
    stopped.cwd = dir.path().join("gone");
    let gone = || ClaudeError::Spawn {
        command: "claude --bg --resume".into(),
        source: std::io::Error::new(std::io::ErrorKind::NotADirectory, "The directory name is invalid."),
    };
    sup.launcher.errors.borrow_mut().extend([gone(), gone()]);
    let needs = sup.tick(std::slice::from_ref(&stopped), &mut free(), NOW);
    assert_eq!(
        sup.launcher().take(),
        [Call::Resume { session_id: stopped.session_id.clone(), cwd: stopped.cwd.clone(), prompt: None }]
    );
    assert_eq!(needs[0].kind, NeedsKind::LaunchFailed);
    assert!(needs[0].detail.contains("directory name is invalid"), "{}", needs[0].detail);
    sup.tick(std::slice::from_ref(&stopped), &mut free(), NOW + 4 * SECOND);
    assert_eq!(sup.launcher().take(), [], "backing off");
    sup.tick(std::slice::from_ref(&stopped), &mut free(), NOW + 5 * SECOND);
    assert!(matches!(sup.launcher().take().as_slice(), [Call::Resume { .. }]), "a resume again, not a launch");
}

#[test]
fn launch_failed_is_listed_while_backing_off() {
    let dir = tempfile::tempdir().unwrap();
    let mut sup = supervisor(true, None, dir.path());
    sup.launcher.errors.borrow_mut().push(ClaudeError::NotInstalled);
    let needs = sup.tick(&[], &mut free(), NOW);
    assert_eq!(needs.len(), 1);
    assert_eq!(needs[0].kind, NeedsKind::LaunchFailed);
    assert_eq!(needs[0].action, Some(Remedy::Retry));
    assert!(needs[0].detail.contains("not on PATH"), "{}", needs[0].detail);
}

#[test]
fn a_resume_that_dies_quickly_counts_as_a_failure() {
    let dir = tempfile::tempdir().unwrap();
    let mut sup = supervisor(true, None, dir.path());
    let dead = row(SessionState::Stopped, None, None);
    sup.tick(std::slice::from_ref(&dead), &mut free(), NOW);
    assert_eq!(sup.launcher().take().len(), 1);
    sup.tick(&[row(SessionState::Working, Some(Status::Busy), Some(9))], &mut free(), NOW + 10 * SECOND);
    let needs = sup.tick(std::slice::from_ref(&dead), &mut free(), NOW + 30 * SECOND);
    assert_eq!(sup.launcher().take(), [], "backing off 5 s");
    assert_eq!(needs[0].kind, NeedsKind::LaunchFailed);
    assert!(needs[0].detail.contains("exited within 30 s"), "{}", needs[0].detail);
    // the backoff is over at +35 s, the respawn grace (last alive at +10 s) at +40 s
    sup.tick(std::slice::from_ref(&dead), &mut free(), NOW + 35 * SECOND);
    assert_eq!(sup.launcher().take(), [], "respawn grace");
    sup.tick(&[dead], &mut free(), NOW + 40 * SECOND);
    assert_eq!(sup.launcher().take().len(), 1);
}

#[test]
fn an_adopted_session_launches_a_copy_of_its_uuid_with_the_supervised_flags_and_forks_once() {
    let home = tempfile::tempdir().unwrap();
    let from = "fc5bf027-48e3-4510-90b2-63a00eb76df8";
    let (sup, minter) = minting(true, home.path());
    let mut sup = sup;
    sup.config.supraman_home = Some(home.path().into());
    sup.desired.sessions.clear();
    let adopted = crate::adopted_session("demo", "Alpha", r"C:\demo\Alpha", "the repo role", from).unwrap();
    sup.desired.sessions.push(adopted.clone());
    sup.desired.save(home.path()).unwrap();
    let original = Session {
        id: "fc5bf027".into(),
        session_id: from.into(),
        name: "mine".into(),
        cwd: r"C:\demo\Alpha".into(),
        ..row(SessionState::Working, Some(Status::Idle), Some(42))
    };

    sup.tick(std::slice::from_ref(&original), &mut free(), NOW);
    assert_eq!(sup.launcher().take(), [], "the original still runs: no copy");
    let need = sup.needs_you().iter().find(|n| n.name == "fm-demo-alpha").cloned().unwrap();
    assert!(need.detail.contains("runs again: stop it, then Retry"), "{}", need.detail);
    assert_eq!(need.action, Some(Remedy::Adopt { from: from.into() }));
    sup.retry("fm-demo-alpha");

    let stopped = Session { pid: None, state: Some(SessionState::Stopped), status: None, ..original };
    sup.tick(std::slice::from_ref(&stopped), &mut free(), NOW + 60 * SECOND);
    let calls = sup.launcher().take();
    assert!(!calls.iter().any(|c| matches!(c, Call::Resume { .. })), "the original is never resumed in place: {calls:?}");
    let argv = launch_argv(&calls);
    assert!(argv.starts_with(&format!("--bg --name fm-demo-alpha --resume {from} ")), "{argv}");
    assert!(argv.contains("--strict-mcp-config") && argv.contains("workstate-fm-demo-alpha.json"), "a fresh mint: {argv}");
    assert!(argv.ends_with(&format!("-- {}", crate::adopt_prompt("demo", &adopted))), "{argv}");
    assert!(!argv.contains("the repo role"), "the role comes with the session's start, not twice: {argv}");
    assert!(matches!(minter.take().as_slice(), [MintCall::Mint { request, .. }] if request.name == "fm-demo-alpha" && request.claude_session_id.is_none()));

    // the copy's row, first with the original's uuid, then its own
    let copy = |session_id: &str| Session {
        id: "f050c443".into(),
        session_id: session_id.into(),
        name: "fm-demo-alpha".into(),
        ..row(SessionState::Working, Some(Status::Busy), Some(43))
    };
    sup.tick(&[stopped.clone(), copy(from)], &mut free(), NOW + 61 * SECOND);
    assert_eq!(sup.desired.sessions[0].fork_from.as_deref(), Some(from));
    let own = "f050c443-6aa7-47b2-a9e4-63a5b6e9d04d";
    sup.tick(&[stopped.clone(), copy(own)], &mut free(), NOW + 62 * SECOND);
    assert_eq!(sup.desired.sessions[0].fork_from, None);
    assert_eq!(DesiredState::load(home.path(), "demo").unwrap().unwrap().sessions[0].fork_from, None, "in the file too");
    assert_eq!(sup.launcher().take(), [], "a live copy needs nothing");
    sup.tick(&[stopped.clone(), copy(own)], &mut free(), NOW + 200 * SECOND);
    // stopped later, it is resumed as itself
    let copy_stopped = Session { pid: None, state: Some(SessionState::Stopped), status: None, ..copy(own) };
    sup.tick(&[stopped, copy_stopped], &mut free(), NOW + 3_600 * SECOND);
    assert!(matches!(sup.launcher().take().as_slice(), [Call::Resume { session_id, .. }] if session_id == own));
}
