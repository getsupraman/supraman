//! What the human sends a session: into its pipe ([`crate::peer`]) when it can, else through the relay
//! inbox ([`supraman_core::send`]); [`Route`] says which. A direct send is never confirmed, so it goes only to
//! a prompting receiver ([`accepts_unmoded`]), carries an `[m-<8hex>]` id (the receiver drops a repeat
//! within 30 s), and leaves [`RESERVE`] of the receiver's 30-per-burst budget (refill 0.5/s), paced by
//! `relay/<w>/direct.json`.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use supraman_core::{CoreError, relay_dir};
use serde::{Deserialize, Serialize};

use crate::now_ms;
use crate::peer::{Liveness, PeerError, System, deliver, find_session, sessions_dir, wrap};

pub const LEDGER_FILE: &str = "direct.json";
pub const BURST: f64 = 30.0;
pub const REFILL_PER_SEC: f64 = 0.5;
/// Never spend the receiver's last tokens: SendMessage from sessions shares nothing with us, but a
/// second Supraman process might.
pub const RESERVE: f64 = 10.0;
const KEEP_MS: i64 = 61_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    Direct,
    /// `reason`: why direct delivery was not used; `drop`: the inbox file.
    Relay { reason: String, drop: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sent {
    pub to: String,
    /// `m-<8hex>`, in the first line of what was written; only a direct send has one.
    pub id: Option<String>,
    pub route: Route,
}

impl Sent {
    /// What the app's Messages row and `sm send` show.
    pub fn label(&self) -> String {
        match &self.route {
            Route::Direct => format!("sent direct [{}]", self.id.as_deref().unwrap_or("?")),
            Route::Relay { reason, .. } => format!("sent via relay ({reason})"),
        }
    }

    pub fn is_direct(&self) -> bool {
        self.route == Route::Direct
    }
}

#[derive(Debug)]
pub struct SendError {
    pub direct: String,
    pub relay: String,
    /// The direct write timed out and can still land; the relay was not tried, so it never arrives twice.
    pub may_arrive: bool,
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.may_arrive {
            write!(f, "not confirmed: the direct write timed out ({}) and may still arrive; not sent again through the relay", self.direct)
        } else {
            write!(f, "not sent: direct delivery failed ({}) and the relay cannot take it ({})", self.direct, self.relay)
        }
    }
}

