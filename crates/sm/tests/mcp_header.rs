use std::path::Path;
use std::process::Output;

use supraman_core::secret::{FileStore, SecretStore};

const TOKEN: &str = "wss_kept_0123456789";

fn sm(args: &[&str]) -> Output {
    supraman_core::process::command(env!("CARGO_BIN_EXE_sm")).args(args).env_remove("APPDATA").output().unwrap()
}

fn mcp_header(home: &Path, name: &str) -> Output {
    sm(&["mcp-header", "--home", &home.to_string_lossy(), name])
}

#[test]
fn mcp_header_prints_the_kept_token_as_claude_reads_it_and_a_missing_one_without_a_token() {
    let home = tempfile::tempdir().unwrap();
    FileStore::new(home.path()).set(&supraman_claude::mcp_token_key("hand-alpha-1"), TOKEN).unwrap();

    let out = mcp_header(home.path(), "hand-alpha-1");
    assert!(out.status.success(), "exit {:?}", out.status.code());
    let printed: serde_json::Value = serde_json::from_slice(&out.stdout).expect("stdout is one JSON object");
    assert!(printed == serde_json::json!({ "Authorization": format!("Bearer {TOKEN}") }), "stdout is the Authorization header of the kept token");
    assert!(out.stderr.is_empty(), "nothing on stderr");

    FileStore::new(home.path()).set(&supraman_claude::mcp_token_key("hand-alpha-1"), "wss_reminted_42").unwrap();
    let again: serde_json::Value = serde_json::from_slice(&mcp_header(home.path(), "hand-alpha-1").stdout).unwrap();
    assert!(again["Authorization"] == "Bearer wss_reminted_42", "a re-mint is what the next connect gets");

    for name in ["hand-beta-1", "../x"] {
        let out = mcp_header(home.path(), name);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(1), "{name}: {stderr}");
        assert!(out.stdout.is_empty(), "{name}: no header");
        assert!(!stderr.contains("wss_"), "{name}: stderr carries no token");
    }
    assert!(String::from_utf8_lossy(&mcp_header(home.path(), "hand-beta-1").stderr).contains("no Workman token is kept for hand-beta-1"));
    let help = String::from_utf8_lossy(&sm(&["--help"]).stdout).into_owned();
    assert!(!help.contains("mcp-header"), "not in the usage");
}

/// What claude's retry after a 401 runs: Workman refuses the kept token, so `sm mcp-header` mints a fresh one with
/// `replace` from the session's record and Supraman's sign-in, keeps it, and prints it.
#[test]
fn mcp_header_mints_afresh_when_workman_refuses_the_kept_token() {
    use supraman_supervisor::{Keeper, MintRequest, MintedSession, MintedSessions, Minter};
    const NAME: &str = "fm-demo-workspace";
    let home = tempfile::tempdir().unwrap();
    let fake = supraman_workman::fake::FakeWorkman::start(Vec::new());
    fake.enable_oauth();
    let sessions = fake.enable_sessions();
    let mut settings = supraman_core::Settings::default();
    settings.workman.url = Some(fake.base().to_string());
    settings.save(home.path()).unwrap();
    let auth = supraman_workman::Auth::open(home.path());
    auth.sign_in(fake.base(), &supraman_workman::auth::http_browser()).unwrap();
    let (_, bearer) = auth.current().unwrap();

    let request = MintRequest {
        name: NAME.into(),
        role: "controller".into(),
        launched_by: "foreman/demo".into(),
        origin: "foreman/demo".into(),
        cwd: r"C:\demo".into(),
        external_role: "workspace".into(),
        replace: true,
        repo: None,
        claude_session_id: None,
    };
    let minted = supraman_supervisor::HttpMinter::default().mint(fake.base(), &bearer, &request).unwrap();
    let mut records = MintedSessions::empty(Some(MintedSessions::path(home.path(), "demo")));
    let session = MintedSession { session_id: minted.session_id, role: "workspace".into(), claude_session_id: None, last_active: None, origin: Some(fake.base().into()) };
    records.insert_minted(NAME, session, &request);
    records.save().unwrap();
    let config = Keeper::open(home.path()).write(NAME, &fake.mcp_url(), &minted.token, &[]).unwrap();
    let text = std::fs::read_to_string(&config).unwrap();
    assert!(text.contains("headersHelper") && !text.contains("Bearer") && !text.contains("wss_"), "{text}");

    let header = |out: Output| serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap()["Authorization"].as_str().unwrap().to_string();
    assert_eq!(header(mcp_header(home.path(), NAME)), format!("Bearer {}", minted.token), "a live kept token");

    sessions.revoke_name(NAME);
    let out = mcp_header(home.path(), NAME);
    assert!(out.stderr.is_empty(), "{}", String::from_utf8_lossy(&out.stderr));
    let fresh = sessions.token_of(NAME).expect("a fresh token on the session's row");
    assert_ne!(fresh, minted.token);
    assert_eq!(header(out), format!("Bearer {fresh}"), "the fresh token is what claude gets");
    assert_eq!(Keeper::open(home.path()).kept(NAME).as_deref(), Some(fresh.as_str()), "and is kept");
    let mints: Vec<serde_json::Value> =
        fake.seen().iter().filter(|s| s.method == "POST" && s.target == "/api/agent-sessions").map(|s| serde_json::from_str(&s.body).unwrap()).collect();
    assert_eq!((mints.len(), &mints[1]["replace"], &mints[1]["name"]), (2, &true.into(), &NAME.into()));
}

/// The `headersHelper` line run as claude runs it on Windows (cmd).
fn through_cmd(command: &str) -> Output {
    use std::os::windows::process::CommandExt as _;
    supraman_core::process::command("cmd").args(["/d", "/s", "/c"]).raw_arg(format!("\"{command}\"")).env_remove("APPDATA").output().unwrap()
}

#[test]
fn a_home_a_shell_would_expand_reaches_mcp_header_as_written_and_a_written_config_still_works() {
    let root = tempfile::tempdir().unwrap();
    let sm = Path::new(env!("CARGO_BIN_EXE_sm"));
    let key = supraman_claude::mcp_token_key("hand-alpha-1");
    let odd = root.path().join("a%TEMP%b$x");
    std::fs::create_dir_all(&odd).unwrap();
    FileStore::new(&odd).set(&key, TOKEN).unwrap();
    let command = supraman_claude::mcp_header_command(sm, &odd, "hand-alpha-1");
    assert!(!command.contains(['%', '$']), "nothing a shell expands: {command}");
    let out = through_cmd(&command);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let printed: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(printed["Authorization"] == format!("Bearer {TOKEN}"), "the token kept in the home as written");

    let plain = root.path().join("plain home");
    FileStore::new(&plain).set(&key, TOKEN).unwrap();
    let written = format!("\"{}\" mcp-header --home \"{}\" hand-alpha-1", sm.display(), plain.display());
    assert_eq!(supraman_claude::mcp_header_command(sm, &plain, "hand-alpha-1"), written, "a plain home keeps its line");
    let out = through_cmd(&written);
    let printed: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(printed["Authorization"] == format!("Bearer {TOKEN}"), "a config written before still works");
}
