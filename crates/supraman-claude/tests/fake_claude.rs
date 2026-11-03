//! `Claude` against `examples/fake_claude.rs`: the plumbing (program, cwd, env, argv, exit code,
//! stderr) end to end. Env vars are per test so the tests can run in parallel.

use std::path::{Path, PathBuf};

use supraman_claude::{Claude, ClaudeError, LaunchSpec};

fn fake() -> Claude {
    let deps = std::env::current_exe().unwrap().parent().unwrap().to_path_buf();
    let exe = deps.parent().unwrap().join("examples").join(format!("fake_claude{}", std::env::consts::EXE_SUFFIX));
    assert!(exe.exists(), "{} (built by `cargo test`)", exe.display());
    Claude::new(exe)
}

/// The fake reads its mode from env, which `LaunchSpec::env` sets on the launch process only.
fn spec(dir: &Path, mode: &str) -> LaunchSpec {
    let mut spec = LaunchSpec::new("aw-test-a", dir, "hello there");
    spec.env = vec![
        ("FAKE_CLAUDE_LOG".into(), dir.join("log.txt").display().to_string()),
        ("FAKE_CLAUDE_MODE".into(), mode.into()),
        ("FAKE_SEEN".into(), "fake-value".into()),
    ];
    spec.mcp_config = Some(PathBuf::from("ws.json"));
    spec
}

fn log(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("log.txt")).unwrap().lines().map(str::to_string).collect()
}

/// `launch` lists rows first; the fake's list is empty, so the name is free (NameTaken is the unit test `name_taken`).
#[test]
fn launch_runs_in_cwd_with_env_and_argv() {
    let dir = tempfile::tempdir().unwrap();
    let out = fake().launch(&spec(dir.path(), "")).unwrap();
    assert_eq!(out, "backgrounded · abcd1234 · aw-test-a");
    let log = log(dir.path());
    let cwd = PathBuf::from(log[0].strip_prefix("cwd=").unwrap());
    assert_eq!(cwd.canonicalize().unwrap(), dir.path().canonicalize().unwrap());
    let args: Vec<&str> = log.iter().filter_map(|l| l.strip_prefix("arg=")).collect();
    assert_eq!(
        args,
        ["--bg", "--name", "aw-test-a", "--permission-mode", "auto", "--mcp-config", "ws.json", "--", "hello there"]
    );
    assert!(log.contains(&"FAKE_SEEN=fake-value".to_string()), "{log:?}");
}

#[test]
fn a_failed_launch_keeps_the_exit_code_and_stderr_and_a_missing_exe_is_not_installed() {
    let dir = tempfile::tempdir().unwrap();
    match fake().launch_unchecked(&spec(dir.path(), "fail")) {
        Err(ClaudeError::CliFailed { code, stderr }) => assert_eq!((code, stderr.as_str()), (Some(7), "boom")),
        other => panic!("{other:?}"),
    }
    let missing = Claude::new(dir.path().join("no-such-claude.exe"));
    assert!(matches!(missing.launch_unchecked(&spec(dir.path(), "")), Err(ClaudeError::NotInstalled)));
}

#[test]
fn stop_rm_resume() {
    let dir = tempfile::tempdir().unwrap();
    let claude = fake();
    assert_eq!(claude.stop("abcd1234").unwrap().trim(), "stopped abcd1234");
    assert_eq!(claude.rm("abcd1234").unwrap().trim(), "removed abcd1234");
    let out = claude.resume("abcd1234-uuid", dir.path(), Some("go on")).unwrap();
    assert!(out.starts_with("backgrounded · abcd1234"), "{out}");
}
