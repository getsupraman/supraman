use std::collections::HashMap as StdHashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui::Context;
use supraman_claude::{Session, list_sessions, now_ms};
use supraman_core::{EventReader, Registry, Workspace};
use supraman_supervisor::DesiredState;
use supraman_ui::nudge::{self, Nudge, Nudged};
use supraman_ui::timeline;

use super::{FleetView, Refresh, load_registry};
use crate::fleet;
use crate::hand::Hands;
use crate::supervise::{self, Report, Supervision};

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
    pub(super) fn start(home: Option<PathBuf>, hands: Option<Arc<Mutex<Hands>>>, cx: &mut Context<FleetView>) -> (Self, Nudged) {
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
            hands,
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

/// Runs on a background thread: the registry read, `claude agents` and the supervisor's `claude` writes all block.
pub(super) struct Poller {
    pub(super) home: Option<PathBuf>,
    pub(super) events: Option<EventReader>,
    pub(super) supervision: Option<Supervision>,
    pub(super) retries: Arc<Mutex<Vec<(String, String)>>>,
    pub(super) injected: Arc<Mutex<Vec<Session>>>,
    pub(super) registry_writes: Arc<AtomicU64>,
    pub(super) snapshot: Option<(u64, Vec<Workspace>, Vec<Session>)>,
    pub(super) first: bool,
    pub(super) store: Option<Arc<dyn supraman_core::secret::SecretStore>>,
    pub(super) hands: Option<Arc<Mutex<Hands>>>,
}

impl Poller {
    pub(super) fn setup(&mut self) -> supervise::Setup {
        let Some(home) = self.home.clone() else {
            return supervise::Setup {
                plugin_dir: None,
                notices: vec!["No Supraman home (set SUPRAMAN_HOME or APPDATA): nothing can be started.".into()],
            };
        };
        let setup = supervise::setup(&home);
        self.events = Some(EventReader::new(&home));
        self.supervision = Some(Supervision::new(home, setup.plugin_dir.clone()).with_hands(self.hands.clone()));
        setup
    }

    pub(super) fn read(&mut self, with_sessions: bool) -> Refresh {
        let started = Instant::now();
        let registry_writes = self.registry_writes.load(Ordering::SeqCst);
        let workspaces = load_registry(Registry::from_env().map_err(|e| e.to_string()));
        let registry_took = started.elapsed();
        let polled_at = now_ms();
        let listing = Instant::now();
        let sessions = with_sessions.then(|| {
            let listed = if crate::selftest_seams::fixtures_only() { Ok(Vec::new()) } else { list_sessions() };
            listed.map_err(|e| e.to_string()).map(|mut rows| {
                rows.extend(self.injected.lock().unwrap_or_else(|e| e.into_inner()).iter().cloned());
                rows
            })
        });
        let list_took = listing.elapsed();
        let main_repos = match &sessions {
            Some(Ok(sessions)) => fleet::main_repos(sessions),
            _ => StdHashMap::new(),
        };
        let events = self.events.as_mut().map(EventReader::poll).unwrap_or_default();
        let took = started.elapsed();
        self.snapshot = match (&workspaces, &sessions) {
            (Ok((workspaces, _)), Some(Ok(sessions))) => Some((registry_writes, workspaces.clone(), sessions.clone())),
            _ => None,
        };
        let loading_desired = Instant::now();
        let desired = match (&self.home, &workspaces) {
            (Some(home), Ok((workspaces, _))) if self.first => Some(
                workspaces
                    .iter()
                    .filter_map(|w| {
                        let d = DesiredState::load(home, &w.name).ok()??;
                        Some((w.name.clone(), (d.started, d.sessions.iter().map(|s| s.name.clone()).collect())))
                    })
                    .collect(),
            ),
            _ => None,
        };
        let schedules = match (&self.home, &workspaces) {
            (Some(home), Ok((workspaces, _))) => {
                for run in crate::schedules::recover(home, workspaces) {
                    log::info!("recovered the dead scheduled run {run} as interrupted");
                }
                let store = self.store.get_or_insert_with(|| supraman_core::secret::default_store(home));
                Some(crate::schedules::read(home, workspaces, &**store, now_ms()))
            }
            _ => None,
        };
        if self.first {
            timeline("registry done", registry_took);
            timeline(&format!("list_sessions done ({} rows)", sessions.as_ref().and_then(|s| s.as_ref().ok()).map_or(0, Vec::len)), list_took);
            timeline("main repos and events done", took - registry_took - list_took);
            timeline("desired state done", loading_desired.elapsed());
        }
        Refresh {
            registry_writes,
            workspaces,
            sessions,
            main_repos,
            polled_at,
            events,
            desired,
            schedules,
            took,
        }
    }

    /// Ticks only on this poll's snapshot, and only while no registry write came after it: a stale list never drives a launch.
    pub(super) fn tick(&mut self) -> Option<Report> {
        let ticking = Instant::now();
        let first = std::mem::take(&mut self.first);
        let (writes, workspaces, sessions) = self.snapshot.take()?;
        if writes != self.registry_writes.load(Ordering::SeqCst) {
            log::info!("supervisor tick skipped: the workspaces changed after this poll read them");
            return None;
        }
        let supervision = self.supervision.as_mut()?;
        let retries = std::mem::take(&mut *self.retries.lock().unwrap_or_else(|e| e.into_inner()));
        let report = supervision.tick(&workspaces, &sessions, retries, now_ms());
        if first {
            timeline("supervisor tick done (relays, messages)", ticking.elapsed());
        }
        Some(report)
    }
}
