use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap as StdHashMap, HashSet},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use collections::HashMap;
use futures::{FutureExt as _, StreamExt as _};
use foreman_claude::{ClaudeVersion, Sent, Session, Transcripts, check_supported, list_sessions, main_repo, now_ms, send_from_human};
use foreman_core::{
    DEFAULT_MAX_WORKING, Event, EventReader, FM_PREFIX, OutboxMessage, Workspace, Registry, Settings, mark_answered,
};
use foreman_supervisor::{DesiredState, NeedsYou, workspace_session_name};
use foreman_term::{
    Event as TerminalEvent, MaybeNavigationTarget, MouseInputMode, Terminal, TerminalBuilder, TerminalMode,
    terminal_settings::{AlternateScroll, CURSOR_BLINK_INTERVAL, CursorShape},
};
use foreman_ui::nudge::{self, Nudge, Nudged};
use foreman_ui::hit::{HitTargets, hit_target};
use foreman_ui::text_input::{InputEvent, TextInput};
use foreman_ui::hover_card;
use foreman_ui::split::SplitPane;
use foreman_ui::tabs::{TabHeader, tab_strip};
use foreman_ui::toast::ToastSlot;
use foreman_ui::{FrameProbe, button, term_element};
use gpui::{
    AnyElement, AnyWindowHandle, ClickEvent, Context, Entity, FocusHandle,
    KeyBinding, KeyDownEvent, MouseButton, MouseDownEvent, Pixels, Render, ScrollEvent, SharedString, Window,
    WindowHandle, div, prelude::*, px,
};
use theme::ActiveTheme as _;
use util::paths::PathStyle;

use crate::compose::{self, Compose};
use crate::fleet::{self, HookSignal, SessionRow, Supervised, Tree};
use crate::hand::{Hands, Launch};
use crate::settings_window::{self, SettingsView};
use crate::dismissed_messages::DismissedMessages;
use crate::stars::Stars;
use crate::supervise::{self, RelayStatus, Report, Supervision};
use crate::tab_card;
use crate::work_view::{FleetLink, LiveSession, WorkHub, WorkInbox, WorkView};
use crate::workstate_setup::{self, WsStatus};

pub(crate) mod icon;
mod delete;
mod drop;
mod folders;
mod left;
mod manage;
mod menu;
pub(crate) mod notices;
mod reopen;
mod results;
pub(crate) mod schedules;
mod term;

pub(crate) use left::grid;
pub use manage::{AddDraft, Removing};
pub use menu::{Confirm, ContextMenu, MenuAction, MenuTarget};

gpui::actions!(foreman, [NextTab, PreviousTab, FocusSearch]);

const KEY_CONTEXT: &str = "FleetView";

/// Matched before [`FleetView::key_down`] runs, so a terminal tab never sees these chords.
pub fn bind_keys(cx: &mut gpui::App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-tab", NextTab, Some(KEY_CONTEXT)),
        KeyBinding::new("ctrl-pagedown", NextTab, Some(KEY_CONTEXT)),
        KeyBinding::new("ctrl-shift-tab", PreviousTab, Some(KEY_CONTEXT)),
        KeyBinding::new("ctrl-pageup", PreviousTab, Some(KEY_CONTEXT)),
        KeyBinding::new("ctrl-shift-e", FocusSearch, Some(KEY_CONTEXT)),
    ]);
}

pub const REFRESH_EVERY: Duration = Duration::from_secs(2);
pub const TRANSCRIPTS_EVERY: Duration = Duration::from_secs(10);
const TRANSCRIPTS_CACHE: &str = "cache/transcripts.json";
pub const CLAUDE_DIR_VAR: &str = "FOREMAN_CLAUDE_DIR";
/// Ctrl+Z: `claude attach`'s own detach, which leaves the session running (LEARNINGS m1/7).
pub const DETACH_INPUT: &[u8] = b"\x1a";
pub const DETACH_WITHIN: Duration = Duration::from_secs(5);
const SHELL_SETTLED: Duration = Duration::from_millis(500);
const SHELL_SETTLE_LIMIT: Duration = Duration::from_secs(15);
/// Claude draws a frame per wakeup; a hand tab looks at its token file at most this often.
const HAND_CHECK_GAP: Duration = Duration::from_millis(250);
const HAND_EXIT_FALLBACK: Duration = Duration::from_secs(30);
/// Zed's `terminal.scroll_multiplier` default; one Windows wheel notch is already 3 lines.
const SCROLL_MULTIPLIER: f32 = 1.0;
const DEFAULT_LEFT_WIDTH: Pixels = px(460.);
pub(crate) const MIN_LEFT_WIDTH: Pixels = px(280.);
const MIN_RIGHT_WIDTH: Pixels = px(320.);
/// The daemon may respawn a background session by itself (~12 s after a death mid-turn); a second resume would run a second process.
const RESUME_GRACE_MS: i64 = 30_000;
pub const MAX_WORKING_LIMIT: usize = 20;
/// The Messages section shows this many, newest first; a body shows this many lines until clicked.
const MESSAGES_LISTED: usize = 10;
const MESSAGE_LINES: usize = 3;

const INBOX_MAX_SHARE: f32 = 0.25;
const MESSAGES_MAX_SHARE: f32 = 0.3;

pub enum Cli {
    Checking,
    Supported(ClaudeVersion),
    Failed(String),
}

pub enum TabKind {
    Attach { session_id: String },
    Shell,
    Work(Entity<WorkView>),
    /// Reopened from the last run, waiting for a click on Resume.
    Saved(crate::saved_tabs::SavedTab),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Detach {
    None,
    Pending,
    Detaching,
    /// The tab and its process stay: nothing is killed that could hold the session.
    Failed,
}

pub struct Tab {
    pub id: u64,
    pub title: String,
    pub kind: TabKind,
    pub terminal: Option<Entity<Terminal>>,
    pub error: Option<String>,
    pub exited: bool,
    pub detach: Detach,
    pub cwd: Option<PathBuf>,
    pub find: Option<term::Find>,
    pub program_title: Option<String>,
    pub session: Option<String>,
    pub hand: bool,
    pub workstate: Option<String>,
    pub hand_key: Option<String>,
    pub restart_to: Option<String>,
    pub opened_at: i64,
    pub claude: Option<String>,
    /// The full uuid it was launched or attached with, and that session's name.
    pub launched_session: Option<String>,
    pub session_name: Option<String>,
    watchers: Vec<Nudge>,
}

impl Tab {
    pub fn new(id: u64, title: String, kind: TabKind, cwd: Option<PathBuf>) -> Self {
        Self {
            id,
            title,
            kind,
            terminal: None,
            error: None,
            exited: false,
            detach: Detach::None,
            cwd,
            find: None,
            program_title: None,
            session: None,
            hand: false,
            workstate: None,
            hand_key: None,
            restart_to: None,
            opened_at: now_ms(),
            claude: None,
            launched_session: None,
            session_name: None,
            watchers: Vec::new(),
        }
    }

    fn wake_watchers(&self) {
        self.watchers.iter().for_each(Nudge::wake);
    }

    pub fn label(&self) -> String {
        let mut title = self.program_title.clone().unwrap_or_else(|| self.title.clone());
        if self.exited {
            title.push_str(" (exited)");
        }
        match self.detach {
            Detach::Pending | Detach::Detaching => title.push_str(" (detaching…)"),
            Detach::Failed => title.push_str(" (detach failed)"),
            Detach::None => {}
        }
        title
    }
}

pub struct FleetView {
    pub cli: Cli,
    pub workspaces: Vec<Workspace>,
    pub registry_error: Option<String>,
    pub registry_skipped: Vec<String>,
    pub sessions: Vec<Session>,
    /// [`fleet::main_repos`] of `sessions`, so rendering never walks the filesystem.
    main_repos: StdHashMap<PathBuf, Option<PathBuf>>,
    pub sessions_error: Option<String>,
    pub last_refresh_took: Option<Duration>,
    pub refreshes: u64,
    pub show_stopped: bool,
    /// Set only by a selftest: see [`fleet::Supervised::own_prefix`].
    pub own_prefix: Option<String>,
    collapsed: HashSet<String>,
    pub tabs: Vec<Tab>,
    pub active: Option<u64>,
    next_tab_id: u64,
    pub shell_args: Vec<String>,
    focus: FocusHandle,
    pub probe: FrameProbe,
    pub hits: HitTargets,
    window: AnyWindowHandle,
    quitting: bool,
    quit_asked: bool,
    pub home: Option<PathBuf>,
    pub plugin_dir: Option<PathBuf>,
    pub setup_notices: Vec<String>,
    pub needs: Vec<NeedsYou>,
    pub desired: BTreeMap<String, (bool, Vec<String>)>,
    pub max_working: usize,
    pub supervise_errors: Vec<String>,
    supervise_dismissed: Vec<String>,
    pub daemon_hold: Option<String>,
    pub action_error: Option<String>,
    pub last_attempt: StdHashMap<String, i64>,
    pub hooks: StdHashMap<String, HookSignal>,
    pub polled_at: i64,
    pub ticks: u64,
    retries: Arc<Mutex<Vec<(String, String)>>>,
    pub injected: Arc<Mutex<Vec<Session>>>,
    /// Bumped by every registry write from this window; a poll that read the registry before one is not applied.
    registry_writes: Arc<AtomicU64>,
    pub poll_hold: Arc<PollHold>,
    pub transcripts_hold: Arc<PollHold>,
    pub left_width: Pixels,
    pub term_font_size: Pixels,
    pub term_font_family: SharedString,
    pub term_ligatures: bool,
    pub installed_fonts: Arc<Vec<String>>,
    font_save: Option<gpui::Task<()>>,
    zoom_carried: f32,
    pub(crate) settings_window: Option<WindowHandle<SettingsView>>,
    last_alive: StdHashMap<String, i64>,
    pub relays: BTreeMap<String, RelayStatus>,
    pub messages: BTreeMap<String, Vec<OutboxMessage>>,
    pub expanded: HashSet<String>,
    pub dismissed_messages: DismissedMessages,
    left_hold: Option<left::Hold>,
    left_hold_timer: Option<gpui::Task<()>>,
    pub left_holds: u64,
    pub answered:BTreeMap<String, BTreeMap<String, String>>,
    pub answered_route: BTreeMap<String, String>,
    pub compose: Option<Compose>,
    work_live: Rc<RefCell<Vec<LiveSession>>>,
    pub work_inbox: Entity<WorkInbox>,
    pub menu: Option<ContextMenu>,
    pub confirm: Option<Confirm>,
    pub adding: Option<AddDraft>,
    pub editing: Option<folders::EditFolders>,
    pub removing: BTreeMap<String, Removing>,
    pub pick_folder: Option<PathBuf>,
    pub open_with: fn(&Path) -> Result<(), isize>,
    pub workstate: WsStatus,
    workstate_dismissed: Option<String>,
    workstate_recheck: Nudge,
    pub start_warning: Option<String>,
    pub signing_in: Option<String>,
    pub sign_in_error: Option<String>,
    hands: Option<Arc<Mutex<Hands>>>,
    /// The last run's hand tokens are released, so a reopened tab's mint is not revoked with them.
    hands_ready: bool,
    pub hand_notices: Vec<String>,
    hands_released: bool,
    quit_release: Option<gpui::Task<()>>,
    desired_writes: futures::channel::mpsc::UnboundedSender<DesiredWrite>,
    synced_server: Option<String>,
    left_servers: std::collections::HashSet<String>,
    left_pending: Vec<String>,
    pub closed: Vec<Session>,
    closed_repos: StdHashMap<PathBuf, Option<PathBuf>>,
    pub transcript_scans: u64,
    pub transcripts_nudge: Nudge,
    poll_nudge: Nudge,
    pub refresh_asks: u64,
    refreshes_answered: u64,
    /// The first poll's snapshot (or its failure) is applied.
    pub sessions_read: bool,
    pub transcripts_read: bool,
    /// `(read, of)` of the files the running transcript scan reads.
    pub transcripts_progress: Option<(usize, usize)>,
    transcripts_answered: u64,
    /// Every source of the tree has completed once (`loading`); until then the action bar says what is loading.
    pub loaded: bool,
    /// `transcript_scans` when `loaded` was set.
    pub loaded_at_scan: u64,
    /// Whether the saved transcript cache held files; `None` until it is read.
    transcripts_cache: Option<bool>,
    transcripts_restart: bool,
    pub stars: Stars,
    pub closed_expanded: HashSet<String>,
    pub claude_command: String,
    pub search: Entity<TextInput>,
    search_marks: StdHashMap<String, crate::search::Marks>,
    tabs_resolving: bool,
    tabs_again: bool,
    pub tab_resolves: u64,
    pub toast: ToastSlot,
    pub pane_toast: ToastSlot,
    pub claude_dir: Option<PathBuf>,
    pub deleted: StdHashMap<String, i64>,
    pub deleting: HashSet<String>,
    painted_sessions: bool,
    pub tabs_restored: bool,
    tabs_reopening: bool,
    tabs_frozen: bool,
    tabs_saved: Option<crate::saved_tabs::SavedTabs>,
    tabs_writes: futures::channel::mpsc::UnboundedSender<crate::saved_tabs::SavedTabs>,
    pub tab_notices: Vec<String>,
    /// Each workspace's schedules as the last poll read them.
    pub schedules: BTreeMap<String, Vec<crate::schedules::ScheduleRow>>,
    pub schedule_errors: Vec<String>,
    schedule_errors_dismissed: Vec<String>,
    /// A failed `sync` by `<ws>/<id>` of the schedule acted on: on its row and in the Inbox until a sync succeeds.
    pub sync_errors: BTreeMap<String, String>,
    pub registrar: schedules::SharedRegistrar,
    pub schedule_store: Arc<dyn foreman_core::secret::SecretStore>,
    pub schedule_edit: Option<schedules::ScheduleEdit>,
    /// Scheduled runs whose Inbox notice was dismissed, by workspace and run id.
    pub runs_dismissed: DismissedMessages,
}

/// The dismissed run notices beside the dismissed messages.
const RUNS_DISMISSED: &str = "schedules/dismissed.json";

const QUIT_RELEASE_WITHIN: Duration = Duration::from_secs(5);

enum DesiredWrite {
    Start { home: PathBuf, plugin_dir: Option<PathBuf>, workspace: Workspace },
    Stop { home: PathBuf, name: String },
}

fn load_registry(registry: Result<Registry, String>) -> Result<(Vec<Workspace>, Vec<String>), String> {
    let (workspaces, failed) = registry?.load_all().map_err(|e| e.to_string())?;
    Ok((workspaces, failed.iter().map(ToString::to_string).collect()))
}

pub fn offers_resume(s: &Session, health: &foreman_claude::Health, last_alive: Option<i64>, now: i64) -> bool {
    !fleet::is_alive(health)
        && !s.session_id.is_empty()
        && !s.name.starts_with(FM_PREFIX)
        && (s.kind != foreman_claude::Kind::Background || last_alive.is_none_or(|t| now - t > RESUME_GRACE_MS))
}

#[derive(Default)]
pub struct PollHold {
    pub armed: AtomicBool,
    pub holding: AtomicBool,
}

impl PollHold {
    const MAX: Duration = Duration::from_secs(10);

