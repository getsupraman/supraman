use std::process::Command;

fn fm(home: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_fm"))
        .args(args)
        .env("FOREMAN_HOME", home)
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
        let out = fm(home.path(), &args);
        assert_eq!(out.status.code(), Some(1), "{args:?}: {out:?}");
    }
}
