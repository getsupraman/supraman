use std::path::{Path, PathBuf};
use std::sync::Arc;

use supraman_workman::fake::FakeWorkman;

use super::super::{Bearer, HttpMinter, Keeping, MintRequest, MintedSession, MintedSessions, Minter as _};
use super::*;

const NAME: &str = "hand-alpha-1";
const SID: &str = "aaaa0001-0000-4000-8000-000000000001";
const T0: Millis = 1_800_000_000_000;
const MINUTE: Millis = 60_000;

fn listed(session_id: &str) -> Session {
    Session {
        id: session_id[..8].into(),
        session_id: session_id.into(),
        name: "x".into(),
        cwd: PathBuf::new(),
        kind: supraman_claude::Kind::Interactive,
        state: Some(supraman_claude::SessionState::Stopped),
        status: None,
        waiting_for: None,
        pid: None,
        started_at: None,
    }
}

fn other() -> Session {
    listed("bbbb0002-0000-4000-8000-000000000002")
}

fn bound() -> Vec<(String, String)> {
    vec![(NAME.to_string(), SID.to_string())]
}

/// A keeper of `home` whose claude dir holds `SID`'s transcript.
fn keeper(home: &Path) -> Keeper {
    let claude = home.join("claude");
    let project = claude.join("projects").join("C--demo-alpha");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join(format!("{SID}.jsonl")), "{}\n").unwrap();
    Keeper::new(home, &home.join("mcp"), Arc::new(supraman_core::secret::MemoryStore::default())).with_claude_dir(Some(claude))
}

fn unlisted_since(home: &Path) -> Option<Millis> {
    keeping::read(home).unwrap().get(NAME).and_then(|k| k.unlisted_since)
}

#[test]
fn one_gap_changes_nothing_two_mark_it_unlisted_and_a_listing_unmarks_it() {
    let home = tempfile::tempdir().unwrap();
    let keeper = keeper(home.path());
    let mut absent = Absent::default();
    let mut look = |observed: &[Session], at: Millis| keeper.gone(&mut absent, &bound(), observed, at);

    assert_eq!(look(&[other()], T0), Gone::default());
    assert_eq!(look(&[listed(SID)], T0 + MINUTE), Gone::default());
    assert_eq!(unlisted_since(home.path()), None, "a single gap changes nothing");
    assert_eq!(look(&[other()], T0 + 2 * MINUTE), Gone::default());
    assert_eq!(look(&[], T0 + 3 * MINUTE), Gone::default(), "an empty listing starts the count over");
    assert_eq!(look(&[other()], T0 + 4 * MINUTE), Gone::default());
    assert_eq!(unlisted_since(home.path()), None);

    assert_eq!(look(&[other()], T0 + 5 * MINUTE), Gone::default(), "with its transcript it is no delete");
    assert_eq!(unlisted_since(home.path()), Some(T0 + 5 * MINUTE));
    look(&[other()], T0 + 6 * MINUTE);
    look(&[other()], T0 + 7 * MINUTE);
    assert_eq!(unlisted_since(home.path()), Some(T0 + 5 * MINUTE), "marked once");

    assert_eq!(look(&[listed(SID)], T0 + 8 * MINUTE), Gone::default());
    assert_eq!(unlisted_since(home.path()), None, "listed again: a resume");
}

#[test]
fn a_session_gone_with_no_transcript_is_deleted_and_not_marked() {
    let home = tempfile::tempdir().unwrap();
    let keeper = Keeper::new(home.path(), &home.path().join("mcp"), Arc::new(supraman_core::secret::MemoryStore::default()));
    let mut absent = Absent::default();
    assert_eq!(keeper.gone(&mut absent, &bound(), &[other()], T0), Gone::default());
    let gone = keeper.gone(&mut absent, &bound(), &[other()], T0 + MINUTE);
    assert_eq!(gone, Gone { deleted: bound(), ..Gone::default() });
    assert!(keeping::read(home.path()).unwrap().is_empty());
}

