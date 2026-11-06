//! What the app knows of each kept token beyond the token (`<home>/supervisor/kept.json`), and [`Keeping`], the check
//! that keeps every kept token good so a session never needs `/mcp` → Reconnect.

use std::collections::{BTreeMap, BTreeSet};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::{Bearer, Keeper, Minter, api_base, record_files, remint};
use crate::{Millis, MintedSessions};

/// How often the app checks every kept token.
pub const CHECK_EVERY: Duration = Duration::from_secs(5 * 60);
/// A kept token is minted afresh this long before its last-known `expiresAt`: a laptop asleep through the check
/// that was due still wakes to a live token.
pub const RENEW_MARGIN: Millis = 6 * 3_600 * 1_000;
/// How long a check waits for Workman's answer to one token's ping.
pub const PING_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Kept {
    /// Epoch ms of the `expiresAt` of its mint; `None`: Workman did not say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<Millis>,
    /// Signed out of the server that minted it: its grant revoke took the token, the next sign-in mints it afresh.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub signed_out: bool,
}

pub(super) type Table = BTreeMap<String, Kept>;

fn path(home: &Path) -> PathBuf {
    home.join("supervisor").join("kept.json")
}

/// Held around every read-modify-write of `kept.json`, and around a re-mint with its keep.
pub(super) fn lock(home: &Path) -> Result<std::fs::File, String> {
    let at = home.join("supervisor").join("kept.lock");
    supraman_core::fs::lock_file(&at).map_err(|e| format!("cannot lock {}: {e}", at.display()))
}

pub(super) fn read(home: &Path) -> Result<Table, String> {
    let at = path(home);
    match std::fs::read_to_string(&at) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| format!("cannot read {}: {e}", at.display())),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(Table::new()),
        Err(e) => Err(format!("cannot read {}: {e}", at.display())),
    }
}

/// `edit` of the table while [`lock`] is held; a table that cannot be read is left as it is.
pub(super) fn edit_locked(home: &Path, edit: impl FnOnce(&mut Table)) -> Result<(), String> {
    let mut table = read(home)?;
    edit(&mut table);
    let text = serde_json::to_string_pretty(&table).map_err(|e| e.to_string())?;
    supraman_core::fs::write_atomic(&path(home), text).map_err(|e| format!("cannot save {}: {e}", path(home).display()))
}

pub(super) fn edit(home: &Path, edit: impl FnOnce(&mut Table)) -> Result<(), String> {
    let _held = lock(home)?;
    edit_locked(home, edit)
}

/// The check of every kept token of a home, run every [`CHECK_EVERY`] off the UI thread. While signed in, each token
/// whose session can still use it (its config is there) and that is from the signed-in server is minted afresh with
/// `replace` ([`super::header`]'s re-mint) when a sign-in came since the last check, when it is marked signed out,
/// when its last-known `expiresAt` is less than the margin away, or when Workman refuses it.
#[derive(Debug)]
pub struct Keeping {
    margin: Millis,
    signed_in: bool,
}

impl Default for Keeping {
    fn default() -> Self {
        Self { margin: RENEW_MARGIN, signed_in: false }
    }
}

impl Keeping {
    pub fn with_margin(margin: Millis) -> Self {
        Self { margin, ..Self::default() }
    }

    /// A sign-in finished: the next check, once it has a bearer, mints every kept token afresh.
    pub fn signed_in(&mut self) {
        self.signed_in = true;
    }

    /// One check at `now`; `refused(mcp_url, token)` pings Workman with a kept token. What failed.
    pub fn check(&mut self, keeper: &Keeper, minter: &dyn Minter, bearer: impl FnOnce() -> Bearer, now: Millis, refused: impl Fn(&str, &str) -> bool) -> Vec<String> {
        let bearer = bearer();
        let Bearer::Signed { url, .. } = &bearer else {
            return Vec::new();
        };
        let origin = api_base(url);
        let mut notices = Vec::new();
        let table = read(&keeper.home).unwrap_or_else(|e| {
            notices.push(format!("{}{}; every kept Workman token is pinged instead", e[..1].to_uppercase(), &e[1..]));
            Table::new()
        });
        for name in kept_names(&keeper.home) {
            let (Some(token), Some(config)) = (keeper.kept(&name), supraman_claude::mcp_config_url(&keeper.mcp_dir, &name)) else {
                continue;
            };
            if api_base(&config) != origin {
                continue;
            }
            let kept = table.get(&name).cloned().unwrap_or_default();
            let why = if self.signed_in || kept.signed_out {
                "after a sign-in"
            } else if kept.expires_at.is_some_and(|at| now >= at - self.margin) {
                "before its idle window ends"
            } else if refused(&config, &token) {
                "as Workman refuses the kept one"
            } else {
                continue;
            };
            log::info!("{name}: a fresh Workman token {why}");
            if let Err(e) = remint(keeper, &name, bearer.clone(), minter) {
                notices.push(format!("Cannot renew the Workman token of {name}: {e}"));
            }
        }
        self.signed_in = false;
        notices
    }
}

