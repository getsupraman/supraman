use std::path::Path;
use std::process::Output;
use std::time::{Duration, Instant};

use supraman_core::secret::{FileStore, SecretStore};

const TOKEN: &str = "wss_kept_0123456789";
const PLACEHOLDER: &str = r#"{"Authorization":"Bearer sm-no-token"}"#;

fn sm(args: &[&str]) -> Output {
    supraman_core::process::command(env!("CARGO_BIN_EXE_sm")).args(args).env_remove("APPDATA").output().unwrap()
}

fn mcp_header(home: &Path, name: &str) -> Output {
    sm(&["mcp-header", "--home", &home.to_string_lossy(), name])
}

fn keep(home: &Path, name: &str, token: &str) {
    FileStore::new(home).set(&supraman_claude::mcp_token_key(name), token).unwrap();
}

/// The one line on stdout, after checking the run exited 0 with nothing on stderr.
fn printed(what: &str, out: &Output) -> String {
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(out.status.code(), Some(0), "{what}: exit");
    assert!(out.stderr.is_empty(), "{what}: nothing on stderr");
    assert_eq!(stdout.lines().count(), 1, "{what}: one line");
    let header: serde_json::Value = serde_json::from_str(stdout.trim_end()).expect("stdout is one JSON object");
    assert!(header["Authorization"].as_str().is_some_and(|a| a.starts_with("Bearer ")), "{what}: an Authorization header");
    stdout.trim_end().to_string()
}

fn log(home: &Path) -> String {
    std::fs::read_to_string(home.join("logs").join("mcp-header.log")).unwrap_or_default()
}

#[test]
fn every_run_prints_one_authorization_header_and_exits_0() {
    let home = tempfile::tempdir().unwrap();
    keep(home.path(), "hand-alpha-1", TOKEN);
    let kept = printed("kept", &mcp_header(home.path(), "hand-alpha-1"));
    assert!(kept == serde_json::json!({ "Authorization": format!("Bearer {TOKEN}") }).to_string(), "the kept token's header");

    keep(home.path(), "hand-alpha-1", "wss_reminted_42");
    assert!(printed("re-minted", &mcp_header(home.path(), "hand-alpha-1")).contains("Bearer wss_reminted_42"), "a re-mint is what the next connect gets");

    let unreadable = FileStore::new(home.path()).path(&supraman_claude::mcp_token_key("hand-gamma-1"));
    std::fs::write(&unreadable, b"not sealed by DPAPI").unwrap();
    for (what, out) in [
        ("none kept", mcp_header(home.path(), "hand-beta-1")),
        ("unreadable store", mcp_header(home.path(), "hand-gamma-1")),
        ("bad name", mcp_header(home.path(), "../x")),
        ("bad hex home", sm(&["mcp-header", "--home-hex", "zz", "hand-alpha-1"])),
        ("no arguments", sm(&["mcp-header"])),
    ] {
        assert!(printed(what, &out) == PLACEHOLDER, "{what}: the placeholder");
    }
    let help = String::from_utf8_lossy(&sm(&["--help"]).stdout).into_owned();
    assert!(!help.contains("mcp-header"), "not in the usage");
}

