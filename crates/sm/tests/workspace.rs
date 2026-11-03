use std::fs;
use std::path::Path;
use std::process::Output;

fn sm(home: &Path, args: &[&str]) -> Output {
    supraman_core::process::command(env!("CARGO_BIN_EXE_sm"))
        .args(args)
        .env("SUPRAMAN_HOME", home)
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

    let out = sm(home.path(), &["workspace", "add", dir]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(home.path().join("workspaces").join("my-workspace.toml").is_file());
    let text = stdout(&out);
    assert!(text.contains("name: my-workspace"), "{text}");
    assert!(text.contains("  Alpha\tkind=git\tbranch=main\tclaude_md=no\tagents_md=no\tenabled=yes\towner=my-workspace\tpath="), "{text}");
    assert!(text.contains("  Beta\tkind=git\tbranch=(detached)"), "{text}");
    assert!(text.contains(&format!("home: {dir}\n")) && text.contains(&format!("scans: 1\n  {dir}\tdepth=1\n")), "{text}");

    let out = sm(home.path(), &["workspace", "add", dir]);
    assert_eq!(out.status.code(), Some(1), "duplicate must fail: {out:?}");
    let out = sm(home.path(), &["workspace", "add", dir, "--force"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(!stdout(&out).contains("notice:"), "a replaced workspace does not compete with itself: {}", stdout(&out));
    let out = sm(home.path(), &["workspace", "add", dir, "--name", "again"]);
    assert_eq!(out.status.code(), Some(0), "a second workspace on the same folder: {out:?}");
    assert!(stdout(&out).contains("notice: Alpha is supervised in workspace my-workspace; disabled here"), "{}", stdout(&out));
    assert!(stdout(&out).contains("  Alpha\tkind=git\tbranch=main\tclaude_md=no\tagents_md=no\tenabled=no"), "{}", stdout(&out));
    assert_eq!(sm(home.path(), &["workspace", "remove", "again"]).status.code(), Some(0));

    let out = sm(home.path(), &["workspace", "list"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(stdout(&out).starts_with("my-workspace\tfolders=2\thome="), "{}", stdout(&out));

    let out = sm(home.path(), &["workspace", "show", "my-workspace"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(stdout(&out).contains("folders: 2"));

    let out = sm(home.path(), &["workspace", "remove", "my-workspace"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(stdout(&sm(home.path(), &["workspace", "list"])), "no workspaces registered\n");
    assert_eq!(sm(home.path(), &["workspace", "show", "my-workspace"]).status.code(), Some(1));
    assert_eq!(sm(home.path(), &["workspace", "remove", "my-workspace"]).status.code(), Some(1));
}

#[test]
fn dry_run_writes_nothing() {
    let ws = workspace();
    let home = tempfile::tempdir().unwrap();
    let fm_home = home.path().join("fm-home");
    let dir = ws.path().join("My.Workspace");
    let out = sm(&fm_home, &["workspace", "add", dir.to_str().unwrap(), "--name", "x", "--dry-run"]);
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
        assert_eq!(sm(home.path(), &args).status.code(), Some(1), "{args:?}");
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
    let out = sm(&blocker, &["workspace", "add", dir.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
}

#[test]
fn a_workspace_made_empty_takes_folders_a_home_and_scans_and_shows_kind_and_owner() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("fm-home");
    let (git, plain, parent) = (tmp.path().join("Gaia"), tmp.path().join("NodeApi"), tmp.path().join("Group"));
    fs::create_dir_all(git.join(".git")).unwrap();
    fs::write(git.join(".git").join("HEAD"), "ref: refs/heads/main\n").unwrap();
    fs::create_dir_all(&plain).unwrap();
    for r in ["One", "Two"] {
        fs::create_dir_all(parent.join(r).join(".git")).unwrap();
        fs::write(parent.join(r).join(".git").join("HEAD"), "ref: refs/heads/dev\n").unwrap();
    }
    let p = |path: &Path| path.to_str().unwrap().to_string();
    let ok = |args: &[&str]| {
        let out = sm(&home, args);
        assert_eq!(out.status.code(), Some(0), "{args:?}: {out:?}");
        stdout(&out)
    };

    let text = ok(&["workspace", "new", "gaia"]);
    assert!(text.contains("name: gaia\nhome: (none)\n") && text.contains("folders: 0"), "{text}");
    assert_eq!(sm(&home, &["workspace", "new", "gaia"]).status.code(), Some(1), "the name is taken");

    let text = ok(&["workspace", "add-folder", "gaia", &p(&git), "--enable"]);
    assert!(text.contains("added folder Gaia") && text.contains("  Gaia\tkind=git\tbranch=main\tclaude_md=no\tagents_md=no\tenabled=yes\towner=gaia\tpath="), "{text}");
    let text = ok(&["workspace", "add-folder", "gaia", &p(&plain)]);
    assert!(text.contains("  NodeApi\tkind=plain\tbranch=-\tclaude_md=no\tagents_md=no\tenabled=no\towner=-\tpath="), "without --enable it is disabled: {text}");
    assert!(ok(&["workspace", "add-folder", "gaia", &p(&plain)]).contains("is already a folder of gaia"));
    assert!(ok(&["workspace", "enable", "gaia", "nodeapi"]).contains("  NodeApi\tkind=plain\tbranch=-\tclaude_md=no\tagents_md=no\tenabled=yes\towner=gaia"));
    assert!(ok(&["workspace", "set-home", "gaia", &p(tmp.path())]).contains(&format!("home: {}\n", p(tmp.path()))));

    let text = ok(&["workspace", "new", "other", "--folder", &p(&plain), "--home", &p(&plain)]);
    assert!(text.contains("notice: NodeApi is supervised in workspace gaia; disabled here"), "{text}");
    assert!(text.contains("  NodeApi\tkind=plain\tbranch=-\tclaude_md=no\tagents_md=no\tenabled=no\towner=gaia"), "{text}");
    let refused = sm(&home, &["workspace", "enable", "other", "NodeApi"]);
    assert_eq!(refused.status.code(), Some(1), "{refused:?}");
    assert!(String::from_utf8_lossy(&refused.stderr).contains("NodeApi is supervised in workspace gaia"), "{refused:?}");

    let text = ok(&["workspace", "add-scan", "gaia", &p(&parent)]);
    assert!(text.contains("added folder One\nadded folder Two\n") && text.contains(&format!("scans: 1\n  {}\tdepth=1\n", p(&parent))), "{text}");
    let text = ok(&["workspace", "remove-folder", "gaia", "One"]);
    assert!(text.contains("removed folder One") && text.contains(&format!("\tskips={}", p(&parent.join("One")))), "{text}");

    fs::create_dir_all(home.join("roles")).unwrap();
    fs::write(home.join("roles").join("repo.md"), "You are the repo session.").unwrap();
    ok(&["want", "gaia", "Two"]);
    let text = ok(&["workspace", "remove-scan", "gaia", &p(&parent)]);
    assert!(text.contains("removed folder Two") && text.contains("fm-gaia-two: no longer wanted; the supervisor stops it at its next tick (stop, never rm)"), "{text}");
    assert!(text.contains("scans: 0") && text.contains("folders: 2"), "{text}");
    ok(&["want", "gaia", "Gaia"]);
    let text = ok(&["workspace", "remove-folder", "gaia", "gaia"]);
    assert!(text.contains("fm-gaia-gaia: no longer wanted"), "{text}");
    let desired = fs::read_to_string(home.join("supervisor").join("gaia.toml")).unwrap();
    assert!(desired.contains("fm-gaia-gaia") && !desired.contains("[[sessions]]"), "retired, not wanted: {desired}");

    for args in [
        vec!["workspace", "remove-folder", "gaia", "Nope"],
        vec!["workspace", "remove-scan", "gaia", "Z:/nope"],
        vec!["workspace", "add-folder", "gaia", "Z:/definitely/missing"],
        vec!["workspace", "set-home", "gaia", "Z:/definitely/missing"],
        vec!["workspace", "add-folder", "missing", &p(&plain)],
        vec!["workspace", "new", "Bad.Name"],
        vec!["workspace", "new", "x", "--folder", "Z:/definitely/missing"],
    ] {
        assert_eq!(sm(&home, &args).status.code(), Some(1), "{args:?}");
    }
    assert!(!home.join("workspaces").join("x.toml").exists(), "a refused new writes nothing");
}

#[test]
fn the_default_home_is_refused_until_the_foreman_home_is_copied() {
    let appdata = tempfile::tempdir().unwrap();
    fs::create_dir_all(appdata.path().join("Foreman").join("workspaces")).unwrap();
    let run = || {
        supraman_core::process::command(env!("CARGO_BIN_EXE_sm"))
            .args(["workspace", "list"])
            .env_remove("SUPRAMAN_HOME")
            .env_remove("FOREMAN_HOME")
            .env("APPDATA", appdata.path())
            .output()
            .unwrap()
    };
    let out = run();
    assert_ne!(out.status.code(), Some(0), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("start Supraman once"), "{out:?}");
    assert!(!appdata.path().join("Supraman").exists(), "sm never starts an empty home beside the old one");

    supraman_core::home::migrate_home(&appdata.path().join("Foreman"), &appdata.path().join("Supraman")).unwrap().unwrap();
    assert_eq!(run().status.code(), Some(0));
}