/// Every name recorded in `home`'s files, once.
fn kept_names(home: &Path) -> BTreeSet<String> {
    record_files(home).into_iter().filter_map(|p| MintedSessions::load(Some(p)).ok()).flat_map(|r| r.names().cloned().collect::<Vec<_>>()).collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use supraman_workman::fake::FakeWorkman;

    use super::super::{HttpMinter, MintRequest, MintedSession};
    use super::*;

    const NAME: &str = "fm-demo-workspace";
    const T0: Millis = 1_800_000_000_000;
    const MINUTE: Millis = 60_000;

    fn request() -> MintRequest {
        MintRequest {
            name: NAME.into(),
            role: "controller".into(),
            launched_by: "foreman/demo".into(),
            origin: "foreman/demo".into(),
            cwd: r"C:\demo".into(),
            external_role: "workspace".into(),
            replace: false,
            repo: None,
            claude_session_id: None,
        }
    }

    /// `NAME` minted on `fake`, recorded in the workspace file, its token kept with its `expiresAt` and its config
    /// written, as the supervisor keeps it.
    fn kept(home: &Path, fake: &FakeWorkman) -> (Keeper, String) {
        let keeper = Keeper::new(home, &home.join("mcp"), Arc::new(supraman_core::secret::MemoryStore::default()));
        let minted = HttpMinter::default().mint(fake.base(), "fake-static", &request()).unwrap();
        let mut records = MintedSessions::empty(Some(MintedSessions::path(home, "demo")));
        let session = MintedSession { session_id: minted.session_id.clone(), role: "workspace".into(), claude_session_id: None, last_active: None, origin: Some(fake.base().to_string()) };
        records.insert_minted(NAME, session, &request());
        records.save().unwrap();
        keeper.write(NAME, &fake.mcp_url(), &minted.token, minted.expires_at.as_deref(), &[]).unwrap();
        (keeper, minted.token)
    }

    fn mints(fake: &FakeWorkman) -> usize {
        fake.seen().iter().filter(|s| s.method == "POST" && s.target == "/api/agent-sessions").count()
    }

    fn pings(fake: &FakeWorkman) -> usize {
        fake.seen().iter().filter(|s| s.method == "POST" && s.target == "/mcp").count()
    }

    fn refuses(url: &str, token: &str) -> bool {
        supraman_workman::refuses(url, token, PING_TIMEOUT)
    }

    #[test]
    fn a_kept_token_workman_refuses_is_minted_afresh_at_the_next_check() {
        let home = tempfile::tempdir().unwrap();
        let fake = FakeWorkman::start(Vec::new());
        let sessions = fake.enable_sessions();
        let (keeper, first) = kept(home.path(), &fake);
        let signed = || Bearer::Signed { url: fake.mcp_url(), token: "fake-static".into() };
        let mut keeping = Keeping::default();

        assert!(keeping.check(&keeper, &HttpMinter::default(), signed, T0, refuses).is_empty());
        assert_eq!((pings(&fake), mints(&fake)), (1, 1), "a live token is pinged, not minted");
        assert_eq!(keeper.kept(NAME).as_deref(), Some(first.as_str()));

        sessions.revoke_name(NAME);
        assert!(keeping.check(&keeper, &HttpMinter::default(), signed, T0 + 5 * MINUTE, refuses).is_empty());
        let fresh = sessions.token_of(NAME).expect("minted afresh");
        assert_ne!(fresh, first);
        assert_eq!(keeper.kept(NAME).as_deref(), Some(fresh.as_str()), "kept for the helper to print");
        let remint: serde_json::Value = serde_json::from_str(&fake.seen().last().unwrap().body).unwrap();
        assert_eq!((&remint["replace"], &remint["name"]), (&true.into(), &NAME.into()), "{remint}");
        assert!(supraman_claude::mcp_config_url(keeper.mcp_dir(), NAME).is_some(), "the config stays");
    }

    #[test]
    fn a_kept_token_is_minted_afresh_before_its_idle_window_ends() {
        let home = tempfile::tempdir().unwrap();
        let fake = FakeWorkman::start(Vec::new());
        let sessions = fake.enable_sessions();
        sessions.set_idle_window(10 * MINUTE, T0);
        let (keeper, first) = kept(home.path(), &fake);
        let signed = || Bearer::Signed { url: fake.mcp_url(), token: "fake-static".into() };
        let mut keeping = Keeping::with_margin(3 * MINUTE);
        let mut at = |t: Millis| {
            sessions.set_now(t);
            keeping.check(&keeper, &HttpMinter::default(), signed, t, refuses)
        };

        assert!(at(T0 + 5 * MINUTE).is_empty());
        assert_eq!(mints(&fake), 1, "five minutes before its window ends: pinged only");
        assert!(at(T0 + 8 * MINUTE).is_empty());
        assert_eq!(mints(&fake), 2, "within the margin of the mint's expiresAt: minted afresh");
        let fresh = keeper.kept(NAME).unwrap();
        assert_ne!(fresh, first);

        sessions.set_now(T0 + 17 * MINUTE);
        assert!(sessions.live(&fresh), "the fresh token holds past the first one's window");
        assert!(at(T0 + 17 * MINUTE).is_empty());
        assert_eq!(mints(&fake), 3, "and is itself minted afresh before its own window ends");
        sessions.set_now(T0 + 26 * MINUTE);
        assert!(sessions.live(&keeper.kept(NAME).unwrap()));
    }

    #[test]
    fn a_sign_out_keeps_the_files_and_mints_nothing_until_the_next_sign_in_mints_every_kept_token() {
        let home = tempfile::tempdir().unwrap();
        let fake = FakeWorkman::start(Vec::new());
        fake.enable_sessions();
        let (keeper, first) = kept(home.path(), &fake);
        let signed = || Bearer::Signed { url: fake.mcp_url(), token: "fake-static".into() };
        let never = |_: &str, _: &str| -> bool { panic!("no ping while signed out") };
        let mut keeping = Keeping::default();

        assert!(keeper.signed_out(&[NAME.to_string()]).is_empty());
        assert!(keeping.check(&keeper, &HttpMinter::default(), || Bearer::SignedOut, T0, never).is_empty());
        assert!(keeping.check(&keeper, &HttpMinter::default(), || Bearer::Unavailable("not signed in".into()), T0, never).is_empty());
        assert_eq!(mints(&fake), 1, "nothing minted while signed out");
        assert_eq!(keeper.kept(NAME).as_deref(), Some(first.as_str()), "the kept token stays");
        assert!(supraman_claude::mcp_config_url(keeper.mcp_dir(), NAME).is_some(), "the config stays");

        let fine = |_: &str, _: &str| false;
        assert!(keeping.check(&keeper, &HttpMinter::default(), signed, T0, fine).is_empty());
        assert_eq!(mints(&fake), 2, "a token marked signed out is minted afresh once signed in");
        assert!(!read(home.path()).unwrap()[NAME].signed_out, "and no longer marked");
        assert!(keeping.check(&keeper, &HttpMinter::default(), signed, T0, fine).is_empty());
        assert_eq!(mints(&fake), 2, "once");

        keeping.signed_in();
        assert!(keeping.check(&keeper, &HttpMinter::default(), signed, T0, fine).is_empty());
        assert_eq!(mints(&fake), 3, "a sign-in mints every kept token afresh");
        assert!(keeping.check(&keeper, &HttpMinter::default(), signed, T0, fine).is_empty());
        assert_eq!(mints(&fake), 3);
    }

    #[test]
    fn a_check_leaves_tokens_no_session_can_use_and_other_servers_tokens() {
        let home = tempfile::tempdir().unwrap();
        let fake = FakeWorkman::start(Vec::new());
        fake.enable_sessions();
        let (keeper, _) = kept(home.path(), &fake);
        let other = FakeWorkman::start(Vec::new());
        let mut keeping = Keeping::default();
        keeping.signed_in();
        let never = |_: &str, _: &str| -> bool { panic!("no ping") };
        assert!(keeping.check(&keeper, &HttpMinter::default(), || Bearer::Signed { url: other.mcp_url(), token: "x".into() }, T0, never).is_empty());
        super::super::remove_mcp_config(keeper.mcp_dir(), NAME).unwrap();
        keeping.signed_in();
        assert!(keeping.check(&keeper, &HttpMinter::default(), || Bearer::Signed { url: fake.mcp_url(), token: "fake-static".into() }, T0, never).is_empty());
        assert_eq!(mints(&fake), 1, "no config: its session ended");
    }
}
