//! The bearer of every mint the app makes (m5/43): the access token of the app's WorkState sign-in
//! ([`crate::signin`]), for the server `settings.toml` names. Not signed in: sessions launch without
//! WorkState (`--strict-mcp-config`, so never with the user-scope server). Signed in but without a token
//! now (unreachable, or the sign-in must be done again): the launch fails with the reason. The token is
//! never logged and never reaches a file: only the session's own minted `wss_` token does
//! ([`foreman_supervisor::mint`]).

use std::sync::Arc;

use foreman_supervisor::{RoleToken, TokenProvider};
use foreman_workstate::{Auth, AuthError};

pub struct AppTokens {
    pub auth: Arc<Auth>,
}

impl TokenProvider for AppTokens {
    fn token(&self, role: &str) -> Result<Option<RoleToken>, String> {
        let Some(ws_role) = workstate_role(role) else {
            return Ok(None);
        };
        Ok(bearer(&self.auth)?.map(|(url, token)| RoleToken { url, token, role: ws_role.into() }))
    }
}

/// (the MCP URL a minted config names, the access token); `None` when not signed in.
pub fn bearer(auth: &Auth) -> Result<Option<(String, String)>, String> {
    match auth.current() {
        Ok((origin, token)) => Ok(Some((format!("{origin}/mcp"), token))),
        Err(AuthError::NotSignedIn { .. }) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// WorkState knows controller, orchestrator and worker. A workspace session runs the workspace's
/// queue (controller); a repo session runs one repo's units (orchestrator). The relay has no
/// WorkState role: it only carries messages, and launches without WorkState.
pub fn workstate_role(role: &str) -> Option<&'static str> {
    match role {
        "workspace" => Some("controller"),
        "repo" | "repo-controller" => Some("orchestrator"),
        _ => None,
    }
}

/// An [`Auth`] for `home`, signed in to `fake` through its fake OAuth server (refresh tokens in memory).
#[cfg(test)]
pub fn signed_in(home: &std::path::Path, base: &str) -> Arc<Auth> {
    let mut settings = foreman_core::Settings::load(home).unwrap_or_default();
    settings.workstate_url = Some(base.to_string());
    settings.save(home).unwrap();
    let auth = Arc::new(Auth::with_store(home, Arc::new(foreman_workstate::MemoryStore::default())));
    auth.sign_in(base, &foreman_workstate::auth::http_browser()).unwrap();
    auth
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use foreman_core::Settings;
    use foreman_workstate::fake::{Answer, FakeWorkState};

    use super::*;
    use crate::supervise::signed_in_server;

    #[test]
    fn provider_is_the_sign_in_none_without_one_an_error_when_broken() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let fake = FakeWorkState::start(vec![]);
        let oauth = fake.enable_oauth();
        Settings { workstate_url: Some(fake.base().into()), ..Settings::default() }.save(&home).unwrap();
        let unsigned = AppTokens { auth: Arc::new(Auth::with_store(&home, Arc::new(foreman_workstate::MemoryStore::default()))) };
        assert_eq!(unsigned.token("workspace"), Ok(None), "not signed in: no WorkState");

        oauth.set_access_ttl(60);
        let tokens = AppTokens { auth: signed_in(&home, fake.base()) };
        let t = tokens.token("workspace").unwrap().unwrap();
        assert_eq!((t.url.as_str(), t.role.as_str()), (format!("{}/mcp", fake.base()).as_str(), "controller"));
        assert!(t.token.starts_with("eyJ"));
        assert_eq!(tokens.token("repo").unwrap().unwrap().role, "orchestrator", "minted as the WorkState role it acts as");
        assert_eq!(tokens.token("relay"), Ok(None), "the relay has no WorkState");
        assert_eq!(tokens.token("worker"), Ok(None));

        oauth.fail_refresh(true);
        let e = tokens.token("workspace").unwrap_err();
        assert!(e.contains("again") && !e.contains("fake-"), "{e}");
    }

    struct NoClaude(std::cell::RefCell<Vec<Vec<String>>>);

    impl foreman_supervisor::Launcher for NoClaude {
        fn launch(&self, spec: &foreman_claude::LaunchSpec) -> foreman_claude::Result<String> {
            self.0.borrow_mut().push(spec.argv().iter().map(|a| a.to_string_lossy().into_owned()).collect());
            Ok("backgrounded · aaaa0001 · fm-demo-workspace".into())
        }
        fn resume(&self, _: &str, _: &Path, _: Option<&str>) -> foreman_claude::Result<String> {
            unreachable!()
        }
        fn stop(&self, _: &str) -> foreman_claude::Result<String> {
            Ok(String::new())
        }
    }

    #[test]
    fn mint_with_the_oauth_bearer_over_http_then_revoke_on_stop() {
        use foreman_supervisor::{Budget, DesiredState, LaunchConfig, Supervisor};
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let fake = FakeWorkState::start(vec![
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
        let config = LaunchConfig { mcp_dir: home.join("mcp"), foreman_home: Some(home.clone()), ..LaunchConfig::default() };
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
        assert!(text.contains("Bearer wss_selftest") && text.contains(&format!("{}/mcp", fake.base())), "{text}");
        let argv = sup.launcher().0.borrow().concat().join(" ");
        assert!(!argv.contains("wss_"), "{argv}");
        for t in oauth.issued() {
            assert!(!text.contains(&t) && !argv.contains(&t), "no OAuth token in the file or argv");
        }

        sup.desired.started = false;
        sup.tick(&[], &mut Budget { max: 1, free: 1 }, now + 1_000);
        let deletes = api("DELETE");
        assert_eq!(deletes.iter().map(|s| s.target.as_str()).collect::<Vec<_>>(), ["/api/agent-sessions/91"]);
        assert!(oauth.accepts(deletes[0].authorization.as_deref()));
        assert!(!file.exists());
        assert!(sup.take_notices().is_empty());
    }

    /// Records every stop and resume; launches and resumes print a start in place.
    #[derive(Default)]
    struct Recorder(std::cell::RefCell<Vec<String>>);

    impl foreman_supervisor::Launcher for Recorder {
        fn launch(&self, spec: &foreman_claude::LaunchSpec) -> foreman_claude::Result<String> {
            self.0.borrow_mut().push(format!("launch {}", spec.name));
            Ok("backgrounded · aaaa0001 · fm-demo-workspace".into())
        }
        fn resume(&self, session_id: &str, _: &Path, _: Option<&str>) -> foreman_claude::Result<String> {
            self.0.borrow_mut().push(format!("resume {session_id}"));
            Ok("backgrounded · aaaa0001 · fm-demo-workspace".into())
        }
        fn stop(&self, id: &str) -> foreman_claude::Result<String> {
            self.0.borrow_mut().push(format!("stop {id}"));
            Ok(String::new())
        }
    }

    fn workspace_row(state: foreman_claude::SessionState, pid: Option<u32>, now: i64) -> foreman_claude::Session {
        foreman_claude::Session {
            id: "aaaa0001".into(),
            session_id: "aaaa0001-0000-4000-8000-000000000001".into(),
            name: "fm-demo-workspace".into(),
            cwd: r"C:\demo".into(),
            kind: foreman_claude::Kind::Background,
            state: Some(state),
            status: pid.map(|_| foreman_claude::Status::Busy),
            waiting_for: None,
            pid,
            started_at: Some(now - 3_600_000),
        }
    }

    #[test]
    fn a_plain_sign_in_on_another_server_restarts_each_session_minted_on_the_old_one() {
        use foreman_claude::SessionState;
        use foreman_supervisor::{Budget, DesiredState, LaunchConfig, Supervisor};
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let minted = |id: &str, token: &str| {
            let body = format!(r#"{{"sessionId":"{id}","name":"fm-demo-workspace","role":"controller","token":"{token}"}}"#);
            vec![("/api/agent-sessions".to_string(), Answer::status(201, body))]
        };
        let a = FakeWorkState::start(minted("91", "wss_from_a"));
        a.enable_oauth();
        let b = FakeWorkState::start(minted("92", "wss_from_b"));
        b.enable_oauth();
        let auth = signed_in(&home, a.base());
        let mut desired = DesiredState::workspace_controller("demo", r"C:\demo", "be the controller");
        desired.started = true;
        let config = LaunchConfig { mcp_dir: home.join("mcp"), foreman_home: Some(home.clone()), ..LaunchConfig::default() };
        let mut sup = Supervisor::new(desired, config, Recorder::default(), AppTokens { auth: auth.clone() });
        let now = 1_800_000_000_000;
        let budget = || Budget { max: 3, free: 3 };
        let server = || signed_in_server(&auth, &Settings::load(&home).unwrap());
        assert!(sup.server_changed(server().as_deref()).is_empty());
        sup.tick(&[], &mut budget(), now);
        let file = home.join("mcp").join("workstate-fm-demo-workspace.json");
        assert!(std::fs::read_to_string(&file).unwrap().contains(&format!("{}/mcp", a.base())));

        // the URL saved on its own, then a plain sign-in there
        let mut settings = Settings::load(&home).unwrap();
        settings.workstate_url = Some(b.base().into());
        settings.save(&home).unwrap();
        assert_eq!(server(), None, "not signed in to B yet");
        assert!(sup.server_changed(server().as_deref()).is_empty());
        auth.sign_in(b.base(), &foreman_workstate::auth::http_browser()).unwrap();
        assert_eq!(sup.server_changed(server().as_deref()), [("fm-demo-workspace".to_string(), Some(a.base().to_string()))]);

        sup.launcher().0.borrow_mut().clear();
        let live = workspace_row(SessionState::Working, Some(7), now);
        sup.tick(std::slice::from_ref(&live), &mut budget(), now + 61_000);
        assert_eq!(*sup.launcher().0.borrow(), ["stop aaaa0001"]);
        let stopped = workspace_row(SessionState::Stopped, None, now);
        sup.tick(std::slice::from_ref(&stopped), &mut budget(), now + 130_000);
        assert_eq!(*sup.launcher().0.borrow(), ["stop aaaa0001", "resume aaaa0001-0000-4000-8000-000000000001"]);
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains(&format!("{}/mcp", b.base())) && text.contains("wss_from_b") && !text.contains("wss_from_a"), "{text}");
        assert_eq!(sup.minted().get("fm-demo-workspace").unwrap().origin.as_deref(), Some(b.base()));
        let posts = b.seen().into_iter().filter(|s| s.method == "POST" && s.target == "/api/agent-sessions").count();
        assert_eq!(posts, 1, "minted on B");
        assert!(a.seen().iter().all(|s| s.method != "DELETE"), "nothing of A's is deleted on A by session id");
    }
}
