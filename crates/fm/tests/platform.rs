use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn fm(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fm"))
        .args(args)
        .env("FOREMAN_HOME", home)
        .env_remove("APPDATA")
        .output()
        .unwrap()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn workspace() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    for (repo, head) in [("Alpha", "ref: refs/heads/main\n"), ("Beta", "0123456789abcdef\n")] {
        let git = tmp.path().join("My.Platform").join(repo).join(".git");
        fs::create_dir_all(&git).unwrap();
        fs::write(git.join("HEAD"), head).unwrap();
    }
    tmp
}

#[test]
fn add_list_show_remove_round_trip() {
    let ws = workspace();
    let home = tempfile::tempdir().unwrap();
    let dir = ws.path().join("My.Platform");
    let dir = dir.to_str().unwrap();

    let out = fm(home.path(), &["platform", "add", dir]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(home.path().join("platforms").join("my-platform.toml").is_file());
    let text = stdout(&out);
    assert!(text.contains("name: my-platform"), "{text}");
    assert!(text.contains("  Alpha\tbranch=main\tclaude_md=no\tagents_md=no\tenabled=yes"), "{text}");
    assert!(text.contains("  Beta\tbranch=(detached)"), "{text}");

    let out = fm(home.path(), &["platform", "add", dir]);
    assert_eq!(out.status.code(), Some(1), "duplicate must fail: {out:?}");
    let out = fm(home.path(), &["platform", "add", dir, "--force"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");

    let out = fm(home.path(), &["platform", "list"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(stdout(&out).starts_with("my-platform\trepos=2\troot="), "{}", stdout(&out));

    let out = fm(home.path(), &["platform", "show", "my-platform"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(stdout(&out).contains("repos: 2"));

    let out = fm(home.path(), &["platform", "remove", "my-platform"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(stdout(&fm(home.path(), &["platform", "list"])), "no platforms registered\n");
    assert_eq!(fm(home.path(), &["platform", "show", "my-platform"]).status.code(), Some(1));
    assert_eq!(fm(home.path(), &["platform", "remove", "my-platform"]).status.code(), Some(1));
}

#[test]
fn dry_run_writes_nothing() {
    let ws = workspace();
    let home = tempfile::tempdir().unwrap();
    let fm_home = home.path().join("fm-home");
    let dir = ws.path().join("My.Platform");
    let out = fm(&fm_home, &["platform", "add", dir.to_str().unwrap(), "--name", "x", "--dry-run"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(stdout(&out).starts_with("dry run: would register platform x (nothing written)\n"));
    assert!(!fm_home.exists());
    assert!(!dir.join("Alpha").join("CLAUDE.md").exists());
}

#[test]
fn user_errors_exit_1() {
    let ws = workspace();
    let home = tempfile::tempdir().unwrap();
    let dir = ws.path().join("My.Platform");
    let dir = dir.to_str().unwrap();
    for args in [
        vec![],
        vec!["platform"],
        vec!["platform", "add"],
        vec!["platform", "add", dir, "--depth", "0"],
        vec!["platform", "add", dir, "--name", "Bad.Name"],
        vec!["platform", "add", dir, "--bogus"],
        vec!["platform", "add", "Z:/definitely/missing/dir"],
    ] {
        assert_eq!(fm(home.path(), &args).status.code(), Some(1), "{args:?}");
    }
    assert!(!home.path().join("platforms").exists());
}

#[test]
fn io_errors_exit_2() {
    let ws = workspace();
    let home = tempfile::tempdir().unwrap();
    // FOREMAN_HOME is a file, so the platforms directory cannot be created.
    let blocker = home.path().join("file");
    fs::write(&blocker, "x").unwrap();
    let dir = ws.path().join("My.Platform");
    let out = fm(&blocker, &["platform", "add", dir.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
}
