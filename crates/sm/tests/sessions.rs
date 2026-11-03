
fn sm(home: &std::path::Path, args: &[&str]) -> std::process::Output {
    supraman_core::process::command(env!("CARGO_BIN_EXE_sm"))
        .args(args)
        .env("SUPRAMAN_HOME", home)
        .env_remove("APPDATA")
        .output()
        .unwrap()
}

// These fail before the `claude` CLI is run, so they never touch real sessions.
#[test]
fn bad_arguments_and_unknown_workspace_exit_1() {
    let home = tempfile::tempdir().unwrap();
    for args in [
        vec!["sessions", "--bogus"],
        vec!["sessions", "--workspace"],
        vec!["sessions", "--workspace", "Bad.Name"],
        vec!["sessions", "--workspace", "missing"],
    ] {
        let out = sm(home.path(), &args);
        assert_eq!(out.status.code(), Some(1), "{args:?}: {out:?}");
    }
}
