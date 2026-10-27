//! The window: fleet tree on the left, terminal tabs on the right.

use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap as StdHashMap, HashSet},
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use collections::HashMap;
use foreman_claude::{ClaudeVersion, Sent, Session, Transcripts, check_supported, list_sessions, main_repo, now_ms, send_from_human};
use foreman_core::{
    DEFAULT_MAX_WORKING, Event, EventReader, OutboxMessage, Workspace, Registry, Settings, mark_answered,
};
use foreman_supervisor::{DesiredState, NeedsYou, workspace_session_name};
use foreman_term::{
    Event as TerminalEvent, MouseInputMode, Terminal, TerminalBuilder, TerminalMode,
    terminal_settings::{AlternateScroll, CURSOR_BLINK_INTERVAL, CursorShape},
};
use foreman_ui::{FrameProbe, term_element};
use gpui::{
    AnyElement, AnyWindowHandle, Bounds, ClickEvent, Context, CursorStyle, DragMoveEvent, Entity, FocusHandle,
    KeyBinding, KeyDownEvent, MouseButton, MouseDownEvent, Pixels, Render, ScrollEvent, SharedString, Window,
    WindowHandle, canvas, div, prelude::*, px,
};
use theme::ActiveTheme as _;
use util::paths::PathStyle;

use crate::compose::{self, Compose, KeyOutcome};
use crate::fleet::{self, HookSignal, SessionRow, Supervised, Tree};
use crate::hand::{Hands, Launch};
use crate::settings_window::{self, SettingsView};
use crate::stars::Stars;
use crate::supervise::{self, RelayStatus, Report, Supervision};
use crate::tab_card;
use crate::text_input::{InputEvent, TextInput};
use crate::work_view::{FleetLink, LiveSession, WorkHub, WorkInbox, WorkView};
use crate::workstate_setup::{self, WsStatus};

pub(crate) mod icon;
mod left;
mod manage;
mod menu;
mod results;
mod term;

pub use manage::{AddDraft, Removing};
pub use menu::{Confirm, ContextMenu, MenuTarget};

gpui::actions!(foreman, [NextTab, PreviousTab, FocusSearch]);

const KEY_CONTEXT: &str = "FleetView";

/// The tab bar's hotkeys. Bindings are matched before [`FleetView::key_down`] runs, so a terminal tab
/// never sees these chords; everything else (Tab, Shift+Tab, PgUp, Ctrl+V…) still reaches it.
pub fn bind_keys(cx: &mut gpui::App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-tab", NextTab, Some(KEY_CONTEXT)),
        KeyBinding::new("ctrl-pagedown", NextTab, Some(KEY_CONTEXT)),
        KeyBinding::new("ctrl-shift-tab", PreviousTab, Some(KEY_CONTEXT)),
        KeyBinding::new("ctrl-pageup", PreviousTab, Some(KEY_CONTEXT)),
        KeyBinding::new("ctrl-shift-e", FocusSearch, Some(KEY_CONTEXT)),
    ]);
}

/// The index `step` tabs away from `at`, wrapping; with none active, the first (forward) or last.
fn cycled(len: usize, at: Option<usize>, step: isize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let at = at.map_or(if step > 0 { -1 } else { 0 }, |at| at as isize);
    Some((at + step).rem_euclid(len as isize) as usize)
}

pub const REFRESH_EVERY: Duration = Duration::from_secs(2);
/// How often the transcripts of closed sessions are looked at again (m5/27); sooner when nudged.
pub const TRANSCRIPTS_EVERY: Duration = Duration::from_secs(10);
/// Where closed sessions' transcripts are read instead of `CLAUDE_CONFIG_DIR` / `~/.claude` (the selftest's fake).
pub const CLAUDE_DIR_VAR: &str = "FOREMAN_CLAUDE_DIR";
/// What closing an Attach tab types into `claude attach`: Ctrl+Z, its own "drop back to your
/// shell", which ends the attach process and leaves the session running (LEARNINGS m1/7).
pub const DETACH_INPUT: &[u8] = b"\x1a";
pub const DETACH_WITHIN: Duration = Duration::from_secs(5);
/// Zed's `terminal.scroll_multiplier` default; one Windows wheel notch is already 3 lines.
const SCROLL_MULTIPLIER: f32 = 1.0;
const DEFAULT_LEFT_WIDTH: Pixels = px(460.);
const MIN_LEFT_WIDTH: Pixels = px(280.);
const MIN_RIGHT_WIDTH: Pixels = px(320.);
/// A background session the daemon may still respawn by itself (it does ~12 s after a death mid-turn) is
/// not offered for resume: two resumes of one transcript run as two processes (m2/4, R13). An interactive
/// one is never respawned, so it is offered once its process is gone (m5/28).
const RESUME_GRACE_MS: i64 = 30_000;
/// The concurrency stepper's range; 0 pauses every launch and resume.
pub const MAX_WORKING_LIMIT: usize = 20;
/// The Messages section shows this many, newest first; a body shows this many lines until clicked.
const MESSAGES_LISTED: usize = 10;
const MESSAGE_LINES: usize = 3;

/// The Inbox and Messages sections take at most this share of the left panel each and scroll inside
/// it, so the workspace tree below them keeps room and stays reachable.
const INBOX_MAX_SHARE: f32 = 0.25;
const MESSAGES_MAX_SHARE: f32 = 0.3;

/// Where a clickable thing was painted, and the part of the window it can be seen in: its clip, i.e.
/// the visible area of the scroll container around it (the whole window when there is none).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    pub bounds: Bounds<Pixels>,
    pub visible: Bounds<Pixels>,
}

impl Hit {
    pub fn fully_visible(&self) -> bool {
        let (b, v) = (self.bounds, self.visible);
        b.left() >= v.left() && b.top() >= v.top() && b.right() <= v.right() && b.bottom() <= v.bottom()
    }
}

/// [`Hit`]s keyed by the same string as their element id, as of the last painted frame. The selftest
/// clicks through these.
pub type HitTargets = Rc<RefCell<StdHashMap<String, Hit>>>;

pub enum Cli {
    Checking,
    Supported(ClaudeVersion),
    Failed(String),
}

