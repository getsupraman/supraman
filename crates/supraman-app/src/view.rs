use std::{
    collections::{BTreeMap, HashMap as StdHashMap, HashSet},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use futures::FutureExt as _;
use supraman_claude::{ClaudeVersion, Sent, Session, list_sessions, now_ms, send_from_human};
use supraman_core::{
    Event, EventReader, FM_PREFIX, Workspace, Registry, Settings, mark_answered,
};
use supraman_supervisor::{DesiredState, workspace_session_name};
use supraman_term::{
    MouseInputMode, Terminal, TerminalBuilder, TerminalMode,
    terminal_settings::{AlternateScroll, CursorShape},
};
use supraman_ui::nudge::{self, Nudge, Nudged};
use supraman_ui::open::{self, Link};
use supraman_ui::timeline;
use supraman_ui::hit::{HitTargets, hit_target};
use supraman_ui::text_input::{InputEvent, TextInput};
use supraman_ui::hover_card;
use supraman_ui::icon::Icon;
use supraman_ui::split::SplitPane;
use supraman_ui::tabs::{TabHeader, tab_strip};
use supraman_ui::toast::ToastSlot;
use supraman_ui::tooltip::Tooltip;
use supraman_ui::{FrameProbe, button, term_element};
use gpui::{
    AnyElement, AnyWindowHandle, App, ClickEvent, Context, Entity, FocusHandle,
    KeyBinding, KeyDownEvent, ModifiersChangedEvent, MouseButton, MouseDownEvent, Pixels, Render, ScrollEvent, SharedString, Window,
    WindowHandle, div, prelude::*, px,
};
use theme::ActiveTheme as _;
use util::paths::PathStyle;

use crate::compose::{self, Compose};
use crate::fleet::{self, HookSignal, SessionRow, Supervised, Tree};
use crate::hand::Extras;
use crate::settings_window::{self, SettingsView};
use crate::stars::Stars;
use crate::supervise::{self, Report, Supervision};
use crate::tab_card;
use crate::transcripts_load::TranscriptsLoad;
use crate::work_view::{WorkHub, WorkInbox, WorkView};
use crate::workman_setup;
use supraman_workman::status::WorkmanStatus;

mod delete;
pub(crate) mod details;
mod drop;
mod folder_list;
mod folders;
pub(crate) mod git_tab;
mod hand_tab;
mod left;
mod left_width;
mod lifecycle;
mod links;
mod mail;
mod manage;
mod menu;
pub(crate) use supraman_git_view::occupancy;
pub(crate) mod notices;
mod polls;
mod reopen;
mod results;
mod right;
pub(crate) mod shells;
mod sidebar;
use sidebar::CachedTree;
pub(crate) mod schedules;
mod schedule_state;
mod startup;
pub(crate) mod start;
pub(crate) mod tab_keys;
mod tab_saving;
mod supervising;
mod term;
mod term_look;
mod work_tab;
mod workman_url;
mod workman_state;
pub(crate) mod workspace_dialog;
pub(crate) mod writes;

#[cfg(test)]
use {links::{link_target, missing}, supraman_term::MaybeNavigationTarget};

#[cfg(feature = "selftest")]
pub(crate) use left::grid;
pub use manage::Removing;
#[cfg(feature = "selftest")]
pub use menu::MenuAction;
pub use menu::{Confirm, Confirmed, ContextMenu, MenuTarget};

gpui::actions!(supraman, [FocusSearch]);

const KEY_CONTEXT: &str = "FleetView";

pub fn bind_keys(cx: &mut gpui::App) {
    tab_keys::bind(cx);
    cx.bind_keys([KeyBinding::new("ctrl-shift-e", FocusSearch, Some(KEY_CONTEXT))]);
}

pub const REFRESH_EVERY: Duration = Duration::from_secs(2);
const TRANSCRIPTS_CACHE: &str = "cache/transcripts.json";
static CLAUDE_DIR_OVERRIDE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// A selftest's fake claude dir, read and confined to by Delete in place of [`supraman_core::paths::claude_dir`].
#[cfg(feature = "selftest")]
pub fn use_claude_dir(dir: PathBuf) {
    CLAUDE_DIR_OVERRIDE.set(dir).ok();
}

fn claude_dir_or(over: Option<&Path>) -> Option<PathBuf> {
    over.map(Path::to_path_buf).or_else(supraman_core::paths::claude_dir)
}
/// Ctrl+Z: `claude attach`'s own detach, which leaves the session running.
pub const DETACH_INPUT: &[u8] = b"\x1a";
pub const DETACH_WITHIN: Duration = Duration::from_secs(5);
const SHELL_SETTLED: Duration = Duration::from_millis(500);
const SHELL_SETTLE_LIMIT: Duration = Duration::from_secs(15);
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
    Git(Entity<crate::git_view::GitView>),
    Schedule(schedules::SchedulePage),
    Details(details::DetailsPage),
    Start(start::StartPage),
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
    /// The branch at `cwd`, read off the UI thread ([`FleetView::read_tab_branches`]).
    pub branch: Option<String>,
    pub find: Option<term::Find>,
    pub program_title: Option<String>,
    pub session: Option<String>,
    pub hand: bool,
    pub workman: Option<String>,
    pub hand_key: Option<String>,
    pub restart_to: Option<String>,
    pub opened_at: i64,
    pub claude: Option<String>,
    /// The full uuid it was launched or attached with, and that session's name.
    pub launched_session: Option<String>,
    pub session_name: Option<String>,
    pub asks_schedule: Option<start::AsksSchedule>,
    pub started: Option<crate::saved_tabs::SavedShell>,
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
            branch: None,
            find: None,
            program_title: None,
            session: None,
            hand: false,
            workman: None,
            hand_key: None,
            restart_to: None,
            opened_at: now_ms(),
            claude: None,
            launched_session: None,
            session_name: None,
            asks_schedule: None,
            started: None,
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
    pub show_stopped: bool,
    /// Set only by a selftest: see [`fleet::Supervised::own_prefix`].
    pub own_prefix: Option<String>,
    pub tabs: Vec<Tab>,
    pub active: Option<u64>,
    next_tab_id: u64,
    pub shell_args: Vec<String>,
    focus: FocusHandle,
    pub probe: FrameProbe,
    pub hits: HitTargets,
    pub(crate) window: AnyWindowHandle,
    pub home: Option<PathBuf>,
    pub plugin_dir: Option<PathBuf>,
    pub setup_notices: Vec<String>,
    pub action_error: Option<String>,
    pub(crate) settings_window: Option<WindowHandle<SettingsView>>,
    writes: writes::Writes,
    occupancy: occupancy::Live,
    pub work_inbox: Entity<WorkInbox>,
    pub menu: Option<ContextMenu>,
    pub confirm: Option<Confirm>,
    pub adding: Option<(Entity<workspace_dialog::WorkspaceDialog>, gpui::Subscription)>,
    pub removing: BTreeMap<String, Removing>,
    pub pick_folder: Option<PathBuf>,
    pub open_with: fn(&Path) -> Result<(), String>,
    pub open_link_with: fn(&Link, &mut App),
    pub workman: WorkmanStatus,
    pub hand_notices: Vec<String>,
    pub closed: Vec<Session>,
    /// A selftest's closed sessions, kept across transcript scans.
    pub injected_closed: Vec<Session>,
    closed_repos: StdHashMap<PathBuf, Option<PathBuf>>,
    pub transcripts: TranscriptsLoad,
    /// Every source of the tree has completed once (`loading`); until then the action bar says what is loading.
    pub loaded: bool,
    pub stars: Stars,
    pub claude_command: String,
    pub search: Entity<TextInput>,
    pub toast: ToastSlot,
    pub pane_toast: ToastSlot,
    pub updates: crate::updates::Updates,
    pub claude_dir: Option<PathBuf>,
    pub deleted: StdHashMap<String, i64>,
    pub deleting: HashSet<String>,
    painted_sessions: bool,
    pub saving: tab_saving::TabSaving,
    pub sidebar: sidebar::Sidebar,
    pub term_look: term_look::TermLook,
    pub workman_state: workman_state::WorkmanState,
    lifecycle: lifecycle::Lifecycle,
    pub polls: polls::Polls,
    pub supervising: supervising::Supervising,
    pub mail: mail::Mail,
    pub scheduling: schedule_state::ScheduleState,
    pub(crate) tab_keys: Entity<tab_keys::TabKeyState>,
}


