//! `sm workspace remove` of a started workspace against a fake `claude` and `FakeWorkman`.

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};

use sm::run::Agents;
use supraman_claude::{Kind, LaunchSpec, Session, SessionState, Status};
use supraman_core::{Registry, Workspace};
use supraman_supervisor::{DesiredState, Launcher, MintedSessions, relay_session, repo_session};
use supraman_workman::fake::{Answer, FakeWorkman};

struct FakeClaude {
    rows: RefCell<Vec<Session>>,
    stops: RefCell<Vec<String>>,
}

fn row(id: &str, name: &str) -> Session {
    Session {
        id: id.into(),
        session_id: format!("{id}-0000-4000-8000-000000000001"),
        name: name.into(),
        cwd: PathBuf::from(r"C:\demo"),
        kind: Kind::Background,
        state: Some(SessionState::Working),
        status: Some(Status::Busy),
        waiting_for: None,
        pid: Some(7),
        started_at: Some(supraman_claude::now_ms() - 1_000),
    }
}

impl Launcher for FakeClaude {
    fn launch(&self, _: &LaunchSpec) -> supraman_claude::Result<String> {
        unreachable!("remove never launches")
    }

    fn resume(&self, _: &str, _: &Path, _: Option<&str>) -> supraman_claude::Result<String> {
        unreachable!("remove never resumes")
    }

    fn stop(&self, id: &str) -> supraman_claude::Result<String> {
        self.stops.borrow_mut().push(id.to_string());
        for s in self.rows.borrow_mut().iter_mut().filter(|s| s.id == id) {
            (s.state, s.status, s.pid) = (Some(SessionState::Stopped), None, None);
        }
        Ok(format!("stopped {id}"))
    }
}

impl Agents for FakeClaude {
    fn list(&self) -> supraman_claude::Result<Vec<Session>> {
        Ok(self.rows.borrow().clone())
    }
}

#[test]
fn removing_a_started_workspace_stops_its_sessions_and_leaves_no_desired_state_and_revokes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let registry = Registry::at(&home);
    for name in ["demo", "other"] {
        registry.save(&Workspace::new(name, tmp.path(), Vec::new()), false).unwrap();
    }
    DesiredState::update(&home, "demo", |d| {
        *d = DesiredState::workspace_controller("demo", r"C:\demo", "p");
        d.started = true;
        d.sessions.insert(0, relay_session("demo", r"C:\demo", r"C:\relay", "r"));
        d.want(repo_session("demo", "Alpha", r"C:\demo\Alpha", "p").unwrap());
        d.want(repo_session("demo", "Old", r"C:\demo\Old", "p").unwrap());
        d.unwant("fm-demo-old");
    })
    .unwrap();
    let fake = FakeWorkman::start(vec![("DELETE /api/agent-sessions/91".into(), Answer::status(204, ""))]);
    let minted = format!(r#"{{"fm-demo-alpha":{{"session_id":"91","role":"repo","origin":"{}"}}}}"#, fake.base());
    fs::write(MintedSessions::path(&home, "demo"), minted).unwrap();
    let mcp = supraman_claude::mcp_config_path(&home.join("mcp"), "fm-demo-alpha").unwrap();
    fs::create_dir_all(mcp.parent().unwrap()).unwrap();
    fs::write(&mcp, "{}").unwrap();

    let claude = FakeClaude {
        rows: RefCell::new(vec![
            row("aaaa0001", "fm-demo-workspace"),
            row("aaaa0002", "fm-demo-relay"),
            row("aaaa0003", "fm-demo-alpha"),
            row("aaaa0004", "fm-demo-old"),
            row("aaaa0005", "fm-other-workspace"),
        ]),
        stops: RefCell::default(),
    };
    let stopped = sm::remove::remove(&home, "demo", &claude).unwrap();

    assert_eq!(stopped, ["fm-demo-workspace", "fm-demo-relay", "fm-demo-alpha", "fm-demo-old"]);
    assert_eq!(*claude.stops.borrow(), ["aaaa0001", "aaaa0002", "aaaa0003", "aaaa0004"], "another workspace's session runs on");
    let now = supraman_claude::now_ms();
    let running: Vec<String> = claude.rows.borrow().iter().filter(|s| supraman_supervisor::is_live(s, now)).map(|s| s.name.clone()).collect();
    assert_eq!(running, ["fm-other-workspace"]);
    assert!(!DesiredState::path(&home, "demo").unwrap().exists(), "no desired state");
    assert!(!MintedSessions::path(&home, "demo").exists());
    assert_eq!(registry.list().unwrap().iter().map(|w| w.name.as_str()).collect::<Vec<_>>(), ["other"]);
    assert!(mcp.is_file(), "a minted config stays");
    assert!(fake.seen().is_empty(), "no revoke: {:?}", fake.seen());
}

#[test]
fn removing_a_workspace_without_desired_state_stops_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    Registry::at(&home).save(&Workspace::new("demo", tmp.path(), Vec::new()), false).unwrap();
    let claude = FakeClaude { rows: RefCell::new(vec![row("aaaa0001", "fm-demo-workspace")]), stops: RefCell::default() };
    assert!(sm::remove::remove(&home, "demo", &claude).unwrap().is_empty());
    assert!(claude.stops.borrow().is_empty());
    assert!(Registry::at(&home).list().unwrap().is_empty());
    assert!(!DesiredState::path(&home, "demo").unwrap().exists(), "nothing written to be deleted");
}