pub enum TabKind {
    Attach { session_id: String },
    Shell,
    /// The workspace's WorkState project ([`crate::work_view`]); no terminal.
    Work(Entity<WorkView>),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Detach {
    None,
    /// Close was asked for before the terminal started; the detach runs once it has.
    Pending,
    Detaching,
    /// `claude attach` did not exit within [`DETACH_WITHIN`]. The tab and its process are left as
    /// they are: nothing is killed that could hold the session.
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
    /// The folder it opened in (a Work tab: its workspace's), for [`fleet::shell_cwd`].
    pub cwd: Option<PathBuf>,
    /// Ctrl+=/Ctrl+-/Ctrl+0 and Ctrl+wheel, per tab as in Windows Terminal.
    pub font_size: Pixels,
    pub find: Option<term::Find>,
    /// What its program named it with OSC 0/2 ([`tab_card::sanitize`]d); the label while there is one.
    pub program_title: Option<String>,
    /// The claude session it runs, as its card names it, when the tab was opened for one.
    pub session: Option<String>,
    /// It runs `claude` from `+` or a resume in the tab.
    pub hand: bool,
    /// Epoch milliseconds.
    pub opened_at: i64,
}

impl Tab {
    /// What its tab shows: the program's title, else its own, and how it ended.
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
    pub sessions: Vec<Session>,
    /// [`fleet::main_repos`] of `sessions`, so rendering never walks the filesystem.
    main_repos: StdHashMap<PathBuf, Option<PathBuf>>,
    pub sessions_error: Option<String>,
    pub last_refresh_took: Option<Duration>,
    pub refreshes: u64,
    pub show_stopped: bool,
    collapsed: HashSet<String>,
    pub tabs: Vec<Tab>,
    pub active: Option<u64>,
    next_tab_id: u64,
    pub shell_args: Vec<String>,
    focus: FocusHandle,
    pub probe: FrameProbe,
    pub hits: HitTargets,
    window: AnyWindowHandle,
    /// The window was asked to close while Attach tabs were open; it closes once they detached.
    quitting: bool,
    /// Quit (the tray) was asked for: the window closes and the app with it. Otherwise closing the
    /// window only hides it (m5/1).
    quit_asked: bool,
    /// `FOREMAN_HOME` (else `%APPDATA%\Foreman`): desired state, settings, events, roles.
    pub home: Option<PathBuf>,
    pub plugin_dir: Option<PathBuf>,
    pub setup_notices: Vec<String>,
    /// The Inbox: every workspace's [`NeedsYou`] from the last tick.
    pub needs: Vec<NeedsYou>,
    /// Per workspace with a desired-state file: started, wanted session names.
    pub desired: BTreeMap<String, (bool, Vec<String>)>,
    pub max_working: usize,
    pub supervise_errors: Vec<String>,
    /// Why a `claude --bg` from here cannot reach the background service (m5/25): a tab that would
    /// run one prints it instead of failing after 45 s.
    pub daemon_hold: Option<String>,
    pub action_error: Option<String>,
    pub last_attempt: StdHashMap<String, i64>,
    pub hooks: StdHashMap<String, HookSignal>,
    /// When the shown `claude agents` snapshot was taken.
    pub polled_at: i64,
    pub ticks: u64,
    /// Retries (`(workspace, name)`) for the next tick.
    retries: Arc<Mutex<Vec<(String, String)>>>,
    /// Rows added to every `claude agents` snapshot, before the supervisor sees it: the selftest's fake
    /// sessions (m5/10: another Foreman's parked row).
    pub injected: Arc<Mutex<Vec<Session>>>,
    /// Bumped by every registry write made from this window ([`FleetView::reload_workspaces`]): a poll
    /// that read the registry before one carries the older count, and its list is not applied (m5/21).
    registry_writes: Arc<AtomicU64>,
    pub poll_hold: Arc<PollHold>,
    pub left_width: Pixels,
    pub(crate) settings_window: Option<WindowHandle<SettingsView>>,
    /// `session_id` → when this app last saw it alive.
    last_alive: StdHashMap<String, i64>,
    /// Per workspace that wants a relay: live, armed.
    pub relays: BTreeMap<String, RelayStatus>,
    /// Per workspace, what its relay received (oldest first).
    pub messages: BTreeMap<String, Vec<OutboxMessage>>,
    /// `<workspace>/<file>` of the messages shown in full.
    pub expanded: HashSet<String>,
    /// While the pointer moves over the left panel, the Inbox and Messages rows it came in on (m5/18).
    left_hold: Option<left::Hold>,
    left_hold_timer: Option<gpui::Task<()>>,
    /// Per workspace, the questions answered from here: outbox file → the answer's first line (this
    /// session's, plus `answered.json` as each tick reads it).
    pub answered: BTreeMap<String, BTreeMap<String, String>>,
    /// `<workspace>/<outbox file>` → how this session's answer went (`sent direct`, `sent via relay (…)`).
    pub answered_route: BTreeMap<String, String>,
    /// The open task box or reply, at most one.
    pub compose: Option<Compose>,
    /// The attachable sessions, for the Work tabs' item pages (Claim → Attach).
    work_live: Rc<RefCell<Vec<LiveSession>>>,
    /// Each workspace's WorkState inbox, shown in the Inbox under the supervisor's rows.
    pub work_inbox: Entity<WorkInbox>,
    /// The open right-click menu, at most one.
    pub menu: Option<ContextMenu>,
    /// The question before Remove… or a Stop that asks, at most one.
    pub confirm: Option<Confirm>,
    /// The Add workspace review, while open.
    pub adding: Option<AddDraft>,
    /// Removes waiting for their workspace's sessions to stop.
    pub removing: BTreeMap<String, Removing>,
    /// What the last menu action did, when it says so (Rescan, Copy, Remove).
    pub action_info: Option<String>,
    /// The next Add workspace takes this folder instead of asking the OS picker (the selftest).
    pub pick_folder: Option<PathBuf>,
    /// Whether launched sessions get WorkState (m5/3), checked off the UI thread every
    /// [`workstate_setup::RECHECK_EVERY`] and on [`Self::recheck_workstate`].
    pub workstate: WsStatus,
    /// The problem whose notice was dismissed; a different problem shows again.
    workstate_dismissed: Option<String>,
    workstate_recheck: Arc<AtomicBool>,
    /// The last Start's warning that its sessions run without WorkState.
    pub start_warning: Option<String>,
    /// The `+` sessions' own WorkState tokens (m5/20); `None` without a Foreman home.
    hands: Option<Arc<Mutex<Hands>>>,
    /// What went wrong minting or revoking them.
    pub hand_notices: Vec<String>,
    /// Closed sessions from their transcripts (m5/27), as stopped interactive rows.
    pub closed: Vec<Session>,
    /// [`main_repo`] of each closed session's cwd, merged into `main_repos` after every poll.
    closed_repos: StdHashMap<PathBuf, Option<PathBuf>>,
    /// Transcript scans finished.
    pub transcript_scans: u64,
    /// Set to scan the transcripts again now (a star, a closed interactive session, Refresh, the selftest).
    pub transcripts_nudge: Arc<AtomicBool>,
    /// Set to poll `claude agents` now instead of at the end of [`REFRESH_EVERY`] (Refresh).
    poll_nudge: Arc<AtomicBool>,
    /// [`Self::refresh_now`] calls.
    pub refresh_asks: u64,
    pub stars: Stars,
    /// `<workspace>/<repo>` whose `more…` row was clicked.
    pub closed_expanded: HashSet<String>,
    /// How a tab's pwsh calls claude for `+` and resume: `claude`, or the selftest's `& '<fake>'`.
    pub claude_command: String,
    /// The search box over the tree (m5/30); while it holds text, the tree shows its results.
    pub search: Entity<TextInput>,
    /// Per session id, where the shown result row matched; filled while the results are drawn.
    search_marks: StdHashMap<String, crate::search::Marks>,
}

/// A session with no process, not the supervisor's (`fm-`: Start resumes it); a background one only when
/// this app has not seen it alive in the last [`RESUME_GRACE_MS`], an interactive one at once.
pub fn offers_resume(s: &Session, health: &foreman_claude::Health, last_alive: Option<i64>, now: i64) -> bool {
    !fleet::is_alive(health)
        && !s.session_id.is_empty()
        && !s.name.starts_with("fm-")
        && (s.kind != foreman_claude::Kind::Background || last_alive.is_none_or(|t| now - t > RESUME_GRACE_MS))
}

/// What the fleet/terminal divider drags.
struct DraggedDivider;

impl Render for DraggedDivider {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

/// The selftest's grip on one poll (m5/21): armed, the next poll stops between its registry read and the
/// rest, `holding` until cleared (at most [`PollHold::MAX`]).
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
    /// [`FleetView::registry_writes`] as it was before `workspaces` was read.
    registry_writes: u64,
    workspaces: Result<Vec<Workspace>, String>,
    sessions: Option<Result<Vec<Session>, String>>,
    /// [`fleet::main_repos`] of `sessions`.
    main_repos: StdHashMap<PathBuf, Option<PathBuf>>,
    polled_at: i64,
    events: Vec<Event>,
    report: Option<Report>,
    took: Duration,
}

/// Owned by the refresh loop and moved onto a background thread for every poll: the registry read,
/// the `claude agents` process and the supervisor's `claude` writes all block.
struct Poller {
    home: Option<PathBuf>,
    events: Option<EventReader>,
    supervision: Option<Supervision>,
    retries: Arc<Mutex<Vec<(String, String)>>>,
    injected: Arc<Mutex<Vec<Session>>>,
    registry_writes: Arc<AtomicU64>,
    hold: Arc<PollHold>,
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

