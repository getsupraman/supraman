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
    assert!(text.contains("  Alpha\tbranch=main\tclaude_md=no\tagents_md=no\tenabled=yes"), "{text}");
    assert!(text.contains("  Beta\tbranch=(detached)"), "{text}");

    let out = fm(home.path(), &["workspace", "add", dir]);
    assert_eq!(out.status.code(), Some(1), "duplicate must fail: {out:?}");
    let out = fm(home.path(), &["workspace", "add", dir, "--force"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");

    let out = fm(home.path(), &["workspace", "list"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(stdout(&out).starts_with("my-workspace\trepos=2\troot="), "{}", stdout(&out));

    let out = fm(home.path(), &["workspace", "show", "my-workspace"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(stdout(&out).contains("repos: 2"));

    let out = fm(home.path(), &["workspace", "remove", "my-workspace"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(stdout(&fm(home.path(), &["workspace", "list"])), "no workspaces registered\n");
    assert_eq!(fm(home.path(), &["workspace", "show", "my-workspace"]).status.code(), Some(1));
    assert_eq!(fm(home.path(), &["workspace", "remove", "my-workspace"]).status.code(), Some(1));
}

#[test]
fn a_home_from_before_the_rename_is_migrated_first() {
    let ws = workspace();
    let home = tempfile::tempdir().unwrap();
    let dir = ws.path().join("My.Workspace");
    assert_eq!(fm(home.path(), &["workspace", "add", dir.to_str().unwrap()]).status.code(), Some(0));
    fs::rename(home.path().join("workspaces"), home.path().join("platforms")).unwrap();
    fs::create_dir_all(home.path().join("supervisor")).unwrap();
    fs::write(
        home.path().join("supervisor").join("my-workspace.toml"),
        "platform = \"my-workspace\"\nstarted = true\n\n[[sessions]]\nname = \"fm-my-workspace-platform\"\n\
         role = \"platform\"\ncwd = 'C:\\x'\nprompt = \"p\"\n",
    )
    .unwrap();

    let out = fm(home.path(), &["workspace", "list"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(stdout(&out).starts_with("my-workspace\trepos=2\troot="), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("notice: workspace my-workspace was started under its old name"), "{err}");
    assert!(!home.path().join("platforms").exists());
    let desired = fs::read_to_string(home.path().join("supervisor").join("my-workspace.toml")).unwrap();
    assert!(desired.contains("fm-my-workspace-workspace") && desired.contains("started = false"), "{desired}");

    let again = fm(home.path(), &["workspace", "list"]);
    assert!(again.stderr.is_empty(), "{again:?}");
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