#[test]
fn each_run_appends_one_line_to_its_log_without_the_token() {
    let home = tempfile::tempdir().unwrap();
    keep(home.path(), "hand-alpha-1", TOKEN);
    let unreadable = FileStore::new(home.path()).path(&supraman_claude::mcp_token_key("hand-gamma-1"));
    std::fs::write(&unreadable, b"not sealed by DPAPI").unwrap();
    for name in ["hand-alpha-1", "hand-beta-1", "hand-gamma-1", "../x"] {
        mcp_header(home.path(), name);
    }
    let log = log(home.path());
    assert!(!log.contains(TOKEN) && !log.contains("wss_") && !log.contains("0123456789"), "no part of the token in the log");
    let lines: Vec<&str> = log.lines().collect();
    assert_eq!(lines.len(), 4, "one line per run:\n{log}");
    let said: Vec<(&str, &str)> = lines.iter().map(|l| l.split_once(' ').unwrap().1.split_once(' ').unwrap()).collect();
    assert_eq!(said[0], ("hand-alpha-1", "kept"));
    assert!(said[1].0 == "hand-beta-1" && said[1].1.starts_with("placeholder: no Workman token is kept"), "{}", lines[1]);
    assert!(said[2].0 == "hand-gamma-1" && said[2].1.starts_with("placeholder: cannot decrypt"), "{}", lines[2]);
    assert!(said[3].1.starts_with("placeholder: not a session name"), "{}", lines[3]);
    for line in &lines {
        let at = line.split_once(' ').unwrap().0;
        assert!(at.ends_with('Z') && supraman_core::time::rfc3339_ms(at).is_some(), "a UTC time first: {line}");
    }

    let blocked = tempfile::tempdir().unwrap();
    keep(blocked.path(), "hand-alpha-1", TOKEN);
    std::fs::write(blocked.path().join("logs"), b"a file where the log folder goes").unwrap();
    assert!(printed("log unwritable", &mcp_header(blocked.path(), "hand-alpha-1")).contains(TOKEN), "a failed log write changes nothing");
}

/// The run as claude makes it, with a deadline well under 5 s.
fn timed(home: &Path, name: &str) -> (Output, Duration) {
    let started = Instant::now();
    let mut cmd = supraman_core::process::command(env!("CARGO_BIN_EXE_sm"));
    cmd.args(["mcp-header", "--home", &home.to_string_lossy(), name]).env_remove("APPDATA");
    let out = supraman_core::process::run(&mut cmd, Duration::from_secs(3), supraman_core::process::Kill::Tree).expect("sm mcp-header returns within 3 s");
    use std::os::windows::process::ExitStatusExt as _;
    let status = std::process::ExitStatus::from_raw(out.code.unwrap_or(-1) as u32);
    (Output { status, stdout: out.stdout.into_bytes(), stderr: out.stderr.into_bytes() }, started.elapsed())
}

#[test]
fn a_run_asks_no_server_and_waits_on_no_lock() {
    const NAME: &str = "fm-demo-workspace";
    let home = tempfile::tempdir().unwrap();
    let fake = supraman_workman::fake::FakeWorkman::start(vec![("POST /mcp".into(), supraman_workman::fake::Answer::silent())]);
    supraman_supervisor::Keeper::open(home.path()).write(NAME, &fake.mcp_url(), TOKEN, None, &[]).unwrap();
    let _refresh = supraman_core::fs::lock_file(&home.path().join("oauth").join("refresh.lock")).unwrap();
    let _kept = supraman_core::fs::lock_file(&home.path().join("supervisor").join("kept.lock")).unwrap();

    let (out, took) = timed(home.path(), NAME);
    assert!(printed("kept, its server silent", &out).contains(TOKEN));
    let (out, took_missing) = timed(home.path(), "hand-beta-1");
    assert!(printed("none kept, the locks held", &out) == PLACEHOLDER, "the placeholder");
    assert!(took < Duration::from_secs(2) && took_missing < Duration::from_secs(2), "{took:?} {took_missing:?}");
    assert!(fake.seen().is_empty(), "no request: {:?}", fake.seen().iter().map(|s| s.target.clone()).collect::<Vec<_>>());
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
    assert_eq!(log(&odd).lines().count(), 1, "logged in the home as written");

    let plain = root.path().join("plain home");
    FileStore::new(&plain).set(&key, TOKEN).unwrap();
    let written = format!("\"{}\" mcp-header --home \"{}\" hand-alpha-1", sm.display(), plain.display());
    assert_eq!(supraman_claude::mcp_header_command(sm, &plain, "hand-alpha-1"), written, "a plain home keeps its line");
    let out = through_cmd(&written);
    let printed: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(printed["Authorization"] == format!("Bearer {TOKEN}"), "a config written before still works");
}
