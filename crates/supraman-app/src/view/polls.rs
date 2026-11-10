use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::Context;
use supraman_claude::Session;
use supraman_ui::nudge::{self, Nudge, Nudged};
use supraman_ui::timeline;

use super::{FleetView, Poller};

/// The background poll and transcript scan: their nudges, what they hand the supervisor, and how far their answers are.
pub struct Polls {
    pub(super) retries: Arc<Mutex<Vec<(String, String)>>>,
    #[cfg(feature = "selftest")]
    pub injected: Arc<Mutex<Vec<Session>>>,
    /// Bumped by every registry write from this window; a poll that read the registry before one is not applied.
    pub(super) registry_writes: Arc<AtomicU64>,
    pub poll_nudge: Nudge,
    pub transcripts_nudge: Nudge,
    pub refresh_asks: u64,
    pub(super) refreshes_answered: u64,
    pub(super) transcripts_answered: u64,
    /// The first poll's snapshot (or its failure) is applied.
    pub sessions_read: bool,
    pub refreshes: u64,
    pub last_refresh_took: Option<Duration>,
    pub polled_at: i64,
    pub ticks: u64,
}

impl Polls {
    /// Starts the poll; the transcript scan waits on the returned [`Nudged`].
    pub(super) fn start(home: Option<PathBuf>, cx: &mut Context<FleetView>) -> (Self, Nudged) {
        let retries: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
        let injected: Arc<Mutex<Vec<Session>>> = Arc::default();
        let registry_writes: Arc<AtomicU64> = Arc::default();
        let (poll_nudge, woken) = nudge::nudge();
        let poller = Poller {
            home,
            events: None,
            supervision: None,
            retries: retries.clone(),
            injected: injected.clone(),
            registry_writes: registry_writes.clone(),
            snapshot: None,
            first: true,
            store: None,
        };
        timeline("window", Duration::ZERO);
        FleetView::start_polling(poller, woken, cx);
        let (transcripts_nudge, transcripts_nudged) = nudge::nudge();
        let polls = Self {
            retries,
            #[cfg(feature = "selftest")]
            injected,
            registry_writes,
            poll_nudge,
            transcripts_nudge,
            refresh_asks: 0,
            refreshes_answered: 0,
            transcripts_answered: 0,
            sessions_read: false,
            refreshes: 0,
            last_refresh_took: None,
            polled_at: 0,
            ticks: 0,
        };
        (polls, transcripts_nudged)
    }
}
