use std::sync::Arc;

use supraman_workman::Auth;

/// The test's own `server.json` naming `base`, inside `home`, never the user's; written again on each call.
pub fn server_json(home: &std::path::Path, base: &str) -> std::path::PathBuf {
    let path = home.join("test-user").join(".workman").join("server.json");
    supraman_core::workman_server::save(&path, &supraman_core::workman_server::ServerJson::new(base)).unwrap();
    path
}

pub fn signed_in(home: &std::path::Path, base: &str) -> Arc<Auth> {
    let auth = Arc::new(Auth::with_store(home, Arc::new(supraman_core::secret::MemoryStore::default())).with_server_json(Some(server_json(home, base))));
    auth.sign_in(base, &supraman_workman::auth::http_browser()).unwrap();
    auth
}

mod tests {
    use std::path::Path;

    use supraman_workman::fake::{Answer, FakeWorkman};

    use supraman_supervisor::{AppTokens, TokenProvider};

    use super::*;
    use crate::supervise::signed_in_server;

    #[test]
    fn provider_is_the_sign_in_none_without_one_an_error_when_broken() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let fake = FakeWorkman::start(vec![]);
        let oauth = fake.enable_oauth();
        let unsigned = AppTokens {
            auth: Arc::new(Auth::with_store(&home, Arc::new(supraman_core::secret::MemoryStore::default())).with_server_json(Some(server_json(&home, fake.base())))),
        };
        assert_eq!(unsigned.token("workspace"), Ok(None), "not signed in: no Workman");

        oauth.set_access_ttl(60);
        let tokens = AppTokens { auth: signed_in(&home, fake.base()) };
        let t = tokens.token("workspace").unwrap().unwrap();
        assert_eq!((t.url.as_str(), t.role.as_str()), (format!("{}/mcp", fake.base()).as_str(), "controller"));
        assert!(t.token.starts_with("eyJ"));
        assert_eq!(tokens.token("repo").unwrap().unwrap().role, "orchestrator", "minted as the Workman role it acts as");
        assert_eq!(tokens.token("relay"), Ok(None), "the relay has no Workman");
        assert_eq!(tokens.token("worker"), Ok(None));

