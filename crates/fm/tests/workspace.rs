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
        let git = tmp.path().join("My.Workspace").join(repo).join(".git");
        fs::create_dir_all(&git).unwrap();
        fs::write(git.join("HEAD"), head).unwrap();
    }
    tmp
}

#[test]
fn add_list_show_remove_round_trip() {
    let ws = workspace();
    let home = tempfile::tempdir().unwrap();
    let dir = ws.path().join("My.Workspace");
    let dir = dir.to_str().unwrap();

    let out = fm(home.path(), &["workspace", "add", dir]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(home.path().join("workspaces").join("my-workspace.toml").is_file());
    let text = stdout(&out);
    assert!(text.contains("name: my-workspace"), "{text}");
    assert!(text.contains("  Alpha\tkind=git\tbranch=main\tclaude_md=no\tagents_md=no\tenabled=yes\tpath="), "{text}");
    assert!(text.contains("  Beta\tkind=git\tbranch=(detached)"), "{text}");
    assert!(text.contains(&format!("home: {dir}\n")) && text.contains(&format!("scans: 1\n  {dir}\tdepth=1\n")), "{text}");

    let out = fm(home.path(), &["workspace", "add", dir]);
    assert_eq!(out.status.code(), Some(1), "duplicate must fail: {out:?}");
    let out = fm(home.path(), &["workspace", "add", dir, "--force"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(!stdout(&out).contains("notice:"), "a replaced workspace does not compete with itself: {}", stdout(&out));
    let out = fm(home.path(), &["workspace", "add", dir, "--name", "again"]);
    assert_eq!(out.status.code(), Some(0), "a second workspace on the same folder: {out:?}");
    assert!(stdout(&out).contains("notice: Alpha is supervised in workspace my-workspace; disabled here"), "{}", stdout(&out));
    assert!(stdout(&out).contains("  Alpha\tkind=git\tbranch=main\tclaude_md=no\tagents_md=no\tenabled=no"), "{}", stdout(&out));
    assert_eq!(fm(home.path(), &["workspace", "remove", "again"]).status.code(), Some(0));

    let out = fm(home.path(), &["workspace", "list"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(stdout(&out).starts_with("my-workspace\tfolders=2\thome="), "{}", stdout(&out));

    let out = fm(home.path(), &["workspace", "show", "my-workspace"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(stdout(&out).contains("folders: 2"));

    let out = fm(home.path(), &["workspace", "remove", "my-workspace"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(stdout(&fm(home.path(), &["workspace", "list"])), "no workspaces registered\n");
    assert_eq!(fm(home.path(), &["workspace", "show", "my-workspace"]).status.code(), Some(1));
    assert_eq!(fm(home.path(), &["workspace", "remove", "my-workspace"]).status.code(), Some(1));
}

#[test]
fn dry_run_writes_nothing() {
    let ws = workspace();
    let home = tempfile::tempdir().unwrap();
    let fm_home = home.path().join("fm-home");
    let dir = ws.path().join("My.Workspace");
    let out = fm(&fm_home, &["workspace", "add", dir.to_str().unwrap(), "--name", "x", "--dry-run"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(stdout(&out).starts_with("dry run: would register workspace x (nothing written)\n"));
    assert!(!fm_home.exists());
    assert!(!dir.join("Alpha").join("CLAUDE.md").exists());
}

#[test]
fn user_errors_exit_1() {
    let ws = workspace();
    let home = tempfile::tempdir().unwrap();
    let dir = ws.path().join("My.Workspace");
    let dir = dir.to_str().unwrap();
    for args in [
        vec![],
        vec!["workspace"],
        vec!["workspace", "add"],
        vec!["workspace", "add", dir, "--depth", "0"],
        vec!["workspace", "add", dir, "--name", "Bad.Name"],
        vec!["workspace", "add", dir, "--bogus"],
        vec!["workspace", "add", "Z:/definitely/missing/dir"],
    ] {
        assert_eq!(fm(home.path(), &args).status.code(), Some(1), "{args:?}");
    }
    assert!(!home.path().join("workspaces").exists());
}

#[test]
fn io_errors_exit_2() {
    let ws = workspace();
    let home = tempfile::tempdir().unwrap();
    // FOREMAN_HOME is a file, so the workspaces directory cannot be created.
    let blocker = home.path().join("file");
    fs::write(&blocker, "x").unwrap();
    let dir = ws.path().join("My.Workspace");
    let out = fm(&blocker, &["workspace", "add", dir.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
}
