//! The OAuth sign-in (m5/43) against the fake authorization server: discovery, DCR, PKCE S256, one
//! resource, the loopback with its state check, the code exchange, refresh rotation, a refused refresh,
//! re-registration after `invalid_client`, sign-out's revoke, and the 401 a read gets without a token.
//! The browser is an HTTP GET from the test; refresh tokens go to a [`MemoryStore`].

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use foreman_core::Settings;
use foreman_workstate::auth::{http_browser, http_browser_with};
use foreman_workstate::fake::{Answer, FakeWorkState, location_query};
use foreman_workstate::{Auth, AuthError, AuthStatus, Client, MemoryStore, SecretStore};

fn overview() -> Answer {
    Answer::json(r#"{"asOf":"2026-10-28T10:00:00+00:00","seq":1,"projects":[],"workingNow":[],"inboxCount":0}"#)
}

fn setup() -> (tempfile::TempDir, FakeWorkState, Arc<foreman_workstate::fake::FakeAuth>, Arc<MemoryStore>, Arc<Auth>) {
    let dir = tempfile::tempdir().unwrap();
    let fake = FakeWorkState::start(vec![("/api/v1/overview".into(), overview())]);
    let oauth = fake.enable_oauth();
    Settings { workstate_url: Some(fake.base().to_string()), ..Settings::default() }.save(dir.path()).unwrap();
    let store = Arc::new(MemoryStore::default());
    let auth = Arc::new(Auth::with_store(dir.path(), store.clone()).with_sign_in_timeout(Duration::from_secs(10)));
    (dir, fake, oauth, store, auth)
}

/// Every file under `dir`, read as text.
fn files(dir: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(files(&p));
        } else {
            out.push((p.display().to_string(), String::from_utf8_lossy(&std::fs::read(&p).unwrap()).into_owned()));
        }
    }
    out
}

#[test]
fn sign_in_discovers_registers_and_reads_with_pkce_and_one_resource() {
    let (dir, fake, oauth, store, auth) = setup();
    let asked = Arc::new(Mutex::new(String::new()));
    let seen_url = asked.clone();
    let browser = http_browser();
    let status = auth
        .sign_in(fake.base(), &|url: &str| {
            *seen_url.lock().unwrap() = url.to_string();
            browser(url)
        })
        .unwrap();
    assert_eq!(status, AuthStatus::SignedIn { who: Some("Fake Operator".into()) });
    assert_eq!(oauth.registrations(), 1);

    let url = asked.lock().unwrap().clone();
    assert!(url.starts_with(&format!("{}/connect/authorize?", fake.base())), "{url}");
    let q = location_query(&url);
    assert_eq!(url.matches("resource=").count(), 1, "exactly one resource");
    assert_eq!(q["resource"], format!("{}/api", fake.base()));
    assert_eq!(q["scope"], "workstate.read workstate.sessions offline_access");
    assert_eq!((q["code_challenge_method"].as_str(), q["response_type"].as_str()), ("S256", "code"));
    assert!(q["redirect_uri"].starts_with("http://127.0.0.1:") && q["redirect_uri"].ends_with("/callback"));
    assert!(q["state"].len() >= 16 && q["code_challenge"].len() == 43);

    assert!(store.get(&oauth.issuer()).unwrap().is_some(), "the refresh token, keyed by issuer");
    let known = std::fs::read_to_string(dir.path().join("oauth").join("servers.json")).unwrap();
    assert!(known.contains("fake-client-1") && known.contains(&oauth.issuer()), "{known}");

    let client = Client::with_source(fake.base(), auth.bearer_for(fake.base()));
    client.overview().unwrap();
    let read = fake.seen().into_iter().find(|s| s.target == "/api/v1/overview").unwrap();
    assert!(read.authorization.as_deref().unwrap().starts_with("Bearer eyJ"), "the OAuth access token");

    // c2: no token in any of Foreman's files
    let issued = oauth.issued();
    assert_eq!(issued.len(), 2);
    for (path, text) in files(dir.path()) {
        for t in &issued {
            assert!(!text.contains(t.as_str()), "{path} holds a token");
        }
    }
    assert!(!format!("{auth:?}").contains("fake-"), "Debug says no token");
}

