//! Workman's status as the app finds it (`detect`) and Settings → Workman's check of server.json; the words are `supraman_workman::status`'s.

use std::path::Path;
use std::time::Duration;

use supraman_core::{paths, workman_server};

use supraman_workman::{Auth, AuthError, AuthStatus};
use supraman_workman::status::WorkmanStatus;

const HEALTH_TIMEOUT: Duration = Duration::from_secs(3);

/// Blocks up to a few seconds; without a server.json nothing dials.
pub fn detect(auth: Option<&Auth>) -> WorkmanStatus {
    let Some(auth) = auth else {
        return WorkmanStatus::NotSetUp { why: "No Supraman home (set SUPRAMAN_HOME or APPDATA), so no Workman sign-in.".into() };
    };
    let server = match auth.server() {
        Ok(server) => server,
        Err(e) => return WorkmanStatus::NotSetUp { why: e.to_string() },
    };
    let url = server.origin();
    if let Err(error) = supraman_workman::health(&url, HEALTH_TIMEOUT) {
        return WorkmanStatus::Unreachable { url, error };
    }
    match auth.access_token(&url) {
        Ok(_) => {
            let who = match auth.status(&url) {
                AuthStatus::SignedIn { who, .. } => who,
                _ => None,
            };
            WorkmanStatus::Ready { url, who, warning: server.warning() }
        }
        Err(AuthError::NotSignedIn { .. }) => WorkmanStatus::NotSignedIn { url },
        Err(AuthError::SignInAgain { why, .. }) => WorkmanStatus::SignInAgain { url, why },
        Err(AuthError::Unreachable { reason, .. }) => WorkmanStatus::Unreachable { url, error: reason },
        Err(e) => WorkmanStatus::SignInAgain { url, why: e.to_string() },
    }
}

/// Settings → Workman's look at server.json and at claude's own entry for its server.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServerCheck {
    /// server.json's `mcpName`; `None` without a usable server.json, which the status says.
    pub mcp_name: Option<String>,
    pub warnings: Vec<String>,
}