const QUIT_RELEASE_WITHIN: Duration = Duration::from_secs(5);

enum DesiredWrite {
    Start { home: PathBuf, plugin_dir: Option<PathBuf>, workspace: Workspace },
    Stop { home: PathBuf, name: String },
}

type RegistryRead = Result<(Vec<Workspace>, Vec<String>), String>;

fn load_registry(registry: Result<Registry, String>) -> RegistryRead {
    let (workspaces, failed) = registry?.load_all().map_err(|e| e.to_string())?;
    Ok((workspaces, failed.iter().map(ToString::to_string).collect()))
}

/// A poll's registry read, or `None` when a registry write from this window came after it.
fn fresh_registry(read: RegistryRead, read_at: u64, writes: u64) -> Option<RegistryRead> {
    match read {
        Ok(_) if read_at != writes => None,
        read => Some(read),
    }
}

pub fn offers_resume(s: &Session, health: &supraman_claude::Health, last_alive: Option<i64>, now: i64) -> bool {
    !fleet::is_alive(health)
        && !s.session_id.is_empty()
        && !s.name.starts_with(FM_PREFIX)
        && (s.kind != supraman_claude::Kind::Background || last_alive.is_none_or(|t| now - t > RESUME_GRACE_MS))
}

struct Refresh {
    registry_writes: u64,
    workspaces: RegistryRead,
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
    snapshot: Option<(u64, Vec<Workspace>, Vec<Session>)>,
    first: bool,
    store: Option<Arc<dyn supraman_core::secret::SecretStore>>,
}

impl Poller {
    fn setup(&mut self) -> supervise::Setup {
        let Some(home) = self.home.clone() else {
            return supervise::Setup {
                plugin_dir: None,
                notices: vec!["No Supraman home (set SUPRAMAN_HOME or APPDATA): nothing can be started.".into()],
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
    fn tick(&mut self) -> Option<Report> {
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
        let defaults = Settings::default();
        let (polls, transcripts_nudged) = polls::Polls::start(home.clone(), cx);
        Self::load_settings(home.clone(), cx);
        Self::load_stores(home.clone(), cx);
        let term_look = term_look::TermLook::new(&defaults.terminal, cx);
        let (workman_recheck, recheck) = nudge::nudge();
        Self::watch_workman(home.clone(), recheck, cx);
        let work_inbox = WorkInbox::shared(home.clone(), cx);
        cx.observe(&work_inbox, |this, _, cx| {
            this.check_loaded(cx);
            cx.notify();
        })
        .detach();
        let hands = home.as_deref().map(|home| Self::open_hands(home, cx));
        let scheduling = schedule_state::ScheduleState::new(home.as_deref());
        let claude_dir = claude_dir_or(CLAUDE_DIR_OVERRIDE.get().map(PathBuf::as_path));
        Self::watch_transcripts(claude_dir.clone(), home.as_deref().map(|h| h.join(TRANSCRIPTS_CACHE)), transcripts_nudged, cx);
        let desired_writes = Self::desired_writer(cx);
        let updates = crate::updates::Updates::start(home.clone(), cx);
        let tabs_writes = Self::tabs_writer(home.clone(), cx);
        Self::detect_shells(cx);
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
            show_stopped: false,
            own_prefix: None,
            tabs: Vec::new(),
            active: None,
            next_tab_id: 1,
            shell_args: vec!["-NoLogo".into()],
            focus,
            probe: FrameProbe::default(),
            hits: HitTargets::default(),
            window: window.window_handle(),
            home,
            plugin_dir: None,
            setup_notices: Vec::new(),
            settings_window: None,
            writes: writes::Writes::default(),
            occupancy: occupancy::Live::default(),
            work_inbox,
            menu: None,
            confirm: None,
            adding: None,
            removing: BTreeMap::new(),
            pick_folder: None,
            open_with: |path| open::open(path.as_os_str()),
            open_link_with: |link, cx| match link {
                Link::Url(url) => open::open_off_thread(url.clone(), cx).detach(),
                Link::Path(path) => open::open_off_thread(path.clone(), cx).detach(),
            },
            workman: WorkmanStatus::Checking,
            hand_notices: Vec::new(),
            closed: Vec::new(),
            injected_closed: Vec::new(),
            closed_repos: StdHashMap::new(),
            transcripts: TranscriptsLoad::new(claude_dir.is_some()),
            loaded: false,
            action_error: None,
            stars: Stars::default(),
            claude_command: "claude".into(),
            search,
            toast: ToastSlot::default(),
            pane_toast: ToastSlot::default(),
            updates,
            claude_dir,
            deleted: StdHashMap::new(),
            deleting: HashSet::new(),
            painted_sessions: false,
            saving: tab_saving::TabSaving::new(tabs_writes),
            sidebar: sidebar::Sidebar::new(cx),
            term_look,
            workman_state: workman_state::WorkmanState::new(workman_recheck),
            lifecycle: lifecycle::Lifecycle::new(hands),
            polls,
            supervising: supervising::Supervising::new(desired_writes),
            mail: mail::Mail::default(),
            scheduling,
            tab_keys: cx.new(|_| tab_keys::TabKeyState::new(defaults.window.tab_keys())),
        }
    }

    /// The closed sessions a transcript scan found, without the ones a known session continues; their stars move on.
    fn set_closed(&mut self, closed: Vec<Session>, continued: StdHashMap<String, String>, cx: &mut Context<Self>) {
        let save = self.stars.follow(&continued);
        self.save_off_thread(save, "Cannot save the starred sessions", cx);
        self.closed = fleet::without_continued(closed, &continued, &self.sessions);
        self.transcripts.continued = continued;
        self.closed.extend(self.injected_closed.iter().cloned());
        self.drop_deleted();
        self.invalidate_tree();
    }

    /// Until every source has loaded once, and from a Refresh until the poll and the transcript scan it woke are applied.
    pub fn refreshing(&self) -> bool {
        !self.loaded || self.polls.refreshes_answered < self.polls.refresh_asks || (self.claude_dir.is_some() && self.polls.transcripts_answered < self.polls.refresh_asks)
    }

    /// The sources a fresh start still waits for, as the action bar names them; empty once all have loaded.
    pub fn loading(&self, cx: &gpui::App) -> Vec<String> {
        if self.loaded {
            return Vec::new();
        }
        let mut pending = Vec::new();
        if !self.polls.sessions_read {
            pending.push("session list".to_string());
        } else if self.polls.refreshes == 0 {
            pending.push("supervisor".to_string());
        }
        pending.extend(self.transcripts.pending());
        let inbox = self.work_inbox.read(cx);
        if matches!(self.workman, WorkmanStatus::Checking) {
            pending.push("Workman".to_string());
        } else if inbox.reads > 0 && inbox.applied == 0 {
            pending.push("Workman inbox".to_string());
        }
        pending
    }

    fn check_loaded(&mut self, cx: &gpui::App) {
        if !self.loaded && self.loading(cx).is_empty() {
            self.loaded = true;
            timeline("every source done", Duration::ZERO);
        }
    }

    pub fn refresh_now(&mut self, cx: &mut Context<Self>) {
        if self.refreshing() {
            return;
        }
        self.polls.refresh_asks += 1;
        self.polls.poll_nudge.wake();
        self.polls.transcripts_nudge.wake();
        cx.notify();
    }

    pub fn toggle_star(&mut self, session_id: &str, cx: &mut Context<Self>) {
        let (_, save) = self.stars.toggle(session_id);
        self.invalidate_tree();
        self.polls.transcripts_nudge.wake();
        self.save_off_thread(save, "Cannot save the star", cx);
        cx.notify();
    }

    pub fn dismiss_message(&mut self, workspace: &str, file: &str, cx: &mut Context<Self>) {
        let save = self.mail.dismissed_messages.dismiss(workspace, file);
        self.save_off_thread(save, "Cannot save the dismissed message", cx);
        self.sidebar.expanded.remove(&format!("{workspace}/{file}"));
        cx.notify();
    }

    pub fn save_off_thread(&self, save: Option<supraman_core::store::Save>, failed: &'static str, cx: &mut Context<Self>) {
        supraman_ui::store::save_off_thread(save, cx, move |this, saved, cx| {
            if let Err(e) = saved {
                this.action_error = Some(format!("{failed}: {e}"));
                cx.notify();
            }
        });
    }

    fn watch_workman(home: Option<PathBuf>, mut recheck: Nudged, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let executor = cx.background_executor().clone();
            let mut first = Some(Instant::now());
            loop {
                let home = home.clone();
                let status = executor
                    .spawn(async move {
                        let auth = home.as_deref().map(crate::signin::auth);
                        workman_setup::detect(auth.as_deref())
                    })
                    .await;
                if let Some(started) = first.take() {
                    timeline("workman status done", started.elapsed());
                }
                let alive = this.update(cx, |this, cx| {
                    if this.workman != status {
                        if matches!(status, WorkmanStatus::Ready { .. }) {
                            this.workman_state.start_warning = None;
                        }
                        this.workman = status;
                        cx.notify();
                    }
                    if let WorkmanStatus::Ready { url, .. } = &this.workman
                        && this.workman_state.synced_server.as_ref() != Some(url)
                    {
                        let url = url.clone();
                        this.sync_server(url, cx);
                    }
                    this.check_loaded(cx);
                });
                if alive.is_err() {
                    return;
                }
                recheck.wait(&executor, supraman_workman::status::RECHECK_EVERY).await;
            }
        })
        .detach();
    }