impl std::error::Error for SendError {}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Ledger {
    sent: Vec<LedgerEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LedgerEntry {
    to: String,
    id: String,
    at: i64,
}

/// Launch modes whose receiver accepts a message without `from-mode` (the "prompting" class). Plan mode is
/// left out: with bypass available it counts as bypass.
pub fn accepts_unmoded(mode: &str) -> bool {
    matches!(mode, "auto" | "default" | "acceptEdits")
}

pub fn new_id() -> String {
    format!("m-{}", &uuid::Uuid::new_v4().simple().to_string()[..8])
}

fn ledger_path(home: &Path, workspace: &str) -> Result<PathBuf, CoreError> {
    Ok(relay_dir(home, workspace)?.join(LEDGER_FILE))
}

/// A missing or broken ledger is an empty one: it only paces.
fn load(path: &Path, now: i64) -> Ledger {
    let (mut ledger, _): (Ledger, _) = supraman_core::fs::load_json_or_notice(path, "");
    ledger.sent.retain(|e| now - e.at < KEEP_MS && e.at <= now);
    ledger
}

fn save(path: &Path, ledger: &Ledger) {
    if let Ok(text) = serde_json::to_string(ledger) {
        let _ = supraman_core::fs::write_atomic(path, text);
    }
}

/// Adds one send under the ledger's lock: a second Supraman's sends between our load and save stay.
#[cfg(test)]
fn record(path: &Path, entry: LedgerEntry) {
    let _lock = lock(path).unwrap();
    append(path, entry);
}

/// The ledger's lock, held across a send's pacing check, its delivery and its record.
fn lock(path: &Path) -> Result<fs::File, String> {
    let at = path.with_extension("lock");
    supraman_core::fs::lock_file(&at).map_err(|e| format!("cannot lock {}: {e}", at.display()))
}

fn append(path: &Path, entry: LedgerEntry) {
    let mut ledger = load(path, now_ms());
    ledger.sent.push(entry);
    save(path, &ledger);
}

/// Why a direct send did not go through; `may_arrive` when the pipe write timed out and can still land.
struct NotDirect {
    reason: String,
    may_arrive: bool,
}

impl From<String> for NotDirect {
    fn from(reason: String) -> Self {
        Self { reason, may_arrive: false }
    }
}

/// The receiver's bucket for us after `sent` (oldest first), at `now`.
pub fn tokens_left(sent_at: &[i64], now: i64) -> f64 {
    let mut tokens = BURST;
    let mut last = None::<i64>;
    for &at in sent_at {
        if let Some(prev) = last {
            tokens = (tokens + (at - prev) as f64 / 1000.0 * REFILL_PER_SEC).min(BURST);
        }
        tokens -= 1.0;
        last = Some(at);
    }
    if let Some(prev) = last {
        tokens = (tokens + (now - prev) as f64 / 1000.0 * REFILL_PER_SEC).min(BURST);
    }
    tokens
}

/// How long until one more send keeps [`RESERVE`] tokens.
pub fn wait_for_token(sent_at: &[i64], now: i64) -> Duration {
    let short = RESERVE + 1.0 - tokens_left(sent_at, now);
    if short <= 0.0 { Duration::ZERO } else { Duration::from_secs_f64(short / REFILL_PER_SEC) }
}

/// The first line every direct message starts with: the receiver sees a peer's message, this says whose.
pub fn framed(workspace: &str, body: &str, id: &str) -> String {
    format!("From the human (via Supraman, workspace {workspace}) [{id}]:\n{body}")
}

pub struct Sender<L = System> {
    pub sessions: Option<PathBuf>,
    pub live: L,
    /// The longest a send waits for the receiver's bucket before it goes through the relay instead.
    pub max_wait: Duration,
    /// Set: why the relay must not be used now (the app's: it is not live and armed).
    pub no_relay: Option<String>,
}

impl Default for Sender {
    fn default() -> Self {
        Self { sessions: sessions_dir(), live: System, max_wait: Duration::ZERO, no_relay: None }
    }
}

impl<L: Liveness> Sender<L> {
    /// Direct first, when `mode` (how Supraman launched `to`) is known and prompting; else, or on any
    /// failure, the relay. Errs only when both failed.
    pub fn send(&self, home: &Path, workspace: &str, to: &str, mode: Option<&str>, body: &str) -> Result<Sent, SendError> {
        let direct = match mode {
            Some(m) if accepts_unmoded(m) => self.direct(home, workspace, to, body),
            Some(m) => Err(format!("{to} runs in {m} mode, which holds Supraman's messages").into()),
            None => Err(format!("{to}'s launch mode is not known").into()),
        };
        let reason = match direct {
            Ok(id) => return Ok(Sent { to: to.into(), id: Some(id), route: Route::Direct }),
            Err(NotDirect { reason, may_arrive: true }) => {
                return Err(SendError { direct: reason, relay: "not tried".into(), may_arrive: true });
            }
            Err(NotDirect { reason, .. }) => reason,
        };
        if let Some(why) = &self.no_relay {
            return Err(SendError { direct: reason, relay: why.clone(), may_arrive: false });
        }
        match supraman_core::send(home, workspace, to, body) {
            Ok(drop) => Ok(Sent { to: to.into(), id: None, route: Route::Relay { reason, drop } }),
            Err(relay) => Err(SendError { direct: reason, relay: relay.to_string(), may_arrive: false }),
        }
    }