    /// The supervisor ticks only on a snapshot read in this poll: a stale list never drives a launch.
    fn load(&mut self, with_sessions: bool) -> Refresh {
        let started = Instant::now();
        let registry_writes = self.registry_writes.load(Ordering::SeqCst);
        let workspaces = Registry::from_env()
            .and_then(|r| r.list())
            .map_err(|e| e.to_string());
        self.hold.wait();
        let polled_at = now_ms();
        let sessions = with_sessions.then(|| {
            list_sessions().map_err(|e| e.to_string()).map(|mut rows| {
                rows.extend(self.injected.lock().unwrap_or_else(|e| e.into_inner()).iter().cloned());
                rows
            })
        });
        let main_repos = match &sessions {
            Some(Ok(sessions)) => fleet::main_repos(sessions),
            _ => StdHashMap::new(),
        };
        let events = self.events.as_mut().map(EventReader::poll).unwrap_or_default();
        let took = started.elapsed();
        let report = match (&workspaces, &sessions, self.supervision.as_mut()) {
            (Ok(workspaces), Some(Ok(sessions)), Some(supervision)) => {
                let retries = std::mem::take(&mut *self.retries.lock().unwrap_or_else(|e| e.into_inner()));
                Some(supervision.tick(workspaces, sessions, retries, now_ms()))
            }
            _ => None,
        };
        Refresh {
            registry_writes,
            workspaces,
            sessions,
            main_repos,
            polled_at,
            events,
            report,
            took,
        }
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
        let left_width = home
            .as_deref()
            .and_then(|h| Settings::load(h).ok())
            .and_then(|s| s.left_width)
            .map_or(DEFAULT_LEFT_WIDTH, |w| px(w as f32).max(MIN_LEFT_WIDTH));
        let retries: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
        let injected: Arc<Mutex<Vec<Session>>> = Arc::default();
        let registry_writes: Arc<AtomicU64> = Arc::default();
        let poll_hold: Arc<PollHold> = Arc::default();
        let poll_nudge = Arc::new(AtomicBool::new(false));
        let woken = poll_nudge.clone();
        let mut poller = Poller {
            home: home.clone(),
            events: None,
            supervision: None,
            retries: retries.clone(),
            injected: injected.clone(),
            registry_writes: registry_writes.clone(),
            hold: poll_hold.clone(),
        };
        cx.spawn(async move |this, cx| {
            let executor = cx.background_executor().clone();
            let checked = executor.spawn(async { check_supported() }).await;
            let supported = checked.is_ok();
            let (ready, setup) = executor
                .spawn(async move {
                    let setup = poller.setup();
                    (poller, setup)
                })
                .await;
            poller = ready;
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
                let (done, refresh) = executor
                    .spawn(async move {
                        let refresh = poller.load(supported);
                        (poller, refresh)
                    })
                    .await;
                poller = done;
                let alive = this.update(cx, |this, cx| {
                    this.apply(refresh);
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
                // an interactive session closed outside Foreman: poll when its process ends (m5/28), not at the
                // end of the wait; a pid this app cannot open is never watched
                let watched: Vec<u32> = pids.into_iter().filter(|&pid| process_alive(pid)).collect();
                let next = Instant::now() + REFRESH_EVERY;
                let mut slices = 0u32;
                while Instant::now() < next && !woken.swap(false, Ordering::SeqCst) {
                    executor.timer(Duration::from_millis(50)).await;
                    slices += 1;
                    if slices % 5 == 0 && watched.iter().any(|&pid| !process_alive(pid)) {
                        break;
                    }
                }
            }
        })
        .detach();
        Self::blink_cursor(cx);
        let workstate_recheck = Arc::new(AtomicBool::new(true));
        Self::watch_workstate(home.clone(), workstate_recheck.clone(), cx);
        let work_inbox = WorkInbox::shared(home.clone(), cx);
        cx.observe(&work_inbox, |_, _, cx| cx.notify()).detach();
        let hands = home.as_deref().map(|home| Self::open_hands(home, cx));
        let (stars, stars_notice) = Stars::load(home.as_deref());
        let transcripts_nudge = Arc::new(AtomicBool::new(true));
        let claude_dir = std::env::var_os(CLAUDE_DIR_VAR).filter(|d| !d.is_empty()).map(PathBuf::from).or_else(foreman_claude::claude_dir);
        Self::watch_transcripts(claude_dir, transcripts_nudge.clone(), cx);
        // an input that held the focus went away (the Add review closed): the window's keys come back here
        cx.on_focus_lost(window, |this, window, cx| this.focus.focus(window, cx)).detach();
        let search = cx.new(|cx| TextInput::new("Search sessions, workspaces, repos (Ctrl+Shift+E)", None, window, cx));
        cx.subscribe_in(&search, window, Self::on_search_event).detach();
        Self {
            cli: Cli::Checking,
            workspaces: Vec::new(),
            registry_error: None,
            sessions: Vec::new(),
            main_repos: StdHashMap::new(),
            sessions_error: None,
            last_refresh_took: None,
            refreshes: 0,
            show_stopped: false,
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
            daemon_hold: None,
            last_attempt: StdHashMap::new(),
            hooks: StdHashMap::new(),
            polled_at: 0,
            ticks: 0,
            retries,
            injected,
            registry_writes,
            poll_hold,
            left_width,
            settings_window: None,
            last_alive: StdHashMap::new(),
            relays: BTreeMap::new(),
            messages: BTreeMap::new(),
            expanded: HashSet::new(),
            left_hold: None,
            left_hold_timer: None,
            answered: BTreeMap::new(),
            answered_route: BTreeMap::new(),
            compose: None,
            work_live: Rc::default(),
            work_inbox,
            menu: None,
            confirm: None,
            adding: None,
            removing: BTreeMap::new(),
            action_info: None,
            pick_folder: None,
            workstate: WsStatus::Checking,
            workstate_dismissed: None,
            workstate_recheck,
            start_warning: None,
            hands,
            hand_notices: Vec::new(),
            closed: Vec::new(),
            closed_repos: StdHashMap::new(),
            transcript_scans: 0,
            transcripts_nudge,
            poll_nudge,
            refresh_asks: 0,
            action_error: stars_notice,
            stars,
            closed_expanded: HashSet::new(),
            claude_command: "claude".into(),
            search,
            search_marks: StdHashMap::new(),
        }
    }

    /// Closed sessions from the transcripts of the registered repos (m5/27), read on the background
    /// executor every [`TRANSCRIPTS_EVERY`], at once when the repos change or `nudge` is set.
    fn watch_transcripts(claude_dir: Option<PathBuf>, nudge: Arc<AtomicBool>, cx: &mut Context<Self>) {
        let Some(claude_dir) = claude_dir else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let executor = cx.background_executor().clone();
            let mut scanner = Transcripts::in_claude_dir(&claude_dir);
            let mut repos: StdHashMap<PathBuf, Option<PathBuf>> = StdHashMap::new();
            let mut last: Option<(Instant, Vec<PathBuf>)> = None;
            loop {
                let Ok((roots, keep, live)) = this.read_with(cx, |this, _| {
                    let roots: Vec<PathBuf> = this
                        .workspaces
                        .iter()
                        .flat_map(|w| w.repos.iter().filter(|r| r.enabled).map(|r| w.root.join(&r.path)))
                        .collect();
                    let keep: std::collections::HashSet<String> = this.stars.ids().iter().cloned().collect();
                    let live: std::collections::HashSet<String> = this.sessions.iter().map(|s| s.session_id.clone()).collect();
                    (roots, keep, live)
                }) else {
                    return;
                };
                let due = match &last {
                    None => true,
                    Some((at, scanned)) => at.elapsed() >= TRANSCRIPTS_EVERY || *scanned != roots,
                };
                if !(nudge.swap(false, Ordering::SeqCst) || due) {
                    executor.timer(Duration::from_millis(250)).await;
                    continue;
                }
                last = Some((Instant::now(), roots.clone()));
                let since = now_ms() - fleet::CLOSED_WINDOW_MS;
                let (back, known, closed) = executor
                    .spawn(async move {
                        let found = scanner.scan(&roots, since, &keep, &live);
                        for t in &found {
                            if let Some(cwd) = &t.cwd {
                                repos.entry(cwd.clone()).or_insert_with(|| main_repo(cwd));
                            }
                        }
                        let closed: Vec<Session> = found.iter().map(|t| t.to_session()).collect();
                        (scanner, repos, closed)
                    })
                    .await;
                scanner = back;
                repos = known.clone();
                if this
                    .update(cx, |this, cx| {
                        this.main_repos.extend(known.clone());
                        this.closed_repos = known;
                        this.closed = closed;
                        this.transcript_scans += 1;
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

    /// Polls `claude agents` and scans the transcripts now (m5/28): the action bar's Refresh, and F5 while no
    /// terminal tab is active (a terminal's program gets its F5).
    pub fn refresh_now(&mut self, cx: &mut Context<Self>) {
        self.refresh_asks += 1;
        self.poll_nudge.store(true, Ordering::SeqCst);
        self.transcripts_nudge.store(true, Ordering::SeqCst);
        cx.notify();
    }

    /// Stars or unstars a session (m5/27); a starred one stays in the tree, stopped or closed.
    pub fn toggle_star(&mut self, session_id: &str, cx: &mut Context<Self>) {
        match self.stars.toggle(session_id) {
            Ok(_) => self.transcripts_nudge.store(true, Ordering::SeqCst),
            Err(e) => self.action_error = Some(format!("Cannot save the star: {e}")),
        }
        cx.notify();
    }

    /// Revokes the hand tokens an earlier run left, off the UI thread, and the live ones on quit.
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
                cx.notify();
            })
            .ok();
        })
        .detach();
        let live = hands.clone();
        cx.on_app_quit(move |_, _| {
            live.lock().unwrap_or_else(|e| e.into_inner()).release_all();
            async {}
        })
        .detach();
        hands
    }

