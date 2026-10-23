use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use foreman_supervisor::DesiredState;

fn fm(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fm"))
        .args(args)
        .env("FOREMAN_HOME", home)
        .env_remove("APPDATA")
        .output()
        .unwrap()
}

fn text(out: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
}

/// `<tmp>/demo/{Alpha,Nomos.Pdf}`, registered as workspace `demo`.
fn registered() -> (tempfile::TempDir, tempfile::TempDir) {
    let ws = tempfile::tempdir().unwrap();
    for repo in ["Alpha", "Nomos.Pdf"] {
        let git = ws.path().join("demo").join(repo).join(".git");
        fs::create_dir_all(&git).unwrap();
        fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    }
    let home = tempfile::tempdir().unwrap();
    let out = fm(home.path(), &["workspace", "add", ws.path().join("demo").to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));
    (ws, home)
}

#[test]
fn want_adds_a_repo_session_once_and_unwant_retires_it() {
    let (ws, home) = registered();
    let task = home.path().join("task.md");
    fs::write(&task, "Implement unit 3.\n").unwrap();

    let out = fm(home.path(), &["want", "demo", "nomos-pdf", "--prompt-file", task.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));
    assert!(text(&out).contains("fm-demo-nomos-pdf: added"), "{}", text(&out));
    assert!(text(&out).contains("not started"), "{}", text(&out));
    let d = DesiredState::load(home.path(), "demo").unwrap().unwrap();
    assert!(!d.started);
    let w = &d.sessions[0];
    assert_eq!((w.name.as_str(), w.role.as_str(), w.repo.as_deref()), ("fm-demo-nomos-pdf", "repo", Some("Nomos.Pdf")));
    assert_eq!(w.cwd, ws.path().join("demo").join("Nomos.Pdf"));
    assert_eq!(w.permission_mode, "auto");
    assert!(w.prompt.starts_with("You are the repo controller for Nomos.Pdf in the Foreman workspace demo"), "{}", w.prompt);
    assert!(w.prompt.ends_with("First task:\nImplement unit 3."), "{}", w.prompt);
    assert!(!w.prompt.contains('{'), "{}", w.prompt);

    let again = fm(home.path(), &["want", "demo", "Nomos.Pdf", "--prompt-file", task.to_str().unwrap()]);
    assert!(text(&again).contains("already wanted"), "{}", text(&again));
    assert_eq!(DesiredState::load(home.path(), "demo").unwrap().unwrap().sessions.len(), 1);

    let unknown = fm(home.path(), &["want", "demo", "gamma"]);
    assert_eq!(unknown.status.code(), Some(1));
    assert!(text(&unknown).contains("workspace demo has no repo 'gamma'; known: Alpha, Nomos.Pdf"), "{}", text(&unknown));
    assert_eq!(fm(home.path(), &["want", "nope", "Alpha"]).status.code(), Some(1), "unknown workspace");

    let out = fm(home.path(), &["unwant", "demo", "Nomos.Pdf"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));
    assert!(text(&out).contains("no longer wanted"), "{}", text(&out));
    let d = DesiredState::load(home.path(), "demo").unwrap().unwrap();
    assert!(d.sessions.is_empty());
    assert_eq!(d.retired, ["fm-demo-nomos-pdf"]);
    assert!(text(&fm(home.path(), &["unwant", "demo", "Nomos.Pdf"])).contains("was not wanted"));
}

#[test]
fn want_uses_the_role_the_app_rendered_and_keeps_the_apps_sessions() {
    let (_ws, home) = registered();
    fs::create_dir_all(home.path().join("roles")).unwrap();
    fs::write(home.path().join("roles").join("repo.md"), "Rendered for {repo} in {workspace}.\n").unwrap();
    let mut started = DesiredState::workspace_controller("demo", home.path(), "be the controller");
    started.started = true;
    started.save(home.path()).unwrap();

    let out = fm(home.path(), &["want", "demo", "Alpha"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));
    assert!(text(&out).contains("launches it at its next tick"), "{}", text(&out));
    let d = DesiredState::load(home.path(), "demo").unwrap().unwrap();
    assert!(d.started);
    let names: Vec<&str> = d.sessions.iter().map(|w| w.name.as_str()).collect();
    assert_eq!(names, ["fm-demo-workspace", "fm-demo-alpha"]);
    assert_eq!(d.sessions[1].prompt, "Rendered for Alpha in demo.");
}