#[test]
fn refresh_rotates_and_stores_the_new_refresh_token() {
    let (_dir, fake, oauth, store, auth) = setup();
    oauth.set_access_ttl(60); // less than the 5 min margin: every use refreshes
    auth.sign_in(fake.base(), &http_browser()).unwrap();
    let first = store.get(&oauth.issuer()).unwrap().unwrap();
    let a1 = auth.access_token(fake.base()).unwrap();
    let second = store.get(&oauth.issuer()).unwrap().unwrap();
    assert_ne!(first, second, "rotated");
    let a2 = auth.access_token(fake.base()).unwrap();
    let third = store.get(&oauth.issuer()).unwrap().unwrap();
    assert_ne!(second, third);
    assert_ne!(a1, a2);
    let refreshes = fake.seen().iter().filter(|s| s.target == "/connect/token" && s.body.contains("grant_type=refresh_token")).count();
    assert_eq!(refreshes, 2);

    // a long-lived token is held, not refreshed on every use
    oauth.set_access_ttl(3600);
    let a3 = auth.access_token(fake.base()).unwrap();
    assert_eq!(auth.access_token(fake.base()).unwrap(), a3);
}

#[test]
fn a_fresh_process_refreshes_from_the_store() {
    let (dir, fake, _oauth, store, auth) = setup();
    auth.sign_in(fake.base(), &http_browser()).unwrap();
    let other = Auth::with_store(dir.path(), store.clone());
    assert_eq!(other.status(fake.base()), AuthStatus::SignedIn { who: None }, "known without a call");
    let (origin, token) = other.current().unwrap();
    assert_eq!(origin, fake.base());
    assert!(token.starts_with("eyJ"));
    assert_eq!(other.status(fake.base()), AuthStatus::SignedIn { who: Some("Fake Operator".into()) });
}

#[test]
fn a_state_that_is_not_ours_signs_nothing_in() {
    let (_dir, fake, oauth, store, auth) = setup();
    let browser = http_browser_with(|location| location.replace("state=", "state=x"));
    assert_eq!(auth.sign_in(fake.base(), &browser), Err(AuthError::StateMismatch));
    assert_eq!(store.get(&oauth.issuer()).unwrap(), None);
    assert_eq!(auth.status(fake.base()), AuthStatus::NotSignedIn);
    assert!(!fake.seen().iter().any(|s| s.target == "/connect/token" && s.body.contains("authorization_code")), "no exchange");
}

#[test]
fn a_refused_or_unfinished_sign_in_says_why() {
    let (_dir, fake, oauth, _store, auth) = setup();
    oauth.deny(true);
    assert!(matches!(auth.sign_in(fake.base(), &http_browser()), Err(AuthError::Denied(why)) if why == "access_denied"));
    let short = Auth::with_store(_dir.path(), Arc::new(MemoryStore::default())).with_sign_in_timeout(Duration::from_millis(300));
    assert_eq!(short.sign_in(fake.base(), &|_: &str| Ok(())), Err(AuthError::TimedOut));
    assert!(matches!(short.sign_in(fake.base(), &|_: &str| Err("no browser".into())), Err(AuthError::Browser(_))));
}

#[test]
fn every_sign_in_registers_a_fresh_client_kept_with_its_refresh_token() {
    let (dir, fake, oauth, _store, auth) = setup();
    auth.sign_in(fake.base(), &http_browser()).unwrap();
    auth.sign_in(fake.base(), &http_browser()).unwrap();
    assert_eq!(oauth.registrations(), 2, "a cached client_id is never sent to authorize");
    let known = std::fs::read_to_string(dir.path().join("oauth").join("servers.json")).unwrap();
    assert!(known.contains("fake-client-2") && !known.contains("fake-client-1"), "{known}");
    assert!(auth.access_token(fake.base()).is_ok());
}

/// The server's database restored (every client and grant gone): the refresh is refused, the pair is
/// dropped, "Sign in again"; the next sign-in registers afresh and works.
#[test]
fn a_wiped_server_is_sign_in_again_then_a_fresh_registration() {
    let (dir, fake, oauth, store, auth) = setup();
    oauth.set_access_ttl(60);
    auth.sign_in(fake.base(), &http_browser()).unwrap();
    oauth.forget_clients();
    let e = auth.access_token(fake.base()).unwrap_err();
    assert!(matches!(&e, AuthError::SignInAgain { why, .. } if why.contains("invalid_client")), "{e}");
    assert!(matches!(auth.status(fake.base()), AuthStatus::SignInAgain(_)));
    assert_eq!(store.get(&oauth.issuer()).unwrap(), None, "the refresh token is dropped");
    let known = std::fs::read_to_string(dir.path().join("oauth").join("servers.json")).unwrap();
    assert!(!known.contains("fake-client-1"), "and its client_id: {known}");
    let fresh = Auth::with_store(dir.path(), store.clone());
    assert_eq!(fresh.status(fake.base()), AuthStatus::NotSignedIn, "a new process sees no grant");

    auth.sign_in(fake.base(), &http_browser()).unwrap();
    assert_eq!(oauth.registrations(), 2);
    assert!(auth.access_token(fake.base()).is_ok());
    assert!(store.get(&oauth.issuer()).unwrap().is_some());
}