/// Blocking: reads server.json and only the `workman` entry's url in claude's config ([`paths::claude_json`]).
pub fn check_server(auth: &Auth, claude_json: Option<&Path>) -> ServerCheck {
    let Ok(server) = auth.server() else {
        return ServerCheck::default();
    };
    let claude = match claude_json {
        Some(path) => workman_server::claude_warning(&server, path, paths::user_mcp_server(path, workman_server::MCP_NAME)),
        None => Some("claude's Workman server is not checked: no user home (USERPROFILE is not set).".into()),
    };
    ServerCheck { warnings: server.warning().into_iter().chain(claude).collect(), mcp_name: Some(server.mcp_name) }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use supraman_core::secret::MemoryStore;
    use supraman_workman::fake::{Answer, FakeWorkman};

    use super::*;

    #[test]
    fn signed_out_signed_in_and_broken_each_say_what_to_do() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let fake = FakeWorkman::start(vec![("/health".into(), Answer::text("Healthy"))]);
        let oauth = fake.enable_oauth();
        let url = fake.base().to_string();

        let fresh = Auth::with_store(&home, Arc::new(MemoryStore::default())).with_server_json(Some(crate::test_auth::server_json(&home, fake.base())));
        let out = detect(Some(&fresh));
        assert_eq!(out, WorkmanStatus::NotSignedIn { url: url.clone() });
        assert!(out.signed_out() && out.problem().unwrap().contains("Not signed in") && out.fix().unwrap().contains("Sign in"));
        assert!(out.start_warning("demo").unwrap().contains("without Workman"));

        oauth.set_access_ttl(60);
        let auth = crate::test_auth::signed_in(&home, fake.base());
        assert_eq!(detect(Some(&auth)), WorkmanStatus::Ready { url: url.clone(), who: Some("Fake Operator".into()), warning: None });
        assert!(fake.seen().iter().filter(|s| s.target == "/health").all(|s| s.authorization.is_none()), "/health carries no token");

        oauth.fail_refresh(true);
        let broken = detect(Some(&auth));
        assert!(matches!(&broken, WorkmanStatus::SignInAgain { why, .. } if why.contains("invalid_grant")), "{broken:?}");
        assert!(broken.signed_out() && broken.problem().unwrap().contains("again"));
        let text = format!("{} {broken:?}", broken.message().unwrap());
        for t in oauth.issued() {
            assert!(!text.contains(&t), "{text}");
        }
    }

    #[test]
    fn settings_warns_of_claudes_workman_entry_and_of_another_mcp_name_and_of_neither_once_both_agree() {
        let dir = tempfile::tempdir().unwrap();
        let server_json = dir.path().join("user").join(".workman").join("server.json");
        let claude_json = dir.path().join("user").join(".claude.json");
        let auth = Auth::with_store(dir.path(), Arc::new(MemoryStore::default())).with_server_json(Some(server_json.clone()));
        assert_eq!(check_server(&auth, Some(&claude_json)), ServerCheck::default(), "no server.json: the status says so");

        supraman_core::workman_server::save(&server_json, &supraman_core::workman_server::ServerJson::new("https://work.example")).unwrap();
        let missing = check_server(&auth, Some(&claude_json));
        assert_eq!(missing.mcp_name.as_deref(), Some("workman"));
        assert!(matches!(missing.warnings.as_slice(), [w] if w.contains("does not exist")), "{missing:?}");

        let entry = |url: &str| format!(r#"{{"mcpServers":{{"workman":{{"type":"http","url":"{url}","headers":{{"Authorization":"Bearer SENTINEL"}}}}}}}}"#);
        std::fs::write(&claude_json, entry("https://elsewhere.example/mcp")).unwrap();
        let differs = check_server(&auth, Some(&claude_json));
        assert!(matches!(differs.warnings.as_slice(), [w] if w.contains("is not https://work.example/mcp") && !w.contains("SENTINEL")), "{differs:?}");

        std::fs::write(&claude_json, entry("https://work.example/mcp")).unwrap();
        assert_eq!(check_server(&auth, Some(&claude_json)).warnings, Vec::<String>::new());

        std::fs::write(&server_json, r#"{"url":"https://work.example","mcpName":"ws"}"#).unwrap();
        let named = check_server(&auth, Some(&claude_json));
        assert_eq!(named.mcp_name.as_deref(), Some("ws"));
        assert!(matches!(named.warnings.as_slice(), [w] if w.contains("\"ws\"")), "{named:?}");
        assert!(check_server(&auth, None).warnings.iter().any(|w| w.contains("not checked")));
    }

    #[test]
    fn a_ready_status_carries_server_jsons_warning_as_the_trees_notice() {
        let warning = Some("names the Workman MCP server \"ws\"".to_string());
        let ready = WorkmanStatus::Ready { url: "https://work.example".into(), who: None, warning: warning.clone() };
        assert_eq!(ready.problem(), warning);
        assert_eq!((ready.fix(), ready.start_warning("demo")), (None, None));
    }

    #[test]
    fn a_closed_port_is_unreachable_before_any_sign_in() {
        let dir = tempfile::tempdir().unwrap();
        let closed = supraman_workman::closed_base();
        let auth = Auth::with_store(dir.path(), Arc::new(MemoryStore::default())).with_server_json(Some(crate::test_auth::server_json(dir.path(), &closed)));
        let down = detect(Some(&auth));
        assert_eq!(down, WorkmanStatus::Unreachable { url: closed.clone(), error: "connection refused".into() });
        assert!(down.problem().unwrap().contains(&closed));
        assert!(down.fix().unwrap().contains("Settings → Workman"));
        assert!(down.start_warning("demo").unwrap().contains("unreachable"));
        assert_eq!(WorkmanStatus::Ready { url: closed, who: None, warning: None }.start_warning("demo"), None);
    }

    #[test]
    fn without_server_json_it_says_run_workman_setup_and_nothing_dials() {
        let dir = tempfile::tempdir().unwrap();
        let fake = FakeWorkman::start(vec![("/health".into(), Answer::text("Healthy"))]);
        let missing = dir.path().join("user").join(".workman").join("server.json");
        let auth = Auth::with_store(dir.path(), Arc::new(MemoryStore::default())).with_server_json(Some(missing));
        let out = detect(Some(&auth));
        let WorkmanStatus::NotSetUp { why } = &out else { panic!("{out:?}") };
        assert!(why.starts_with("Workman is not set up: run /workman:setup"), "{why}");
        assert_eq!(out.problem().as_deref(), Some(why.as_str()));
        assert!(!out.signed_out() && !out.unreachable() && out.checked_url().is_none());
        assert!(out.start_warning("demo").unwrap().contains("/workman:setup"));
        assert!(matches!(detect(None), WorkmanStatus::NotSetUp { .. }));
        assert!(fake.seen().is_empty(), "no /health, no sign-in: {:?}", fake.seen().iter().map(|s| s.target.clone()).collect::<Vec<_>>());
    }
}
