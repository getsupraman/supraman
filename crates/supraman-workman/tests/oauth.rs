//! The browser is an HTTP GET from the test; refresh tokens go to a [`MemoryStore`].

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use supraman_core::Settings;
use supraman_workman::auth::{http_browser, http_browser_with};
use supraman_workman::fake::{Answer, FakeWorkState, location_query};
use supraman_workman::{Auth, AuthError, AuthStatus, Client, MemoryStore, SecretStore};

fn overview() -> Answer {
    Answer::json(r#"{"asOf":"2026-10-28T10:00:00+00:00","seq":1,"projects":[],"workingNow":[],"inboxCount":0}"#)
}

fn setup() -> (tempfile::TempDir, FakeWorkState, Arc<supraman_workman::fake::FakeAuth>, Arc<MemoryStore>, Arc<Auth>) {
    let dir = tempfile::tempdir().unwrap();
    let fake = FakeWorkState::start(vec![("/api/v1/overview".into(), overview())]);
    let oauth = fake.enable_oauth();
    Settings { workstate: supraman_core::WorkStateSettings { url: Some(fake.base().to_string()) }, ..Settings::default() }.save(dir.path()).unwrap();
    let store = Arc::new(MemoryStore::default());
    let auth = Arc::new(Auth::with_store(dir.path(), store.clone()).with_sign_in_timeout(Duration::from_secs(10)));
    (dir, fake, oauth, store, auth)
}

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
fn servers_json_is_written_past_another_writers_temp_name() {
    let (dir, fake, _oauth, _store, auth) = setup();
    std::fs::create_dir_all(dir.path().join("oauth").join("servers.json.tmp")).unwrap();
    let status = auth.sign_in(fake.base(), &http_browser()).unwrap();
    assert!(matches!(status, AuthStatus::SignedIn { .. }), "{status:?}");
    assert!(std::fs::read_to_string(dir.path().join("oauth").join("servers.json")).unwrap().contains(fake.base()));
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
    assert_eq!(status, AuthStatus::SignedIn { who: Some("Fake Operator".into()), unsaved: None });
    assert_eq!(oauth.registrations(), 1);

    let url = asked.lock().unwrap().clone();
    assert!(url.starts_with(&format!("{}/connect/authorize?", fake.base())), "{url}");
    let q = location_query(&url);
    assert_eq!(url.matches("resource=").count(), 1, "exactly one resource");
    assert_eq!(q["resource"], format!("{}/api", fake.base()));
    assert_eq!(q["scope"], "workman.read workman.sessions offline_access");
    assert_eq!((q["code_challenge_method"].as_str(), q["response_type"].as_str()), ("S256", "code"));
    assert!(q["redirect_uri"].starts_with("http://127.0.0.1:") && q["redirect_uri"].ends_with("/callback"));
    assert!(q["state"].len() >= 16 && q["code_challenge"].len() == 43);

    assert!(store.get(&oauth.issuer()).unwrap().is_some(), "the refresh token, keyed by issuer");
    let known = std::fs::read_to_string(dir.path().join("oauth").join("servers.json")).unwrap();
    assert!(known.contains(&oauth.registered()[0]) && known.contains(&oauth.issuer()), "{known}");

    let client = Client::with_source(fake.base(), auth.bearer_for(fake.base()));
    client.overview().unwrap();
    let read = fake.seen().into_iter().find(|s| s.target == "/api/v1/overview").unwrap();
    assert!(read.authorization.as_deref().unwrap().starts_with("Bearer eyJ"), "the OAuth access token");

    // no token in any of Foreman's files
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
fn issuer_of_names_the_signed_in_origins_issuer_only() {
    let (_dir, fake, oauth, _store, auth) = setup();
    assert_eq!(auth.issuer_of(fake.base()), None);
    auth.sign_in(fake.base(), &http_browser()).unwrap();
    assert_eq!(auth.issuer_of(fake.base()), Some(oauth.issuer()));
    assert_eq!(auth.issuer_of(&format!("{}/mcp", fake.base())), Some(oauth.issuer()), "by the URL's origin");
    assert_eq!(auth.issuer_of("http://127.0.0.1:1"), None);
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
fn sign_in_asks_for_the_workman_scopes() {
    let (_dir, fake, oauth, _store, auth) = setup();
    auth.sign_in(fake.base(), &http_browser()).unwrap();
    assert_eq!(oauth.grant_scopes(), ["workman.read workman.sessions offline_access"]);
    let register = fake.seen().into_iter().find(|s| s.target == "/connect/register").unwrap();
    assert!(register.body.contains(r#""scope":"workman.read workman.sessions offline_access""#), "{}", register.body);
}

#[test]
fn a_grant_under_the_old_scope_names_refreshes_until_they_go_then_one_sign_in_mends_it() {
    let (_dir, fake, oauth, store, auth) = setup();
    oauth.set_access_ttl(60);
    auth.sign_in(fake.base(), &http_browser()).unwrap();
    oauth.rescope_grants("workstate.read workstate.sessions offline_access");
    let client = Client::with_source(fake.base(), auth.bearer_for(fake.base()));
    assert!(auth.access_token(fake.base()).is_ok(), "Workman 2.0 still refreshes a grant under the old names");

    oauth.drop_legacy_scopes();
    let e = auth.access_token(fake.base()).unwrap_err();
    assert!(matches!(&e, AuthError::SignInAgain { why, .. } if why.contains("invalid_scope")), "{e}");
    assert!(matches!(auth.status(fake.base()), AuthStatus::SignInAgain(_)));
    assert!(client.overview().unwrap_err().to_string().contains("sign in to Workman"));

    auth.sign_in(fake.base(), &http_browser()).unwrap();
    assert_eq!(oauth.grant_scopes().last().map(String::as_str), Some("workman.read workman.sessions offline_access"));
    let rt = store.get(&oauth.issuer()).unwrap().unwrap();
    assert!(auth.access_token(fake.base()).is_ok());
    assert_ne!(store.get(&oauth.issuer()).unwrap().unwrap(), rt, "the new grant refreshes");
    assert!(client.overview().is_ok());
}

#[test]
fn a_fresh_process_refreshes_from_the_store() {
    let (dir, fake, _oauth, store, auth) = setup();
    auth.sign_in(fake.base(), &http_browser()).unwrap();
    let other = Auth::with_store(dir.path(), store.clone());
    assert_eq!(other.status(fake.base()), AuthStatus::SignedIn { who: None, unsaved: None }, "known without a call");
    let (origin, token) = other.current().unwrap();
    assert_eq!(origin, fake.base());
    assert!(token.starts_with("eyJ"));
    assert_eq!(other.status(fake.base()), AuthStatus::SignedIn { who: Some("Fake Operator".into()), unsaved: None });
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
    let ids = oauth.registered();
    assert!(known.contains(&ids[1]) && !known.contains(&ids[0]), "{known}");
    assert!(auth.access_token(fake.base()).is_ok());
}

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
    assert!(!known.contains(&oauth.registered()[0]), "and its client_id: {known}");
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
    assert_eq!(oauth.refusals(), ["token (refresh_token) invalid_grant: refused on request"]);
    assert!(matches!(auth.status(fake.base()), AuthStatus::SignInAgain(_)));
    let refreshes = || fake.seen().iter().filter(|s| s.body.contains("grant_type=refresh_token")).count();
    let before = refreshes();
    for _ in 0..3 {
        assert!(matches!(auth.access_token(fake.base()), Err(AuthError::SignInAgain { .. })));
    }
    assert_eq!(refreshes(), before, "no endless retry");
    let client = Client::with_source(fake.base(), auth.bearer_for(fake.base()));
    assert!(client.overview().unwrap_err().to_string().contains("sign in to Workman"));

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
    let res = supraman_workman::http_agent(Duration::from_secs(5)).get(format!("{}/api/v1/overview", fake.base())).call().unwrap();
    assert_eq!(res.status().as_u16(), 401);
    let www = res.headers().get("www-authenticate").unwrap().to_str().unwrap().to_string();
    assert_eq!(www, format!(r#"Bearer resource_metadata="{}/.well-known/oauth-protected-resource/api""#, fake.base()));

    auth.sign_in(fake.base(), &http_browser()).unwrap();
    let client = Client::with_source(fake.base(), auth.bearer_for(fake.base()));
    client.overview().unwrap();
    // the server forgets the access token (not the grant): the 401 makes the client refresh once
    let held = auth.access_token(fake.base()).unwrap();
    let revoke = supraman_workman::http_agent(Duration::from_secs(5))
        .post(format!("{}/connect/revoke", fake.base()))
        .content_type("application/x-www-form-urlencoded")
        .send(format!("token={held}").as_str())
        .unwrap();
    assert_eq!(revoke.status().as_u16(), 200);
    client.overview().unwrap();
    assert_ne!(auth.access_token(fake.base()).unwrap(), held);
    assert!(oauth.revoked() == 0);
}

fn revoke_at(home: &Path, origin: &str, url: &str) {
    let path = home.join("oauth").join("servers.json");
    let mut known: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    known["servers"][origin]["revocation_endpoint"] = url.into();
    std::fs::write(&path, known.to_string()).unwrap();
}

#[test]
fn signing_out_of_an_unreachable_old_server_is_quick_once_and_blocks_no_other_token() {
    let (dir, old, _old_oauth, _store, auth) = setup();
    auth.sign_in(old.base(), &http_browser()).unwrap();
    let new = FakeWorkState::start(vec![]);
    new.enable_oauth();
    auth.sign_in(new.base(), &http_browser()).unwrap();
    // accepts the connection and never answers: the slowest a gone server can be
    let silent = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    revoke_at(dir.path(), old.base(), &format!("http://{}/connect/revoke", silent.local_addr().unwrap()));

    let signing_out = auth.clone();
    let origin = old.base().to_string();
    let started = std::time::Instant::now();
    let revoke = std::thread::spawn(move || signing_out.sign_out(&origin));
    std::thread::sleep(Duration::from_millis(300));
    let asked = std::time::Instant::now();
    auth.access_token(new.base()).unwrap();
    assert!(asked.elapsed() < Duration::from_secs(1), "the new server's token waited {:?} on the revoke", asked.elapsed());
    let e = revoke.join().unwrap().unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(8), "{:?}", started.elapsed());
    assert!(e.contains("lapses"), "{e}");
    assert_eq!(auth.status(old.base()), AuthStatus::NotSignedIn, "forgotten even though the revoke failed");
    assert_eq!(auth.sign_out(old.base()), Ok(()), "nothing left: no second notice");
    assert!(matches!(auth.status(new.base()), AuthStatus::SignedIn { .. }));

    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
    auth.sign_in(old.base(), &http_browser()).unwrap();
    revoke_at(dir.path(), old.base(), &format!("http://{closed}/connect/revoke"));
    let started = std::time::Instant::now();
    assert!(auth.sign_out(old.base()).is_err());
    assert!(started.elapsed() < Duration::from_secs(3), "a closed port answers at once: {:?}", started.elapsed());
    drop(silent);
}

#[test]
fn the_fake_names_why_it_refuses_a_refresh_and_its_ids_are_not_guessable() {
    let (_dir, fake, oauth, store, auth) = setup();
    oauth.set_access_ttl(60);
    auth.sign_in(fake.base(), &http_browser()).unwrap();
    let first = store.get(&oauth.issuer()).unwrap().unwrap();
    auth.access_token(fake.base()).unwrap();
    let other = FakeWorkState::start(vec![]);
    let other_oauth = other.enable_oauth();
    Auth::with_store(_dir.path(), Arc::new(MemoryStore::default())).sign_in(other.base(), &http_browser()).unwrap();
    for id in oauth.registered().iter().chain(oauth.issued().iter()) {
        assert!(!other_oauth.registered().contains(id) && !other_oauth.issued().contains(id), "an id repeats across fakes");
    }
    store.set(&oauth.issuer(), &first).unwrap();
    assert!(auth.access_token(fake.base()).is_err());
    auth.sign_in(fake.base(), &http_browser()).unwrap();
    store.set(&oauth.issuer(), "fake-rt-never-issued").unwrap();
    assert!(auth.access_token(fake.base()).is_err());
    assert_eq!(
        oauth.refusals(),
        ["token (refresh_token) invalid_grant: redeemed", "token (refresh_token) invalid_grant: unknown"]
    );
}

/// A refresh in flight holds only the refresh lock: `status()`, which the UI thread calls, answers meanwhile.
#[test]
fn status_answers_while_a_refresh_is_in_flight() {
    let (_dir, fake, oauth, _store, auth) = setup();
    oauth.set_access_ttl(60);
    auth.sign_in(fake.base(), &http_browser()).unwrap();
    oauth.stall_tokens(true);
    let refreshing = auth.clone();
    let origin = fake.base().to_string();
    let refresh = std::thread::spawn(move || refreshing.access_token(&origin));
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while oauth.stalled() == 0 {
        assert!(std::time::Instant::now() < deadline, "the refresh never reached the token endpoint");
        std::thread::sleep(Duration::from_millis(10));
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let asking = auth.clone();
    let origin = fake.base().to_string();
    std::thread::spawn(move || tx.send(asking.status(&origin)).unwrap());
    let status = rx.recv_timeout(Duration::from_secs(2));
    oauth.stall_tokens(false);
    assert!(matches!(status, Ok(AuthStatus::SignedIn { .. })), "status() waited on the refresh: {status:?}");
    assert!(refresh.join().unwrap().is_ok());
}

/// Two threads that both need a refresh: one refreshes, the other takes its token (one rotation).
#[test]
fn concurrent_callers_share_one_refresh() {
    let (_dir, fake, oauth, _store, auth) = setup();
    auth.sign_in(fake.base(), &http_browser()).unwrap();
    auth.forget_access(&auth.access_token(fake.base()).unwrap());
    oauth.stall_tokens(true);
    let callers: Vec<_> = (0..2)
        .map(|_| {
            let (auth, origin) = (auth.clone(), fake.base().to_string());
            std::thread::spawn(move || auth.access_token(&origin))
        })
        .collect();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while oauth.stalled() == 0 {
        assert!(std::time::Instant::now() < deadline, "no refresh reached the token endpoint");
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(Duration::from_millis(200));
    oauth.stall_tokens(false);
    let tokens: Vec<String> = callers.into_iter().map(|c| c.join().unwrap().unwrap()).collect();
    assert_eq!(tokens[0], tokens[1]);
    let refreshes = fake.seen().iter().filter(|s| s.body.contains("grant_type=refresh_token")).count();
    assert_eq!(refreshes, 1);
    assert!(oauth.refusals().is_empty(), "{:?}", oauth.refusals());
}

#[test]
fn a_damaged_servers_json_is_reported_and_never_written_over() {
    let (dir, fake, _oauth, _store, auth) = setup();
    auth.sign_in(fake.base(), &http_browser()).unwrap();
    let path = dir.path().join("oauth").join("servers.json");
    let damaged = b"{\"servers\": {\"https://other.example\": ".to_vec();
    std::fs::write(&path, &damaged).unwrap();
    let fresh = Auth::with_store(dir.path(), Arc::new(MemoryStore::default()));

    let status = fresh.status(fake.base());
    assert!(matches!(&status, AuthStatus::SignInAgain(why) if why.contains("servers.json") && why.contains("damaged")), "{status:?}");
    assert!(matches!(fresh.access_token(fake.base()), Err(AuthError::Store(why)) if why.contains("damaged")));
    assert!(matches!(fresh.sign_in(fake.base(), &http_browser()), Err(AuthError::Store(_))));
    assert!(fresh.sign_out(fake.base()).unwrap_err().contains("damaged"));
    assert_eq!(std::fs::read(&path).unwrap(), damaged, "the file is untouched");

    std::fs::remove_file(&path).unwrap();
    assert_eq!(fresh.status(fake.base()), AuthStatus::NotSignedIn, "no file is an empty one");
}

/// A store that refuses a secret over 2560 bytes.
#[derive(Default)]
struct BlobLimited(MemoryStore);

impl SecretStore for BlobLimited {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        self.0.get(key)
    }

    fn set(&self, key: &str, secret: &str) -> Result<(), String> {
        if secret.len() > 2560 { Err("The stub received bad data.".into()) } else { self.0.set(key, secret) }
    }

    fn delete(&self, key: &str) -> Result<(), String> {
        self.0.delete(key)
    }
}

/// The server spent the old refresh token, so a new one the store refuses is kept for this process
/// and the status says the sign-in will not outlive it.
#[test]
fn a_refresh_token_the_store_refuses_is_kept_in_memory_and_reported() {
    let (dir, fake, oauth, _store, _auth) = setup();
    let store = Arc::new(BlobLimited::default());
    let auth = Auth::with_store(dir.path(), store.clone());
    oauth.set_access_ttl(60);
    auth.sign_in(fake.base(), &http_browser()).unwrap();
    oauth.set_refresh_len(6 * 1024);
    auth.access_token(fake.base()).unwrap();
    let status = auth.status(fake.base());
    assert!(
        matches!(&status, AuthStatus::SignedIn { unsaved: Some(why), .. } if why.contains("stub received bad data") && why.contains("until this process ends")),
        "{status:?}"
    );
    for _ in 0..2 {
        auth.access_token(fake.base()).unwrap();
    }
    assert!(oauth.refusals().is_empty(), "each refresh used the token held in memory: {:?}", oauth.refusals());
    oauth.set_refresh_len(0);
    auth.access_token(fake.base()).unwrap();
    assert_eq!(auth.status(fake.base()), AuthStatus::SignedIn { who: Some("Fake Operator".into()), unsaved: None }, "stored again");
    assert_eq!(store.get(&oauth.issuer()).unwrap().as_ref(), oauth.issued().last());
}

/// WorkState 1.0.0's refresh tokens are 6 KB.
#[cfg(windows)]
#[test]
fn a_6_kb_refresh_token_rotates_through_the_file_store() {
    use supraman_workman::secret::FileStore;
    let (dir, fake, oauth, _store, _auth) = setup();
    let store = || Arc::new(FileStore::new(dir.path()));
    oauth.set_access_ttl(60);
    oauth.set_refresh_len(6 * 1024);
    let auth = Auth::with_store(dir.path(), store());
    auth.sign_in(fake.base(), &http_browser()).unwrap();
    for _ in 0..2 {
        auth.access_token(fake.base()).unwrap();
        let stored = store().get(&oauth.issuer()).unwrap().unwrap();
        assert_eq!((stored.len(), Some(&stored)), (6 * 1024, oauth.issued().last()));
    }
    assert_eq!(auth.status(fake.base()), AuthStatus::SignedIn { who: Some("Fake Operator".into()), unsaved: None });
    let next = Auth::with_store(dir.path(), store());
    next.access_token(fake.base()).unwrap();
    assert!(oauth.refusals().is_empty(), "{:?}", oauth.refusals());
}