    /// [`workstate_setup::detect`] on the background executor at start, every
    /// [`workstate_setup::RECHECK_EVERY`] and soon after `recheck` is set.
    fn watch_workstate(home: Option<PathBuf>, recheck: Arc<AtomicBool>, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let executor = cx.background_executor().clone();
            let mut checked: Option<Instant> = None;
            loop {
                let due = checked.is_none_or(|t| t.elapsed() >= workstate_setup::RECHECK_EVERY);
                if recheck.swap(false, Ordering::SeqCst) || due {
                    checked = Some(Instant::now());
                    let home = home.clone();
                    let status = executor
                        .spawn(async move { workstate_setup::detect(home.as_deref(), workstate_setup::installed_env().as_deref()) })
                        .await;
                    let alive = this.update(cx, |this, cx| {
                        if this.workstate != status {
                            if matches!(status, WsStatus::Ready { .. }) {
                                this.start_warning = None;
                            }
                            this.workstate = status;
                            cx.notify();
                        }
                    });
                    if alive.is_err() {
                        return;
                    }
                } else if this.upgrade().is_none() {
                    return;
                }
                executor.timer(Duration::from_millis(250)).await;
            }
        })
        .detach();
    }

    /// Check WorkState again now (Retry, a changed `.env`), off the UI thread.
    pub fn recheck_workstate(&self) {
        self.workstate_recheck.store(true, Ordering::SeqCst);
    }

    /// The notice's one click: `workstate_env = path` in settings.toml, then a recheck; the Work tabs
    /// and the Inbox read WorkState with it from now on.
    pub fn use_workstate_env(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let Some(home) = self.home.clone() else {
            return;
        };
        let saved = Settings::load(&home).and_then(|mut settings| {
            settings.workstate_env = Some(path);
            settings.save(&home)
        });
        match saved {
            Ok(_) => {
                self.workstate = WsStatus::Checking;
                self.action_error = None;
            }
            Err(e) => self.action_error = Some(format!("Cannot save settings.toml: {e}")),
        }
        self.recheck_workstate();
        WorkHub::shared(Some(home), cx);
        cx.notify();
    }

    pub fn dismiss_workstate_notice(&mut self, cx: &mut Context<Self>) {
        self.workstate_dismissed = self.workstate.problem();
        cx.notify();
    }

    /// The WorkState notice's problem, unless it was dismissed.
    pub fn workstate_notice(&self) -> Option<String> {
        self.workstate.problem().filter(|p| self.workstate_dismissed.as_ref() != Some(p))
    }

    fn apply(&mut self, refresh: Refresh) {
        match refresh.workspaces {
            // read before a write from here, so older than the list `reload_workspaces` showed after it
            Ok(_) if refresh.registry_writes != self.registry_writes.load(Ordering::SeqCst) => {}
            Ok(workspaces) => {
                self.workspaces = workspaces;
                self.registry_error = None;
            }
            Err(error) => self.registry_error = Some(error),
        }
        // A failed read keeps the last good list on screen, flagged as stale.
        match refresh.sessions {
            Some(Ok(sessions)) => {
                let now = now_ms();
                // an interactive session leaves `claude agents` when it exits; its transcript makes it a closed row
                let ended = self.sessions.iter().any(|old| {
                    old.kind == foreman_claude::Kind::Interactive && !sessions.iter().any(|s| s.session_id == old.session_id)
                });
                if ended {
                    self.transcripts_nudge.store(true, Ordering::SeqCst);
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
                self.main_repos = refresh.main_repos;
                self.main_repos.extend(self.closed_repos.iter().map(|(k, v)| (k.clone(), v.clone())));
                self.sessions_error = None;
                self.polled_at = refresh.polled_at;
            }
            Some(Err(error)) => self.sessions_error = Some(error),
            None => {}
        }
        for event in refresh.events {
            HookSignal::record(&mut self.hooks, event);
        }
        if let Some(report) = refresh.report {
            self.desired = report.desired;
            // by construction only this home's wanted sessions raise one; kept so, whatever a tick reports
            self.needs = report.needs;
            let (workspaces, desired) = (&self.workspaces, &self.desired);
            self.needs.retain(|n| !fleet::is_foreign(&n.name, workspaces, Some(desired)));
            self.max_working = report.max_working;
            self.supervise_errors = report.errors;
            self.daemon_hold = report.daemon_hold;
            self.last_attempt = report.last_attempt.into_iter().collect();
            self.relays = report.relays;
            self.messages = report.messages;
            for (workspace, on_disk) in report.answered {
                let answered = self.answered.entry(workspace).or_default();
                for (file, line) in on_disk {
                    answered.entry(file).or_insert(line);
                }
            }
            self.ticks += 1;
        }
        self.last_refresh_took = Some(refresh.took);
        self.refreshes += 1;
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
        };
        fleet::build(&self.workspaces, &self.sessions, now_ms(), self.show_stopped, &sup)
    }

    pub fn start_workspace(&mut self, name: &str, cx: &mut Context<Self>) {
        let result = match (&self.home, self.workspaces.iter().find(|p| p.name == name)) {
            (Some(home), Some(workspace)) => supervise::start_workspace(home, self.plugin_dir.as_deref(), workspace),
            (None, _) => Err(anyhow::anyhow!("no Foreman home")),
            (_, None) => Err(anyhow::anyhow!("not registered")),
        };
        match result {
            Ok(desired) => {
                let names = desired.sessions.iter().map(|w| w.name.clone()).collect();
                self.desired.insert(name.to_string(), (true, names));
                self.action_error = None;
                self.start_warning = self.workstate.start_warning(name);
            }
            Err(e) => self.action_error = Some(format!("Cannot start {name}: {e:#}")),
        }
        cx.notify();
    }

    pub fn stop_workspace(&mut self, name: &str, cx: &mut Context<Self>) {
        let result = match &self.home {
            Some(home) => supervise::stop_workspace(home, name),
            None => Err(anyhow::anyhow!("no Foreman home")),
        };
        match result {
            Ok(()) => {
                if let Some(entry) = self.desired.get_mut(name) {
                    entry.0 = false;
                }
                self.action_error = None;
                self.start_warning = None;
            }
            Err(e) => self.action_error = Some(format!("Cannot stop {name}: {e:#}")),
        }
        cx.notify();
    }

    /// Why nothing can be sent through `workspace`'s relay now, if so: it must be started. The relay
    /// itself need not run: a drop makes the supervisor start it (m5/32), and it waits in the inbox till then.
    pub fn relay_refusal(&self, workspace: &str) -> Option<String> {
        if self.home.is_none() {
            return Some("no Foreman home".into());
        }
        match self.relays.get(workspace) {
            Some(r) if r.started => None,
            _ => Some(format!("{workspace} is stopped")),
        }
    }

    /// Opens the task box (`reply_to: None`) or the reply to an outbox message, focused; an open one
    /// for something else is replaced.
    pub fn open_compose(&mut self, workspace: &str, reply_to: Option<String>, cx: &mut Context<Self>) {
        match &mut self.compose {
            Some(c) if c.workspace == workspace && c.reply_to == reply_to => c.focused = true,
            compose => *compose = Some(Compose::new(workspace, reply_to)),
        }
        cx.notify();
    }

    /// The box's Ctrl+Enter: to `fm-<w>-workspace` as `fm send` does (direct, else the relay). A task
    /// clears the box on success; a reply marks its question answered and closes it. A refusal or a
    /// failure stays in the box.
    pub fn send_compose(&mut self, cx: &mut Context<Self>) {
        let Some(c) = &self.compose else {
            return;
        };
        let (workspace, reply_to, text) = (c.workspace.clone(), c.reply_to.clone(), c.text.trim().to_string());
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
                c.clear();
                c.sent = Some(compose::first_line(&text));
                c.route = Some(sent.label());
                c.error = None;
            }
            (Err(e), Some(c)) => c.error = Some(e),
            (_, None) => {}
        }
        cx.notify();
    }

    /// Into the workspace session's pipe, else through its relay when that is live and armed.
    fn send_to_workspace(&self, workspace: &str, text: &str) -> Result<Sent, String> {
        let home = self.home.as_deref().ok_or("Not sent: no Foreman home.")?;
        let to = workspace_session_name(workspace);
        let mode = DesiredState::launch_mode(home, workspace, &to);
        send_from_human(home, workspace, &to, mode.as_deref(), text, self.relay_refusal(workspace))
            .map_err(|e| format!("Not sent: direct delivery failed ({}), and the relay cannot take it: {}.", e.direct, e.relay))
    }

    /// Sends `text` as the answer to the outbox message `file` and marks it answered, in memory and in
    /// `answered.json` (a failed save is reported; the row still shows it answered).
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

    /// The Messages row's Approve: the answer `approve`.
    pub fn approve(&mut self, workspace: &str, file: &str, cx: &mut Context<Self>) {
        match self.answer(workspace, file, "approve") {
            Ok(_) => self.action_error = None,
            Err(e) => self.action_error = Some(format!("Approve {workspace}: {e}")),
        }
        cx.notify();
    }

    /// The shared concurrency budget, saved to `settings.toml`; the next tick applies it.
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

    /// The Inbox's Attach: the existing attach-tab path, by the row's 8-hex id.
    pub fn attach_by_id(&mut self, workspace: &str, id: &str, window: &mut Window, cx: &mut Context<Self>) -> Option<u64> {
        let session = self.sessions.iter().find(|s| s.id == id)?.clone();
        Some(self.attach(workspace, "supervised", &session, window, cx))
    }

    /// A shell tab in `cwd` with `command` typed at its prompt and NOT run: the human presses Enter
    /// and answers whatever it asks (the trust prompt is theirs; Foreman never answers it).
    pub fn run_in_terminal(&mut self, cwd: PathBuf, command: String, window: &mut Window, cx: &mut Context<Self>) -> u64 {
        let title = format!("trust · {}", cwd.display());
        self.type_in_shell(cwd, title, command, window, cx)
    }

    fn type_in_shell(&mut self, cwd: PathBuf, title: String, command: String, window: &mut Window, cx: &mut Context<Self>) -> u64 {
        self.type_in_shell_when(cwd, title, gpui::Task::ready(command), window, cx)
    }

    /// [`Self::type_in_shell`], typed once `command` is known as well.
    fn type_in_shell_when(&mut self, cwd: PathBuf, title: String, command: gpui::Task<String>, window: &mut Window, cx: &mut Context<Self>) -> u64 {
        let shell = util::shell::Shell::WithArguments {
            program: "pwsh".into(),
            args: self.shell_args.clone(),
            title_override: None,
        };
        let id = self.open_tab(title, TabKind::Shell, shell, Some(cwd), window, cx);
        // Typed once the shell's output has settled (its prompt is up, whatever the profile makes
        // it look like), or after 15 s regardless.
        cx.spawn(async move |this, cx| {
            let command = command.await;
            let started = Instant::now();
            let mut last = String::new();
            let mut since = Instant::now();
            loop {
                let Ok(terminal) = this.read_with(cx, |this, _| this.tab(id).and_then(|t| t.terminal.clone())) else {
                    return;
                };
                if let Some(terminal) = terminal {
                    let content = terminal.read_with(cx, |t, _| t.get_content());
                    if content != last {
                        last = content;
                        since = Instant::now();
                    } else if (!last.trim().is_empty() && since.elapsed() > Duration::from_millis(500))
                        || started.elapsed() > Duration::from_secs(15)
                    {
                        terminal.update(cx, |t, _| t.input(command.into_bytes()));
                        return;
                    }
                }
                cx.background_executor().timer(Duration::from_millis(50)).await;
            }
        })
        .detach();
        id
    }

    /// Brings the settings window forward, or opens it.
    pub fn open_settings(&mut self, cx: &mut Context<Self>) {
        self.open_settings_at(None, cx);
    }

    /// [`Self::open_settings`], on `page` when given.
    pub fn open_settings_at(&mut self, page: Option<settings_window::Page>, cx: &mut Context<Self>) {
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
        if let Ok(mut settings) = Settings::load(home)
            && settings.left_width != Some(width)
        {
            settings.left_width = Some(width);
            if let Err(e) = settings.save(home) {
                self.action_error = Some(format!("Cannot save the panel width: {e}"));
            }
        }
    }

    /// Whether [`Self::resume`] is offered: see [`offers_resume`].
    pub fn resumable(&self, row: &SessionRow) -> bool {
        offers_resume(&row.session, &row.health, self.last_alive.get(&row.session.session_id).copied(), now_ms())
    }

    /// A background session wakes in the background (`claude --bg --resume`, no other flag: the
    /// same id) and the tab attaches to it, so closing the tab detaches and leaves it running. An
    /// interactive one resumes in the tab, as it ran before.
    pub fn resume(&mut self, workspace: &str, repo: &str, session: &Session, window: &mut Window, cx: &mut Context<Self>) -> u64 {
        let name = if session.name.is_empty() { fleet::short_id(session) } else { &session.name };
        let title = format!("{workspace} · {repo} · {name}");
        let sid = &session.session_id;
        if session.kind == foreman_claude::Kind::Background {
            let script = match &self.daemon_hold {
                Some(why) => held_line(why),
                None => format!(
                    "Write-Host 'resuming {} in the background, then attaching…'; claude --bg --resume {sid}; if ($LASTEXITCODE -eq 0) {{ claude attach {} }}",
                    session.id, session.id
                ),
            };
            let shell = util::shell::Shell::WithArguments {
                program: "pwsh".into(),
                args: vec!["-NoLogo".into(), "-NoProfile".into(), "-Command".into(), script],
                title_override: None,
            };
            let kind = TabKind::Attach { session_id: session.id.clone() };
            self.open_tab(title, kind, shell, Some(session.cwd.clone()), window, cx)
        } else {
            // through the `+` launch (m5/27): its own WorkState token, and the shell stays when claude exits
            let resume = format!("{} --resume '{}'", self.claude_command, sid.replace('\'', "''"));
            let id = self.hand_claude(session.cwd.clone(), title, resume, cx);
            if let Some(tab) = self.tab_mut(id) {
                tab.session = Some(tab_card::session_label(&session.name, sid));
            }
            id
        }
    }

    /// A shell tab in `cwd` whose pwsh starts `claude` unseen ([`Launch::shell_args`]): the shell stays
    /// when it exits. With WorkState it runs with its own token ([`crate::hand`]), revoked when claude
    /// exits or the tab closes. The tab shows at once; its pwsh starts once the token is minted.
    pub fn new_claude(&mut self, cwd: PathBuf, title: String, _window: &mut Window, cx: &mut Context<Self>) -> u64 {
        let claude = self.claude_command.clone();
        self.hand_claude(cwd, title, claude, cx)
    }

    /// [`Self::new_claude`] running `claude` (pwsh's program and arguments).
    fn hand_claude(&mut self, cwd: PathBuf, title: String, claude: String, cx: &mut Context<Self>) -> u64 {
        let id = self.push_tab(title, TabKind::Shell, Some(cwd.clone()), cx);
        if let Some(tab) = self.tab_mut(id) {
            tab.hand = true;
        }
        let Some(hands) = self.hands.clone() else {
            self.start_terminal(id, self.claude_shell(&Launch::Plain, &claude), Some(cwd), cx);
            return id;
        };
        let at = cwd.clone();
        let mint = hands.clone();
        let launch = cx.background_executor().spawn(async move { mint.lock().unwrap_or_else(|e| e.into_inner()).launch(&at) });
        cx.spawn(async move |this, cx| {
            let launch = launch.await;
            this.update(cx, |this, cx| {
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
            loop {
                cx.background_executor().timer(Duration::from_millis(500)).await;
                let open = this.read_with(cx, |this, _| this.tab(id).is_some_and(|t| !t.exited)).unwrap_or(false);
                // try_lock: another tab's mint may hold it over HTTP, and this is the UI thread.
                if open && !hands.try_lock().is_ok_and(|h| h.ended(&name)) {
                    continue;
                }
                let hands = hands.clone();
                let notice = cx
                    .background_executor()
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

    /// With [`Self::daemon_hold`] the tab says first why its claude cannot reach the background service.
    fn claude_shell(&self, launch: &Launch, claude: &str) -> util::shell::Shell {
        let launch_args = match &self.daemon_hold {
            Some(why) => {
                let script = format!("{}; {}", held_line(why), launch.script_with(claude));
                ["-NoExit".into(), "-EncodedCommand".into(), crate::hand::encoded_command(&script)]
            }
            None => launch.shell_args_with(claude),
        };
        util::shell::Shell::WithArguments {
            program: "pwsh".into(),
            args: self.shell_args.iter().cloned().chain(launch_args).collect(),
            title_override: None,
        }
    }

    /// Repaints at each blink edge of the shown terminal's cursor (Windows Terminal's blinking bar);
    /// nothing while it does not blink. A text input blinks its own caret.
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
                // a cursor move restarts the blink; the next edge is never later than one interval
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

    /// What clicking the tab does.
    pub fn activate_tab(&mut self, id: u64, cx: &mut Context<Self>) {
        if self.tab(id).is_some() {
            self.active = Some(id);
            cx.notify();
        }
    }

    fn cycle_tab(&mut self, step: isize, cx: &mut Context<Self>) {
        let at = self.active.and_then(|id| self.tabs.iter().position(|t| t.id == id));
        if let Some(next) = cycled(self.tabs.len(), at, step) {
            self.activate_tab(self.tabs[next].id, cx);
        }
    }

    /// The window's keys are this view's: no task box or text input holds them.
    pub fn has_keys(&self, window: &Window) -> bool {
        self.focus.is_focused(window) && !self.compose.as_ref().is_some_and(|c| c.focused)
    }

    pub fn attach(
        &mut self,
        workspace: &str,
        repo: &str,
        session: &Session,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> u64 {
        let name = if session.name.is_empty() {
            fleet::short_id(session)
        } else {
            &session.name
        };
        let shell = util::shell::Shell::WithArguments {
            program: "claude".into(),
            args: vec!["attach".into(), session.id.clone()],
            title_override: None,
        };
        self.open_tab(
            format!("{workspace} · {repo} · {name}"),
            TabKind::Attach {
                session_id: session.id.clone(),
            },
            shell,
            Some(session.cwd.clone()),
            window,
            cx,
        )
    }

    /// Focuses the workspace's Work tab, or opens it.
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
        let cwd = self.workspaces.iter().find(|w| w.name == workspace).map(|w| w.root.clone());
        self.tabs.push(Tab {
            id,
            title: format!("Work · {workspace}"),
            kind: TabKind::Work(view),
            terminal: None,
            error: None,
            exited: false,
            detach: Detach::None,
            cwd,
            font_size: term_element::FONT_SIZE,
            find: None,
            program_title: None,
            session: None,
            hand: false,
            opened_at: now_ms(),
        });
        self.active = Some(id);
        cx.notify();
        id
    }

    /// A WorkState Inbox row: the workspace's Work tab, on the row's item page.
    pub fn open_work_item(&mut self, workspace: &str, item_id: &str, cx: &mut Context<Self>) -> u64 {
        let id = self.open_work(workspace, cx);
        if let Some(Tab { kind: TabKind::Work(view), .. }) = self.tab(id) {
            let (view, item) = (view.clone(), item_id.to_string());
            view.update(cx, |view, cx| view.open_from_inbox(item, cx));
        }
        id
    }

    /// What `tab`'s hover card says, but its branch; `title` is the label as the tab shows it.
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
        let kind = match tab.kind {
            TabKind::Attach { .. } => Some(tab_card::Kind::Attach),
            TabKind::Shell if tab.hand => Some(tab_card::Kind::Hand),
            TabKind::Shell => Some(tab_card::Kind::Shell),
            TabKind::Work(_) => None,
        };
        tab_card::Card {
            title,
            workspace,
            repo,
            folder: tab.cwd.as_ref().map(|c| c.display().to_string()),
            branch: None,
            session,
            kind,
            started,
        }
    }

    /// A shell tab in `cwd`.
    pub fn open_shell_in(&mut self, cwd: PathBuf, title: String, window: &mut Window, cx: &mut Context<Self>) -> u64 {
        let shell = util::shell::Shell::WithArguments {
            program: "pwsh".into(),
            args: self.shell_args.clone(),
            title_override: None,
        };
        self.open_tab(title, TabKind::Shell, shell, Some(cwd), window, cx)
    }

    pub fn new_shell(&mut self, window: &mut Window, cx: &mut Context<Self>) -> u64 {
        let shell = util::shell::Shell::WithArguments {
            program: "pwsh".into(),
            args: self.shell_args.clone(),
            title_override: None,
        };
        let title = format!("pwsh {}", self.next_tab_id);
        let cwd = self.new_shell_cwd();
        self.open_tab(title, TabKind::Shell, shell, cwd, window, cx)
    }

    /// [`fleet::shell_cwd`] for the active tab, never the exe's folder.
    pub fn new_shell_cwd(&self) -> Option<PathBuf> {
        let active = self.active.and_then(|id| self.tab(id)).and_then(|t| t.cwd.as_deref());
        let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from);
        fleet::shell_cwd(active, &self.workspaces, home)
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

    /// The active tab, its terminal not started yet ([`Self::start_terminal`]).
    fn push_tab(&mut self, title: String, kind: TabKind, cwd: Option<PathBuf>, cx: &mut Context<Self>) -> u64 {
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        self.tabs.push(Tab {
            id,
            title,
            kind,
            terminal: None,
            error: None,
            exited: false,
            detach: Detach::None,
            cwd,
            font_size: term_element::FONT_SIZE,
            find: None,
            program_title: None,
            session: None,
            hand: false,
            opened_at: now_ms(),
        });
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
                        cx.subscribe(&terminal, move |this, emitter, event, cx| match event {
                            TerminalEvent::Wakeup | TerminalEvent::TitleChanged => cx.notify(),
                            TerminalEvent::BreadcrumbsChanged => {
                                let title = emitter.read(cx).breadcrumb_text.clone();
                                if let Some(tab) = this.tab_mut(id) {
                                    tab.program_title = tab_card::sanitize(&title);
                                }
                                cx.notify();
                            }
                            TerminalEvent::CloseTerminal => {
                                if let Some(tab) = this.tab_mut(id) {
                                    tab.exited = true;
                                }
                                cx.notify();
                            }
                            _ => {}
                        })
                        .detach();
                        match this.tab_mut(id) {
                            Some(tab) => {
                                tab.terminal = Some(terminal);
                                if tab.detach == Detach::Pending {
                                    this.start_detach(id, cx);
                                }
                            }
                            // Closed while it was starting.
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

    /// A shell tab ends its process. An Attach tab first asks `claude attach` to detach by itself
    /// ([`DETACH_INPUT`]) and is removed only once that process has exited; see [`Detach`].
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
        let pid = terminal.update(cx, |t, _| {
            t.input(DETACH_INPUT);
            t.pid().map(|p| p.as_u32())
        });
        cx.notify();
        cx.spawn(async move |this, cx| {
            let started = Instant::now();
            loop {
                let exited = this
                    .read_with(cx, |this, _| this.tab(id).is_none_or(|t| t.exited))
                    .unwrap_or(true);
                if exited || !pid.is_some_and(process_alive) {
                    this.update(cx, |this, cx| this.remove_tab(id, cx)).ok();
                    return;
                }
                if started.elapsed() > DETACH_WITHIN {
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
                cx.background_executor().timer(Duration::from_millis(20)).await;
            }
        })
        .detach();
    }

    fn remove_tab(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.tabs.iter().position(|t| t.id == id) else {
            return;
        };
        let tab = self.tabs.remove(index);
        if let Some(terminal) = tab.terminal {
            terminal.update(cx, |t, _| t.release_pty_resources());
        }
        if self.active == Some(id) {
            self.active = self
                .tabs
                .get(index.min(self.tabs.len().saturating_sub(1)))
                .map(|t| t.id);
        }
        if self.quitting && !self.has_attach_tabs() {
            self.close_settings(cx);
            self.window
                .update(cx, |_, window, cx| crate::tray::close(window, cx))
                .ok();
        }
        cx.notify();
    }

    fn has_attach_tabs(&self) -> bool {
        self.tabs.iter().any(|t| matches!(t.kind, TabKind::Attach { .. }))
    }

    /// The Inbox's Restart as administrator (m5/25): Settings → Run Foreman as administrator on, then an
    /// elevated copy (UAC) that waits for this one to quit. Declined, this one stays and says so.
    pub fn restart_elevated(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(home) = self.home.clone() else {
            return;
        };
        let saved = Settings::load(&home).and_then(|mut settings| {
            settings.run_as_admin = true;
            settings.save(&home)
        });
        if let Err(e) = saved {
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

    /// Quit: the window closes as [`Self::should_close`] lets it, and the app ends with it.
    pub fn quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.quit_asked = true;
        if self.should_close(window, cx) {
            crate::tray::close(window, cx);
        }
    }

    /// Closing the window hides it (with the settings window) unless Quit was asked for. Quitting with
    /// Attach tabs open detaches them first; the window closes when the last one is gone, or stays open
    /// if one fails.
    fn should_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.quit_asked {
            self.close_settings(cx);
            crate::tray::hide(window, cx);
            return false;
        }
        if !self.has_attach_tabs() {
            self.close_settings(cx);
            return true;
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
        // Tabs whose process had already exited are gone by now.
        let close = !self.has_attach_tabs();
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

    /// Whether one of the text inputs (the search box, the Add review's name) has the keys.
    pub fn input_focused(&self, window: &Window, cx: &gpui::App) -> bool {
        self.search.read(cx).is_focused(window) || self.adding.as_ref().is_some_and(|d| d.name.read(cx).is_focused(window))
    }

    /// Ctrl+Shift+E: the search box, its text selected.
    pub fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search.update(cx, |s, cx| s.focus_all(window, cx));
    }

    /// The search box's text, trimmed; empty while it shows the tree.
    pub fn search_query(&self, cx: &gpui::App) -> String {
        self.search.read(cx).text().trim().to_string()
    }

    fn on_search_event(&mut self, _: &Entity<TextInput>, event: &InputEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            InputEvent::Edited => cx.notify(),
            // Esc closes an open menu first, then clears the box, then gives the keys back to the window
            InputEvent::Cancel if self.menu.is_some() || self.confirm.is_some() => {
                if self.menu.take().is_none() {
                    self.confirm = None;
                }
                cx.notify();
            }
            InputEvent::Cancel if !self.search.read(cx).text().is_empty() => self.clear_search(cx),
            InputEvent::Cancel => self.focus.focus(window, cx),
            InputEvent::Confirm | InputEvent::Refused(_) => {}
        }
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
        // An input's own keys ran as its actions already. What is left must go on unhandled: Windows turns
        // a key into its character (WM_CHAR, a dead key's too) only after nothing took the key down.
        if self.input_focused(window, cx) {
            return;
        }
        if k.key == "escape" && (self.menu.is_some() || self.confirm.is_some()) {
            if self.menu.take().is_none() {
                self.confirm = None;
            }
            cx.notify();
            cx.stop_propagation();
            return;
        }
        // the review keeps Enter and Esc after a click on one of its repos took the keys from its name
        if self.adding.is_some() && !k.modifiers.modified() && matches!(k.key.as_str(), "enter" | "escape") {
            if k.key == "enter" {
                self.save_add(cx);
            } else {
                self.adding = None;
            }
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if let Some(compose) = self.compose.as_mut().filter(|c| c.focused) {
            match compose.key(k, || cx.read_from_clipboard().and_then(|c| c.text())) {
                KeyOutcome::Send => self.send_compose(cx),
                KeyOutcome::Cancel => self.compose = None,
                KeyOutcome::Edited => compose.error = None,
                KeyOutcome::Ignored => {}
            }
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
            && self.terminal_key(id, &terminal, k, cx)
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

/// An invisible overlay that records its parent's painted bounds and clip under `key`.
pub(crate) fn hit_target(hits: &HitTargets, key: String) -> impl IntoElement {
    let hits = hits.clone();
    canvas(
        move |bounds, window, _| {
            let visible = window.content_mask().bounds;
            hits.borrow_mut().insert(key, Hit { bounds, visible });
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// A small bordered button whose painted bounds are recorded under `key` (the selftest clicks it): one
/// height, the label centred, a lighter fill and border on hover, a deeper fill pressed.
/// The pwsh line a tab runs in place of a `claude --bg` that cannot reach the background service.
fn held_line(why: &str) -> String {
    format!("Write-Host -ForegroundColor Yellow '{}'", why.replace('\'', "''"))
}

fn button(hits: &HitTargets, key: String, label: impl Into<SharedString>, cx: &gpui::App) -> gpui::Stateful<gpui::Div> {
    let colors = cx.theme().colors();
    div()
        .id(SharedString::from(key.clone()))
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .h(px(20.))
        .px_2()
        .rounded_sm()
        .border_1()
        .border_color(colors.border)
        .bg(colors.element_background)
        .text_xs()
        .text_color(colors.text)
        .whitespace_nowrap()
        .cursor_pointer()
        .hover(|s| s.bg(colors.element_hover).border_color(colors.border_selected))
        .active(|s| s.bg(colors.element_active))
        .child(label.into())
        .child(hit_target(hits, key))
}

impl FleetView {
    fn render_right(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let mut bar = div()
            .flex()
            .flex_none()
            .h(px(32.))
            .items_end()
            .bg(colors.tab_bar_background)
            .border_b_1()
            .border_color(colors.border);
        for tab in &self.tabs {
            let id = tab.id;
            let active = self.active == Some(id);
            let close_key = format!("close-tab-{id}");
            let title = tab.label();
            bar = bar.child(
                div()
                    .id(SharedString::from(format!("tab-{id}")))
                    .relative()
                    .flex()
                    .items_center()
                    .gap_2()
                    .h_full()
                    .px_3()
                    .text_sm()
                    .border_r_1()
                    .border_color(colors.border)
                    .cursor_pointer()
                    .bg(if active { colors.tab_active_background } else { colors.tab_inactive_background })
                    .when(!active, |d| d.text_color(colors.text_muted).hover(|s| s.bg(colors.ghost_element_hover).text_color(colors.text)))
                    // the active tab: an accent line on top
                    .when(active, |d| {
                        d.child(div().absolute().top_0().left_0().right_0().h(px(2.)).bg(colors.border_focused))
                    })
                    .child(crate::clipped::clipped_with(format!("tab-title-{id}"), title.clone(), false, false).max_w(px(280.)))
                    .tooltip({
                        let card = self.tab_card(tab, title);
                        let cwd = tab.cwd.clone();
                        tab_card::tooltip(move || tab_card::Card { branch: cwd.as_deref().and_then(tab_card::branch_at), ..card.clone() })
                    })
                    .child({
                        let group = SharedString::from(close_key.clone());
                        div()
                            .id(group.clone())
                            .group(group.clone())
                            .relative()
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .size(px(18.))
                            .rounded_sm()
                            .hover(|s| s.bg(colors.ghost_element_hover))
                            .active(|s| s.bg(colors.ghost_element_active))
                            .child(
                                icon::Icon::Close
                                    .svg(14., colors.text_muted)
                                    .group_hover(group, |s| s.text_color(colors.text)),
                            )
                            .child(hit_target(&self.hits, close_key))
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                this.close_tab(id, cx);
                            }))
                    })
                    .child(hit_target(&self.hits, format!("tab-{id}")))
                    .on_mouse_down(
                        MouseButton::Middle,
                        cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            this.close_tab(id, cx);
                        }),
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.activate_tab(id, cx))),
            );
        }
        bar = bar.child(
            div()
                .id("new-shell")
                .group("new-shell")
                .relative()
                .flex()
                .items_center()
                .justify_center()
                .h_full()
                .w(px(36.))
                .cursor_pointer()
                .hover(|s| s.bg(colors.ghost_element_hover))
                .active(|s| s.bg(colors.ghost_element_active))
                .child(icon::Icon::Plus.svg(16., colors.text_muted).group_hover("new-shell", |s| s.text_color(colors.text)))
                .child(hit_target(&self.hits, "new-shell".into()))
                .tooltip(icon::Tooltip::text("New shell"))
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.new_shell(window, cx);
                })),
        );

        let active = self.active.and_then(|id| self.tab(id));
        let status = cx.theme().status().clone();
        let content: AnyElement = match active {
            Some(Tab { kind: TabKind::Work(view), .. }) => view.clone().into_any_element(),
            Some(Tab { id: tab_id, terminal: Some(terminal), detach, font_size, find, .. }) => div()
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
                        .flex_1()
                        .min_h_0()
                        // a click in the terminal gives it the keys back from the task box and the
                        // find bar
                        .on_mouse_down(MouseButton::Left, {
                            let tab_id = *tab_id;
                            cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                                if let Some(c) = &mut this.compose
                                    && c.focused
                                {
                                    c.focused = false;
                                    cx.notify();
                                }
                                if let Some(find) = this.tab_mut(tab_id).and_then(|t| t.find.as_mut())
                                    && find.focused
                                {
                                    find.focused = false;
                                    cx.notify();
                                }
                            })
                        })
                        .on_scroll({
                            // as Zed's TerminalView::scroll_wheel: the Terminal decides between
                            // scrollback, a mouse report and alternate-scroll arrow keys; Ctrl+wheel
                            // zooms (Windows Terminal)
                            let terminal = terminal.clone();
                            let tab_id = *tab_id;
                            cx.listener(move |this, e: &ScrollEvent, window, cx| {
                                if e.modifiers.control {
                                    let lines = e.delta.pixel_delta(window.line_height()).y;
                                    if lines != px(0.) {
                                        this.wheel_font_size(tab_id, lines > px(0.), cx);
                                    }
                                    return;
                                }
                                terminal.update(cx, |terminal, _| {
                                    terminal.scroll_wheel(e, SCROLL_MULTIPLIER, MouseInputMode::ReportToTerminal);
                                });
                                // the queued Scroll is applied by the grid's prepaint sync
                                cx.notify();
                            })
                        })
                        .child(term_element::terminal_grid(terminal.clone(), *font_size)),
                )
                .into_any_element(),
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
            .child(div().flex_1().min_h_0().child(content))
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
        crate::selftest::injected_frame_sleep();
        self.hits.borrow_mut().clear();
        let colors = cx.theme().colors().clone();
        div()
            .size_full()
            .flex()
            .font_family("Segoe UI")
            .bg(colors.background)
            .text_color(colors.text)
            .track_focus(&self.focus)
            .key_context(KEY_CONTEXT)
            .on_action(cx.listener(|this, _: &NextTab, _, cx| this.cycle_tab(1, cx)))
            .on_action(cx.listener(|this, _: &PreviousTab, _, cx| this.cycle_tab(-1, cx)))
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| this.focus_search(window, cx)))
            .on_key_down(cx.listener(Self::key_down))
            // a click anywhere but in a text input gives the keys back to the window (an input stops its own)
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
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .on_drag_move(cx.listener(|this, e: &DragMoveEvent<DraggedDivider>, window, cx| {
                        let max = (window.viewport_size().width - MIN_RIGHT_WIDTH).max(MIN_LEFT_WIDTH);
                        this.left_width = (e.event.position.x - e.bounds.left()).clamp(MIN_LEFT_WIDTH, max);
                        cx.notify();
                    }))
                    .child(self.render_left(cx))
                    .child(
                        // A 1 px border with a 7 px grab area over both panels' edges.
                        div()
                            .relative()
                            .w(px(1.))
                            .h_full()
                            .flex_none()
                            .bg(colors.border)
                            .child(
                                div()
                                    .id("divider")
                                    .absolute()
                                    .top_0()
                                    .left(px(-3.))
                                    .w(px(7.))
                                    .h_full()
                                    .occlude()
                                    .cursor(CursorStyle::ResizeLeftRight)
                                    .on_drag(DraggedDivider, |_, _, _, cx| cx.new(|_| DraggedDivider))
                                    .on_mouse_up(
                                        MouseButton::Left,
                                        cx.listener(|this, _, _, _| this.save_left_width()),
                                    )
                                    .on_click(cx.listener(|this, e: &ClickEvent, _, cx| {
                                        if e.click_count() == 2 {
                                            this.left_width = DEFAULT_LEFT_WIDTH;
                                            this.save_left_width();
                                            cx.notify();
                                        }
                                    })),
                            ),
                    )
                    .child(self.render_right(cx)),
            )
            .children(self.render_menu(cx))
            .child(self.probe.paint_marker())
    }
}