#[test]
fn a_refused_refresh_is_sign_in_again_without_retrying() {
    let (_dir, fake, oauth, _store, auth) = setup();
    oauth.set_access_ttl(60);
    auth.sign_in(fake.base(), &http_browser()).unwrap();
    oauth.fail_refresh(true);
    let e = auth.access_token(fake.base()).unwrap_err();
    assert!(matches!(&e, AuthError::SignInAgain { why, .. } if why.contains("invalid_grant")), "{e}");
    assert!(matches!(auth.status(fake.base()), AuthStatus::SignInAgain(_)));
    let refreshes = || fake.seen().iter().filter(|s| s.body.contains("grant_type=refresh_token")).count();
    let before = refreshes();
    for _ in 0..3 {
        assert!(matches!(auth.access_token(fake.base()), Err(AuthError::SignInAgain { .. })));
    }
    assert_eq!(refreshes(), before, "no endless retry");
    let client = Client::with_source(fake.base(), auth.bearer_for(fake.base()));
    assert!(client.overview().unwrap_err().to_string().contains("sign in to WorkState"));

    oauth.fail_refresh(false);
    auth.sign_in(fake.base(), &http_browser()).unwrap();
    assert!(auth.access_token(fake.base()).is_ok(), "a sign-in clears it");
}

#[test]
fn sign_out_revokes_the_grant_and_forgets_everything() {
    let (_dir, fake, oauth, store, auth) = setup();
    auth.sign_in(fake.base(), &http_browser()).unwrap();
    let token = auth.access_token(fake.base()).unwrap();
    assert!(oauth.accepts(Some(&format!("Bearer {token}"))));
    auth.sign_out(fake.base()).unwrap();
    assert_eq!(oauth.revoked(), 1);
    let revoke = fake.seen().into_iter().find(|s| s.target == "/connect/revoke").unwrap();
    assert!(revoke.body.contains("token_type_hint=refresh_token") && revoke.body.contains("token=fake-rt-"));
    assert!(!oauth.accepts(Some(&format!("Bearer {token}"))), "the grant is gone, its access tokens with it");
    assert_eq!(store.get(&oauth.issuer()).unwrap(), None);
    assert_eq!(auth.status(fake.base()), AuthStatus::NotSignedIn);
    assert!(matches!(auth.access_token(fake.base()), Err(AuthError::NotSignedIn { .. })));
}

#[test]
fn a_read_without_a_token_is_401_with_where_to_sign_in_and_a_stale_one_is_replaced() {
    let (_dir, fake, oauth, _store, auth) = setup();
    let res = foreman_workstate::http_agent(Duration::from_secs(5)).get(format!("{}/api/v1/overview", fake.base())).call().unwrap();
    assert_eq!(res.status().as_u16(), 401);
    let www = res.headers().get("www-authenticate").unwrap().to_str().unwrap().to_string();
    assert_eq!(www, format!(r#"Bearer resource_metadata="{}/.well-known/oauth-protected-resource/api""#, fake.base()));

    auth.sign_in(fake.base(), &http_browser()).unwrap();
    let client = Client::with_source(fake.base(), auth.bearer_for(fake.base()));
    client.overview().unwrap();
    // the server forgets the access token (not the grant): the 401 makes the client refresh once
    let held = auth.access_token(fake.base()).unwrap();
    let revoke = foreman_workstate::http_agent(Duration::from_secs(5))
        .post(format!("{}/connect/revoke", fake.base()))
        .content_type("application/x-www-form-urlencoded")
        .send(format!("token={held}").as_str())
        .unwrap();
    assert_eq!(revoke.status().as_u16(), 200);
    client.overview().unwrap();
    assert_ne!(auth.access_token(fake.base()).unwrap(), held);
    assert!(oauth.revoked() == 0);
}