    pub fn recheck_workman(&self) {
        self.workman_state.workman_recheck.wake();
    }

    pub fn sign_in(&mut self, cx: &mut Context<Self>) {
        let Some(origin) = self.workman.checked_url().map(str::to_string) else {
            return;
        };
        self.sign_in_to(origin, None, cx);
    }

    pub fn sign_in_to(&mut self, origin: String, from: Option<String>, cx: &mut Context<Self>) {
        let Some(home) = self.home.clone() else {
            return;
        };
        if self.workman_state.signing_in.is_some() {
            return;
        }
        self.workman_state.signing_in = Some(origin.clone());
        self.workman_state.sign_in_error = None;
        let task = {
            let (home, origin) = (home.clone(), origin.clone());
            cx.background_executor().spawn(async move { crate::signin::sign_in(&home, &origin) })
        };
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.workman_state.signing_in = None;
                match result {
                    Ok(_) => {
                        this.workman_state.left_servers.remove(&origin);
                        if let Some(from) = from {
                            this.save_workman_url(home.clone(), origin.clone(), Some(from), cx);
                        }
                        this.workman_state.synced_server = None;
                    }
                    Err(e) => this.workman_state.sign_in_error = Some(format!("Signing in to {origin} did not work: {e}")),
                }
                this.workman = WorkmanStatus::Checking;
                this.recheck_workman();
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
        let (auth, hands) = (crate::signin::auth(&home), self.lifecycle.hands.clone());
        let task = cx.background_executor().spawn(async move { supervise::sign_out(&auth, hands) });
        cx.spawn(async move |this, cx| {
            let notices = task.await;
            this.update(cx, |this, cx| {
                this.hand_notices.extend(notices);
                this.workman = WorkmanStatus::Checking;
                this.recheck_workman();
                WorkHub::reconfigure(Some(home), cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn dismiss_workman_notice(&mut self, cx: &mut Context<Self>) {
        self.workman_state.workman_dismissed = self.workman.problem();
        cx.notify();
    }

    pub fn workman_notice(&self) -> Option<String> {
        self.workman.problem().filter(|p| self.workman_state.workman_dismissed.as_ref() != Some(p))
    }

    fn apply(&mut self, refresh: Refresh, cx: &mut Context<Self>) {
        let _stall = supraman_ui::stall::section("poll: apply");
        let before = (self.workspaces.clone(), self.sessions.clone(), self.main_repos.clone(), self.supervising.hooks.clone(), self.supervising.desired.clone(), self.polls.polled_at);
        match fresh_registry(refresh.workspaces, refresh.registry_writes, self.polls.registry_writes.load(Ordering::SeqCst)) {
            None => {}
            Some(Ok((workspaces, skipped))) => {
                if self.workspaces != workspaces {
                    self.polls.transcripts_nudge.wake();
                }
                self.workspaces = workspaces;
                self.registry_skipped = skipped;
                self.registry_error = None;
            }
            Some(Err(error)) => self.registry_error = Some(error),
        }
        match refresh.sessions {
            Some(Ok(sessions)) => {
                let now = now_ms();
                // an interactive session leaves `claude agents` when it exits; its transcript makes it a closed row
                let ended = self.sessions.iter().any(|old| {
                    old.kind == supraman_claude::Kind::Interactive && !sessions.iter().any(|s| s.session_id == old.session_id)
                });
                if ended {
                    self.polls.transcripts_nudge.wake();
                }
                for s in &sessions {
                    if fleet::is_alive(&s.health(now)) {
                        self.supervising.last_alive.insert(s.session_id.clone(), now);
                    }
                }
                self.occupancy.set_sessions(fleet::live_sessions(&sessions, now));
                self.sessions = sessions;
                self.drop_deleted();
                self.main_repos = refresh.main_repos;
                self.main_repos.extend(self.closed_repos.iter().map(|(k, v)| (k.clone(), v.clone())));
                self.sessions_error = None;
                self.polls.polled_at = refresh.polled_at;
            }
            Some(Err(error)) => self.sessions_error = Some(error),
            None => {}
        }
        if !self.polls.sessions_read {
            self.polls.transcripts_nudge.wake();
        }
        self.polls.sessions_read = true;
        for event in refresh.events {
            HookSignal::record(&mut self.supervising.hooks, event);
        }
        if let Some(desired) = refresh.desired {
            self.supervising.desired = desired;
        }
        if let Some((schedules, errors)) = refresh.schedules {
            self.scheduling.schedules = schedules;
            self.scheduling.schedule_errors = errors;
            supraman_ui::notices::keep_dismissed(&mut self.scheduling.schedule_errors_dismissed, &self.scheduling.schedule_errors);
            let (_, save) = self.scheduling.waiting_dismissed.prune_keys(&crate::schedules::waiting_keys(&self.scheduling.schedules));
            self.save_off_thread(save, "Cannot save the dismissed approvals", cx);
            self.find_asked_schedules();
        }
        let (workspaces, sessions, main_repos, hooks, desired, polled_at) = &before;
        // a permission prompt newer than the last poll makes its row Blocked (fleet::with_hook)
        let prompted = self.polls.polled_at != *polled_at && self.supervising.hooks.values().any(|h| h.is_permission_prompt() && h.last.ts > (*polled_at).min(self.polls.polled_at));
        if prompted || self.workspaces != *workspaces || self.sessions != *sessions || self.main_repos != *main_repos || self.supervising.hooks != *hooks || self.supervising.desired != *desired {
            self.invalidate_tree();
        }
        self.polls.last_refresh_took = Some(refresh.took);
    }

    fn apply_report(&mut self, report: Report, cx: &mut Context<Self>) {
        self.supervising.desired = report.desired;
        self.invalidate_tree();
        self.supervising.needs = report.needs;
        let (workspaces, desired, own) = (&self.workspaces, &self.supervising.desired, self.own_prefix.as_deref());
        self.supervising.needs.retain(|n| !fleet::is_foreign(&n.name, workspaces, Some(desired), own));
        if self.writes.max.report_counts() {
            self.supervising.max_working = report.max_working;
        }
        self.term_look.shell_default.ticked(report.terminal);
        self.supervising.supervise_errors = report.errors;
        supraman_ui::notices::keep_dismissed(&mut self.supervising.supervise_dismissed, &self.supervising.supervise_errors);
        self.supervising.daemon_hold = report.daemon_hold;
        self.workman_state.left_pending.extend(report.left_servers);
        self.supervising.last_attempt = report.last_attempt.into_iter().collect();
        self.mail.relays = report.relays;
        self.mail.messages = report.messages;
        let (_, save) = self.mail.dismissed_messages.prune(&self.mail.messages);
        self.save_off_thread(save, "Cannot save the dismissed messages", cx);
        for (workspace, on_disk) in report.answered {
            let answered = self.mail.answered.entry(workspace).or_default();
            for (file, line) in on_disk {
                answered.entry(file).or_insert(line);
            }
        }
        self.polls.ticks += 1;
    }

    /// The tree built once per change of its inputs; a mutation of one calls [`Self::invalidate_tree`].
    pub fn tree(&self) -> Rc<Tree> {
        let now = now_ms();
        if let Some(cached) = self.sidebar.tree_cache.borrow().as_ref()
            && cached.show_stopped == self.show_stopped
            && cached.own_prefix == self.own_prefix
            && cached.until.is_none_or(|until| now < until)
        {
            if tree_check::on() {
                tree_check::compare(&cached.tree, &self.build_tree(cached.at));
            }
            return cached.tree.clone();
        }
        tree_check::BUILDS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tree = Rc::new(self.build_tree(now));
        let until = fleet::next_change(&self.sessions, &self.closed, now);
        *self.sidebar.tree_cache.borrow_mut() = Some(CachedTree { at: now, until, show_stopped: self.show_stopped, own_prefix: self.own_prefix.clone(), tree: tree.clone() });
        tree
    }

    pub fn invalidate_tree(&self) {
        self.sidebar.tree_cache.borrow_mut().take();
    }

    fn build_tree(&self, now: i64) -> Tree {
        let _stall = supraman_ui::stall::section("tree");
        let sup = Supervised {
            desired: Some(&self.supervising.desired),
            hooks: Some(&self.supervising.hooks),
            polled_at: self.polls.polled_at,
            main_repos: Some(&self.main_repos),
            closed: &self.closed,
            starred: Some(self.stars.ids()),
            expanded: Some(&self.sidebar.closed_expanded),
            disabled_folders: false,
            own_prefix: self.own_prefix.as_deref(),
        };
        fleet::build(&self.workspaces, &self.sessions, now, self.show_stopped, &sup)
    }

    pub fn start_workspace(&mut self, name: &str, cx: &mut Context<Self>) {
        let write = match (&self.home, self.workspaces.iter().find(|p| p.name == name)) {
            (Some(home), Some(workspace)) => {
                DesiredWrite::Start { home: home.clone(), plugin_dir: self.plugin_dir.clone(), workspace: workspace.clone() }
            }
            (None, _) => return self.desired_failed(format!("Cannot start {name}: no Supraman home"), cx),
            (_, None) => return self.desired_failed(format!("Cannot start {name}: not registered"), cx),
        };
        self.supervising.desired_writes.unbounded_send(write).ok();
    }

    pub fn stop_workspace(&mut self, name: &str, cx: &mut Context<Self>) {
        let Some(home) = self.home.clone() else {
            return self.desired_failed(format!("Cannot stop {name}: no Supraman home"), cx);
        };
        self.supervising.desired_writes.unbounded_send(DesiredWrite::Stop { home, name: name.to_string() }).ok();
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
                            this.supervising.desired.insert(name.clone(), (true, names));
                            this.invalidate_tree();
                            this.action_error = None;
                            this.workman_state.start_warning = this.workman.start_warning(&name);
                        }
                        Ok(None) => {
                            if let Some(entry) = this.supervising.desired.get_mut(&name) {
                                entry.0 = false;
                            }
                            this.invalidate_tree();
                            this.action_error = None;
                            this.workman_state.start_warning = None;
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
            return Some("no Supraman home".into());
        }
        match self.mail.relays.get(workspace) {
            Some(r) if r.started => None,
            _ => Some(format!("{workspace} is stopped")),
        }
    }

    pub fn open_compose(&mut self, workspace: &str, reply_to: String, window: &mut Window, cx: &mut Context<Self>) {
        let input = match &self.mail.compose {
            Some(c) if c.workspace == workspace && c.reply_to == reply_to => c.input.clone(),
            _ => {
                let input = cx.new(|cx| TextInput::new("Type here; Enter adds a line.", None, window, cx).multi_line());
                let events = cx.subscribe_in(&input, window, |this: &mut Self, _, event: &InputEvent, window, cx| this.on_compose_event(event, window, cx));
                self.mail.compose = Some(Compose::new(workspace, reply_to, input.clone(), events));
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
                self.mail.compose = None;
                self.focus.focus(window, cx);
            }
            InputEvent::Edited => {
                if let Some(c) = &mut self.mail.compose {
                    c.error = None;
                }
            }
            InputEvent::ConfirmBack | InputEvent::Refused(_) | InputEvent::Submit => {}
        }
        cx.notify();
    }

    pub fn send_compose(&mut self, cx: &mut Context<Self>) {
        let _stall = supraman_ui::stall::section(SEND_SECTION);
        let Some(c) = &self.mail.compose else { return };
        let (workspace, reply_to, text) = (c.workspace.clone(), c.reply_to.clone(), c.text(cx).trim().to_string());
        if c.sending {
            return;
        }
        if text.is_empty() {
            if let Some(c) = &mut self.mail.compose {
                c.error = Some("Nothing to send.".into());
            }
            cx.notify();
            return;
        }
        if let Some(c) = &mut self.mail.compose {
            c.sending = true;
        }
        let sending = (workspace.clone(), reply_to.clone());
        self.answer(&workspace, &reply_to, &text, cx, move |this, result| {
            let still = this.mail.compose.as_ref().is_some_and(|c| (c.workspace.clone(), c.reply_to.clone()) == sending);
            match result {
                Ok(_) if still => this.mail.compose = None,
                Err(e) if still => this.mail.compose.as_mut().into_iter().for_each(|c| {
                    c.sending = false;
                    c.error = Some(e.clone());
                }),
                Err(e) => this.action_error = Some(e),
                Ok(_) => {}
            }
        });
    }

    /// Off the UI thread: a stalled pipe holds a direct write for up to `DELIVER_TIMEOUT`.
    pub fn answer(&mut self, workspace: &str, file: &str, text: &str, cx: &mut Context<Self>, done: impl FnOnce(&mut Self, Result<Sent, String>) + 'static) {
        let Some(home) = self.home.clone() else {
            done(self, Err("Not sent: no Supraman home.".into()));
            cx.notify();
            return;
        };
        let (workspace, file, text) = (workspace.to_string(), file.to_string(), text.to_string());
        let no_relay = self.relay_refusal(&workspace);
        let line = compose::first_line(&text);
        let previous = self.writes.sends.previous();
        let task = cx.spawn(async move |this, cx| {
            if let Some(previous) = previous {
                previous.await;
            }
            let (w, f, l) = (workspace.clone(), file.clone(), line.clone());
            let (sent, saved) = cx
                .background_executor()
                .spawn(async move {
                    let to = workspace_session_name(&w);
                    let mode = DesiredState::launch_mode(&home, &w, &to);
                    let sent = send_from_human(&home, &w, &to, mode.as_deref(), &text, no_relay).map_err(|e| not_sent(&to, &e));
                    let saved = sent.as_ref().ok().map(|_| mark_answered(&home, &w, &f, &l));
                    (sent, saved)
                })
                .await;
            this.update(cx, |this, cx| {
                if let Ok(sent) = &sent {
                    this.mail.answered_route.insert(format!("{workspace}/{file}"), sent.label());
                    this.mail.answered.entry(workspace.clone()).or_default().insert(file.clone(), line);
                }
                if let Some(Err(e)) = saved {
                    this.action_error = Some(format!("Answered {file}, but cannot save that: {e}"));
                }
                done(this, sent);
                cx.notify();
            })
            .ok();
        });
        self.writes.sends.push(task);
    }

    pub fn approve(&mut self, workspace: &str, file: &str, cx: &mut Context<Self>) {
        let _stall = supraman_ui::stall::section(APPROVE_SECTION);
        let w = workspace.to_string();
        self.answer(workspace, file, "approve", cx, move |this, result| match result {
            Ok(_) => this.action_error = None,
            Err(e) => this.action_error = Some(format!("Approve {w}: {e}")),
        });
    }

    /// Shown at once; written to `settings.toml` behind it, each write after the one set before it.
    pub fn set_max_working(&mut self, max_working: usize, cx: &mut Context<Self>) {
        let max_working = max_working.min(MAX_WORKING_LIMIT);
        let Some(home) = self.home.clone() else {
            self.action_error = Some("Cannot save the busy-session limit: no Supraman home".into());
            cx.notify();
            return;
        };
        self.supervising.max_working = max_working;
        let generation = self.writes.max.set();
        let previous = self.writes.max.serial.previous();
        let task = cx.spawn(async move |this, cx| {
            if let Some(previous) = previous {
                previous.await;
            }
            let saved = cx.background_executor().spawn(async move { supervise::set_max_working(&home, max_working) }).await;
            this.update(cx, |this, cx| {
                this.writes.max.landed(generation);
                this.action_error = saved.err().map(|e| format!("Cannot save the busy-session limit: {e:#}"));
                cx.notify();
            })
            .ok();
        });
        self.writes.max.serial.push(task);
        cx.notify();
    }

    pub fn retry(&mut self, workspace: &str, name: &str, cx: &mut Context<Self>) {
        self.polls.retries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((workspace.to_string(), name.to_string()));
        self.supervising.needs.retain(|n| !(n.workspace == workspace && n.name == name));
        cx.notify();
    }

    /// After a failed Adopt: `name` drops the conversation it forks from and launches as a new session.
    pub fn start_fresh(&mut self, workspace: &str, name: &str, cx: &mut Context<Self>) {
        let Some(home) = self.home.clone() else {
            self.action_error = Some(format!("Cannot start {name} fresh: no Supraman home"));
            cx.notify();
            return;
        };
        let (workspace, name) = (workspace.to_string(), name.to_string());
        let task = {
            let (workspace, name) = (workspace.clone(), name.clone());
            cx.background_executor().spawn(async move { DesiredState::update(&home, &workspace, |d| d.start_fresh(&name)).map(drop).map_err(|e| e.to_string()) })
        };
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(()) => this.retry(&workspace, &name, cx),
                    Err(e) => this.action_error = Some(format!("Cannot start {name} fresh: {e}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn attach_by_id(&mut self, workspace: &str, id: &str, window: &mut Window, cx: &mut Context<Self>) -> Option<u64> {
        let session = self.sessions.iter().find(|s| s.id == id)?.clone();
        Some(self.attach(workspace, "supervised", &session, window, cx))
    }

    /// Typed, not run: the human answers the trust prompt, never Supraman; a PowerShell tab (see C113).
    pub fn run_in_terminal(&mut self, cwd: PathBuf, command: String, window: &mut Window, cx: &mut Context<Self>) -> u64 {
        let title = format!("trust · {}", cwd.display());
        self.type_in_shell_when(cwd, title, gpui::Task::ready(entered(&command)), window, cx)
    }

    fn type_in_shell_when(&mut self, cwd: PathBuf, title: String, command: gpui::Task<String>, window: &mut Window, cx: &mut Context<Self>) -> u64 {
        let id = self.open_tab(title, TabKind::Shell, shells::powershell(self.shell_args.clone()), Some(cwd), window, cx);
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

    pub fn resumable(&self, row: &SessionRow) -> bool {
        offers_resume(&row.session, &row.health, self.supervising.last_alive.get(&row.session.session_id).copied(), now_ms())
    }

    /// `claude --bg --resume` with no other flag keeps the session id.
    pub fn resume(&mut self, workspace: &str, repo: &str, session: &Session, _window: &mut Window, cx: &mut Context<Self>) -> u64 {
        self.resume_titled(format!("{workspace} · {repo} · {}", fleet::display_name(session)), session, cx)
    }

    fn resume_titled(&mut self, title: String, session: &Session, cx: &mut Context<Self>) -> u64 {
        let sid = &session.session_id;
        if session.kind == supraman_claude::Kind::Background {
            let script = match &self.supervising.daemon_hold {
                Some(why) => held_line(why),
                None => format!(
                    "Write-Host 'resuming {} in the background, then attaching…'; claude --bg --resume {sid}; if ($LASTEXITCODE -eq 0) {{ claude attach {} }}",
                    session.id, session.id
                ),
            };
            let shell = shells::powershell(vec!["-NoLogo".into(), "-NoProfile".into(), "-Command".into(), script]);
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
            let id = self.hand_claude_in(None, session.cwd.clone(), title, resume, own, Some(sid.clone()), None, cx);
            if let Some(tab) = self.tab_mut(id) {
                tab.session = Some(tab_card::session_label(&session.name, sid));
            }
            id
        }
    }

    fn active_terminal(&self) -> Option<Entity<Terminal>> {
        self.active.and_then(|id| self.tab(id)).and_then(|t| t.terminal.clone())
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
        if self.saving.tabs_resolving {
            self.saving.tabs_again = true;
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
        self.saving.tabs_resolving = true;
        let found = cx.background_executor().spawn(async move { tab_sessions(&shells) });
        cx.spawn(async move |this, cx| {
            let found = found.await;
            this.update(cx, |this, cx| {
                this.saving.tabs_resolving = false;
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
                    this.polls.poll_nudge.wake();
                }
                if changed {
                    cx.notify();
                }
                if std::mem::take(&mut this.saving.tabs_again) {
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
            TabKind::Work(_) | TabKind::Git(_) | TabKind::Schedule(_) | TabKind::Details(_) | TabKind::Start(_) => false,
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
        if let Some(next) = supraman_ui::tabs::cycled(self.tabs.len(), at, step) {
            self.activate_tab(self.tabs[next].id, cx);
        }
    }

    #[cfg(feature = "selftest")]
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
            TabKind::Work(_) | TabKind::Git(_) | TabKind::Schedule(_) | TabKind::Details(_) | TabKind::Start(_) => None,
            TabKind::Saved(saved) => reopen::kind_of(saved),
        };
        tab_card::Card {
            title,
            workspace,
            repo,
            folder: tab.cwd.as_ref().map(|c| c.display().to_string()),
            branch: tab.branch.clone(),
            session,
            kind,
            workman: tab.workman.clone().map(|w| match &tab.restart_to {
                Some(to) => format!("{w} · restart to use {to}"),
                None => w,
            }),
            started,
        }
    }

    pub fn open_shell_in(&mut self, cwd: PathBuf, title: String, _window: &mut Window, cx: &mut Context<Self>) -> u64 {
        let (_, shell) = self.default_shell();
        self.open_plain(title, Some(cwd), shell, cx)
    }

    pub fn open_terminal_here(&mut self, cwd: PathBuf, place: &str, cx: &mut Context<Self>) -> u64 {
        let (short, shell) = self.default_shell();
        self.open_plain(format!("{place} · {short}"), Some(cwd), shell, cx)
    }

    pub fn new_shell(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> u64 {
        let (short, shell) = self.default_shell();
        let title = format!("{short} {}", self.next_tab_id);
        let cwd = self.new_shell_cwd();
        self.open_plain(title, cwd, shell, cx)
    }

    pub fn new_shell_cwd(&self) -> Option<PathBuf> {
        let active = self.active.and_then(|id| self.tab(id)).and_then(|t| t.cwd.as_deref());
        fleet::shell_cwd(active, &self.workspaces, supraman_core::paths::user_home())
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
        self.read_tab_branches(cx);
        self.active = Some(id);
        cx.notify();
        id
    }

    fn start_terminal(&mut self, id: u64, shell: util::shell::Shell, cwd: Option<PathBuf>, cx: &mut Context<Self>) {
        self.start_terminal_env(id, shell, cwd, &Extras::default(), cx);
    }

    fn start_terminal_env(&mut self, id: u64, shell: util::shell::Shell, cwd: Option<PathBuf>, extras: &Extras, cx: &mut Context<Self>) {
        if let (Some(tab), util::shell::Shell::WithArguments { program, args, .. }) = (self.tab_mut(id), &shell) {
            tab.started = Some(crate::saved_tabs::SavedShell { program: program.into(), args: args.clone() });
        }
        let build = TerminalBuilder::new(
            cwd,
            TerminalMode::interactive(),
            shell,
            extras.env.iter().cloned().collect(),
            CursorShape::Bar,
            AlternateScroll::On,
            None,
            supraman_term::terminal_settings::PATH_HYPERLINK_REGEXES.iter().map(|regex| regex.to_string()).collect(),
            supraman_term::terminal_settings::PATH_HYPERLINK_TIMEOUT,
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
                        cx.subscribe(&terminal, move |this, emitter, event, cx| this.on_terminal_event(id, &emitter, event, cx)).detach();
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
        self.remember_closed(id, cx);
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
                let alive = match pid {
                    Some(pid) if !exited => executor.spawn(async move { supraman_core::process::alive(pid) }).await,
                    _ => false,
                };
                if !alive {
                    this.update(cx, |this, cx| this.remove_tab(id, cx)).ok();
                    return;
                }
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    this.update(cx, |this, cx| {
                        if let Some(tab) = this.tab_mut(id) {
                            tab.detach = Detach::Failed;
                        }
                        this.lifecycle.quitting = false;
                        this.lifecycle.quit_asked = false;
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
        if !self.lifecycle.quitting || self.has_attach_tabs() {
            return false;
        }
        let Some(hands) = self.lifecycle.hands.clone().filter(|_| !self.lifecycle.hands_released) else {
            return true;
        };
        if self.lifecycle.quit_release.is_none() {
            let release = cx.background_executor().spawn(async move { crate::hand::release_all_within(hands, QUIT_RELEASE_WITHIN) });
            self.lifecycle.quit_release = Some(cx.spawn(async move |this, cx| {
                for notice in release.await.unwrap_or_default() {
                    log::warn!("{notice}");
                }
                this.update(cx, |this, cx| {
                    this.lifecycle.hands_released = true;
                    this.lifecycle.quit_release = None;
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
        self.lifecycle.quitting = false;
        self.close_settings(cx);
        self.window.update(cx, |_, window, cx| crate::tray::close(window, cx)).ok();
    }

    pub(crate) fn has_attach_tabs(&self) -> bool {
        self.tabs.iter().any(|t| matches!(t.kind, TabKind::Attach { .. }))
    }

    /// [+] tabs whose claude runs in Supraman's PTY: quitting ends them.
    pub(crate) fn running_hand_tabs(&self) -> usize {
        self.tabs.iter().filter(|t| t.hand && !t.exited && t.terminal.is_some()).count()
    }

    pub fn restart_elevated(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(home) = self.home.clone() else {
            return;
        };
        if let Err(e) = Settings::update(&home, |settings| settings.sessions.run_as_admin = true) {
            self.action_error = Some(format!("Cannot save Run Supraman as administrator: {e}"));
            cx.notify();
            return;
        }
        match crate::elevate::relaunch(&home, Some(std::process::id())) {
            Ok(()) => self.quit(window, cx),
            Err(e) => {
                self.action_error = Some(format!("Supraman stays unelevated: {e:#}"));
                cx.notify();
            }
        }
    }

    pub fn quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.lifecycle.quit_asked = true;
        if self.should_close(window, cx) {
            crate::tray::close(window, cx);
        }
    }

    fn should_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.lifecycle.quit_asked {
            self.close_settings(cx);
            crate::tray::hide(window, cx);
            return false;
        }
        self.lifecycle.quitting = true;
        let ids: Vec<u64> = self
            .tabs
            .iter()
            .filter(|t| matches!(t.kind, TabKind::Attach { .. }))
            .map(|t| t.id)
            .collect();
        for id in ids {
            self.close_tab(id, cx);
        }
        let close = self.lifecycle.quitting && self.quit_step(cx);
        if close {
            self.lifecycle.quitting = false;
            self.close_settings(cx);
        }
        close
    }

    fn toggle(&mut self, key: String, cx: &mut Context<Self>) {
        if !self.sidebar.collapsed.remove(&key) {
            self.sidebar.collapsed.insert(key);
        }
        cx.notify();
    }

    pub fn input_focused(&self, window: &Window, cx: &gpui::App) -> bool {
        self.search.read(cx).is_focused(window)
            || self.add_dialog().is_some_and(|d| d.read(cx).focused(window, cx))
            || self.mail.compose.as_ref().is_some_and(|c| c.input.read(cx).is_focused(window))
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
            InputEvent::Confirm | InputEvent::ConfirmBack | InputEvent::Refused(_) | InputEvent::Submit => {}
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
        if !k.modifiers.modified() && self.add_key(&k.key, window, cx) {
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

fn any_exit(executor: &gpui::BackgroundExecutor, pids: Vec<u32>, timeout: Duration) -> impl Future<Output = ()> + use<> {
    let exited = (!pids.is_empty()).then(|| executor.spawn(async move { supraman_core::platform::process::wait_any_exit(&pids, timeout) }));
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

fn stale_tabs(tabs: &[Tab], elsewhere: &[(String, Option<String>)]) -> Vec<u64> {
    tabs.iter()
        .filter(|t| t.hand && !t.exited)
        .filter(|t| t.hand_key.as_ref().is_some_and(|k| elsewhere.iter().any(|(n, _)| n == k)))
        .map(|t| t.id)
        .collect()
}

fn entered(command: &str) -> String {
    format!("{command}\r")
}

fn held_line(why: &str) -> String {
    format!("Write-Host -ForegroundColor Yellow '{}'", why.replace('\'', "''"))
}

fn placeholder(text: String, cx: &gpui::App) -> AnyElement {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(cx.theme().colors().editor_background)
        .overflow_hidden()
        .text_color(cx.theme().colors().text_muted)
        .child(div().w_full().px_6().text_center().child(SharedString::from(text)))
        .into_any_element()
}

impl Render for FleetView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _stall = supraman_ui::stall::section("render");
        tree_check::RENDERS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.probe.on_render(window);
        if !self.painted_sessions && self.polls.sessions_read {
            self.painted_sessions = true;
            timeline("first paint with sessions", Duration::ZERO);
        }
        crate::selftest_seams::injected_frame_sleep();
        self.persist_tabs(cx);
        self.wake_blink();
        self.hits.borrow_mut().clear();
        let colors = cx.theme().colors().clone();
        div()
            .size_full()
            .flex()
            .font_family(supraman_ui::fonts::UI)
            .bg(colors.background)
            .text_color(colors.text)
            .track_focus(&self.focus)
            .key_context(KEY_CONTEXT)
            .on_action(cx.listener(Self::on_tab_chord))
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| this.focus_search(window, cx)))
            .on_key_down(cx.listener(Self::key_down))
            .on_modifiers_changed(cx.listener(|this, _: &ModifiersChangedEvent, window, cx| {
                if let Some(terminal) = this.active.and_then(|id| this.tab(id)).and_then(|t| t.terminal.clone()) {
                    terminal.update(cx, |t, cx| {
                        t.refresh_hovered_word(window, cx);
                        cx.notify();
                    });
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, window, cx| {
                    if !window.default_prevented() && !this.focus.is_focused(window) {
                        this.focus.focus(window, cx);
                    }
                }),
            )
            .flex_col()
            .child(crate::titlebar::render(
                "Supraman",
                Some(
                    crate::titlebar::icon_button(&self.hits, "open-settings", "\u{e713}", cx)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.open_settings(cx)))
                        .into_any_element(),
                ),
                &self.hits,
                window,
                cx,
            ))
            .child(
                SplitPane { id: "divider".into(), min_first: MIN_LEFT_WIDTH, min_second: MIN_RIGHT_WIDTH, default: DEFAULT_LEFT_WIDTH }
                    .render(
                        self.render_left(cx),
                        self.render_right(window, cx),
                        Self::drag_left,
                        Self::save_left_width,
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
    use supraman_claude::{peer, proc_tree};
    let Some(dir) = peer::sessions_dir().filter(|_| !shells.is_empty()) else {
        return StdHashMap::new();
    };
    let procs = supraman_core::platform::process::snapshot();
    shells
        .iter()
        .filter_map(|&(tab, pid)| {
            let entry = proc_tree::session_under(&procs, pid, peer::started(pid), &dir, &peer::System)?;
            Some((tab, entry.session_id?))
        })
        .collect()
}

/// A selftest's check that every cached tree equals a fresh build at its instant: a missed invalidation is a mismatch.
pub mod tree_check {
    use std::sync::{Mutex, atomic::{AtomicBool, AtomicUsize, Ordering}};

    use crate::fleet::Tree;

    static ON: AtomicBool = AtomicBool::new(false);
    static MISMATCHES: Mutex<Vec<String>> = Mutex::new(Vec::new());
    static COMPARED: AtomicUsize = AtomicUsize::new(0);
    pub(super) static BUILDS: AtomicUsize = AtomicUsize::new(0);
    pub(super) static RENDERS: AtomicUsize = AtomicUsize::new(0);

    #[cfg(feature = "selftest")]
    pub fn counts() -> (usize, usize) {
        (BUILDS.load(Ordering::Relaxed), RENDERS.load(Ordering::Relaxed))
    }

    #[cfg(feature = "selftest")]
    pub fn set(on: bool) -> bool {
        ON.swap(on, Ordering::SeqCst)
    }

    pub(super) fn on() -> bool {
        ON.load(Ordering::Relaxed)
    }

    pub(super) fn compare(cached: &Tree, fresh: &Tree) {
        COMPARED.fetch_add(1, Ordering::Relaxed);
        let (cached, fresh) = (format!("{cached:?}"), format!("{fresh:?}"));
        if cached != fresh {
            let at = cached.bytes().zip(fresh.bytes()).position(|(a, b)| a != b).unwrap_or(cached.len().min(fresh.len()));
            let around = |s: &str| s.get(at.saturating_sub(120)..(at + 120).min(s.len())).unwrap_or_default().to_string();
            let mut m = MISMATCHES.lock().unwrap_or_else(|e| e.into_inner());
            if m.len() < 8 {
                m.push(format!("cached …{}… vs fresh …{}…", around(&cached), around(&fresh)));
            }
        }
    }

    #[cfg(feature = "selftest")]
    pub fn take() -> (Vec<String>, usize) {
        (std::mem::take(&mut *MISMATCHES.lock().unwrap_or_else(|e| e.into_inner())), COMPARED.swap(0, Ordering::Relaxed))
    }
}

pub const SEND_SECTION: &str = "compose send";
pub const APPROVE_SECTION: &str = "approve";

fn not_sent(to: &str, e: &supraman_claude::SendError) -> String {
    if e.may_arrive {
        format!("Not confirmed: the direct write to {to} timed out ({}) and may still arrive; it was not sent again through the relay.", e.direct)
    } else {
        format!("Not sent: direct delivery failed ({}), and the relay cannot take it: {}.", e.direct, e.relay)
    }
}

#[cfg(test)]
mod tests {
    use supraman_term::PathLikeTarget;

    use super::*;

    #[test]
    fn the_claude_dir_is_the_selftests_or_claudes_own_never_a_stray_env_var() {
        unsafe { std::env::set_var("SUPRAMAN_CLAUDE_DIR", r"C:\elsewhere") };
        assert_eq!(claude_dir_or(None), supraman_core::paths::claude_dir());
        let fake = Path::new(r"C:\scratch\fake-claude");
        assert_eq!(claude_dir_or(Some(fake)), Some(fake.to_path_buf()));
    }

    #[test]
    fn a_timed_out_send_says_it_may_still_arrive_and_a_failed_one_says_not_sent() {
        let late = supraman_claude::SendError { direct: "the pipe write timed out".into(), relay: "not tried".into(), may_arrive: true };
        assert_eq!(
            not_sent("fm-demo-workspace", &late),
            "Not confirmed: the direct write to fm-demo-workspace timed out (the pipe write timed out) and may still arrive; it was not sent again through the relay."
        );
        let failed = supraman_claude::SendError { direct: "no pipe".into(), relay: "demo is stopped".into(), may_arrive: false };
        assert_eq!(not_sent("fm-demo-workspace", &failed), "Not sent: direct delivery failed (no pipe), and the relay cannot take it: demo is stopped.");
    }

    #[test]
    fn an_open_event_opens_web_mail_and_file_links_and_resolves_relative_paths() {
        for uri in ["https://example.com/", "HTTP://example.com/a?b=c", "mailto:a@b.c"] {
            assert_eq!(link_target(&MaybeNavigationTarget::Url(uri.into())), Ok(Link::Url(uri.into())));
        }
        assert_eq!(link_target(&MaybeNavigationTarget::Url("file:///C:/tmp/a%20b.txt".into())), Ok(Link::Path(PathBuf::from(r"C:\tmp\a b.txt"))));
        let relative = PathLikeTarget { maybe_path: "src/x.rs".into(), working_directory: Some(PathBuf::from("C:/repo")) };
        assert_eq!(link_target(&MaybeNavigationTarget::PathLike(relative)), Ok(Link::Path(PathBuf::from("C:/repo").join("src/x.rs"))));
        let absolute = PathLikeTarget { maybe_path: "C:/tmp/a.txt".into(), working_directory: Some(PathBuf::from("C:/repo")) };
        assert_eq!(link_target(&MaybeNavigationTarget::PathLike(absolute)), Ok(Link::Path(PathBuf::from("C:/tmp/a.txt"))));
    }

    #[test]
    fn a_path_link_opens_the_file_without_its_line_and_column() {
        let at = |maybe_path: &str| link_target(&MaybeNavigationTarget::PathLike(PathLikeTarget { maybe_path: maybe_path.into(), working_directory: Some(PathBuf::from(r"C:\repo")) }));
        for (shown, file) in [
            (r"C:\x\y.rs", r"C:\x\y.rs"),
            (r"C:\x\y.rs:12", r"C:\x\y.rs"),
            (r"C:\x\y.rs:12:3", r"C:\x\y.rs"),
            ("C:/x/y.rs:12:3", "C:/x/y.rs"),
            (r"C:\x\y.cpp(12,3)", r"C:\x\y.cpp"),
            (r"src\x.rs:12:3", r"C:\repo\src\x.rs"),
            ("src/x.rs:12", r"C:\repo\src/x.rs"),
            (r"C:\x\folder", r"C:\x\folder"),
        ] {
            assert_eq!(at(shown), Ok(Link::Path(PathBuf::from(file))), "{shown}");
        }
    }

    #[test]
    fn a_path_that_names_nothing_on_disk_is_refused_with_why() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("x.rs");
        std::fs::write(&file, "").unwrap();
        let at = |maybe_path: &str| match link_target(&MaybeNavigationTarget::PathLike(PathLikeTarget { maybe_path: maybe_path.into(), working_directory: Some(tmp.path().into()) })) {
            Ok(Link::Path(path)) => path,
            other => panic!("{maybe_path}: {other:?}"),
        };
        assert_eq!(missing(&at("x.rs:12:3")), None);
        assert_eq!(missing(&at(&format!("{}:4", file.display()))), None);
        assert_eq!(missing(&at(&tmp.path().display().to_string())), None, "a folder opens");
        let gone = at("gone.rs:1");
        assert_eq!(missing(&gone), Some(format!("Not opened: there is no file or folder {}.", gone.display())));
        assert!(missing(Path::new("relative.rs")).is_some(), "a path never resolved against a folder");
    }

    #[test]
    fn run_in_terminal_enters_its_command_at_the_prompt() {
        assert_eq!(entered("claude"), "claude\r");
    }

    #[test]
    fn any_other_scheme_is_refused_by_name() {
        for (uri, scheme) in [
            ("ms-settings:privacy", "ms-settings"),
            ("vscode://file/c:/x", "vscode"),
            ("ftp://host/x", "ftp"),
            ("C:/tmp/a.exe", "c"),
            ("javascript:alert(1)", "javascript"),
            ("no-scheme", ""),
        ] {
            assert_eq!(link_target(&MaybeNavigationTarget::Url(uri.into())), Err(supraman_ui::open::refused(scheme)), "{uri}");
        }
    }

    #[test]
    fn a_tab_runs_its_attached_id_or_the_claude_found_under_its_shell() {
        let session = Session {
            id: "abcd0001".into(),
            session_id: "abcd0001-0000".into(),
            name: "s".into(),
            cwd: PathBuf::from(r"C:\demo"),
            kind: supraman_claude::Kind::Interactive,
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
        assert_eq!(load_registry(Err("no Supraman home".into())), Err("no Supraman home".to_string()));
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
            state: Some(supraman_claude::SessionState::Stopped),
            status: None,
            waiting_for: None,
            pid: None,
            started_at: None,
        };
        let (gone, live) = (supraman_claude::Health::Stopped, supraman_claude::Health::Idle);
        let now = 1_000_000;
        let just_seen = Some(now - 1_000);
        let interactive = session(supraman_claude::Kind::Interactive, "3cx-bug-fixer");
        assert!(offers_resume(&interactive, &gone, just_seen, now), "closed a second ago, never respawned: at once");
        assert!(offers_resume(&interactive, &gone, None, now), "only ever seen in its transcript");
        assert!(!offers_resume(&interactive, &live, just_seen, now), "still running");
        let bg = session(supraman_claude::Kind::Background, "worker");
        assert!(!offers_resume(&bg, &gone, just_seen, now), "the daemon may still respawn it");
        assert!(!offers_resume(&bg, &gone, Some(now - RESUME_GRACE_MS), now), "the grace's last millisecond");
        assert!(offers_resume(&bg, &gone, Some(now - RESUME_GRACE_MS - 1), now), "past the grace");
        assert!(offers_resume(&bg, &gone, None, now), "never seen alive here");
        assert!(!offers_resume(&session(supraman_claude::Kind::Interactive, "fm-alpha-workspace"), &gone, None, now), "the supervisor's");
    }

    fn poller(home: &Path, writes: Arc<AtomicU64>) -> Poller {
        Poller {
            home: Some(home.to_path_buf()),
            events: None,
            supervision: Some(Supervision::new(home.to_path_buf(), None)),
            retries: Arc::default(),
            injected: Arc::default(),
            registry_writes: writes,
            snapshot: None,
            first: false,
            store: None,
        }
    }

    #[test]
    fn a_registry_write_after_the_read_skips_that_tick() {
        let home = tempfile::tempdir().unwrap();
        let writes: Arc<AtomicU64> = Arc::default();
        let mut poller = poller(home.path(), writes.clone());
        poller.snapshot = Some((0, Vec::new(), Vec::new()));
        assert!(poller.tick().is_some(), "an unchanged registry ticks");

        poller.snapshot = Some((0, Vec::new(), Vec::new()));
        writes.fetch_add(1, Ordering::SeqCst);
        assert!(poller.tick().is_none(), "a write between read and tick: the supervisor does not run on the old list");
        assert!(poller.snapshot.is_none(), "the stale snapshot is gone");
        assert!(poller.tick().is_none(), "nothing to tick until the next read");

        poller.snapshot = Some((1, Vec::new(), Vec::new()));
        assert!(poller.tick().is_some(), "the next read's snapshot ticks");
    }

    #[test]
    fn a_poll_that_read_the_registry_before_a_save_keeps_the_saved_list() {
        let writes = AtomicU64::new(0);
        let read_at = writes.load(Ordering::SeqCst);
        let before_save: RegistryRead = Ok((Vec::new(), vec!["read before Save".into()]));
        writes.fetch_add(1, Ordering::SeqCst);
        assert!(fresh_registry(before_save, read_at, writes.load(Ordering::SeqCst)).is_none(), "the list Save showed stays");

        let read_at = writes.load(Ordering::SeqCst);
        let after_save: RegistryRead = Ok((Vec::new(), vec!["read after Save".into()]));
        let applied = fresh_registry(after_save, read_at, writes.load(Ordering::SeqCst)).and_then(Result::ok).map(|(_, skipped)| skipped);
        assert_eq!(applied, Some(vec!["read after Save".to_string()]), "the next poll applies");

        let failed = fresh_registry(Err("unreadable".into()), 0, 1);
        assert!(matches!(failed, Some(Err(e)) if e == "unreadable"), "a failed read is shown even across a Save");
    }
}