#[cfg(windows)]
pub fn process_alive(pid: u32) -> bool {
    use windows::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return false;
        };
        let mut code = 0u32;
        let ok = GetExitCodeProcess(handle, &mut code).is_ok();
        let _ = CloseHandle(handle);
        ok && code == STILL_ACTIVE.0 as u32
    }
}

#[cfg(not(windows))]
pub fn process_alive(_pid: u32) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use gpui::{point, size};

    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(x), px(y)), size(px(w), px(h)))
    }

    #[test]
    fn a_hit_is_visible_only_when_wholly_inside_its_clip() {
        let tree = rect(0., 560., 480., 260.);
        assert!(Hit { bounds: rect(406., 700., 41., 19.), visible: tree }.fully_visible());
        assert!(Hit { bounds: rect(406., 560., 41., 260.), visible: tree }.fully_visible(), "edges touch");
        // m3/3: painted 804..823 under an 820 px window, centre inside, bottom outside
        assert!(!Hit { bounds: rect(406., 804., 41., 19.33), visible: tree }.fully_visible());
        assert!(!Hit { bounds: rect(406., 846., 41., 19.), visible: tree }.fully_visible(), "below the fold");
        assert!(!Hit { bounds: rect(406., 550., 41., 19.), visible: tree }.fully_visible(), "under the section above");
        assert!(!Hit { bounds: rect(470., 700., 41., 19.), visible: tree }.fully_visible(), "past the right edge");
    }

    #[test]
    fn tabs_cycle_wrapping_both_ways() {
        assert_eq!(cycled(3, Some(0), 1), Some(1));
        assert_eq!(cycled(3, Some(2), 1), Some(0), "wraps forward");
        assert_eq!(cycled(3, Some(0), -1), Some(2), "wraps back");
        assert_eq!(cycled(1, Some(0), 1), Some(0));
        assert_eq!(cycled(3, None, 1), Some(0), "none active: the first");
        assert_eq!(cycled(3, None, -1), Some(2), "none active: the last");
        assert_eq!(cycled(0, None, 1), None);
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