#[test]
fn an_unlisted_record_lapses_at_its_expires_at_or_unlisted_lapse_after_its_mark() {
    let home = tempfile::tempdir().unwrap();
    let keeper = keeper(home.path());
    let mut absent = Absent::default();
    for at in [T0, T0 + MINUTE] {
        keeper.gone(&mut absent, &bound(), &[other()], at);
    }
    let since = T0 + MINUTE;
    assert_eq!(unlisted_since(home.path()), Some(since));
    assert!(keeper.gone(&mut absent, &bound(), &[other()], since + UNLISTED_LAPSE - 1).lapsed.is_empty(), "its expiry unknown");
    assert_eq!(keeper.gone(&mut absent, &bound(), &[other()], since + UNLISTED_LAPSE).lapsed, bound());

    let home = tempfile::tempdir().unwrap();
    let keeper = self::keeper(home.path());
    keeper.keep(NAME, "wss_fake", Some(&supraman_core::time::rfc3339_utc(T0 + 30 * MINUTE))).unwrap();
    let mut absent = Absent::default();
    for at in [T0, T0 + MINUTE, T0 + 29 * MINUTE] {
        assert!(keeper.gone(&mut absent, &bound(), &[other()], at).lapsed.is_empty());
    }
    assert_eq!(keeper.gone(&mut absent, &bound(), &[other()], T0 + 30 * MINUTE).lapsed, bound(), "its kept expiresAt passed");
}

#[test]
fn keeping_leaves_an_unlisted_token_alone_and_renews_it_once_listed_again() {
    let home = tempfile::tempdir().unwrap();
    let fake = FakeWorkman::start(Vec::new());
    fake.enable_sessions();
    let keeper = keeper(home.path());
    let request = MintRequest {
        name: NAME.into(),
        role: "orchestrator".into(),
        launched_by: "supraman/hand".into(),
        origin: "supraman/hand/alpha".into(),
        cwd: r"C:\demo\alpha".into(),
        external_role: "hand".into(),
        replace: false,
        repo: None,
        claude_session_id: Some(SID.into()),
        takeover_from: None,
    };
    let minted = HttpMinter::default().mint(fake.base(), "fake-static", &request).unwrap();
    let mut records = MintedSessions::empty(MintedSessions::hands_path(home.path()));
    let session = MintedSession { session_id: minted.session_id, role: "hand".into(), claude_session_id: Some(SID.into()), origin: Some(fake.base().to_string()) };
    records.insert_minted(NAME, session, &request);
    records.save().unwrap();
    keeper.write(NAME, &fake.mcp_url(), &minted.token, None, &[]).unwrap();
    let mints = || fake.seen().iter().filter(|s| s.method == "POST" && s.target == "/api/agent-sessions").count();
    let signed = || Bearer::Signed { url: fake.mcp_url(), token: "fake-static".into() };
    let refused = |_: &str, _: &str| true;
    let mut keeping = Keeping::default();
    let mut absent = Absent::default();

    for at in [T0, T0 + MINUTE] {
        keeper.gone(&mut absent, &bound(), &[other()], at);
    }
    keeping.signed_in();
    let checked = keeping.check(&keeper, &HttpMinter::default(), signed, T0 + MINUTE, refused);
    assert!(checked.renewed.is_empty(), "{checked:?}");
    assert_eq!(mints(), 1, "an unlisted token is never minted afresh");

    keeper.gone(&mut absent, &bound(), &[listed(SID)], T0 + 2 * MINUTE);
    let checked = keeping.check(&keeper, &HttpMinter::default(), signed, T0 + 2 * MINUTE, refused);
    assert_eq!(checked.renewed.get(NAME), Some(&Ok(())), "{checked:?}");
    assert_eq!(mints(), 2, "listed again, it is renewed as usual");
}
