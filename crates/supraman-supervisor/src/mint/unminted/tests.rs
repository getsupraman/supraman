use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use supraman_claude::Session;

use super::Unlisted;
use crate::mint::{Keeper, MintRequest, MintedSession, MintedSessions};

const URL: &str = "http://127.0.0.1:1/mcp";
const LISTED: &str = "fm-demo-tab";
const RUN: &str = "fm-demo-sched-nightly-20261113-0900";
const CLOSED_TAB: &str = "hand-alpha-1";
const OPEN_TAB: &str = "hand-alpha-2";
const GAP: &str = "fm-demo-alpha";
const RECORDED: &str = "fm-demo-recorded";
const KEPT: &str = "fm-demo-kept";
const RELAUNCHED: &str = "fm-demo-relaunched";

fn row(name: &str) -> Session {
    Session {
        id: "aaaa0001".into(),
        session_id: format!("aaaa0001-0000-4000-8000-{:012}", name.len()),
        name: name.into(),
        cwd: PathBuf::new(),
        kind: supraman_claude::Kind::Background,
        state: Some(supraman_claude::SessionState::Stopped),
        status: None,
        waiting_for: None,
        pid: None,
        started_at: None,
    }
}

/// `name`'s token-less config, written a minute ago.
fn written_before(keeper: &Keeper, name: &str) -> PathBuf {
    let path = keeper.write_unminted(name, URL, &[]).unwrap();
    std::fs::File::options().write(true).open(&path).unwrap().set_modified(SystemTime::now() - Duration::from_secs(60)).unwrap();
    path
}

fn recorded(home: &Path, name: &str) {
    let mut records = MintedSessions::load(MintedSessions::hands_path(home)).unwrap();
    let request = MintRequest {
        name: name.into(),
        role: "orchestrator".into(),
        launched_by: "supraman/hand".into(),
        origin: "supraman/hand/alpha".into(),
        cwd: r"C:\demo\alpha".into(),
        external_role: "hand".into(),
        replace: false,
        repo: None,
        claude_session_id: None,
        takeover_from: None,
    };
    records.insert_minted(name, MintedSession { session_id: "9".into(), role: "hand".into(), claude_session_id: None, origin: None }, &request);
    records.save().unwrap();
}

#[test]
fn a_token_less_config_goes_once_two_listings_in_a_row_lack_its_session_and_nothing_holds_it() {
    let home = tempfile::tempdir().unwrap();
    let keeper = Keeper::new(home.path(), &home.path().join("mcp"), Arc::new(supraman_core::secret::MemoryStore::default()));
    let configs: Vec<(&str, PathBuf)> = [LISTED, RUN, CLOSED_TAB, OPEN_TAB, GAP, RECORDED, KEPT, RELAUNCHED].iter().map(|n| (*n, written_before(&keeper, n))).collect();
    recorded(home.path(), RECORDED);
    keeper.write(KEPT, URL, "wss_kept", None, &[]).unwrap();
    let left = || configs.iter().filter(|(_, p)| p.exists()).map(|(n, _)| *n).collect::<Vec<_>>();
    let held = [OPEN_TAB.to_string()];
    let mut unlisted = Unlisted::default();
    let other = row("x");
    let mut sweep = |observed: &[Session]| keeper.sweep_unminted(&mut unlisted, observed, &held, supraman_core::time::now_ms());

    assert!(sweep(&[]).is_empty());
    assert!(sweep(std::slice::from_ref(&other)).is_empty());
    assert_eq!(left().len(), configs.len(), "one listing without them deletes nothing");
    assert!(sweep(&[]).is_empty());
    assert!(sweep(std::slice::from_ref(&other)).is_empty());
    assert_eq!(left().len(), configs.len(), "an empty listing starts the count over");

    keeper.write_unminted(RELAUNCHED, URL, &[]).unwrap();
    assert!(sweep(&[other.clone(), row(GAP)]).is_empty());
    assert_eq!(left(), [OPEN_TAB, GAP, RECORDED, KEPT, RELAUNCHED], "the second listing in a row without them");
    assert!(sweep(std::slice::from_ref(&other)).is_empty());
    assert!(left().starts_with(&[OPEN_TAB, GAP, RECORDED, KEPT]), "listed in between: counted again");
    assert!(sweep(std::slice::from_ref(&other)).is_empty());
    assert_eq!(left(), [OPEN_TAB, RECORDED, KEPT], "a record, a kept token or a hand tab keeps one");
    assert_eq!(keeper.forget_unminted(RECORDED), None);
    assert_eq!(keeper.forget_unminted(KEPT), None);
    assert_eq!(left(), [OPEN_TAB, RECORDED, KEPT], "revoke's to delete");
}