    fn wait(&self) {
        if !self.armed.swap(false, Ordering::SeqCst) {
            return;
        }
        self.holding.store(true, Ordering::SeqCst);
        let deadline = Instant::now() + Self::MAX;
        while self.holding.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        self.holding.store(false, Ordering::SeqCst);
    }
}

struct Refresh {
    registry_writes: u64,
    workspaces: Result<(Vec<Workspace>, Vec<String>), String>,
    sessions: Option<Result<Vec<Session>, String>>,
    main_repos: StdHashMap<PathBuf, Option<PathBuf>>,
    polled_at: i64,
    events: Vec<Event>,
    /// The first poll's desired states, so the tree shows started workspaces before the first tick lands.
    desired: Option<BTreeMap<String, (bool, Vec<String>)>>,
    /// Each workspace's schedules and the errors reading them; `None` without a home or a registry.
    schedules: Option<(BTreeMap<String, Vec<crate::schedules::ScheduleRow>>, Vec<String>)>,
    took: Duration,
}

/// Runs on a background thread: the registry read, `claude agents` and the supervisor's `claude` writes all block.
struct Poller {
    home: Option<PathBuf>,
    events: Option<EventReader>,
    supervision: Option<Supervision>,
    retries: Arc<Mutex<Vec<(String, String)>>>,
    injected: Arc<Mutex<Vec<Session>>>,
    registry_writes: Arc<AtomicU64>,
    hold: Arc<PollHold>,
    snapshot: Option<(Vec<Workspace>, Vec<Session>)>,
    first: bool,
    store: Option<Arc<dyn foreman_core::secret::SecretStore>>,
}

pub(crate) fn timeline(phase: &str, took: Duration) {
    log::info!("startup {phase}: {} ms (at +{:.0} ms)", took.as_millis(), foreman_ui::ms_since_start(Instant::now()));
}

impl Poller {
    fn setup(&mut self) -> supervise::Setup {
        let Some(home) = self.home.clone() else {
            return supervise::Setup {
                plugin_dir: None,
                notices: vec!["No Foreman home (set FOREMAN_HOME or APPDATA): nothing can be started.".into()],
            };
        };
        let setup = supervise::setup(&home);
        self.events = Some(EventReader::new(&home));
        self.supervision = Some(Supervision::new(home, setup.plugin_dir.clone()));
        setup
    }

