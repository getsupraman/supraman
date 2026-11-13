use std::path::Path;
use std::sync::Arc;

use supraman_workman::fake::FakeWorkman;

use super::end_controllers;
use crate::mint::{Bearer, HttpMinter, Keeper, MintRequest, MintedSession, MintedSessions, Minter};

const CONTROLLER: &str = "fm-demo-workspace";
const RELAY: &str = "fm-demo-relay";

fn request(name: &str) -> MintRequest {
    MintRequest {
        name: name.into(),
        role: "controller".into(),
        launched_by: "supraman/demo".into(),
        origin: "supraman/demo".into(),
        cwd: r"C:\demo".into(),
        external_role: "workspace".into(),
        replace: false,
        repo: None,
        claude_session_id: None,
        takeover_from: None,
    }
}

/// What an older home kept for `demo`'s controller and relay: their records, configs and kept tokens, and the files beside them.
fn left_home(home: &Path, fake: &FakeWorkman, keeper: &Keeper) {
    let supervisor = home.join("supervisor");
    std::fs::create_dir_all(&supervisor).unwrap();
    let mut records = MintedSessions::empty(supervisor.join("demo.sessions.json"));
    for name in [CONTROLLER, RELAY] {
        let minted = HttpMinter::default().mint(fake.base(), "fake-static", &request(name)).unwrap();
        let session = MintedSession { session_id: minted.session_id, role: "workspace".into(), claude_session_id: None, origin: Some(fake.base().into()) };
        records.insert_minted(name, session, &request(name));
        keeper.write(name, &fake.mcp_url(), &minted.token, None, &[]).unwrap();
    }
    records.save().unwrap();
    for file in ["demo.toml", "demo.lock", "kept.lock", "hands.json"] {
        std::fs::write(supervisor.join(file), "x").unwrap();
    }
    std::fs::create_dir_all(home.join("roles")).unwrap();
    for role in ["workspace.md", "relay.md", "repo.md", "hand.md"] {
        std::fs::write(home.join("roles").join(role), "role").unwrap();
    }
    std::fs::create_dir_all(home.join("relay").join("demo").join("outbox")).unwrap();
    std::fs::write(home.join("relay").join("demo").join("outbox").join("1.json"), "{}").unwrap();
}

#[test]
fn a_left_controller_record_is_revoked_once_and_its_files_go_but_the_hands_stay() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let fake = FakeWorkman::start(Vec::new());
    let sessions = fake.enable_sessions();
    let keeper = Keeper::new(home, &home.join("mcp"), Arc::new(supraman_core::secret::MemoryStore::default()));
    left_home(home, &fake, &keeper);
    let signed = || Bearer::Signed { url: fake.mcp_url(), token: "fake-static".into() };
    let minter: &dyn Minter = &HttpMinter::default();

    assert_eq!(end_controllers(&keeper, minter, signed), Vec::<String>::new());
    for name in [CONTROLLER, RELAY] {
        assert_eq!(sessions.token_of(name), None, "{name} is revoked");
        assert_eq!(keeper.kept(name), None, "{name}'s kept token goes");
        assert_eq!(supraman_claude::mcp_config_url(keeper.mcp_dir(), name), None, "{name}'s config goes");
    }
    let supervisor = home.join("supervisor");
    for gone in ["demo.sessions.json", "demo.toml", "demo.lock"] {
        assert!(!supervisor.join(gone).exists(), "{gone}");
    }
    for gone in ["workspace.md", "relay.md", "repo.md"] {
        assert!(!home.join("roles").join(gone).exists(), "{gone}");
    }
    assert!(!home.join("relay").exists(), "the relay's folder");
    for kept in [supervisor.join("kept.lock"), supervisor.join("hands.json"), home.join("roles").join("hand.md")] {
        assert!(kept.exists(), "{}", kept.display());
    }

    let deletes = || fake.seen().iter().filter(|s| s.method == "DELETE").count();
    let before = deletes();
    assert_eq!(before, 2, "one revoke each");
    let asked = std::cell::Cell::new(false);
    let bearer = || {
        asked.set(true);
        signed()
    };
    assert_eq!(end_controllers(&keeper, minter, bearer), Vec::<String>::new());
    assert_eq!((deletes(), asked.get()), (before, false), "nothing left: no revoke, no bearer read");
}

#[test]
fn a_record_left_while_signed_out_still_goes_and_says_its_token_lapses() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let fake = FakeWorkman::start(Vec::new());
    fake.enable_sessions();
    let keeper = Keeper::new(home, &home.join("mcp"), Arc::new(supraman_core::secret::MemoryStore::default()));
    left_home(home, &fake, &keeper);

    let failed = end_controllers(&keeper, &HttpMinter::default(), || Bearer::Unavailable("not signed in to Workman any more".into()));
    assert_eq!(failed.len(), 2, "{failed:?}");
    assert!(failed.iter().all(|f| f.contains("lapses after its idle window")), "{failed:?}");
    assert!(!home.join("supervisor").join("demo.sessions.json").exists());
    assert_eq!(keeper.kept(CONTROLLER), None);
}
