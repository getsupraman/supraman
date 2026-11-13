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
    use supraman_workman::fake::FakeWorkman;

    use supraman_supervisor::{AppTokens, TokenProvider};

    use super::*;

    #[test]
    fn provider_is_the_sign_in_none_without_one_an_error_when_broken() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let fake = FakeWorkman::start(vec![]);
        let oauth = fake.enable_oauth();
        let unsigned = AppTokens {
            auth: Arc::new(Auth::with_store(&home, Arc::new(supraman_core::secret::MemoryStore::default())).with_server_json(Some(server_json(&home, fake.base())))),
        };
        assert_eq!(unsigned.token("repo"), Ok(None), "not signed in: no Workman");

        oauth.set_access_ttl(60);
        let tokens = AppTokens { auth: signed_in(&home, fake.base()) };
        let t = tokens.token("repo").unwrap().unwrap();
        assert_eq!((t.url.as_str(), t.role.as_str()), (format!("{}/mcp", fake.base()).as_str(), "orchestrator"), "a scheduled run's Workman role");
        assert!(t.token.starts_with("eyJ"));
        assert_eq!(tokens.token("worker"), Ok(None));

        oauth.fail_refresh(true);
        let e = tokens.token("repo").unwrap_err();
        assert!(e.contains("again") && !e.contains("fake-"), "{e}");
    }
}
