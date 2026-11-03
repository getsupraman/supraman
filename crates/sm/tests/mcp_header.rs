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
