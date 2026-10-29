//! The window: fleet tree on the left, terminal tabs on the right.

use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap as StdHashMap, HashSet},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use collections::HashMap;
use futures::FutureExt as _;
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
use foreman_ui::{FrameProbe, button, term_element};
use gpui::{
    AnyElement, AnyWindowHandle, ClickEvent, Context, CursorStyle, DragMoveEvent, Entity, FocusHandle,
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
mod left;
mod manage;
mod menu;
pub(crate) mod notices;
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
/// A command for a new shell is typed once its output has been still this long, or at the limit.
const SHELL_SETTLED: Duration = Duration::from_millis(500);
const SHELL_SETTLE_LIMIT: Duration = Duration::from_secs(15);
/// Claude draws a frame per wakeup; a hand tab looks at its token file at most this often.
const HAND_CHECK_GAP: Duration = Duration::from_millis(250);
/// A hand tab with no terminal event looks at its token file this often anyway.
const HAND_EXIT_FALLBACK: Duration = Duration::from_secs(30);
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
    pub find: Option<term::Find>,
    /// What its program named it with OSC 0/2 ([`tab_card::sanitize`]d); the label while there is one.
    pub program_title: Option<String>,
    /// The claude session it runs, as its card names it, when the tab was opened for one.
    pub session: Option<String>,
    /// It runs `claude` from `+` or a resume in the tab.
    pub hand: bool,
    /// How a hand session reaches WorkState, as its card says: `Foreman token · <role> · <session>`.
    pub workstate: Option<String>,
    /// The `hands.json` key of its minted token.
    pub hand_key: Option<String>,
    /// The server it should be restarted for: its token is from another one.
    pub restart_to: Option<String>,
    /// Epoch milliseconds.
    pub opened_at: i64,
    /// The claude session found under a shell tab's pwsh ([`FleetView::resolve_tabs`]): its `sessionId`.
    pub claude: Option<String>,
    /// Woken on the terminal's output, title and end, and when the tab is removed.
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
            watchers: Vec::new(),
        }
    }

    fn wake_watchers(&self) {
        self.watchers.iter().for_each(Nudge::wake);
    }

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
    /// The workspace files that do not load; the others are listed and supervised.
    pub registry_skipped: Vec<String>,
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
    /// Supervise errors hidden with their × while the poll keeps reporting them (m5/37).
    supervise_dismissed: Vec<String>,
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
    /// Every terminal tab's font size (m5/41): `settings.toml` `terminal_font_size`, changed by
    /// Ctrl+= / Ctrl+- / Ctrl+0 / Ctrl+wheel in any terminal and by Settings → Appearance.
    pub term_font_size: Pixels,
    /// Every terminal tab's font family and ligatures (m5/47), from `settings.toml` and Settings → Appearance.
    pub term_font_family: SharedString,
    pub term_ligatures: bool,
    /// The font families the system has, read at start.
    pub installed_fonts: Arc<Vec<String>>,
    /// The pending debounced write of `term_font_size` ([`term::FONT_SAVE_AFTER`]).
    font_save: Option<gpui::Task<()>>,
    /// A touchpad's Ctrl+wheel pixels short of a font step ([`term::zoom_steps`]).
    zoom_carried: f32,
    pub(crate) settings_window: Option<WindowHandle<SettingsView>>,
    /// `session_id` → when this app last saw it alive.
    last_alive: StdHashMap<String, i64>,
    /// Per workspace that wants a relay: live, armed.
    pub relays: BTreeMap<String, RelayStatus>,
    /// Per workspace, what its relay received (oldest first).
    pub messages: BTreeMap<String, Vec<OutboxMessage>>,
    /// `<workspace>/<file>` of the messages shown in full.
    pub expanded: HashSet<String>,
    /// Messages hidden with their × (m5/36), kept in `<FOREMAN_HOME>`.
    pub dismissed_messages: DismissedMessages,
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
    /// The next Add workspace takes this folder instead of asking the OS picker (the selftest).
    pub pick_folder: Option<PathBuf>,
    /// Whether launched sessions get WorkState (m5/3), checked off the UI thread every
    /// [`workstate_setup::RECHECK_EVERY`] and on [`Self::recheck_workstate`].
    pub workstate: WsStatus,
    /// The problem whose notice was dismissed; a different problem shows again.
    workstate_dismissed: Option<String>,
    workstate_recheck: Nudge,
    /// The last Start's warning that its sessions run without WorkState.
    pub start_warning: Option<String>,
    /// The server a sign-in is waiting on (m5/43): its browser page is open.
    pub signing_in: Option<String>,
    /// Why the last sign-in did not work.
    pub sign_in_error: Option<String>,
    /// The `+` sessions' own WorkState tokens (m5/20); `None` without a Foreman home.
    hands: Option<Arc<Mutex<Hands>>>,
    /// What went wrong minting or revoking them.
    pub hand_notices: Vec<String>,
    /// Quit's revoke of every hand token has ended, or ran out of [`QUIT_RELEASE_WITHIN`].
    hands_released: bool,
    quit_release: Option<gpui::Task<()>>,
    /// Start and Stop, written one after another off the UI thread ([`Self::write_desired`]).
    desired_writes: futures::channel::mpsc::UnboundedSender<DesiredWrite>,
    /// The server open hand tabs were last checked against ([`Self::sync_server`]).
    synced_server: Option<String>,
    /// Servers whose grant this run revoked or tried to ([`Self::leave_servers`]).
    left_servers: std::collections::HashSet<String>,
    /// [`supervise::Report::left_servers`] of the last poll, for [`Self::leave_servers`].
    left_pending: Vec<String>,
    /// Closed sessions from their transcripts (m5/27), as stopped interactive rows.
    pub closed: Vec<Session>,
    /// [`main_repo`] of each closed session's cwd, merged into `main_repos` after every poll.
    closed_repos: StdHashMap<PathBuf, Option<PathBuf>>,
    /// Transcript scans finished.
    pub transcript_scans: u64,
    /// Scans the transcripts again now (a star, a closed interactive session, Refresh, the selftest).
    pub transcripts_nudge: Nudge,
    /// Polls `claude agents` now instead of at the end of [`REFRESH_EVERY`] (Refresh).
    poll_nudge: Nudge,
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
    /// A [`Self::resolve_tabs`] runs; another is asked for while it does.
    tabs_resolving: bool,
    tabs_again: bool,
    /// [`Self::resolve_tabs`] runs finished.
    pub tab_resolves: u64,
    /// The last quiet feedback (m5/34): it overlays the tree instead of pushing it down.
    pub toast: Option<Toast>,
    /// Where the transcripts are read, and the confirmed ones deleted (m5/38): `FOREMAN_CLAUDE_DIR`, else claude's.
    pub claude_dir: Option<PathBuf>,
    /// Session ids deleted from here → when, kept out of the tree until polls catch up ([`delete::HIDE_DELETED_MS`]).
    pub deleted: StdHashMap<String, i64>,
    /// Session ids whose delete runs.
    pub deleting: HashSet<String>,
}