    fn direct(&self, home: &Path, workspace: &str, to: &str, body: &str) -> Result<String, NotDirect> {
        let dir = self.sessions.as_deref().ok_or_else(|| "no home directory to find ~/.claude/sessions in".to_string())?;
        let target = find_session(dir, to, &self.live).map_err(|e| e.to_string())?;
        let path = ledger_path(home, workspace).map_err(|e| e.to_string())?;
        let _held = lock(&path)?;
        let ledger = load(&path, now_ms());
        let to_them = |l: &Ledger| -> Vec<i64> { l.sent.iter().filter(|e| e.to == to).map(|e| e.at).collect() };
        let wait = wait_for_token(&to_them(&ledger), now_ms());
        if wait > self.max_wait {
            return Err(format!("paced: {} sent to {to} in the last minute", to_them(&ledger).len()).into());
        }
        if !wait.is_zero() {
            std::thread::sleep(wait);
        }
        let id = new_id();
        let delivered = deliver(&target, &wrap(&framed(workspace, body, &id)));
        if matches!(delivered, Ok(()) | Err(PeerError::Timeout { .. })) {
            append(&path, LedgerEntry { to: to.into(), id: id.clone(), at: now_ms() });
        }
        match delivered {
            Ok(()) => Ok(id),
            Err(e @ PeerError::Timeout { .. }) => Err(NotDirect { reason: format!("{e}; it may still arrive"), may_arrive: true }),
            Err(e) => Err(e.to_string().into()),
        }
    }
}

/// The app's path: never waits on the bucket; `mode` and `no_relay` as [`Sender::send`] and
/// [`Sender::no_relay`].
pub fn send_from_human(home: &Path, workspace: &str, to: &str, mode: Option<&str>, body: &str, no_relay: Option<String>) -> Result<Sent, SendError> {
    Sender { no_relay, ..Sender::default() }.send(home, workspace, to, mode, body)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoneAlive;

    impl Liveness for NoneAlive {
        fn alive(&self, _: u32, _: Option<u64>) -> bool {
            false
        }
    }

    #[test]
    fn the_bucket_refills_at_half_a_token_a_second_and_caps_at_thirty() {
        assert_eq!(tokens_left(&[], 0), BURST);
        let burst: Vec<i64> = (0..20).map(|i| 1_000 + i).collect();
        let left = tokens_left(&burst, 1_019);
        assert!((left - 10.0).abs() < 0.01, "{left}");
        assert!(wait_for_token(&burst, 1_019) >= Duration::from_secs(1), "20 at once leaves the reserve only");
        assert_eq!(tokens_left(&burst, 1_019 + 120_000), BURST);
        // one every 2 s is the refill rate: never runs dry; one a second spends half a token each
        let steady: Vec<i64> = (0..60).map(|i| i * 2_000).collect();
        assert!(tokens_left(&steady, 120_000) > 28.0);
        let quick: Vec<i64> = (0..30).map(|i| i * 1_000).collect();
        let left = tokens_left(&quick, 30_000);
        assert!((14.0..16.0).contains(&left), "{left}");
        assert_eq!(wait_for_token(&quick, 30_000), Duration::ZERO);
    }

    #[test]
    fn every_direct_message_has_its_own_id_in_the_first_line() {
        assert_eq!(framed("demo", "approve", "m-0123abcd"), "From the human (via Supraman, workspace demo) [m-0123abcd]:\napprove");
        let (a, b) = (new_id(), new_id());
        assert_ne!(a, b);
        assert!(a.starts_with("m-") && a.len() == 10 && a[2..].chars().all(|c| c.is_ascii_hexdigit()), "{a}");
        let sent = Sent { to: "x".into(), id: Some(a.clone()), route: Route::Direct };
        assert_eq!(sent.label(), format!("sent direct [{a}]"));
    }

    #[test]
    fn only_a_prompting_launch_mode_is_written_to_directly() {
        for mode in ["auto", "default", "acceptEdits"] {
            assert!(accepts_unmoded(mode), "{mode}");
        }
        for mode in ["bypassPermissions", "plan", "dontAsk", ""] {
            assert!(!accepts_unmoded(mode), "{mode}");
        }
        let home = tempfile::tempdir().unwrap();
        let sender = Sender { sessions: None, live: NoneAlive, max_wait: Duration::ZERO, no_relay: None };
        let held = sender.send(home.path(), "demo", "fm-demo-x", Some("bypassPermissions"), "hi").unwrap();
        assert!(matches!(&held.route, Route::Relay { reason, .. } if reason == "fm-demo-x runs in bypassPermissions mode, which holds Supraman's messages"), "{held:?}");
        let unknown = sender.send(home.path(), "demo", "fm-demo-x", None, "hi").unwrap();
        assert!(matches!(&unknown.route, Route::Relay { reason, .. } if reason == "fm-demo-x's launch mode is not known"), "{unknown:?}");
        assert_eq!(unknown.id, None);
    }

    #[test]
    fn no_live_session_falls_back_to_the_relay_and_says_why() {
        let home = tempfile::tempdir().unwrap();
        let sessions = tempfile::tempdir().unwrap();
        let sender = Sender { sessions: Some(sessions.path().into()), live: NoneAlive, max_wait: Duration::ZERO, no_relay: None };
        let sent = sender.send(home.path(), "demo", "fm-demo-workspace", Some("auto"), "a task").unwrap();
        let Route::Relay { reason, drop } = &sent.route else { panic!("{sent:?}") };
        assert!(reason.contains("no session named fm-demo-workspace"), "{reason}");
        assert!(sent.label().starts_with("sent via relay (no session named fm-demo-workspace"), "{}", sent.label());
        let text = fs::read_to_string(drop).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v, serde_json::json!({"to": "fm-demo-workspace", "body": "a task"}), "the relay gets the body unframed");

        fs::write(sessions.path().join("4.json"), r#"{"pid": 4, "name": "fm-demo-workspace", "peerProtocol": 1, "procStart": "1"}"#).unwrap();
        let stale = sender.send(home.path(), "demo", "fm-demo-workspace", Some("auto"), "again").unwrap();
        assert!(matches!(&stale.route, Route::Relay { reason, .. } if reason.contains("not running")), "{stale:?}");

        let nowhere = Sender { sessions: None, live: NoneAlive, max_wait: Duration::ZERO, no_relay: None };
        assert!(!nowhere.send(home.path(), "demo", "x", Some("auto"), "y").unwrap().is_direct());
        let both = sender.send(home.path(), "../bad", "fm-demo-workspace", Some("auto"), "x").unwrap_err();
        assert!(both.to_string().starts_with("not sent: direct delivery failed"), "{both}");
        let refused = Sender { no_relay: Some("demo is stopped".into()), ..sender };
        let err = refused.send(home.path(), "demo", "fm-demo-workspace", Some("auto"), "z").unwrap_err();
        assert_eq!(err.relay, "demo is stopped");
    }

    #[test]
    fn two_writers_lose_no_ledger_entry() {
        let home = tempfile::tempdir().unwrap();
        let path = ledger_path(home.path(), "demo").unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::thread::scope(|s| {
            for w in 0..2 {
                let path = &path;
                s.spawn(move || {
                    for i in 0..40 {
                        record(path, LedgerEntry { to: "x".into(), id: format!("m-{w}-{i}"), at: now_ms() });
                    }
                });
            }
        });
        assert_eq!(load(&path, now_ms()).sent.len(), 80);
    }

    #[test]
    fn a_paced_receiver_goes_through_the_relay() {
        let home = tempfile::tempdir().unwrap();
        let sessions = tempfile::tempdir().unwrap();
        fs::write(
            sessions.path().join("4.json"),
            serde_json::json!({"pid": std::process::id(), "name": "busy", "peerProtocol": 1, "messagingSocketPath": r"\\.\pipe\LOCAL\cc-msg-nobody"}).to_string(),
        )
        .unwrap();
        let path = ledger_path(home.path(), "demo").unwrap();
        let now = now_ms();
        let ledger = Ledger { sent: (0..25).map(|i| LedgerEntry { to: "busy".into(), id: format!("m-{i}"), at: now - 100 + i }).collect() };
        save(&path, &ledger);
        let sender = Sender { sessions: Some(sessions.path().into()), live: System, max_wait: Duration::ZERO, no_relay: None };
        if cfg!(windows) {
            fs::write(sessions.path().join(crate::peer::key_file_name(std::process::id(), r"\\.\pipe\LOCAL\cc-msg-nobody")), "{}").unwrap();
        }
        let sent = sender.send(home.path(), "demo", "busy", Some("auto"), "x").unwrap();
        assert!(matches!(&sent.route, Route::Relay { reason, .. } if reason.starts_with("paced: 25 sent to busy")), "{sent:?}");
        fs::write(&path, "broken").unwrap();
        assert!(load(&path, now).sent.is_empty());
    }
}