    fn read(&mut self, with_sessions: bool) -> Refresh {
        let started = Instant::now();
        let registry_writes = self.registry_writes.load(Ordering::SeqCst);
        let workspaces = load_registry(Registry::from_env().map_err(|e| e.to_string()));
        let registry_took = started.elapsed();
        self.hold.wait();
        let polled_at = now_ms();
        let listing = Instant::now();
        let sessions = with_sessions.then(|| {
            list_sessions().map_err(|e| e.to_string()).map(|mut rows| {
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
            (Ok((workspaces, _)), Some(Ok(sessions))) => Some((workspaces.clone(), sessions.clone())),
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
                let store = self.store.get_or_insert_with(|| foreman_core::secret::default_store(home));
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

    /// The supervisor ticks only on the snapshot [`Poller::read`] took in this poll: a stale list never drives a launch.
    fn tick(&mut self) -> Option<Report> {
        let ticking = Instant::now();
        let first = std::mem::take(&mut self.first);
        let (workspaces, sessions) = self.snapshot.take()?;
        let supervision = self.supervision.as_mut()?;
        let retries = std::mem::take(&mut *self.retries.lock().unwrap_or_else(|e| e.into_inner()));
        let report = supervision.tick(&workspaces, &sessions, retries, now_ms());
        if first {
            timeline("supervisor tick done (relays, messages)", ticking.elapsed());
        }
        Some(report)
    }
}

impl FleetView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let this = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            if this.update(cx, |this, cx| this.should_close(window, cx)).unwrap_or(true) {
                crate::tray::close(window, cx);
            }
            false
        });
        let home = Registry::from_env().ok().map(|r| r.home().to_path_buf());
        let loaded = home.as_deref().and_then(|h| Settings::load(h).ok());
        let left_width = loaded
            .as_ref()
            .and_then(|s| s.window.left_width)
            .map_or(DEFAULT_LEFT_WIDTH, |w| px(w as f32).max(MIN_LEFT_WIDTH));
        let loaded = loaded.unwrap_or_default();
        let term_font_size = px(loaded.terminal.font_size() as f32);
        let installed_fonts: Arc<Vec<String>> = Arc::new(cx.text_system().all_font_names());
        let term_font_family = SharedString::from(loaded.terminal.font_family(&installed_fonts));
        let term_ligatures = loaded.terminal.ligatures();
        let retries: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
        let injected: Arc<Mutex<Vec<Session>>> = Arc::default();
        let registry_writes: Arc<AtomicU64> = Arc::default();
        let poll_hold: Arc<PollHold> = Arc::default();
        let (poll_nudge, mut woken) = nudge::nudge();
        let mut poller = Poller {
            home: home.clone(),
            events: None,
            supervision: None,
            retries: retries.clone(),
            injected: injected.clone(),
            registry_writes: registry_writes.clone(),
            hold: poll_hold.clone(),
            snapshot: None,
            first: true,
            store: None,
        };
        timeline("window", Duration::ZERO);
        cx.spawn(async move |this, cx| {
            let executor = cx.background_executor().clone();
            let checking = Instant::now();
            let checked = executor.spawn(async move {
                let checked = check_supported();
                timeline("version check done", checking.elapsed());
                checked
            });
            let first = executor.spawn(async move {
                let setting_up = Instant::now();
                let setup = poller.setup();
                timeline("setup done", setting_up.elapsed());
                let refresh = poller.read(true);
                (poller, setup, refresh)
            });
            let (checked, (ready, setup, mut refresh)) = futures::join!(checked, first);
            poller = ready;
            let supported = checked.is_ok();
            if !supported {
                refresh.sessions = None;
                poller.snapshot = None;
            }
            let mut first = Some(refresh);
            if this
                .update(cx, |this, cx| {
                    this.cli = match checked {
                        Ok(version) => Cli::Supported(version),
                        Err(error) => Cli::Failed(error.to_string()),
                    };
                    this.plugin_dir = setup.plugin_dir;
                    this.setup_notices = setup.notices;
                    cx.notify();
                })
                .is_err()
            {
                return;
            }
            loop {
                let Ok(asks) = this.read_with(cx, |this, _| this.refresh_asks) else {
                    return;
                };
                let refresh = match first.take() {
                    Some(refresh) => refresh,
                    None => {
                        let (done, refresh) = executor
                            .spawn(async move {
                                let refresh = poller.read(supported);
                                (poller, refresh)
                            })
                            .await;
                        poller = done;
                        refresh
                    }
                };
                if this
                    .update(cx, |this, cx| {
                        this.apply(refresh);
                        cx.notify();
                    })
                    .is_err()
                {
                    return;
                }
                let (done, report) = executor
                    .spawn(async move {
                        let report = poller.tick();
                        (poller, report)
                    })
                    .await;
                poller = done;
                let alive = this.update(cx, |this, cx| {
                    if let Some(report) = report {
                        this.apply_report(report);
                    }
                    this.refreshes += 1;
                    this.refreshes_answered = asks;
                    this.check_loaded(cx);
                    let left = std::mem::take(&mut this.left_pending);
                    this.leave_servers(left, cx);
                    this.resolve_tabs(cx);
                    this.reopen_tabs(cx);
                    let pids: Vec<u32> = this
                        .sessions
                        .iter()
                        .filter(|s| s.kind == foreman_claude::Kind::Interactive)
                        .filter_map(|s| s.pid)
                        .collect();
                    if !this.removing.is_empty() {
                        this.finish_removals(cx);
                    }
                    let names = this.workspaces.iter().map(|w| w.name.clone()).collect();
                    this.work_inbox.update(cx, |inbox, cx| inbox.set_workspaces(names, cx));
                    cx.notify();
                    pids
                });
                let Ok(pids) = alive else {
                    return;
                };
                let exited = std::pin::pin!(any_exit(&executor, pids, REFRESH_EVERY));
                futures::future::select(std::pin::pin!(woken.wait(&executor, REFRESH_EVERY)), exited).await;
            }
        })
        .detach();
        Self::blink_cursor(cx);
        let (workstate_recheck, recheck) = nudge::nudge();
        Self::watch_workstate(home.clone(), recheck, cx);
        let work_inbox = WorkInbox::shared(home.clone(), cx);
        cx.observe(&work_inbox, |this, _, cx| {
            this.check_loaded(cx);
            cx.notify();
        })
        .detach();
        let hands = home.as_deref().map(|home| Self::open_hands(home, cx));
        let (stars, stars_notice) = Stars::load(home.as_deref());
        let (dismissed_messages, dismissed_notice) = DismissedMessages::load(home.as_deref());
        let (runs_dismissed, runs_notice) = DismissedMessages::load_at(home.as_deref().map(|h| h.join(RUNS_DISMISSED)), "every failed scheduled run shows");
        let schedule_store: Arc<dyn foreman_core::secret::SecretStore> = match home.as_deref() {
            Some(h) => foreman_core::secret::default_store(h),
            None => Arc::new(foreman_core::secret::MemoryStore::default()),
        };
        let (transcripts_nudge, transcripts_nudged) = nudge::nudge();
        let claude_dir = std::env::var_os(CLAUDE_DIR_VAR).filter(|d| !d.is_empty()).map(PathBuf::from).or_else(foreman_claude::claude_dir);
        let transcripts_hold: Arc<PollHold> = Arc::default();
        Self::watch_transcripts(claude_dir.clone(), home.as_deref().map(|h| h.join(TRANSCRIPTS_CACHE)), transcripts_hold.clone(), transcripts_nudged, cx);
        let desired_writes = Self::desired_writer(cx);
        let tabs_writes = Self::tabs_writer(home.clone(), cx);
        cx.on_focus_lost(window, |this, window, cx| this.focus.focus(window, cx)).detach();
        let search = cx.new(|cx| TextInput::new("Search sessions, workspaces, repos (Ctrl+Shift+E)", None, window, cx));
        cx.subscribe_in(&search, window, Self::on_search_event).detach();
        Self {
            cli: Cli::Checking,
            workspaces: Vec::new(),
            registry_error: None,
            registry_skipped: Vec::new(),
            sessions: Vec::new(),
            main_repos: StdHashMap::new(),
            sessions_error: None,
            last_refresh_took: None,
            refreshes: 0,
            show_stopped: false,
            own_prefix: None,
            collapsed: HashSet::new(),
            tabs: Vec::new(),
            active: None,
            next_tab_id: 1,
            shell_args: vec!["-NoLogo".into()],
            focus,
            probe: FrameProbe::default(),
            hits: HitTargets::default(),
            window: window.window_handle(),
            quitting: false,
            quit_asked: false,
            home,
            plugin_dir: None,
            setup_notices: Vec::new(),
            needs: Vec::new(),
            desired: BTreeMap::new(),
            max_working: DEFAULT_MAX_WORKING,
            supervise_errors: Vec::new(),
            supervise_dismissed: Vec::new(),
            daemon_hold: None,
            last_attempt: StdHashMap::new(),
            hooks: StdHashMap::new(),
            polled_at: 0,
            ticks: 0,
            retries,
            injected,
            registry_writes,
            poll_hold,
            transcripts_hold,
            left_width,
            term_font_size,
            term_font_family,
            term_ligatures,
            installed_fonts,
            font_save: None,
            zoom_carried: 0.,
            settings_window: None,
            last_alive: StdHashMap::new(),
            relays: BTreeMap::new(),
            messages: BTreeMap::new(),
            expanded: HashSet::new(),
            dismissed_messages,
            left_hold: None,
            left_hold_timer: None,
            left_holds: 0,
            answered: BTreeMap::new(),
            answered_route: BTreeMap::new(),
            compose: None,
            work_live: Rc::default(),
            work_inbox,
            menu: None,
            confirm: None,
            adding: None,
            editing: None,
            removing: BTreeMap::new(),
            pick_folder: None,
            open_with: |path| foreman_core::process::shell_execute("open", path.as_os_str(), None, None),
            workstate: WsStatus::Checking,
            workstate_dismissed: None,
            workstate_recheck,
            start_warning: None,
            signing_in: None,
            sign_in_error: None,
            hands_ready: hands.is_none(),
            hands,
            hand_notices: Vec::new(),
            hands_released: false,
            quit_release: None,
            desired_writes,
            synced_server: None,
            left_servers: Default::default(),
            left_pending: Vec::new(),
            closed: Vec::new(),
            closed_repos: StdHashMap::new(),
            transcript_scans: 0,
            transcripts_nudge,
            poll_nudge,
            refresh_asks: 0,
            refreshes_answered: 0,
            sessions_read: false,
            transcripts_read: claude_dir.is_none(),
            transcripts_progress: None,
            transcripts_answered: 0,
            loaded: false,
            loaded_at_scan: 0,
            transcripts_cache: None,
            transcripts_restart: false,
            action_error: stars_notice.or(dismissed_notice).or(runs_notice),
            stars,
            closed_expanded: HashSet::new(),
            claude_command: "claude".into(),
            search,
            search_marks: StdHashMap::new(),
            tabs_resolving: false,
            tabs_again: false,
            tab_resolves: 0,
            toast: ToastSlot::default(),
            pane_toast: ToastSlot::default(),
            claude_dir,
            deleted: StdHashMap::new(),
            deleting: HashSet::new(),
            painted_sessions: false,
            tabs_restored: false,
            tabs_reopening: false,
            tabs_frozen: false,
            tabs_saved: None,
            tabs_writes,
            tab_notices: Vec::new(),
            schedules: BTreeMap::new(),
            schedule_errors: Vec::new(),
            schedule_errors_dismissed: Vec::new(),
            sync_errors: BTreeMap::new(),
            registrar: Arc::new(foreman_core::schedule::WindowsRegistrar),
            schedule_store,
            schedule_edit: None,
            runs_dismissed,
        }
    }

    fn watch_transcripts(claude_dir: Option<PathBuf>, cache: Option<PathBuf>, hold: Arc<PollHold>, mut nudged: Nudged, cx: &mut Context<Self>) {
        let Some(claude_dir) = claude_dir else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let executor = cx.background_executor().clone();
            let mut scanner: Option<Transcripts> = None;
            let mut repos: StdHashMap<PathBuf, Option<PathBuf>> = StdHashMap::new();
            let (mut from_cache, mut wait) = (false, true);
            loop {
                if scanner.is_none() {
                    let (claude_dir, cache) = (claude_dir.clone(), cache.clone());
                    let opened = executor
                        .spawn(async move {
                            let s = Transcripts::in_claude_dir(&claude_dir);
                            match cache {
                                Some(cache) => s.with_cache(cache),
                                None => s,
                            }
                        })
                        .await;
                    from_cache = opened.has_cache();
                    scanner = Some(opened);
                    if this
                        .update(cx, |this, cx| {
                            this.transcripts_cache = Some(from_cache);
                            this.check_loaded(cx);
                            cx.notify();
                        })
                        .is_err()
                    {
                        return;
                    }
                }
                // the first scan waits for the first poll's apply, which wakes it with the workspaces and sessions
                if std::mem::replace(&mut wait, true) {
                    nudged.wait(&executor, TRANSCRIPTS_EVERY).await;
                }
                let Ok(asked) = this.update(cx, |this, _| {
                    let restart = std::mem::take(&mut this.transcripts_restart);
                    this.sessions_read.then(|| {
                        let roots: Vec<PathBuf> = this
                            .workspaces
                            .iter()
                            .flat_map(|w| w.folders.iter().filter(|r| r.enabled).map(|r| r.path.clone()))
                            .collect();
                        let keep: std::collections::HashSet<String> = this.stars.ids().iter().cloned().collect();
                        let live: std::collections::HashSet<String> = this.sessions.iter().map(|s| s.session_id.clone()).collect();
                        (roots, keep, live, this.refresh_asks, restart)
                    })
                }) else {
                    return;
                };
                let Some((roots, keep, live, asks, restart)) = asked else { continue };
                if restart {
                    (scanner, wait) = (None, false);
                    repos.clear();
                    continue;
                }
                let since = now_ms() - fleet::CLOSED_WINDOW_MS;
                let scanning = Instant::now();
                if std::mem::take(&mut from_cache)
                    && let Some(s) = scanner.take()
                {
                    let (roots, keep) = (roots.clone(), keep.clone());
                    let (back, known, closed) = executor
                        .spawn(async move {
                            let mut repos = StdHashMap::new();
                            let found = s.cached(&roots, since, &keep);
                            for t in &found {
                                if let Some(cwd) = &t.cwd {
                                    repos.entry(cwd.clone()).or_insert_with(|| main_repo(cwd));
                                }
                            }
                            (s, repos, found.iter().map(|t| t.to_session()).collect::<Vec<Session>>())
                        })
                        .await;
                    scanner = Some(back);
                    repos = known.clone();
                    if this
                        .update(cx, |this, cx| {
                            timeline(&format!("transcripts from the cache done ({} listed)", closed.len()), scanning.elapsed());
                            this.main_repos.extend(known.clone());
                            this.closed_repos = known;
                            this.closed = closed;
                            this.drop_deleted();
                            this.transcripts_read = true;
                            this.check_loaded(cx);
                            cx.notify();
                        })
                        .is_err()
                    {
                        return;
                    }
                }
                let (progress, mut progressed) = futures::channel::mpsc::unbounded::<(usize, usize)>();
                let shown = this.clone();
                let showing = cx.spawn(async move |cx| {
                    while let Some(at) = progressed.next().await {
                        if shown
                            .update(cx, |this, cx| {
                                this.transcripts_progress = Some(at);
                                cx.notify();
                            })
                            .is_err()
                        {
                            return;
                        }
                    }
                });
                let Some(mut s) = scanner.take() else { continue };
                let hold = hold.clone();
                let (back, known, closed, read) = executor
                    .spawn(async move {
                        hold.wait();
                        let read = AtomicUsize::new(0);
                        let found = s.scan_with(&roots, since, &keep, &live, &|done, of| {
                            read.store(of, Ordering::Relaxed);
                            if done == of || done % (of / 100).max(1) == 0 {
                                progress.unbounded_send((done, of)).ok();
                            }
                        });
                        drop(progress);
                        for t in &found {
                            if let Some(cwd) = &t.cwd {
                                repos.entry(cwd.clone()).or_insert_with(|| main_repo(cwd));
                            }
                        }
                        let closed: Vec<Session> = found.iter().map(|t| t.to_session()).collect();
                        (s, repos, closed, read.into_inner())
                    })
                    .await;
                showing.await;
                scanner = Some(back);
                repos = known.clone();
                if this
                    .update(cx, |this, cx| {
                        this.main_repos.extend(known.clone());
                        this.closed_repos = known;
                        if this.transcript_scans == 0 {
                            timeline(&format!("transcripts done ({read} files read, {} listed)", closed.len()), scanning.elapsed());
                        }
                        this.transcripts_read = true;
                        this.closed = closed;
                        this.drop_deleted();
                        this.transcripts_progress = None;
                        this.transcripts_answered = asks;
                        this.transcript_scans += 1;
                        this.check_loaded(cx);
                        cx.notify();
                    })
                    .is_err()
                {
                    return;
                }
            }
        })
        .detach();
    }

    /// Until every source has loaded once, and from a Refresh until the poll and the transcript scan it woke are applied.
    pub fn refreshing(&self) -> bool {
        !self.loaded || self.refreshes_answered < self.refresh_asks || (self.claude_dir.is_some() && self.transcripts_answered < self.refresh_asks)
    }

    /// The sources a fresh start still waits for, as the action bar names them; empty once all have loaded.
    pub fn loading(&self, cx: &gpui::App) -> Vec<String> {
        if self.loaded {
            return Vec::new();
        }
        let mut pending = Vec::new();
        if !self.sessions_read {
            pending.push("session list".to_string());
        } else if self.refreshes == 0 {
            pending.push("supervisor".to_string());
        }
        if !self.transcripts_read && self.transcripts_cache != Some(true) {
            pending.push(match self.transcripts_progress {
                Some((read, of)) => format!("transcripts {read}/{of}"),
                None => "transcripts".to_string(),
            });
        }
        let inbox = self.work_inbox.read(cx);
        if matches!(self.workstate, WsStatus::Checking) {
            pending.push("WorkState".to_string());
        } else if inbox.reads > 0 && inbox.applied == 0 {
            pending.push("WorkState inbox".to_string());
        }
        pending
    }

    fn check_loaded(&mut self, cx: &gpui::App) {
        if !self.loaded && self.loading(cx).is_empty() {
            self.loaded = true;
            self.loaded_at_scan = self.transcript_scans;
            timeline("every source done", Duration::ZERO);
        }
    }

    /// The transcripts as a start reads them, from the saved cache when `warm`; for the selftests.
    pub fn restart_transcripts(&mut self, warm: bool, cx: &mut Context<Self>) {
        self.transcripts_restart = true;
        self.transcripts_read = false;
        self.closed.clear();
        self.transcripts_cache = Some(warm);
        self.loaded = false;
        self.transcripts_nudge.wake();
        cx.notify();
    }

    pub fn refresh_now(&mut self, cx: &mut Context<Self>) {
        if self.refreshing() {
            return;
        }
        self.refresh_asks += 1;
        self.poll_nudge.wake();
        self.transcripts_nudge.wake();
        cx.notify();
    }

    pub fn toggle_star(&mut self, session_id: &str, cx: &mut Context<Self>) {
        match self.stars.toggle(session_id) {
            Ok(_) => self.transcripts_nudge.wake(),
            Err(e) => self.action_error = Some(format!("Cannot save the star: {e}")),
        }
        cx.notify();
    }

    pub fn dismiss_message(&mut self, workspace: &str, file: &str, cx: &mut Context<Self>) {
        if let Err(e) = self.dismissed_messages.dismiss(workspace, file) {
            self.action_error = Some(format!("Cannot save the dismissed message: {e}"));
        }
        self.expanded.remove(&format!("{workspace}/{file}"));
        cx.notify();
    }