/// A short note over the bottom of the left panel, gone after [`TOAST_FOR`].
pub struct Toast {
    pub text: String,
    pub shown_at: Instant,
}

pub const TOAST_FOR: Duration = Duration::from_millis(2500);

/// How long Quit waits on the hand tokens' revokes before it closes anyway; the next start revokes the rest.
const QUIT_RELEASE_WITHIN: Duration = Duration::from_secs(5);

enum DesiredWrite {
    Start { home: PathBuf, plugin_dir: Option<PathBuf>, workspace: Workspace },
    Stop { home: PathBuf, name: String },
}

/// Every workspace file that loads, and why each other one does not.
fn load_registry(registry: Result<Registry, String>) -> Result<(Vec<Workspace>, Vec<String>), String> {
    let (workspaces, failed) = registry?.load_all().map_err(|e| e.to_string())?;
    Ok((workspaces, failed.iter().map(ToString::to_string).collect()))
}

/// A session with no process, not the supervisor's (`fm-`: Start resumes it); a background one only when
/// this app has not seen it alive in the last [`RESUME_GRACE_MS`], an interactive one at once.
pub fn offers_resume(s: &Session, health: &foreman_claude::Health, last_alive: Option<i64>, now: i64) -> bool {
    !fleet::is_alive(health)
        && !s.session_id.is_empty()
        && !s.name.starts_with(FM_PREFIX)
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
    workspaces: Result<(Vec<Workspace>, Vec<String>), String>,
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
        let workspaces = load_registry(Registry::from_env().map_err(|e| e.to_string()));
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
            (Ok((workspaces, _)), Some(Ok(sessions)), Some(supervision)) => {
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
        let loaded = home.as_deref().and_then(|h| Settings::load(h).ok());
        let left_width = loaded
            .as_ref()
            .and_then(|s| s.left_width)
            .map_or(DEFAULT_LEFT_WIDTH, |w| px(w as f32).max(MIN_LEFT_WIDTH));
        let loaded = loaded.unwrap_or_default();
        let term_font_size = px(loaded.terminal_font_size() as f32);
        let installed_fonts: Arc<Vec<String>> = Arc::new(cx.text_system().all_font_names());
        let term_font_family = SharedString::from(loaded.terminal_font_family(&installed_fonts));
        let term_ligatures = loaded.terminal_ligatures();
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
                    let left = std::mem::take(&mut this.left_pending);
                    this.leave_servers(left, cx);
                    this.resolve_tabs(cx);
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
                let exited = std::pin::pin!(any_exit(&executor, pids, REFRESH_EVERY));
                futures::future::select(std::pin::pin!(woken.wait(&executor, REFRESH_EVERY)), exited).await;
            }
        })
        .detach();
        Self::blink_cursor(cx);
        let (workstate_recheck, recheck) = nudge::nudge();
        Self::watch_workstate(home.clone(), recheck, cx);
        let work_inbox = WorkInbox::shared(home.clone(), cx);
        cx.observe(&work_inbox, |_, _, cx| cx.notify()).detach();
        let hands = home.as_deref().map(|home| Self::open_hands(home, cx));
        let (stars, stars_notice) = Stars::load(home.as_deref());
        let (dismissed_messages, dismissed_notice) = DismissedMessages::load(home.as_deref());
        let (transcripts_nudge, transcripts_nudged) = nudge::nudge();
        let claude_dir = std::env::var_os(CLAUDE_DIR_VAR).filter(|d| !d.is_empty()).map(PathBuf::from).or_else(foreman_claude::claude_dir);
        Self::watch_transcripts(claude_dir.clone(), transcripts_nudged, cx);
        let desired_writes = Self::desired_writer(cx);
        // an input that held the focus went away (the Add review closed): the window's keys come back here
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
            answered: BTreeMap::new(),
            answered_route: BTreeMap::new(),
            compose: None,
            work_live: Rc::default(),
            work_inbox,
            menu: None,
            confirm: None,
            adding: None,
            removing: BTreeMap::new(),
            pick_folder: None,
            workstate: WsStatus::Checking,
            workstate_dismissed: None,
            workstate_recheck,
            start_warning: None,
            signing_in: None,
            sign_in_error: None,
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
            action_error: stars_notice.or(dismissed_notice),
            stars,
            closed_expanded: HashSet::new(),
            claude_command: "claude".into(),
            search,
            search_marks: StdHashMap::new(),
            tabs_resolving: false,
            tabs_again: false,
            tab_resolves: 0,
            toast: None,
            claude_dir,
            deleted: StdHashMap::new(),
            deleting: HashSet::new(),
        }
    }

    /// Closed sessions from the transcripts of the registered repos (m5/27), read on the background
    /// executor every [`TRANSCRIPTS_EVERY`], at once when nudged (the workspaces changed, a star, Refresh).
    fn watch_transcripts(claude_dir: Option<PathBuf>, mut nudged: Nudged, cx: &mut Context<Self>) {
        let Some(claude_dir) = claude_dir else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let executor = cx.background_executor().clone();
            let mut scanner = Transcripts::in_claude_dir(&claude_dir);
            let mut repos: StdHashMap<PathBuf, Option<PathBuf>> = StdHashMap::new();
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
                        this.drop_deleted();
                        this.transcript_scans += 1;
                        cx.notify();
                    })
                    .is_err()
                {
                    return;
                }
                nudged.wait(&executor, TRANSCRIPTS_EVERY).await;
            }
        })
        .detach();
    }

    /// Polls `claude agents` and scans the transcripts now (m5/28): the action bar's Refresh, and F5 while no
    /// terminal tab is active (a terminal's program gets its F5).
    pub fn refresh_now(&mut self, cx: &mut Context<Self>) {
        self.refresh_asks += 1;
        self.poll_nudge.wake();
        self.transcripts_nudge.wake();
        cx.notify();
    }

    /// Stars or unstars a session (m5/27); a starred one stays in the tree, stopped or closed.
    pub fn toggle_star(&mut self, session_id: &str, cx: &mut Context<Self>) {
        match self.stars.toggle(session_id) {
            Ok(_) => self.transcripts_nudge.wake(),
            Err(e) => self.action_error = Some(format!("Cannot save the star: {e}")),
        }
        cx.notify();
    }

    /// Hides a message here (m5/36); its outbox file stays.
    pub fn dismiss_message(&mut self, workspace: &str, file: &str, cx: &mut Context<Self>) {
        if let Err(e) = self.dismissed_messages.dismiss(workspace, file) {
            self.action_error = Some(format!("Cannot save the dismissed message: {e}"));
        }
        self.expanded.remove(&format!("{workspace}/{file}"));
        cx.notify();
    }

    /// Revokes the hand tokens an earlier run left, off the UI thread. Quit revokes the live ones
    /// ([`Self::quit_step`]); any other end of the app tries within gpui's shutdown wait, and the next start
    /// revokes what is left.
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

    /// [`workstate_setup::detect`] on the background executor at start, every
    /// [`workstate_setup::RECHECK_EVERY`] and at once when nudged.
    fn watch_workstate(home: Option<PathBuf>, mut recheck: Nudged, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let executor = cx.background_executor().clone();
            loop {
                let home = home.clone();
                let status = executor
                    .spawn(async move {
                        let auth = home.as_deref().map(crate::signin::auth);
                        workstate_setup::detect(home.as_deref(), auth.as_deref())
                    })
                    .await;
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
                });
                if alive.is_err() {
                    return;
                }
                recheck.wait(&executor, workstate_setup::RECHECK_EVERY).await;
            }
        })
        .detach();
    }

    /// Check WorkState again now (Retry, a changed `.env`), off the UI thread.
    pub fn recheck_workstate(&self) {
        self.workstate_recheck.wake();
    }

    /// Sign in to the server `settings.toml` names (the notice's and Settings' Sign in).
    pub fn sign_in(&mut self, cx: &mut Context<Self>) {
        let Some(home) = self.home.clone() else {
            return;
        };
        let origin = crate::signin::auth(&home).configured_origin();
        self.sign_in_to(origin, None, cx);
    }

    /// Signs in to `origin` off the UI thread: the browser opens WorkState's sign-in page and the
    /// loopback waits up to 5 minutes. With `from` (Settings changed the server URL) the URL is saved
    /// only once that worked, and `from`'s grant goes. Either way [`Self::sync_server`] follows.
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

    /// Stops a sign-in that is waiting on the browser.
    pub fn cancel_sign_in(&self) {
        if let Some(home) = &self.home {
            crate::signin::auth(home).cancel_sign_in();
        }
    }

    /// Revokes the grant (every `wss_` token minted under it ends with it), deletes the credential and
    /// drops the held tokens; a revoke that failed is a notice.
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

    /// Settings' server URL. While signed in, the new server is used only after a sign-in there
    /// ([`Self::sign_in_to`]); otherwise the URL is saved. The sign-in is looked up off the UI thread, so a
    /// save that fails after that is an action error.
    pub fn set_workstate_url(&mut self, text: &str, cx: &mut Context<Self>) -> Result<(), String> {
        let home = self.home.clone().ok_or("No Foreman home: settings cannot be saved.")?;
        let origin = foreman_core::validate_workstate_url(text)?;
        let current = Settings::load(&home).map_err(|e| format!("Cannot read settings.toml: {e}"))?.workstate_origin();
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

    /// Signed in on `origin`, at start or after a sign-in: every open claude tab whose token is from
    /// another server is told to restart, and the grants of the servers left go. The supervisors re-mint
    /// their own sessions ([`foreman_supervisor::Supervisor::server_changed`]).
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

    /// Revokes the grant of each of `origins` that is not the server in use, once per run: every token
    /// minted under it goes with it. One that cannot be reached costs one notice, off the UI thread.
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

    /// The WorkState notice's problem, unless it was dismissed.
    pub fn workstate_notice(&self) -> Option<String> {
        self.workstate.problem().filter(|p| self.workstate_dismissed.as_ref() != Some(p))
    }

    fn apply(&mut self, refresh: Refresh) {
        match refresh.workspaces {
            // read before a write from here, so older than the list `reload_workspaces` showed after it
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
        // A failed read keeps the last good list on screen, flagged as stale.
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
            notices::keep_dismissed(&mut self.supervise_dismissed, &self.supervise_errors);
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

    /// Runs Start and Stop in the order they were asked for, each on the background executor:
    /// [`foreman_supervisor::DesiredState::update`] waits on the workspace's lock file.
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

    /// The task box's keys: Ctrl+Enter sends, Esc closes it and gives the keys back to the window.
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

    /// The box's Ctrl+Enter: to `fm-<w>-workspace` as `fm send` does (direct, else the relay). A task
    /// clears the box on success; a reply marks its question answered and closes it. A refusal or a
    /// failure stays in the box.
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
        let id = self.open_tab(title, TabKind::Shell, pwsh(self.shell_args.clone()), Some(cwd), window, cx);
        let (watch, mut output) = nudge::nudge();
        if let Some(tab) = self.tab_mut(id) {
            tab.watchers.push(watch);
        }
        // Typed once the shell's output has settled (its prompt is up, whatever the profile makes
        // it look like), or after SHELL_SETTLE_LIMIT regardless.
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
        if let Err(e) = Settings::update(home, |settings| settings.left_width = Some(width)) {
            self.action_error = Some(format!("Cannot save the panel width: {e}"));
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
        let title = format!("{workspace} · {repo} · {}", fleet::display_name(session));
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
            self.open_tab(title, kind, shell, Some(session.cwd.clone()), window, cx)
        } else {
            // through the `+` launch (m5/27): its own WorkState token, and the shell stays when claude exits
            let resume = format!("{} --resume '{}'", self.claude_command, sid.replace('\'', "''"));
            let own = (!session.name.is_empty()).then(|| session.name.clone());
            let id = self.hand_claude(session.cwd.clone(), title, resume, own, Some(sid.clone()), cx);
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
        self.hand_claude(cwd, title, claude, None, None, cx)
    }

    /// [`Self::new_claude`] running `claude` (pwsh's program and arguments); `own` is the session's
    /// name in the tree, for its WorkState session (m5/35); `resumes` the uuid of the session it resumes.
    fn hand_claude(&mut self, cwd: PathBuf, title: String, claude: String, own: Option<String>, resumes: Option<String>, cx: &mut Context<Self>) -> u64 {
        let id = self.push_tab(title, TabKind::Shell, Some(cwd.clone()), cx);
        let (watch, mut changed) = nudge::nudge();
        if let Some(tab) = self.tab_mut(id) {
            tab.hand = true;
            tab.watchers.push(watch);
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
                if let Launch::Minted { name, session, notice, .. } = &launch {
                    if let Some(tab) = this.tab_mut(id) {
                        tab.workstate = Some(format!("Foreman token · {} · {session}", crate::hand::ROLE));
                        tab.hand_key = Some(name.clone());
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

    /// With [`Self::daemon_hold`] the tab says first why its claude cannot reach the background service.
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

    /// Finds the claude session under every shell tab's pwsh off the UI thread (m5/33), on a title
    /// change, a terminal's end and every refresh tick. One run at a time; asked for while one runs,
    /// it runs once more after it.
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
                // a claude just started in a tab: its row comes with the next `claude agents`, so poll now
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

    /// Whether `tab` runs `session`: an attach tab by its id, a shell tab by the claude under it.
    pub fn tab_runs(tab: &Tab, session: &Session) -> bool {
        match &tab.kind {
            TabKind::Attach { session_id } => *session_id == session.id,
            TabKind::Shell => tab.claude.as_deref() == Some(session.session_id.as_str()),
            TabKind::Work(_) => false,
        }
    }

    /// The row the active tab runs is marked in the tree (m5/33).
    pub fn is_marked(&self, session: &Session) -> bool {
        self.active.and_then(|id| self.tab(id)).is_some_and(|t| Self::tab_runs(t, session))
    }

    /// A tab that runs `session`.
    pub fn session_tab(&self, session: &Session) -> Option<u64> {
        self.tabs.iter().find(|t| Self::tab_runs(t, session)).map(|t| t.id)
    }

    fn cycle_tab(&mut self, step: isize, cx: &mut Context<Self>) {
        let at = self.active.and_then(|id| self.tabs.iter().position(|t| t.id == id));
        if let Some(next) = cycled(self.tabs.len(), at, step) {
            self.activate_tab(self.tabs[next].id, cx);
        }
    }

    /// The window's keys are this view's: no task box or text input holds them.
    pub fn has_keys(&self, window: &Window) -> bool {
        self.focus.is_focused(window)
    }

    pub fn attach(
        &mut self,
        workspace: &str,
        repo: &str,
        session: &Session,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> u64 {
        let shell = util::shell::Shell::WithArguments {
            program: "claude".into(),
            args: vec!["attach".into(), session.id.clone()],
            title_override: None,
        };
        self.open_tab(
            format!("{workspace} · {repo} · {}", fleet::display_name(session)),
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
        self.tabs.push(Tab::new(id, format!("Work · {workspace}"), TabKind::Work(view), cwd));
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
            workstate: tab.workstate.clone().map(|w| match &tab.restart_to {
                Some(to) => format!("{w} · restart to use {to}"),
                None => w,
            }),
            started,
        }
    }

    /// A shell tab in `cwd`.
    pub fn open_shell_in(&mut self, cwd: PathBuf, title: String, window: &mut Window, cx: &mut Context<Self>) -> u64 {
        self.open_tab(title, TabKind::Shell, pwsh(self.shell_args.clone()), Some(cwd), window, cx)
    }

    pub fn new_shell(&mut self, window: &mut Window, cx: &mut Context<Self>) -> u64 {
        let title = format!("pwsh {}", self.next_tab_id);
        let cwd = self.new_shell_cwd();
        self.open_tab(title, TabKind::Shell, pwsh(self.shell_args.clone()), cwd, window, cx)
    }

    /// [`fleet::shell_cwd`] for the active tab, never the exe's folder.
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

    /// The active tab, its terminal not started yet ([`Self::start_terminal`]).
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
                                // claude names the console when it starts, idles, works and exits (m5/29)
                                TerminalEvent::TitleChanged => {
                                    this.resolve_tabs(cx);
                                    cx.notify();
                                }
                                TerminalEvent::BreadcrumbsChanged => {
                                    let title = emitter.read(cx).breadcrumb_text.clone();
                                    if let Some(tab) = this.tab_mut(id) {
                                        tab.program_title = tab_card::sanitize(&title);
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

    /// While quitting: once no Attach tab is left, every hand token is revoked off the UI thread for at most
    /// [`QUIT_RELEASE_WITHIN`]. `true` once both are done and the window may close.
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

    /// The Inbox's Restart as administrator (m5/25): Settings → Run Foreman as administrator on, then an
    /// elevated copy (UAC) that waits for this one to quit. Declined, this one stays and says so.
    pub fn restart_elevated(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(home) = self.home.clone() else {
            return;
        };
        if let Err(e) = Settings::update(&home, |settings| settings.run_as_admin = true) {
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
    /// Attach tabs open detaches them first, then the hand tokens are revoked ([`Self::quit_step`]); the
    /// window closes after both, or stays open if a detach fails.
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

    /// Whether one of the text inputs (the search box, the Add review's name) has the keys.
    pub fn input_focused(&self, window: &Window, cx: &gpui::App) -> bool {
        self.search.read(cx).is_focused(window)
            || self.adding.as_ref().is_some_and(|d| d.name.read(cx).is_focused(window))
            || self.compose.as_ref().is_some_and(|c| c.input.read(cx).is_focused(window))
            || self.tabs.iter().filter_map(|t| t.find.as_ref()).any(|f| f.input.read(cx).is_focused(window))
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
            InputEvent::Cancel if self.dismiss_popup() => cx.notify(),
            InputEvent::Cancel if !self.search.read(cx).text().is_empty() => self.clear_search(cx),
            InputEvent::Cancel => self.focus.focus(window, cx),
            InputEvent::Confirm | InputEvent::ConfirmBack | InputEvent::Refused(_) => {}
        }
    }

    /// Esc on an open menu closes it, else on the question closes that; `false` when neither is open.
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
        // An input's own keys ran as its actions already. What is left must go on unhandled: Windows turns
        // a key into its character (WM_CHAR, a dead key's too) only after nothing took the key down.
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
                self.save_add(cx);
            } else {
                self.adding = None;
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

/// Done once one of `pids` exits within `timeout`; never done otherwise.
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

/// The open hand tabs whose token is one of `elsewhere`'s (minted on another server).
fn stale_tabs(tabs: &[Tab], elsewhere: &[(String, Option<String>)]) -> Vec<u64> {
    tabs.iter()
        .filter(|t| t.hand && !t.exited)
        .filter(|t| t.hand_key.as_ref().is_some_and(|k| elsewhere.iter().any(|(n, _)| n == k)))
        .map(|t| t.id)
        .collect()
}

/// The pwsh line a tab runs in place of a `claude --bg` that cannot reach the background service.
fn held_line(why: &str) -> String {
    format!("Write-Host -ForegroundColor Yellow '{}'", why.replace('\'', "''"))
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
                    .child(button::icon(&self.hits, close_key, icon::Icon::Close.svg(14., colors.text_muted), px(18.), cx).on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            cx.stop_propagation();
                            this.close_tab(id, cx);
                        },
                    )))
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
                        .flex_1()
                        .min_h_0()
                        .on_scroll({
                            // as Zed's TerminalView::scroll_wheel: the Terminal decides between
                            // scrollback, a mouse report and alternate-scroll arrow keys; Ctrl+wheel
                            // zooms every terminal (m5/41)
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
                        .child(term_element::terminal_grid(terminal.clone(), self.term_font())),
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
            .font_family(foreman_ui::fonts::UI)
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

/// Tab id → the `sessionId` of the claude under its shell pid, from one process snapshot.
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
        assert!(!FleetView::tab_runs(&tab(TabKind::Shell, None), &session), "a shell without claude runs nothing");
    }

    /// A-46: one workspace file that does not parse leaves the others listed, and is named.
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
    Settings::update(home, |settings| settings.workstate_url = Some(origin.to_string()))
        .map(drop)
        .map_err(|e| format!("Cannot save settings.toml: {e}"))
}