        oauth.fail_refresh(true);
        let e = tokens.token("workspace").unwrap_err();
        assert!(e.contains("again") && !e.contains("fake-"), "{e}");
    }

    struct NoClaude(std::cell::RefCell<Vec<Vec<String>>>);

    impl supraman_supervisor::Launcher for NoClaude {
        fn launch(&self, spec: &supraman_claude::LaunchSpec) -> supraman_claude::Result<String> {
            self.0.borrow_mut().push(spec.argv().iter().map(|a| a.to_string_lossy().into_owned()).collect());
            Ok("backgrounded · aaaa0001 · fm-demo-workspace".into())
        }
        fn resume(&self, _: &str, _: &Path, _: Option<&str>) -> supraman_claude::Result<String> {
            unreachable!()
        }
        fn stop(&self, _: &str) -> supraman_claude::Result<String> {
            Ok(String::new())
        }
    }

    #[test]
    fn mint_with_the_oauth_bearer_over_http_keep_on_stop_and_revoke_once_deleted() {
        use supraman_supervisor::{Budget, DesiredState, LaunchConfig, Supervisor};
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let fake = FakeWorkman::start(vec![
            (
                "/api/agent-sessions".into(),
                Answer::status(
                    201,
                    r#"{"sessionId":"91","name":"fm-demo-workspace","role":"controller","token":"wss_selftest","expiresAt":"2026-10-29T00:00:00Z"}"#,
                ),
            ),
            ("/api/agent-sessions/91".into(), Answer::status(204, "")),
        ]);
        let oauth = fake.enable_oauth();
        let auth = signed_in(&home, fake.base());
        let mut desired = DesiredState::workspace_controller("demo", r"C:\demo", "be the controller");
        desired.started = true;
        let config = LaunchConfig { mcp_dir: home.join("mcp"), supraman_home: Some(home.clone()), ..LaunchConfig::default() };
        let mut sup = Supervisor::new(desired, config, NoClaude(Default::default()), AppTokens { auth });
        let now = 1_800_000_000_000;
        sup.tick(&[], &mut Budget { max: 1, free: 1 }, now);

        let api = |method: &str| fake.seen().into_iter().filter(|s| s.method == method && s.target.starts_with("/api/agent-sessions")).collect::<Vec<_>>();
        let posts = api("POST");
        let [post] = posts.as_slice() else { panic!("{posts:?}") };
        let bearer = post.authorization.as_deref().and_then(|a| a.strip_prefix("Bearer ")).unwrap().to_string();
        assert!(oauth.issued().contains(&bearer), "the fake AS's access token is the bearer");
        let body: serde_json::Value = serde_json::from_str(&post.body).unwrap();
        assert_eq!((&body["role"], &body["externalRole"], &body["replace"]), (&"controller".into(), &"workspace".into(), &true.into()));
        let file = home.join("mcp").join("workstate-fm-demo-workspace.json");
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(sup.keeper().kept("fm-demo-workspace").as_deref() == Some("wss_selftest") && text.contains(&format!("{}/mcp", fake.base())), "{text}");
        assert!(text.contains("headersHelper") && !text.contains("Bearer") && !text.contains("wss_"), "no token in the config: {text}");
        let argv = sup.launcher().0.borrow().concat().join(" ");
        assert!(!argv.contains("wss_"), "{argv}");
        for t in oauth.issued() {
            assert!(!text.contains(&t) && !argv.contains(&t), "no OAuth token in the file or argv");
        }

        let live = workspace_row(supraman_claude::SessionState::Working, Some(7), now);
        sup.tick(std::slice::from_ref(&live), &mut Budget { max: 1, free: 1 }, now + 1_000);
        sup.desired.started = false;
        sup.tick(std::slice::from_ref(&live), &mut Budget { max: 1, free: 1 }, now + 2_000);
        assert!(api("DELETE").is_empty() && file.exists(), "a Stop keeps the token and its config");
        let other = supraman_claude::Session { session_id: "bbbb0002-0000-4000-8000-000000000002".into(), name: "fm-demo-other".into(), ..workspace_row(supraman_claude::SessionState::Stopped, None, now) };
        for t in [3_000, 4_000] {
            sup.tick(std::slice::from_ref(&other), &mut Budget { max: 1, free: 1 }, now + t);
        }
        let deletes = api("DELETE");
        assert_eq!(deletes.iter().map(|s| s.target.as_str()).collect::<Vec<_>>(), ["/api/agent-sessions/91"]);
        assert!(oauth.accepts(deletes[0].authorization.as_deref()));
        assert!(!file.exists());
        assert!(sup.take_notices().is_empty());
    }

    #[derive(Default)]
    struct Recorder(std::cell::RefCell<Vec<String>>);

    impl supraman_supervisor::Launcher for Recorder {
        fn launch(&self, spec: &supraman_claude::LaunchSpec) -> supraman_claude::Result<String> {
            self.0.borrow_mut().push(format!("launch {}", spec.name));
            Ok("backgrounded · aaaa0001 · fm-demo-workspace".into())
        }
        fn resume(&self, session_id: &str, _: &Path, _: Option<&str>) -> supraman_claude::Result<String> {
            self.0.borrow_mut().push(format!("resume {session_id}"));
            Ok("backgrounded · aaaa0001 · fm-demo-workspace".into())
        }
        fn stop(&self, id: &str) -> supraman_claude::Result<String> {
            self.0.borrow_mut().push(format!("stop {id}"));
            Ok(String::new())
        }
    }

    fn workspace_row(state: supraman_claude::SessionState, pid: Option<u32>, now: i64) -> supraman_claude::Session {
        supraman_claude::Session {
            id: "aaaa0001".into(),
            session_id: "aaaa0001-0000-4000-8000-000000000001".into(),
            name: "fm-demo-workspace".into(),
            cwd: r"C:\demo".into(),
            kind: supraman_claude::Kind::Background,
            state: Some(state),
            status: pid.map(|_| supraman_claude::Status::Busy),
            waiting_for: None,
            pid,
            started_at: Some(now - 3_600_000),
        }
    }

    #[test]
    fn a_plain_sign_in_on_another_server_restarts_each_session_minted_on_the_old_one() {
        use supraman_claude::SessionState;
        use supraman_supervisor::{Budget, DesiredState, LaunchConfig, Supervisor};
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let minted = |id: &str, token: &str| {
            let body = format!(r#"{{"sessionId":"{id}","name":"fm-demo-workspace","role":"controller","token":"{token}"}}"#);
            vec![("/api/agent-sessions".to_string(), Answer::status(201, body))]
        };
        let a = FakeWorkman::start(minted("91", "wss_from_a"));
        a.enable_oauth();
        let b = FakeWorkman::start(minted("92", "wss_from_b"));
        b.enable_oauth();
        let auth = signed_in(&home, a.base());
        let mut desired = DesiredState::workspace_controller("demo", r"C:\demo", "be the controller");
        desired.started = true;
        let config = LaunchConfig { mcp_dir: home.join("mcp"), supraman_home: Some(home.clone()), ..LaunchConfig::default() };
        let mut sup = Supervisor::new(desired, config, Recorder::default(), AppTokens { auth: auth.clone() });
        let now = 1_800_000_000_000;
        let budget = || Budget { max: 3, free: 3 };
        let server = || signed_in_server(&auth);
        assert!(sup.server_changed(server().as_deref()).is_empty());
        sup.tick(&[], &mut budget(), now);
        let file = home.join("mcp").join("workstate-fm-demo-workspace.json");
        assert!(std::fs::read_to_string(&file).unwrap().contains(&format!("{}/mcp", a.base())));

        server_json(&home, b.base());
        assert_eq!(server(), None, "not signed in to B yet");
        assert!(sup.server_changed(server().as_deref()).is_empty());
        auth.sign_in(b.base(), &supraman_workman::auth::http_browser()).unwrap();
        assert_eq!(sup.server_changed(server().as_deref()), [("fm-demo-workspace".to_string(), Some(a.base().to_string()))]);

        sup.launcher().0.borrow_mut().clear();
        let live = workspace_row(SessionState::Working, Some(7), now);
        sup.tick(std::slice::from_ref(&live), &mut budget(), now + 61_000);
        assert_eq!(*sup.launcher().0.borrow(), ["stop aaaa0001"]);
        let stopped = workspace_row(SessionState::Stopped, None, now);
        sup.tick(std::slice::from_ref(&stopped), &mut budget(), now + 130_000);
        assert_eq!(*sup.launcher().0.borrow(), ["stop aaaa0001", "resume aaaa0001-0000-4000-8000-000000000001"]);
        let text = std::fs::read_to_string(&file).unwrap();
        let kept = sup.keeper().kept("fm-demo-workspace").unwrap_or_default();
        assert!(text.contains(&format!("{}/mcp", b.base())) && kept == "wss_from_b" && kept != "wss_from_a", "{text}");
        assert!(text.contains("headersHelper") && !text.contains("Bearer") && !text.contains("wss_"), "no token in the config: {text}");
        assert_eq!(sup.minted().get("fm-demo-workspace").unwrap().origin.as_deref(), Some(b.base()));
        let posts = b.seen().into_iter().filter(|s| s.method == "POST" && s.target == "/api/agent-sessions").count();
        assert_eq!(posts, 1, "minted on B");
        assert!(a.seen().iter().all(|s| s.method != "DELETE"), "nothing of A's is deleted on A by session id");
    }
}