    fn open_hands(home: &std::path::Path, cx: &mut Context<Self>) -> Arc<Mutex<Hands>> {
        let (hands, notice) = Hands::open(home);
        let hands = Arc::new(Mutex::new(hands));
        let stale = hands.clone();
        cx.spawn(async move |this, cx| {
            let mut notices = cx
                .background_executor()
                .spawn(async move { stale.lock().unwrap_or_else(|e| e.into_inner()).release_all() })
                .await;
            notices.extend(notice);
            this.update(cx, |this, cx| {
                this.hand_notices.extend(notices);
                this.hands_ready = true;
                this.reopen_tabs(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
        let live = hands.clone();
        cx.on_app_quit(move |_, cx| {
            let live = live.clone();
            let release = cx.background_executor().spawn(async move { crate::hand::release_all_within(live, QUIT_RELEASE_WITHIN) });
            async move {
                release.await;
            }
        })
        .detach();
        hands
    }

    fn watch_workstate(home: Option<PathBuf>, mut recheck: Nudged, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let executor = cx.background_executor().clone();
            let mut first = Some(Instant::now());
            loop {
                let home = home.clone();
                let status = executor
                    .spawn(async move {
                        let auth = home.as_deref().map(crate::signin::auth);
                        workstate_setup::detect(home.as_deref(), auth.as_deref())
                    })
                    .await;
                if let Some(started) = first.take() {
                    timeline("workstate status done", started.elapsed());
                }
                let alive = this.update(cx, |this, cx| {
                    if this.workstate != status {
                        if matches!(status, WsStatus::Ready { .. }) {
                            this.start_warning = None;
                        }
                        this.workstate = status;
                        cx.notify();
                    }
                    if let WsStatus::Ready { url, .. } = &this.workstate
                        && this.synced_server.as_ref() != Some(url)
                    {
                        let url = url.clone();
                        this.sync_server(url, cx);
                    }
                    this.check_loaded(cx);
                });
                if alive.is_err() {
                    return;
                }
                recheck.wait(&executor, workstate_setup::RECHECK_EVERY).await;
            }
        })
        .detach();
    }

    pub fn recheck_workstate(&self) {
        self.workstate_recheck.wake();
    }

    pub fn sign_in(&mut self, cx: &mut Context<Self>) {
        let Some(home) = self.home.clone() else {
            return;
        };
        let origin = crate::signin::auth(&home).configured_origin();
        self.sign_in_to(origin, None, cx);
    }

    pub fn sign_in_to(&mut self, origin: String, from: Option<String>, cx: &mut Context<Self>) {
        let Some(home) = self.home.clone() else {
            return;
        };
        if self.signing_in.is_some() {
            return;
        }
        self.signing_in = Some(origin.clone());
        self.sign_in_error = None;
        let task = {
            let (home, origin) = (home.clone(), origin.clone());
            cx.background_executor().spawn(async move { crate::signin::sign_in(&home, &origin) })
        };
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.signing_in = None;
                match result {
                    Ok(_) => {
                        this.left_servers.remove(&origin);
                        if let Some(from) = from {
                            match save_workstate_url(&home, &origin) {
                                Ok(()) => this.leave_servers(vec![from], cx),
                                Err(e) => this.action_error = Some(e),
                            }
                        }
                        this.synced_server = None;
                    }
                    Err(e) => this.sign_in_error = Some(format!("Signing in to {origin} did not work: {e}")),
                }
                this.workstate = WsStatus::Checking;
                this.recheck_workstate();
                WorkHub::reconfigure(Some(home), cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn cancel_sign_in(&self) {
        if let Some(home) = &self.home {
            crate::signin::auth(home).cancel_sign_in();
        }
    }

    pub fn sign_out(&mut self, cx: &mut Context<Self>) {
        let Some(home) = self.home.clone() else {
            return;
        };
        let auth = crate::signin::auth(&home);
        let task = cx.background_executor().spawn(async move { auth.sign_out(&auth.configured_origin()) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                if let Err(e) = result {
                    this.hand_notices.push(e);
                }
                this.workstate = WsStatus::Checking;
                this.recheck_workstate();
                WorkHub::reconfigure(Some(home), cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn set_workstate_url(&mut self, text: &str, cx: &mut Context<Self>) -> Result<(), String> {
        let home = self.home.clone().ok_or("No Foreman home: settings cannot be saved.")?;
        let origin = foreman_core::validate_workstate_url(text)?;
        let current = Settings::load(&home).map_err(|e| format!("Cannot read settings.toml: {e}"))?.workstate.origin();
        if origin == current {
            return Ok(());
        }
        let status = {
            let (auth, current) = (crate::signin::auth(&home), current.clone());
            cx.background_executor().spawn(async move { auth.status(&current) })
        };
        cx.spawn(async move |this, cx| {
            let signed_in = matches!(status.await, foreman_workstate::AuthStatus::SignedIn { .. });
            this.update(cx, |this, cx| {
                if signed_in {
                    this.sign_in_to(origin, Some(current), cx);
                    return;
                }
                if let Err(e) = save_workstate_url(&home, &origin) {
                    this.action_error = Some(e);
                }
                this.workstate = WsStatus::Checking;
                this.recheck_workstate();
                WorkHub::reconfigure(Some(home), cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
        Ok(())
    }

    fn sync_server(&mut self, origin: String, cx: &mut Context<Self>) {
        self.synced_server = Some(origin.clone());
        let Some(hands) = self.hands.clone() else {
            return;
        };
        let at = origin.clone();
        let task = cx.background_executor().spawn(async move {
            let mut hands = hands.lock().unwrap_or_else(|e| e.into_inner());
            (hands.elsewhere(&at), hands.take_left())
        });
        cx.spawn(async move |this, cx| {
            let (elsewhere, left) = task.await;
            this.update(cx, |this, cx| {
                let stale = stale_tabs(&this.tabs, &elsewhere);
                for tab in this.tabs.iter_mut().filter(|t| t.hand && !t.exited) {
                    tab.restart_to = stale.contains(&tab.id).then(|| origin.clone());
                }
                this.hand_notices.extend(crate::hand::restart_notice(stale.len(), &origin));
                this.leave_servers(elsewhere.into_iter().filter_map(|(_, o)| o).chain(left).collect(), cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn leave_servers(&mut self, origins: Vec<String>, cx: &mut Context<Self>) {
        let Some(home) = self.home.clone().filter(|_| !origins.is_empty()) else {
            return;
        };
        let auth = crate::signin::auth(&home);
        let current = auth.configured_origin();
        let origins: Vec<String> = origins.into_iter().filter(|o| *o != current && self.left_servers.insert(o.clone())).collect();
        if origins.is_empty() {
            return;
        }
        let revoke = cx.background_executor().spawn(async move { origins.iter().filter_map(|o| auth.sign_out(o).err()).collect::<Vec<_>>() });
        cx.spawn(async move |this, cx| {
            let failed = revoke.await;
            if !failed.is_empty() {
                this.update(cx, |this, cx| {
                    this.hand_notices.push(format!("The old server's grant was not revoked: {}", failed.join("; ")));
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    pub fn dismiss_workstate_notice(&mut self, cx: &mut Context<Self>) {
        self.workstate_dismissed = self.workstate.problem();
        cx.notify();
    }

    pub fn workstate_notice(&self) -> Option<String> {
        self.workstate.problem().filter(|p| self.workstate_dismissed.as_ref() != Some(p))
    }

    fn apply(&mut self, refresh: Refresh) {
        match refresh.workspaces {
            Ok(_) if refresh.registry_writes != self.registry_writes.load(Ordering::SeqCst) => {}
            Ok((workspaces, skipped)) => {
                if self.workspaces != workspaces {
                    self.transcripts_nudge.wake();
                }
                self.workspaces = workspaces;
                self.registry_skipped = skipped;
                self.registry_error = None;
            }
            Err(error) => self.registry_error = Some(error),
        }
        match refresh.sessions {
            Some(Ok(sessions)) => {
                let now = now_ms();
                // an interactive session leaves `claude agents` when it exits; its transcript makes it a closed row
                let ended = self.sessions.iter().any(|old| {
                    old.kind == foreman_claude::Kind::Interactive && !sessions.iter().any(|s| s.session_id == old.session_id)
                });
                if ended {
                    self.transcripts_nudge.wake();
                }
                for s in &sessions {
                    if fleet::is_alive(&s.health(now)) {
                        self.last_alive.insert(s.session_id.clone(), now);
                    }
                }
                *self.work_live.borrow_mut() = sessions
                    .iter()
                    .filter(|s| fleet::attachable(s, &s.health(now)))
                    .map(|s| LiveSession { claude_session_id: s.session_id.clone(), id: s.id.clone(), name: s.name.clone() })
                    .collect();
                self.sessions = sessions;
                self.drop_deleted();
                self.main_repos = refresh.main_repos;
                self.main_repos.extend(self.closed_repos.iter().map(|(k, v)| (k.clone(), v.clone())));
                self.sessions_error = None;
                self.polled_at = refresh.polled_at;
            }
            Some(Err(error)) => self.sessions_error = Some(error),
            None => {}
        }
        if !self.sessions_read {
            self.transcripts_nudge.wake();
        }
        self.sessions_read = true;
        for event in refresh.events {
            HookSignal::record(&mut self.hooks, event);
        }
        if let Some(desired) = refresh.desired {
            self.desired = desired;
        }
        if let Some((schedules, errors)) = refresh.schedules {
            self.schedules = schedules;
            self.schedule_errors = errors;
            foreman_ui::notices::keep_dismissed(&mut self.schedule_errors_dismissed, &self.schedule_errors);
        }
        self.last_refresh_took = Some(refresh.took);
    }

    fn apply_report(&mut self, report: Report) {
        self.desired = report.desired;
        self.needs = report.needs;
        let (workspaces, desired, own) = (&self.workspaces, &self.desired, self.own_prefix.as_deref());
        self.needs.retain(|n| !fleet::is_foreign(&n.name, workspaces, Some(desired), own));
        self.max_working = report.max_working;
        self.supervise_errors = report.errors;
        foreman_ui::notices::keep_dismissed(&mut self.supervise_dismissed, &self.supervise_errors);
        self.daemon_hold = report.daemon_hold;
        self.left_pending.extend(report.left_servers);
        self.last_attempt = report.last_attempt.into_iter().collect();
        self.relays = report.relays;
        self.messages = report.messages;
        if let Err(e) = self.dismissed_messages.prune(&self.messages) {
            self.action_error = Some(format!("Cannot save the dismissed messages: {e}"));
        }
        for (workspace, on_disk) in report.answered {
            let answered = self.answered.entry(workspace).or_default();
            for (file, line) in on_disk {
                answered.entry(file).or_insert(line);
            }
        }
        self.ticks += 1;
    }

    pub fn tree(&self) -> Tree {
        let sup = Supervised {
            desired: Some(&self.desired),
            hooks: Some(&self.hooks),
            polled_at: self.polled_at,
            main_repos: Some(&self.main_repos),
            closed: &self.closed,
            starred: Some(self.stars.ids()),
            expanded: Some(&self.closed_expanded),
            disabled_folders: false,
            own_prefix: self.own_prefix.as_deref(),
        };
        fleet::build(&self.workspaces, &self.sessions, now_ms(), self.show_stopped, &sup)
    }

    pub fn start_workspace(&mut self, name: &str, cx: &mut Context<Self>) {
        let write = match (&self.home, self.workspaces.iter().find(|p| p.name == name)) {
            (Some(home), Some(workspace)) => {
                DesiredWrite::Start { home: home.clone(), plugin_dir: self.plugin_dir.clone(), workspace: workspace.clone() }
            }
            (None, _) => return self.desired_failed(format!("Cannot start {name}: no Foreman home"), cx),
            (_, None) => return self.desired_failed(format!("Cannot start {name}: not registered"), cx),
        };
        self.desired_writes.unbounded_send(write).ok();
    }

    pub fn stop_workspace(&mut self, name: &str, cx: &mut Context<Self>) {
        let Some(home) = self.home.clone() else {
            return self.desired_failed(format!("Cannot stop {name}: no Foreman home"), cx);
        };
        self.desired_writes.unbounded_send(DesiredWrite::Stop { home, name: name.to_string() }).ok();
    }

    fn desired_failed(&mut self, error: String, cx: &mut Context<Self>) {
        self.action_error = Some(error);
        cx.notify();
    }

    fn desired_writer(cx: &mut Context<Self>) -> futures::channel::mpsc::UnboundedSender<DesiredWrite> {
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<DesiredWrite>();
        cx.spawn(async move |this, cx| {
            use futures::StreamExt as _;
            while let Some(write) = rx.next().await {
                let done = cx.background_executor().spawn(async move {
                    match write {
                        DesiredWrite::Start { home, plugin_dir, workspace } => {
                            let result = supervise::start_workspace(&home, plugin_dir.as_deref(), &workspace);
                            (workspace.name, "start", result.map(Some))
                        }
                        DesiredWrite::Stop { home, name } => {
                            let result = supervise::stop_workspace(&home, &name);
                            (name, "stop", result.map(|()| None))
                        }
                    }
                });
                let (name, verb, result) = done.await;
                let alive = this.update(cx, |this, cx| {
                    match result {
                        Ok(Some(desired)) => {
                            let names = desired.sessions.iter().map(|w| w.name.clone()).collect();
                            this.desired.insert(name.clone(), (true, names));
                            this.action_error = None;
                            this.start_warning = this.workstate.start_warning(&name);
                        }
                        Ok(None) => {
                            if let Some(entry) = this.desired.get_mut(&name) {
                                entry.0 = false;
                            }
                            this.action_error = None;
                            this.start_warning = None;
                        }
                        Err(e) => this.action_error = Some(format!("Cannot {verb} {name}: {e:#}")),
                    }
                    cx.notify();
                });
                if alive.is_err() {
                    return;
                }
            }
        })
        .detach();
        tx
    }

    pub fn relay_refusal(&self, workspace: &str) -> Option<String> {
        if self.home.is_none() {
            return Some("no Foreman home".into());
        }
        match self.relays.get(workspace) {
            Some(r) if r.started => None,
            _ => Some(format!("{workspace} is stopped")),
        }
    }

    pub fn open_compose(&mut self, workspace: &str, reply_to: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let input = match &self.compose {
            Some(c) if c.workspace == workspace && c.reply_to == reply_to => c.input.clone(),
            _ => {
                let input = cx.new(|cx| TextInput::new("Type here; Enter adds a line.", None, window, cx).multi_line());
                let events = cx.subscribe_in(&input, window, |this: &mut Self, _, event: &InputEvent, window, cx| this.on_compose_event(event, window, cx));
                self.compose = Some(Compose::new(workspace, reply_to, input.clone(), events));
                input
            }
        };
        input.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn on_compose_event(&mut self, event: &InputEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            InputEvent::Confirm => self.send_compose(cx),
            InputEvent::Cancel => {
                self.compose = None;
                self.focus.focus(window, cx);
            }
            InputEvent::Edited => {
                if let Some(c) = &mut self.compose {
                    c.error = None;
                }
            }
            InputEvent::ConfirmBack | InputEvent::Refused(_) => {}
        }
        cx.notify();
    }

    pub fn send_compose(&mut self, cx: &mut Context<Self>) {
        let Some(c) = &self.compose else {
            return;
        };
        let (workspace, reply_to, text) = (c.workspace.clone(), c.reply_to.clone(), c.text(cx).trim().to_string());
        let result = if text.is_empty() {
            Err("Nothing to send.".to_string())
        } else {
            match &reply_to {
                Some(file) => self.answer(&workspace, file, &text),
                None => self.send_to_workspace(&workspace, &text),
            }
        };
        match (result, &mut self.compose) {
            (Ok(_), _) if reply_to.is_some() => self.compose = None,
            (Ok(sent), Some(c)) => {
                c.input.update(cx, |input, cx| input.set_text("", cx));
                c.sent = Some(compose::first_line(&text));
                c.route = Some(sent.label());
                c.error = None;
            }
            (Err(e), Some(c)) => c.error = Some(e),
            (_, None) => {}
        }
        cx.notify();
    }

    fn send_to_workspace(&self, workspace: &str, text: &str) -> Result<Sent, String> {
        let home = self.home.as_deref().ok_or("Not sent: no Foreman home.")?;
        let to = workspace_session_name(workspace);
        let mode = DesiredState::launch_mode(home, workspace, &to);
        send_from_human(home, workspace, &to, mode.as_deref(), text, self.relay_refusal(workspace))
            .map_err(|e| format!("Not sent: direct delivery failed ({}), and the relay cannot take it: {}.", e.direct, e.relay))
    }

    pub fn answer(&mut self, workspace: &str, file: &str, text: &str) -> Result<Sent, String> {
        let sent = self.send_to_workspace(workspace, text)?;
        self.answered_route.insert(format!("{workspace}/{file}"), sent.label());
        let line = compose::first_line(text);
        self.answered.entry(workspace.to_string()).or_default().insert(file.to_string(), line.clone());
        if let Some(home) = &self.home
            && let Err(e) = mark_answered(home, workspace, file, &line)
        {
            self.action_error = Some(format!("Answered {file}, but cannot save that: {e}"));
        }
        Ok(sent)
    }

    pub fn approve(&mut self, workspace: &str, file: &str, cx: &mut Context<Self>) {
        match self.answer(workspace, file, "approve") {
            Ok(_) => self.action_error = None,
            Err(e) => self.action_error = Some(format!("Approve {workspace}: {e}")),
        }
        cx.notify();
    }

    pub fn set_max_working(&mut self, max_working: usize, cx: &mut Context<Self>) {
        let max_working = max_working.min(MAX_WORKING_LIMIT);
        let result = match &self.home {
            Some(home) => supervise::set_max_working(home, max_working),
            None => Err(anyhow::anyhow!("no Foreman home")),
        };
        match result {
            Ok(()) => {
                self.max_working = max_working;
                self.action_error = None;
            }
            Err(e) => self.action_error = Some(format!("Cannot save the busy-session limit: {e:#}")),
        }
        cx.notify();
    }

    pub fn retry(&mut self, workspace: &str, name: &str, cx: &mut Context<Self>) {
        self.retries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((workspace.to_string(), name.to_string()));
        self.needs.retain(|n| !(n.workspace == workspace && n.name == name));
        cx.notify();
    }

    /// After a failed Adopt: `name` drops the conversation it forks from and launches as a new session.
    pub fn start_fresh(&mut self, workspace: &str, name: &str, cx: &mut Context<Self>) {
        let result = match &self.home {
            Some(home) => DesiredState::update(home, workspace, |d| d.start_fresh(name)).map(drop).map_err(|e| e.to_string()),
            None => Err("no Foreman home".into()),
        };
        match result {
            Ok(()) => self.retry(workspace, name, cx),
            Err(e) => self.action_error = Some(format!("Cannot start {name} fresh: {e}")),
        }
        cx.notify();
    }

    pub fn attach_by_id(&mut self, workspace: &str, id: &str, window: &mut Window, cx: &mut Context<Self>) -> Option<u64> {
        let session = self.sessions.iter().find(|s| s.id == id)?.clone();
        Some(self.attach(workspace, "supervised", &session, window, cx))
    }

    /// Typed, not run: the human presses Enter and answers the trust prompt; Foreman never answers it.
    pub fn run_in_terminal(&mut self, cwd: PathBuf, command: String, window: &mut Window, cx: &mut Context<Self>) -> u64 {
        let title = format!("trust · {}", cwd.display());
        self.type_in_shell(cwd, title, command, window, cx)
    }

    fn type_in_shell(&mut self, cwd: PathBuf, title: String, command: String, window: &mut Window, cx: &mut Context<Self>) -> u64 {
        self.type_in_shell_when(cwd, title, gpui::Task::ready(command), window, cx)
    }

    fn type_in_shell_when(&mut self, cwd: PathBuf, title: String, command: gpui::Task<String>, window: &mut Window, cx: &mut Context<Self>) -> u64 {
        let id = self.open_tab(title, TabKind::Shell, pwsh(self.shell_args.clone()), Some(cwd), window, cx);
        let (watch, mut output) = nudge::nudge();
        if let Some(tab) = self.tab_mut(id) {
            tab.watchers.push(watch);
        }
        cx.spawn(async move |this, cx| {
            let command = command.await;
            let executor = cx.background_executor().clone();
            let deadline = Instant::now() + SHELL_SETTLE_LIMIT;
            let mut spoke = false;
            loop {
                let left = deadline.saturating_duration_since(Instant::now());
                let woken = output.wait(&executor, if spoke { SHELL_SETTLED.min(left) } else { left }).await;
                let Ok(Some((terminal, failed))) = this.read_with(cx, |this, _| this.tab(id).map(|t| (t.terminal.clone(), t.error.is_some()))) else {
                    return;
                };
                let Some(terminal) = terminal else {
                    if failed {
                        return;
                    }
                    output.wait(&executor, SHELL_SETTLE_LIMIT).await;
                    continue;
                };
                if Instant::now() < deadline {
                    if woken {
                        spoke = true;
                        continue;
                    }
                    if !spoke || terminal.read_with(cx, |t, _| t.get_content()).trim().is_empty() {
                        spoke = false;
                        continue;
                    }
                }
                terminal.update(cx, |t, _| t.input(command.into_bytes()));
                return;
            }
        })
        .detach();
        id
    }

    pub fn open_settings(&mut self, cx: &mut Context<Self>) {
        self.open_settings_at(None, cx);
    }

    pub fn open_settings_at(&mut self, page: Option<&'static str>, cx: &mut Context<Self>) {
        if let Some(handle) = self.settings_window
            && handle
                .update(cx, |settings, window, cx| {
                    if let Some(page) = page {
                        settings.show(page, cx);
                    }
                    window.activate_window()
                })
                .is_ok()
        {
            return;
        }
        let (home, fleet) = (self.home.clone(), cx.weak_entity());
        let this = cx.weak_entity();
        cx.defer(move |cx| {
            let handle = settings_window::open(home, fleet, page, cx);
            this.update(cx, |this, _| this.settings_window = handle).ok();
        });
    }

    pub(crate) fn close_settings(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.settings_window.take() {
            handle.update(cx, |_, window, cx| crate::tray::close(window, cx)).ok();
        }
    }

    fn save_left_width(&mut self) {
        let Some(home) = &self.home else {
            return;
        };
        let width = f32::from(self.left_width).round() as u32;
        if let Err(e) = Settings::update(home, |settings| settings.window.left_width = Some(width)) {
            self.action_error = Some(format!("Cannot save the panel width: {e}"));
        }
    }

    pub fn resumable(&self, row: &SessionRow) -> bool {
        offers_resume(&row.session, &row.health, self.last_alive.get(&row.session.session_id).copied(), now_ms())
    }

    /// `claude --bg --resume` with no other flag keeps the session id.
    pub fn resume(&mut self, workspace: &str, repo: &str, session: &Session, _window: &mut Window, cx: &mut Context<Self>) -> u64 {
        self.resume_titled(format!("{workspace} · {repo} · {}", fleet::display_name(session)), session, cx)
    }

    fn resume_titled(&mut self, title: String, session: &Session, cx: &mut Context<Self>) -> u64 {
        let sid = &session.session_id;
        if session.kind == foreman_claude::Kind::Background {
            let script = match &self.daemon_hold {
                Some(why) => held_line(why),
                None => format!(
                    "Write-Host 'resuming {} in the background, then attaching…'; claude --bg --resume {sid}; if ($LASTEXITCODE -eq 0) {{ claude attach {} }}",
                    session.id, session.id
                ),
            };
            let shell = pwsh(vec!["-NoLogo".into(), "-NoProfile".into(), "-Command".into(), script]);
            let kind = TabKind::Attach { session_id: session.id.clone() };
            let id = self.push_tab(title, kind, Some(session.cwd.clone()), cx);
            self.start_terminal(id, shell, Some(session.cwd.clone()), cx);
            if let Some(tab) = self.tab_mut(id) {
                tab.launched_session = Some(sid.clone());
                tab.session_name = Some(session.name.clone());
            }
            id
        } else {
            let resume = format!("{} --resume '{}'", self.claude_command, sid.replace('\'', "''"));
            let own = (!session.name.is_empty()).then(|| session.name.clone());
            let id = self.hand_claude(session.cwd.clone(), title, resume, own, Some(sid.clone()), cx);
            if let Some(tab) = self.tab_mut(id) {
                tab.session = Some(tab_card::session_label(&session.name, sid));
            }
            id
        }
    }

    pub fn new_claude(&mut self, cwd: PathBuf, title: String, _window: &mut Window, cx: &mut Context<Self>) -> u64 {
        let claude = self.claude_command.clone();
        self.hand_claude(cwd, title, claude, None, None, cx)
    }

    fn hand_claude(&mut self, cwd: PathBuf, title: String, claude: String, own: Option<String>, resumes: Option<String>, cx: &mut Context<Self>) -> u64 {
        let id = self.push_tab(title, TabKind::Shell, Some(cwd.clone()), cx);
        let (watch, mut changed) = nudge::nudge();
        if let Some(tab) = self.tab_mut(id) {
            tab.hand = true;
            tab.watchers.push(watch);
            tab.launched_session = resumes.clone();
            tab.session_name = own.clone();
        }
        let Some(hands) = self.hands.clone() else {
            self.start_terminal(id, self.claude_shell(&Launch::Plain, &claude), Some(cwd), cx);
            return id;
        };
        let at = cwd.clone();
        let mint = hands.clone();
        let launch = cx.background_executor().spawn(async move { mint.lock().unwrap_or_else(|e| e.into_inner()).launch_for(&at, own.as_deref(), resumes.as_deref()) });
        cx.spawn(async move |this, cx| {
            let launch = launch.await;
            this.update(cx, |this, cx| {
                if let Launch::Minted { name, session, notice, new_session, .. } = &launch {
                    if let Some(tab) = this.tab_mut(id) {
                        tab.workstate = Some(format!("Foreman token · {} · {session}", crate::hand::ROLE));
                        tab.hand_key = Some(name.clone());
                        if new_session.is_some() {
                            tab.launched_session = new_session.clone();
                        }
                    }
                    this.hand_notices.extend(notice.clone());
                }
                if this.tab(id).is_some() {
                    let shell = this.claude_shell(&launch, &claude);
                    this.start_terminal(id, shell, Some(cwd), cx);
                }
            })
            .ok();
            let name = match launch {
                Launch::Minted { name, .. } => name,
                Launch::Unminted(notice) => {
                    this.update(cx, |this, cx| {
                        this.hand_notices.push(notice);
                        cx.notify();
                    })
                    .ok();
                    return;
                }
                Launch::Plain => return,
            };
            let executor = cx.background_executor().clone();
            // claude's exit deletes the token file, then the shell prints its prompt: a wakeup
            loop {
                changed.wait(&executor, HAND_EXIT_FALLBACK).await;
                let open = this.read_with(cx, |this, _| this.tab(id).is_some_and(|t| !t.exited)).unwrap_or(false);
                if open {
                    let (hands, name) = (hands.clone(), name.clone());
                    if !executor.spawn(async move { hands.lock().unwrap_or_else(|e| e.into_inner()).ended(&name) }).await {
                        executor.timer(HAND_CHECK_GAP).await;
                        continue;
                    }
                }
                let hands = hands.clone();
                let notice = executor
                    .spawn(async move { hands.lock().unwrap_or_else(|e| e.into_inner()).release(&name) })
                    .await;
                if let Some(notice) = notice {
                    this.update(cx, |this, cx| {
                        this.hand_notices.push(notice);
                        cx.notify();
                    })
                    .ok();
                }
                return;
            }
        })
        .detach();
        id
    }

    fn claude_shell(&self, launch: &Launch, claude: &str) -> util::shell::Shell {
        let launch_args = match &self.daemon_hold {
            Some(why) => {
                let script = format!("{}; {}", held_line(why), launch.script_with(claude));
                ["-NoExit".into(), "-EncodedCommand".into(), crate::hand::encoded_command(&script)]
            }
            None => launch.shell_args_with(claude),
        };
        pwsh(self.shell_args.iter().cloned().chain(launch_args).collect())
    }

    fn blink_cursor(cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                let wait = this.update(cx, |this, cx| {
                    let now = Instant::now();
                    let terminal = this.active.and_then(|id| this.tab(id)).and_then(|t| t.terminal.clone());
                    let next = terminal.and_then(|t| t.read(cx).cursor_blink(now).1);
                    if next.is_some() {
                        cx.notify();
                    }
                    next
                });
                let Ok(wait) = wait else {
                    return;
                };
                // a cursor move restarts the blink
                let wait = wait.unwrap_or(CURSOR_BLINK_INTERVAL).min(CURSOR_BLINK_INTERVAL);
                cx.background_executor().timer(wait + Duration::from_millis(5)).await;
            }
        })
        .detach();
    }

    pub fn tab(&self, id: u64) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == id)
    }

    fn tab_mut(&mut self, id: u64) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    pub fn activate_tab(&mut self, id: u64, cx: &mut Context<Self>) {
        if self.tab(id).is_some() {
            self.active = Some(id);
            cx.notify();
        }
    }

    pub fn resolve_tabs(&mut self, cx: &mut Context<Self>) {
        if self.tabs_resolving {
            self.tabs_again = true;
            return;
        }
        let shells: Vec<(u64, u32)> = self
            .tabs
            .iter()
            .filter(|t| matches!(t.kind, TabKind::Shell) && !t.exited)
            .filter_map(|t| Some((t.id, t.terminal.as_ref()?.read(cx).pid()?.as_u32())))
            .collect();
        if shells.is_empty() && self.tabs.iter().all(|t| t.claude.is_none()) {
            return;
        }
        self.tabs_resolving = true;
        let found = cx.background_executor().spawn(async move { tab_sessions(&shells) });
        cx.spawn(async move |this, cx| {
            let found = found.await;
            this.update(cx, |this, cx| {
                this.tabs_resolving = false;
                this.tab_resolves += 1;
                let mut changed = false;
                let mut unlisted = false;
                for tab in &mut this.tabs {
                    let now = found.get(&tab.id).cloned();
                    if tab.claude != now {
                        unlisted |= now.as_ref().is_some_and(|id| !this.sessions.iter().any(|s| &s.session_id == id));
                        tab.claude = now;
                        changed = true;
                    }
                }
                if unlisted {
                    this.poll_nudge.wake();
                }
                if changed {
                    cx.notify();
                }
                if std::mem::take(&mut this.tabs_again) {
                    this.resolve_tabs(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    pub fn tab_runs(tab: &Tab, session: &Session) -> bool {
        match &tab.kind {
            TabKind::Attach { session_id } => *session_id == session.id,
            TabKind::Shell => tab.claude.as_deref() == Some(session.session_id.as_str()),
            TabKind::Saved(saved) => saved.session().is_some_and(|s| s.session_id == session.session_id),
            TabKind::Work(_) => false,
        }
    }

    pub fn is_marked(&self, session: &Session) -> bool {
        self.active.and_then(|id| self.tab(id)).is_some_and(|t| Self::tab_runs(t, session))
    }

    pub fn session_tab(&self, session: &Session) -> Option<u64> {
        self.tabs.iter().find(|t| Self::tab_runs(t, session)).map(|t| t.id)
    }

    fn cycle_tab(&mut self, step: isize, cx: &mut Context<Self>) {
        let at = self.active.and_then(|id| self.tabs.iter().position(|t| t.id == id));
        if let Some(next) = foreman_ui::tabs::cycled(self.tabs.len(), at, step) {
            self.activate_tab(self.tabs[next].id, cx);
        }
    }

    pub fn has_keys(&self, window: &Window) -> bool {
        self.focus.is_focused(window)
    }

    pub fn attach(
        &mut self,
        workspace: &str,
        repo: &str,
        session: &Session,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> u64 {
        self.attach_titled(format!("{workspace} · {repo} · {}", fleet::display_name(session)), session, cx)
    }

    fn attach_titled(&mut self, title: String, session: &Session, cx: &mut Context<Self>) -> u64 {
        let shell = util::shell::Shell::WithArguments {
            program: "claude".into(),
            args: vec!["attach".into(), session.id.clone()],
            title_override: None,
        };
        let id = self.push_tab(title, TabKind::Attach { session_id: session.id.clone() }, Some(session.cwd.clone()), cx);
        self.start_terminal(id, shell, Some(session.cwd.clone()), cx);
        if let Some(tab) = self.tab_mut(id) {
            tab.launched_session = Some(session.session_id.clone());
            tab.session_name = Some(session.name.clone());
        }
        id
    }

    pub fn open_work(&mut self, workspace: &str, cx: &mut Context<Self>) -> u64 {
        let open = self.tabs.iter().find(|t| matches!(&t.kind, TabKind::Work(v) if v.read(cx).workspace == workspace));
        if let Some(id) = open.map(|t| t.id) {
            self.active = Some(id);
            cx.notify();
            return id;
        }
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        let fleet = cx.weak_entity();
        let name = workspace.to_string();
        let link = FleetLink {
            live: self.work_live.clone(),
            attach: Rc::new(move |id, window, cx| {
                fleet.update(cx, |fleet, cx| fleet.attach_by_id(&name, id, window, cx)).ok();
            }),
        };
        let view = WorkView::open(workspace, self.home.clone(), self.hits.clone(), link, cx);
        let cwd = self.workspaces.iter().find(|w| w.name == workspace).and_then(|w| w.home()).map(Path::to_path_buf);
        self.tabs.push(Tab::new(id, format!("Work · {workspace}"), TabKind::Work(view), cwd));
        self.active = Some(id);
        cx.notify();
        id
    }

    pub fn open_work_item(&mut self, workspace: &str, item_id: &str, cx: &mut Context<Self>) -> u64 {
        let id = self.open_work(workspace, cx);
        if let Some(Tab { kind: TabKind::Work(view), .. }) = self.tab(id) {
            let (view, item) = (view.clone(), item_id.to_string());
            view.update(cx, |view, cx| view.open_from_inbox(item, cx));
        }
        id
    }

    pub(crate) fn tab_card(&self, tab: &Tab, title: String) -> tab_card::Card {
        let (workspace, repo) = tab.cwd.as_deref().map(|cwd| tab_card::place(cwd, &self.workspaces)).unwrap_or_default();
        let attached = match &tab.kind {
            TabKind::Attach { session_id } => Some(self.sessions.iter().find(|s| &s.id == session_id).ok_or(session_id)),
            _ => None,
        };
        let (session, started) = match attached {
            Some(Ok(s)) => (Some(tab_card::session_label(&s.name, &s.session_id)), s.started_at.or(Some(tab.opened_at))),
            Some(Err(id)) => (Some(id.clone()), Some(tab.opened_at)),
            None => (tab.session.clone(), Some(tab.opened_at)),
        };
        let kind = match &tab.kind {
            TabKind::Attach { .. } => Some(tab_card::Kind::Attach),
            TabKind::Shell if tab.hand => Some(tab_card::Kind::Hand),
            TabKind::Shell => Some(tab_card::Kind::Shell),
            TabKind::Work(_) => None,
            TabKind::Saved(saved) => reopen::kind_of(saved),
        };
        tab_card::Card {
            title,
            workspace,
            repo,
            folder: tab.cwd.as_ref().map(|c| c.display().to_string()),
            branch: None,
            session,
            kind,
            workstate: tab.workstate.clone().map(|w| match &tab.restart_to {
                Some(to) => format!("{w} · restart to use {to}"),
                None => w,
            }),
            started,
        }
    }

    pub fn open_shell_in(&mut self, cwd: PathBuf, title: String, window: &mut Window, cx: &mut Context<Self>) -> u64 {
        self.open_tab(title, TabKind::Shell, pwsh(self.shell_args.clone()), Some(cwd), window, cx)
    }

    pub fn new_shell(&mut self, window: &mut Window, cx: &mut Context<Self>) -> u64 {
        let title = format!("pwsh {}", self.next_tab_id);
        let cwd = self.new_shell_cwd();
        self.open_tab(title, TabKind::Shell, pwsh(self.shell_args.clone()), cwd, window, cx)
    }

    pub fn new_shell_cwd(&self) -> Option<PathBuf> {
        let active = self.active.and_then(|id| self.tab(id)).and_then(|t| t.cwd.as_deref());
        fleet::shell_cwd(active, &self.workspaces, foreman_core::paths::user_home())
    }

    fn open_tab(
        &mut self,
        title: String,
        kind: TabKind,
        shell: util::shell::Shell,
        cwd: Option<PathBuf>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> u64 {
        let id = self.push_tab(title, kind, cwd.clone(), cx);
        self.start_terminal(id, shell, cwd, cx);
        id
    }

    fn push_tab(&mut self, title: String, kind: TabKind, cwd: Option<PathBuf>, cx: &mut Context<Self>) -> u64 {
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        self.tabs.push(Tab::new(id, title, kind, cwd));
        self.active = Some(id);
        cx.notify();
        id
    }

    fn start_terminal(&mut self, id: u64, shell: util::shell::Shell, cwd: Option<PathBuf>, cx: &mut Context<Self>) {
        let build = TerminalBuilder::new(
            cwd,
            TerminalMode::interactive(),
            shell,
            HashMap::default(),
            CursorShape::Bar,
            AlternateScroll::On,
            None,
            Vec::new(),
            Duration::ZERO,
            false,
            self.window.window_id().as_u64(),
            cx,
            Vec::new(),
            PathStyle::local(),
        );
        cx.spawn(async move |this, cx| {
            let built = build.await;
            this.update(cx, |this, cx| {
                match built {
                    Ok(builder) => {
                        let terminal = cx.new(|cx| builder.subscribe(cx));
                        cx.subscribe(&terminal, move |this, emitter, event, cx| {
                            match event {
                                TerminalEvent::Wakeup => cx.notify(),
                                // The foreground process changed (pty_info.rs); the OSC title is BreadcrumbsChanged.
                                TerminalEvent::TitleChanged => {
                                    this.resolve_tabs(cx);
                                    cx.notify();
                                }
                                TerminalEvent::BreadcrumbsChanged => {
                                    let title = emitter.read(cx).breadcrumb_text.clone();
                                    if let Some(tab) = this.tab_mut(id) {
                                        tab.program_title = foreman_ui::tabs::sanitize(&title);
                                    }
                                    this.resolve_tabs(cx);
                                    cx.notify();
                                }
                                TerminalEvent::CloseTerminal => {
                                    if let Some(tab) = this.tab_mut(id) {
                                        tab.exited = true;
                                        tab.claude = None;
                                    }
                                    cx.notify();
                                }
                                TerminalEvent::Open(target) => {
                                    open_link(target, |url| cx.open_url(url), |path| cx.open_with_system(path))
                                }
                                _ => return,
                            }
                            if let Some(tab) = this.tab(id) {
                                tab.wake_watchers();
                            }
                        })
                        .detach();
                        match this.tab_mut(id) {
                            Some(tab) => {
                                tab.terminal = Some(terminal);
                                if tab.detach == Detach::Pending {
                                    this.start_detach(id, cx);
                                }
                            }
                            None => terminal.update(cx, |t, _| t.release_pty_resources()),
                        }
                    }
                    Err(error) => {
                        if let Some(tab) = this.tab_mut(id) {
                            tab.error = Some(format!("{error:#}"));
                            if tab.detach == Detach::Pending {
                                this.remove_tab(id, cx);
                            }
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub fn close_tab(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(tab) = self.tab_mut(id) else {
            return;
        };
        let attach = matches!(tab.kind, TabKind::Attach { .. });
        let started = tab.terminal.is_some();
        if !attach || tab.exited || (!started && tab.error.is_some()) {
            self.remove_tab(id, cx);
            return;
        }
        match tab.detach {
            Detach::Detaching | Detach::Pending => {}
            Detach::None | Detach::Failed if !started => {
                tab.detach = Detach::Pending;
                cx.notify();
            }
            Detach::None | Detach::Failed => self.start_detach(id, cx),
        }
    }

    fn start_detach(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(tab) = self.tab_mut(id) else {
            return;
        };
        let Some(terminal) = tab.terminal.clone() else {
            return;
        };
        tab.detach = Detach::Detaching;
        let (watch, mut changed) = nudge::nudge();
        tab.watchers.push(watch);
        let pid = terminal.update(cx, |t, _| {
            t.input(DETACH_INPUT);
            t.pid().map(|p| p.as_u32())
        });
        cx.notify();
        cx.spawn(async move |this, cx| {
            let executor = cx.background_executor().clone();
            let deadline = Instant::now() + DETACH_WITHIN;
            let mut ended = std::pin::pin!(any_exit(&executor, pid.into_iter().collect(), DETACH_WITHIN).fuse());
            loop {
                let exited = this
                    .read_with(cx, |this, _| this.tab(id).is_none_or(|t| t.exited))
                    .unwrap_or(true);
                if exited || !pid.is_some_and(foreman_core::process::alive) {
                    this.update(cx, |this, cx| this.remove_tab(id, cx)).ok();
                    return;
                }
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    this.update(cx, |this, cx| {
                        if let Some(tab) = this.tab_mut(id) {
                            tab.detach = Detach::Failed;
                        }
                        this.quitting = false;
                        this.quit_asked = false;
                        cx.notify();
                    })
                    .ok();
                    return;
                }
                futures::future::select(std::pin::pin!(changed.wait(&executor, left)), ended.as_mut()).await;
            }
        })
        .detach();
    }

    fn remove_tab(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.tabs.iter().position(|t| t.id == id) else {
            return;
        };
        let tab = self.tabs.remove(index);
        tab.wake_watchers();
        if let Some(terminal) = tab.terminal {
            terminal.update(cx, |t, _| t.release_pty_resources());
        }
        if self.active == Some(id) {
            self.active = self
                .tabs
                .get(index.min(self.tabs.len().saturating_sub(1)))
                .map(|t| t.id);
        }
        if self.quit_step(cx) {
            self.close_for_quit(cx);
        }
        cx.notify();
    }

    fn quit_step(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.quitting || self.has_attach_tabs() {
            return false;
        }
        let Some(hands) = self.hands.clone().filter(|_| !self.hands_released) else {
            return true;
        };
        if self.quit_release.is_none() {
            let release = cx.background_executor().spawn(async move { crate::hand::release_all_within(hands, QUIT_RELEASE_WITHIN) });
            self.quit_release = Some(cx.spawn(async move |this, cx| {
                for notice in release.await.unwrap_or_default() {
                    log::warn!("{notice}");
                }
                this.update(cx, |this, cx| {
                    this.hands_released = true;
                    this.quit_release = None;
                    if this.quit_step(cx) {
                        this.close_for_quit(cx);
                    }
                })
                .ok();
            }));
        }
        false
    }

    fn close_for_quit(&mut self, cx: &mut Context<Self>) {
        self.quitting = false;
        self.close_settings(cx);
        self.window.update(cx, |_, window, cx| crate::tray::close(window, cx)).ok();
    }

    fn has_attach_tabs(&self) -> bool {
        self.tabs.iter().any(|t| matches!(t.kind, TabKind::Attach { .. }))
    }

    pub fn restart_elevated(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(home) = self.home.clone() else {
            return;
        };
        if let Err(e) = Settings::update(&home, |settings| settings.sessions.run_as_admin = true) {
            self.action_error = Some(format!("Cannot save Run Foreman as administrator: {e}"));
            cx.notify();
            return;
        }
        match crate::elevate::relaunch(&home, Some(std::process::id())) {
            Ok(()) => self.quit(window, cx),
            Err(e) => {
                self.action_error = Some(format!("Foreman stays unelevated: {e:#}"));
                cx.notify();
            }
        }
    }

    pub fn quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.quit_asked = true;
        if self.should_close(window, cx) {
            crate::tray::close(window, cx);
        }
    }

    fn should_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.quit_asked {
            self.close_settings(cx);
            crate::tray::hide(window, cx);
            return false;
        }
        self.quitting = true;
        let ids: Vec<u64> = self
            .tabs
            .iter()
            .filter(|t| matches!(t.kind, TabKind::Attach { .. }))
            .map(|t| t.id)
            .collect();
        for id in ids {
            self.close_tab(id, cx);
        }
        let close = self.quitting && self.quit_step(cx);
        if close {
            self.quitting = false;
            self.close_settings(cx);
        }
        close
    }

    fn toggle(&mut self, key: String, cx: &mut Context<Self>) {
        if !self.collapsed.remove(&key) {
            self.collapsed.insert(key);
        }
        cx.notify();
    }

    pub fn input_focused(&self, window: &Window, cx: &gpui::App) -> bool {
        self.search.read(cx).is_focused(window)
            || self.adding.as_ref().is_some_and(|d| d.name.read(cx).is_focused(window))
            || self.editing.as_ref().is_some_and(|e| e.path.read(cx).is_focused(window))
            || self.compose.as_ref().is_some_and(|c| c.input.read(cx).is_focused(window))
            || self.tabs.iter().filter_map(|t| t.find.as_ref()).any(|f| f.input.read(cx).is_focused(window))
    }

    pub fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search.update(cx, |s, cx| s.focus_all(window, cx));
    }

    pub fn search_query(&self, cx: &gpui::App) -> String {
        self.search.read(cx).text().trim().to_string()
    }

    fn on_search_event(&mut self, _: &Entity<TextInput>, event: &InputEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            InputEvent::Edited => cx.notify(),
            InputEvent::Cancel if self.dismiss_popup() => cx.notify(),
            InputEvent::Cancel if !self.search.read(cx).text().is_empty() => self.clear_search(cx),
            InputEvent::Cancel => self.focus.focus(window, cx),
            InputEvent::Confirm | InputEvent::ConfirmBack | InputEvent::Refused(_) => {}
        }
    }

    fn dismiss_popup(&mut self) -> bool {
        self.menu.take().is_some() || self.confirm.take().is_some()
    }

    pub fn clear_search(&mut self, cx: &mut Context<Self>) {
        self.search.update(cx, |s, cx| s.set_text("", cx));
        cx.notify();
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &event.keystroke;
        if k.key == "," && k.modifiers.control && !k.modifiers.alt && !k.modifiers.shift {
            self.open_settings(cx);
            return;
        }
        // Left unhandled: Windows turns a key into its character (WM_CHAR) only after nothing took the key down.
        if self.input_focused(window, cx) {
            return;
        }
        if k.key == "escape" && self.dismiss_popup() {
            cx.notify();
            cx.stop_propagation();
            return;
        }
        // the review keeps Enter and Esc after a click on one of its repos took the keys from its name
        if self.adding.is_some() && !k.modifiers.modified() && matches!(k.key.as_str(), "enter" | "escape") {
            if k.key == "enter" {
                self.save_add(window, cx);
            } else {
                self.adding = None;
            }
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if self.editing.is_some() && !k.modifiers.modified() && k.key == "escape" {
            self.editing = None;
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if k.key == "escape"
            && let Some(Tab { kind: TabKind::Work(work), .. }) = self.active.and_then(|id| self.tab(id))
            && work.update(cx, |work, cx| work.escape(cx))
        {
            cx.stop_propagation();
            return;
        }
        let terminal = self.active.and_then(|id| self.tab(id)).and_then(|t| t.terminal.clone());
        if k.key == "f5" && !k.modifiers.modified() && terminal.is_none() {
            self.refresh_now(cx);
            cx.stop_propagation();
            return;
        }
        let Some(terminal) = terminal else {
            return;
        };
        if let Some(id) = self.active
            && self.terminal_key(id, &terminal, k, window, cx)
        {
            cx.stop_propagation();
            return;
        }
        terminal.update(cx, |terminal, _| {
            if !terminal.try_keystroke(&event.keystroke, false)
                && let Some(text) = &event.keystroke.key_char
            {
                terminal.input(text.clone().into_bytes());
            }
        });
        // the cursor is lit at once after a key, also when the program echoes nothing
        cx.notify();
        cx.stop_propagation();
    }
}

fn open_link(target: &MaybeNavigationTarget, open_url: impl FnOnce(&str), open_path: impl FnOnce(&Path)) {
    match target {
        MaybeNavigationTarget::Url(url) => open_url(url),
        MaybeNavigationTarget::PathLike(link) => {
            let path = Path::new(&link.maybe_path);
            match &link.working_directory {
                Some(cwd) if path.is_relative() => open_path(&cwd.join(path)),
                _ => open_path(path),
            }
        }
    }
}

fn any_exit(executor: &gpui::BackgroundExecutor, pids: Vec<u32>, timeout: Duration) -> impl Future<Output = ()> + use<> {
    let exited = (!pids.is_empty()).then(|| executor.spawn(async move { foreman_core::process::wait_any_exit(&pids, timeout) }));
    async move {
        let exited = match exited {
            Some(task) => task.await,
            None => false,
        };
        if !exited {
            futures::future::pending::<()>().await;
        }
    }
}

fn pwsh(args: Vec<String>) -> util::shell::Shell {
    util::shell::Shell::WithArguments { program: "pwsh".into(), args, title_override: None }
}

fn stale_tabs(tabs: &[Tab], elsewhere: &[(String, Option<String>)]) -> Vec<u64> {
    tabs.iter()
        .filter(|t| t.hand && !t.exited)
        .filter(|t| t.hand_key.as_ref().is_some_and(|k| elsewhere.iter().any(|(n, _)| n == k)))
        .map(|t| t.id)
        .collect()
}

fn held_line(why: &str) -> String {
    format!("Write-Host -ForegroundColor Yellow '{}'", why.replace('\'', "''"))
}

impl FleetView {
    fn render_right(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let headers = self
            .tabs
            .iter()
            .map(|tab| {
                let title = tab.label();
                let card = self.tab_card(tab, title.clone());
                let cwd = tab.cwd.clone();
                TabHeader {
                    id: tab.id,
                    label: crate::clipped::clipped_with(format!("tab-title-{}", tab.id), title.clone(), false, false).max_w(px(280.)).into_any_element(),
                    title: title.into(),
                    tooltip: Some(Box::new(hover_card::tooltip(move || {
                        tab_card::Card { branch: cwd.as_deref().and_then(foreman_core::branch_at), ..card.clone() }.hover_card()
                    }))),
                }
            })
            .collect();
        let muted = colors.text_muted;
        let mut bar = tab_strip(
            &self.hits,
            headers,
            self.active,
            || icon::Icon::Close.svg(14., muted),
            |this: &mut Self, id, cx| this.activate_tab(id, cx),
            |this: &mut Self, id, cx| this.close_tab(id, cx),
            |this: &mut Self, from, to, cx| {
                foreman_ui::tabs::reorder(&mut this.tabs, from, to);
                cx.notify();
            },
            cx,
        );
        bar = bar.child(
            button::ghost(&self.hits, "new-shell", cx)
                .flex()
                .items_center()
                .justify_center()
                .h_full()
                .w(px(36.))
                .child(icon::Icon::Plus.svg(16., colors.text_muted).group_hover("new-shell", |s| s.text_color(colors.text)))
                .tooltip(icon::Tooltip::text("New shell"))
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.new_shell(window, cx);
                })),
        );

        let active = self.active.and_then(|id| self.tab(id));
        let status = cx.theme().status().clone();
        let content: AnyElement = match active {
            Some(Tab { kind: TabKind::Work(view), .. }) => view.clone().into_any_element(),
            Some(Tab { id: tab_id, terminal: Some(terminal), detach, find, .. }) => div()
                .size_full()
                .flex()
                .flex_col()
                .bg(colors.terminal_background)
                .when(*detach == Detach::Failed, |d| {
                    d.child(
                        div()
                            .p_2()
                            .bg(status.error_background)
                            .text_sm()
                            .text_color(status.error)
                            .child(SharedString::from(format!(
                                "Detach failed: claude attach did not exit within {} s of Ctrl+Z. Nothing was killed and the session was not touched. Close the tab to try again.",
                                DETACH_WITHIN.as_secs()
                            ))),
                    )
                })
                .when_some(find.as_ref(), |d, find| d.child(self.find_bar(*tab_id, find, cx)))
                .child(
                    div()
                        .relative()
                        .flex_1()
                        .min_h_0()
                        .child(hit_target(&self.hits, format!("terminal-{tab_id}")))
                        .on_scroll({
                            // as Zed's TerminalView::scroll_wheel
                            let terminal = terminal.clone();
                            cx.listener(move |this, e: &ScrollEvent, _, cx| {
                                if e.modifiers.control {
                                    this.wheel_font_size(e.delta, cx);
                                    return;
                                }
                                terminal.update(cx, |terminal, _| {
                                    terminal.scroll_wheel(e, SCROLL_MULTIPLIER, MouseInputMode::ReportToTerminal);
                                });
                                // the queued Scroll is applied by the grid's prepaint sync
                                cx.notify();
                            })
                        })
                        .child(term_element::terminal_grid(terminal.clone(), self.term_font()))
                        .child(self.terminal_drop(*tab_id, cx)),
                )
                .into_any_element(),
            Some(Tab { id, kind: TabKind::Saved(saved), .. }) => self.saved_placeholder(*id, saved, cx),
            Some(Tab { error: Some(error), .. }) => placeholder(format!("Cannot start the terminal: {error}"), cx),
            Some(_) => placeholder("starting…".into(), cx),
            None => placeholder(
                "Click [attach] on a live session to open it here, or + for a shell.".into(),
                cx,
            ),
        };

        div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .min_w_0()
            .child(bar)
            .child(div().relative().flex_1().min_h_0().child(content).children(self.pane_toast.render(&self.hits, "pane-toast", cx)))
            .into_any_element()
    }
}

fn placeholder(text: String, cx: &gpui::App) -> AnyElement {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(cx.theme().colors().editor_background)
        .text_color(cx.theme().colors().text_muted)
        .child(SharedString::from(text))
        .into_any_element()
}

impl Render for FleetView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.probe.on_render(window);
        if !self.painted_sessions && self.sessions_read {
            self.painted_sessions = true;
            timeline("first paint with sessions", Duration::ZERO);
        }
        crate::selftest::injected_frame_sleep();
        self.persist_tabs(cx);
        self.hits.borrow_mut().clear();
        let colors = cx.theme().colors().clone();
        div()
            .size_full()
            .flex()
            .font_family(foreman_ui::fonts::UI)
            .bg(colors.background)
            .text_color(colors.text)
            .track_focus(&self.focus)
            .key_context(KEY_CONTEXT)
            .on_action(cx.listener(|this, _: &NextTab, _, cx| this.cycle_tab(1, cx)))
            .on_action(cx.listener(|this, _: &PreviousTab, _, cx| this.cycle_tab(-1, cx)))
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| this.focus_search(window, cx)))
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, window, cx| {
                    if !this.focus.is_focused(window) {
                        this.focus.focus(window, cx);
                    }
                }),
            )
            .flex_col()
            .child(crate::titlebar::render(
                "Foreman",
                Some(
                    crate::titlebar::icon_button("open-settings", "\u{e713}", cx)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.open_settings(cx)))
                        .into_any_element(),
                ),
                window,
                cx,
            ))
            .child(
                SplitPane { id: "divider".into(), min_first: MIN_LEFT_WIDTH, min_second: MIN_RIGHT_WIDTH, default: DEFAULT_LEFT_WIDTH }
                    .render(
                        self.render_left(cx),
                        self.render_right(cx),
                        |this: &mut Self, width, cx| {
                            this.left_width = width;
                            cx.notify();
                        },
                        |this: &mut Self, _| this.save_left_width(),
                        cx,
                    )
                    .flex_1()
                    .min_h_0(),
            )
            .children(self.render_menu(cx))
            .child(self.probe.paint_marker())
    }
}

fn tab_sessions(shells: &[(u64, u32)]) -> StdHashMap<u64, String> {
    use foreman_claude::{peer, proc_tree};
    let Some(dir) = peer::sessions_dir().filter(|_| !shells.is_empty()) else {
        return StdHashMap::new();
    };
    let procs = proc_tree::snapshot();
    shells
        .iter()
        .filter_map(|&(tab, pid)| {
            let entry = proc_tree::session_under(&procs, pid, peer::started(pid), &dir, &peer::System)?;
            Some((tab, entry.session_id?))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use foreman_term::PathLikeTarget;

    use super::*;

    fn opened(target: &MaybeNavigationTarget) -> (Option<String>, Option<PathBuf>) {
        let (mut url, mut path) = (None, None);
        open_link(target, |u| url = Some(u.to_owned()), |p| path = Some(p.to_owned()));
        (url, path)
    }

    #[test]
    fn an_open_event_opens_any_scheme_and_resolves_relative_paths() {
        for uri in ["https://example.com/", "ftp://host/x", "vscode://file/c:/x", "mailto:a@b.c"] {
            assert_eq!(opened(&MaybeNavigationTarget::Url(uri.into())), (Some(uri.into()), None));
        }
        let relative = PathLikeTarget { maybe_path: "src/x.rs".into(), working_directory: Some(PathBuf::from("C:/repo")) };
        assert_eq!(opened(&MaybeNavigationTarget::PathLike(relative)), (None, Some(PathBuf::from("C:/repo").join("src/x.rs"))));
        let absolute = PathLikeTarget { maybe_path: "C:/tmp/a.txt".into(), working_directory: Some(PathBuf::from("C:/repo")) };
        assert_eq!(opened(&MaybeNavigationTarget::PathLike(absolute)), (None, Some(PathBuf::from("C:/tmp/a.txt"))));
    }

    #[test]
    fn a_tab_runs_its_attached_id_or_the_claude_found_under_its_shell() {
        let session = Session {
            id: "abcd0001".into(),
            session_id: "abcd0001-0000".into(),
            name: "s".into(),
            cwd: PathBuf::from(r"C:\demo"),
            kind: foreman_claude::Kind::Interactive,
            state: None,
            status: None,
            waiting_for: None,
            pid: Some(5),
            started_at: None,
        };
        let tab = |kind, claude: Option<&str>| Tab { claude: claude.map(String::from), ..Tab::new(1, "t".into(), kind, None) };
        assert!(FleetView::tab_runs(&tab(TabKind::Attach { session_id: "abcd0001".into() }, None), &session));
        assert!(!FleetView::tab_runs(&tab(TabKind::Attach { session_id: "ffff0001".into() }, None), &session));
        assert!(FleetView::tab_runs(&tab(TabKind::Shell, Some("abcd0001-0000")), &session));
        assert!(!FleetView::tab_runs(&tab(TabKind::Shell, Some("abcd0001-ffff")), &session), "/clear gave the tab a new session");
        let placeholder = |sid: &str| {
            let saved = crate::saved_tabs::SavedTab::Claude { title: "c".into(), cwd: r"C:\demo".into(), session_id: Some(sid.into()), name: "s".into() };
            tab(TabKind::Saved(saved), None)
        };
        let full = Session { session_id: "abcd0001-0000-4000-8000-000000000001".into(), ..session.clone() };
        assert!(FleetView::tab_runs(&placeholder(&full.session_id), &full), "the row opens its placeholder, not a second tab");
        assert!(!FleetView::tab_runs(&placeholder("abcd0001-0000-4000-8000-000000000002"), &full));
        assert!(!FleetView::tab_runs(&tab(TabKind::Shell, None), &session), "a shell without claude runs nothing");
    }

    #[test]
    fn a_corrupt_workspace_file_is_skipped_and_named() {
        let home = tempfile::tempdir().unwrap();
        let registry = Registry::at(home.path());
        registry.save(&Workspace::new("alpha", r"C:\demo\alpha", Vec::new()), false).unwrap();
        let broken = registry.save(&Workspace::new("beta", r"C:\demo\beta", Vec::new()), false).unwrap();
        std::fs::write(&broken, "name = [").unwrap();
        let (workspaces, skipped) = load_registry(Ok(registry)).unwrap();
        assert_eq!(workspaces.iter().map(|w| w.name.as_str()).collect::<Vec<_>>(), ["alpha"]);
        assert_eq!(skipped.len(), 1);
        assert!(skipped[0].contains("beta"), "{skipped:?}");
        assert_eq!(load_registry(Err("no Foreman home".into())), Err("no Foreman home".to_string()));
    }

    #[test]
    fn only_open_hand_tabs_holding_a_token_from_elsewhere_restart() {
        let tab = |id, hand, key: Option<&str>, exited| Tab {
            hand,
            hand_key: key.map(String::from),
            exited,
            ..Tab::new(id, format!("t{id}"), TabKind::Shell, None)
        };
        let tabs = [
            tab(1, true, Some("hand-a-1"), false),
            tab(2, true, Some("hand-a-2"), false),
            tab(3, true, Some("hand-a-3"), true),
            tab(4, false, Some("hand-a-4"), false),
            tab(5, true, None, false),
        ];
        let elsewhere = [
            ("hand-a-1".to_string(), Some("https://old.example".to_string())),
            ("hand-a-3".to_string(), None),
            ("hand-a-4".to_string(), None),
        ];
        assert_eq!(stale_tabs(&tabs, &elsewhere), [1], "2 is current, 3 exited, 4 is no hand tab, 5 runs plain");
        assert!(stale_tabs(&tabs, &[]).is_empty());
    }

    #[test]
    fn only_a_background_session_waits_out_the_grace_before_resume() {
        let session = |kind, name: &str| Session {
            id: "abcd0001".into(),
            session_id: "abcd0001-0000".into(),
            name: name.into(),
            cwd: PathBuf::from(r"C:\demo"),
            kind,
            state: Some(foreman_claude::SessionState::Stopped),
            status: None,
            waiting_for: None,
            pid: None,
            started_at: None,
        };
        let (gone, live) = (foreman_claude::Health::Stopped, foreman_claude::Health::Idle);
        let now = 1_000_000;
        let just_seen = Some(now - 1_000);
        let interactive = session(foreman_claude::Kind::Interactive, "3cx-bug-fixer");
        assert!(offers_resume(&interactive, &gone, just_seen, now), "closed a second ago, never respawned: at once");
        assert!(offers_resume(&interactive, &gone, None, now), "only ever seen in its transcript");
        assert!(!offers_resume(&interactive, &live, just_seen, now), "still running");
        let bg = session(foreman_claude::Kind::Background, "worker");
        assert!(!offers_resume(&bg, &gone, just_seen, now), "the daemon may still respawn it");
        assert!(!offers_resume(&bg, &gone, Some(now - RESUME_GRACE_MS), now), "the grace's last millisecond");
        assert!(offers_resume(&bg, &gone, Some(now - RESUME_GRACE_MS - 1), now), "past the grace");
        assert!(offers_resume(&bg, &gone, None, now), "never seen alive here");
        assert!(!offers_resume(&session(foreman_claude::Kind::Interactive, "fm-alpha-workspace"), &gone, None, now), "the supervisor's");
    }
}

fn save_workstate_url(home: &std::path::Path, origin: &str) -> Result<(), String> {
    Settings::update(home, |settings| settings.workstate.url = Some(origin.to_string()))
        .map(drop)
        .map_err(|e| format!("Cannot save settings.toml: {e}"))
}
