//! The window: fleet tree on the left, terminal tabs on the right.

use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap as StdHashMap, HashSet},
    path::PathBuf,
    rc::Rc,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use collections::HashMap;
use foreman_claude::{ClaudeVersion, Health, Session, check_supported, list_sessions, now_ms};
use foreman_core::{
    Ask, AskState, DEFAULT_MAX_WORKING, Event, EventReader, OutboxMessage, Workspace, Registry, Settings, ask_states, mark_answered,
};
use foreman_supervisor::{NeedsKind, NeedsYou, Remedy, workspace_session_name};
use foreman_term::{
    Event as TerminalEvent, MouseInputMode, Terminal, TerminalBuilder, TerminalMode,
    terminal_settings::{AlternateScroll, CursorShape},
};
use foreman_ui::{FrameProbe, term_element};
use gpui::{
    AnyElement, AnyWindowHandle, Bounds, ClickEvent, Context, CursorStyle, DragMoveEvent, Entity, FocusHandle,
    FontWeight, Hsla, KeyDownEvent, MouseButton, MouseDownEvent, Pixels, Render, ScrollEvent, SharedString, Window,
    WindowHandle, canvas, div, prelude::*, px, relative,
};
use theme::ActiveTheme as _;
use util::paths::PathStyle;

use crate::compose::{self, Compose, KeyOutcome};
use crate::fleet::{self, HookSignal, SessionRow, Supervised, Tree};
use crate::settings_window::{self, SettingsView};
use crate::supervise::{self, RelayStatus, Report, Supervision};
use crate::work_view::WorkView;

pub const REFRESH_EVERY: Duration = Duration::from_secs(2);
/// What closing an Attach tab types into `claude attach`: Ctrl+Z, its own "drop back to your
/// shell", which ends the attach process and leaves the session running (LEARNINGS m1/7).
pub const DETACH_INPUT: &[u8] = b"\x1a";
pub const DETACH_WITHIN: Duration = Duration::from_secs(5);
/// Zed's `terminal.scroll_multiplier` default; one Windows wheel notch is already 3 lines.
const SCROLL_MULTIPLIER: f32 = 1.0;
const DEFAULT_LEFT_WIDTH: Pixels = px(460.);
const MIN_LEFT_WIDTH: Pixels = px(280.);
const MIN_RIGHT_WIDTH: Pixels = px(320.);
/// A session the daemon may still respawn by itself (it does ~12 s after a death mid-turn) is not
/// offered for resume: two resumes of one transcript run as two processes (m2/4, R13).
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
}

pub struct FleetView {
    pub cli: Cli,
    pub workspaces: Vec<Workspace>,
    pub registry_error: Option<String>,
    pub sessions: Vec<Session>,
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
    pub action_error: Option<String>,
    pub last_attempt: StdHashMap<String, i64>,
    pub hooks: StdHashMap<String, HookSignal>,
    /// When the shown `claude agents` snapshot was taken.
    pub polled_at: i64,
    pub ticks: u64,
    /// Retries (`(workspace, name)`) for the next tick.
    retries: Arc<Mutex<Vec<(String, String)>>>,
    pub left_width: Pixels,
    settings_window: Option<WindowHandle<SettingsView>>,
    /// `session_id` → when this app last saw it alive.
    last_alive: StdHashMap<String, i64>,
    /// Per workspace that wants a relay: live, armed.
    pub relays: BTreeMap<String, RelayStatus>,
    /// Per workspace, what its relay received (oldest first).
    pub messages: BTreeMap<String, Vec<OutboxMessage>>,
    /// `<workspace>/<file>` of the messages shown in full.
    expanded: HashSet<String>,
    /// Per workspace, the questions answered from here: outbox file → the answer's first line (this
    /// session's, plus `answered.json` as each tick reads it).
    pub answered: BTreeMap<String, BTreeMap<String, String>>,
    /// The open task box or reply, at most one.
    pub compose: Option<Compose>,
}

/// What the fleet/terminal divider drags.
struct DraggedDivider;

impl Render for DraggedDivider {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

struct Refresh {
    workspaces: Result<Vec<Workspace>, String>,
    sessions: Option<Result<Vec<Session>, String>>,
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
        let workspaces = Registry::from_env()
            .and_then(|r| r.list())
            .map_err(|e| e.to_string());
        let polled_at = now_ms();
        let sessions = with_sessions.then(|| list_sessions().map_err(|e| e.to_string()));
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
            workspaces,
            sessions,
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
        window.on_window_should_close(cx, move |_, cx| {
            this.update(cx, |this, cx| this.should_close(cx)).unwrap_or(true)
        });
        let home = Registry::from_env().ok().map(|r| r.home().to_path_buf());
        let left_width = home
            .as_deref()
            .and_then(|h| Settings::load(h).ok())
            .and_then(|s| s.left_width)
            .map_or(DEFAULT_LEFT_WIDTH, |w| px(w as f32).max(MIN_LEFT_WIDTH));
        let retries: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
        let mut poller = Poller {
            home: home.clone(),
            events: None,
            supervision: None,
            retries: retries.clone(),
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
                    cx.notify();
                });
                if alive.is_err() {
                    return;
                }
                executor.timer(REFRESH_EVERY).await;
            }
        })
        .detach();
        Self {
            cli: Cli::Checking,
            workspaces: Vec::new(),
            registry_error: None,
            sessions: Vec::new(),
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
            home,
            plugin_dir: None,
            setup_notices: Vec::new(),
            needs: Vec::new(),
            desired: BTreeMap::new(),
            max_working: DEFAULT_MAX_WORKING,
            supervise_errors: Vec::new(),
            action_error: None,
            last_attempt: StdHashMap::new(),
            hooks: StdHashMap::new(),
            polled_at: 0,
            ticks: 0,
            retries,
            left_width,
            settings_window: None,
            last_alive: StdHashMap::new(),
            relays: BTreeMap::new(),
            messages: BTreeMap::new(),
            expanded: HashSet::new(),
            answered: BTreeMap::new(),
            compose: None,
        }
    }

    fn apply(&mut self, refresh: Refresh) {
        match refresh.workspaces {
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
                for s in &sessions {
                    if fleet::is_alive(&s.health(now)) {
                        self.last_alive.insert(s.session_id.clone(), now);
                    }
                }
                self.sessions = sessions;
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
            self.needs = report.needs;
            self.desired = report.desired;
            self.max_working = report.max_working;
            self.supervise_errors = report.errors;
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
            }
            Err(e) => self.action_error = Some(format!("Cannot stop {name}: {e:#}")),
        }
        cx.notify();
    }

    /// Why nothing can be sent through `workspace`'s relay now, if so: it must be started with its
    /// relay live and its watcher armed (what the row shows as `relay ready`).
    pub fn relay_refusal(&self, workspace: &str) -> Option<String> {
        if self.home.is_none() {
            return Some("no Foreman home".into());
        }
        match self.relays.get(workspace).and_then(|r| r.label(now_ms())) {
            Some((_, true)) => None,
            Some((text, false)) => Some(text),
            None => Some(format!("{workspace} is stopped")),
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

    /// The box's Ctrl+Enter: `relay::send(home, w, "fm-<w>-workspace", text)`, as `fm send` does. A task
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
            (Ok(()), _) if reply_to.is_some() => self.compose = None,
            (Ok(()), Some(c)) => {
                c.clear();
                c.sent = Some(compose::first_line(&text));
                c.error = None;
            }
            (Err(e), Some(c)) => c.error = Some(e),
            (_, None) => {}
        }
        cx.notify();
    }

    fn send_to_workspace(&self, workspace: &str, text: &str) -> Result<(), String> {
        if let Some(why) = self.relay_refusal(workspace) {
            return Err(format!("Not sent: {why}."));
        }
        let home = self.home.as_deref().ok_or("no Foreman home")?;
        foreman_core::send(home, workspace, &workspace_session_name(workspace), text)
            .map(|_| ())
            .map_err(|e| format!("Not sent: {e}"))
    }

    /// Sends `text` as the answer to the outbox message `file` and marks it answered, in memory and in
    /// `answered.json` (a failed save is reported; the row still shows it answered).
    pub fn answer(&mut self, workspace: &str, file: &str, text: &str) -> Result<(), String> {
        self.send_to_workspace(workspace, text)?;
        let line = compose::first_line(text);
        self.answered.entry(workspace.to_string()).or_default().insert(file.to_string(), line.clone());
        if let Some(home) = &self.home
            && let Err(e) = mark_answered(home, workspace, file, &line)
        {
            self.action_error = Some(format!("Answered {file}, but cannot save that: {e}"));
        }
        Ok(())
    }

    /// The Messages row's Approve: the answer `approve`.
    pub fn approve(&mut self, workspace: &str, file: &str, cx: &mut Context<Self>) {
        match self.answer(workspace, file, "approve") {
            Ok(()) => self.action_error = None,
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
        let shell = util::shell::Shell::WithArguments {
            program: "pwsh".into(),
            args: self.shell_args.clone(),
            title_override: None,
        };
        let id = self.open_tab(title, TabKind::Shell, shell, Some(cwd), window, cx);
        // Typed once the shell's output has settled (its prompt is up, whatever the profile makes
        // it look like), or after 15 s regardless.
        cx.spawn(async move |this, cx| {
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
        if let Some(handle) = self.settings_window
            && handle.update(cx, |_, window, _| window.activate_window()).is_ok()
        {
            return;
        }
        let (home, fleet) = (self.home.clone(), cx.weak_entity());
        let this = cx.weak_entity();
        cx.defer(move |cx| {
            let handle = settings_window::open(home, fleet, cx);
            this.update(cx, |this, _| this.settings_window = handle).ok();
        });
    }

    fn close_settings(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.settings_window.take() {
            handle.update(cx, |_, window, _| window.remove_window()).ok();
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

    /// Whether [`Self::resume`] is offered: a session with no process that this app has not seen
    /// alive in the last [`RESUME_GRACE_MS`]. `fm-` sessions are the supervisor's (Start resumes).
    pub fn resumable(&self, row: &SessionRow) -> bool {
        let s = &row.session;
        !fleet::is_alive(&row.health)
            && !s.session_id.is_empty()
            && !s.name.starts_with("fm-")
            && self
                .last_alive
                .get(&s.session_id)
                .is_none_or(|t| now_ms() - t > RESUME_GRACE_MS)
    }

    /// A background session wakes in the background (`claude --bg --resume`, no other flag: the
    /// same id) and the tab attaches to it, so closing the tab detaches and leaves it running. An
    /// interactive one resumes in the tab, as it ran before.
    pub fn resume(&mut self, workspace: &str, repo: &str, session: &Session, window: &mut Window, cx: &mut Context<Self>) -> u64 {
        let name = if session.name.is_empty() { fleet::short_id(session) } else { &session.name };
        let title = format!("{workspace} · {repo} · {name}");
        let sid = &session.session_id;
        if session.kind == foreman_claude::Kind::Background {
            let script = format!(
                "Write-Host 'resuming {} in the background, then attaching…'; claude --bg --resume {sid}; if ($LASTEXITCODE -eq 0) {{ claude attach {} }}",
                session.id, session.id
            );
            let shell = util::shell::Shell::WithArguments {
                program: "pwsh".into(),
                args: vec!["-NoLogo".into(), "-NoProfile".into(), "-Command".into(), script],
                title_override: None,
            };
            let kind = TabKind::Attach { session_id: session.id.clone() };
            self.open_tab(title, kind, shell, Some(session.cwd.clone()), window, cx)
        } else {
            let shell = util::shell::Shell::WithArguments {
                program: "claude".into(),
                args: vec!["--resume".into(), sid.clone()],
                title_override: None,
            };
            self.open_tab(title, TabKind::Shell, shell, Some(session.cwd.clone()), window, cx)
        }
    }

    /// A shell tab in `cwd` running `claude`, as if typed there: the shell stays when it exits.
    pub fn new_claude(&mut self, cwd: PathBuf, title: String, window: &mut Window, cx: &mut Context<Self>) -> u64 {
        self.type_in_shell(cwd, title, "claude\r".into(), window, cx)
    }

    pub fn tab(&self, id: u64) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == id)
    }

    fn tab_mut(&mut self, id: u64) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
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
        let view = WorkView::open(workspace, self.home.clone(), self.hits.clone(), cx);
        self.tabs.push(Tab {
            id,
            title: format!("Work · {workspace}"),
            kind: TabKind::Work(view),
            terminal: None,
            error: None,
            exited: false,
            detach: Detach::None,
        });
        self.active = Some(id);
        cx.notify();
        id
    }

    pub fn new_shell(&mut self, window: &mut Window, cx: &mut Context<Self>) -> u64 {
        let shell = util::shell::Shell::WithArguments {
            program: "pwsh".into(),
            args: self.shell_args.clone(),
            title_override: None,
        };
        let title = format!("pwsh {}", self.next_tab_id);
        self.open_tab(title, TabKind::Shell, shell, std::env::current_dir().ok(), window, cx)
    }

    fn open_tab(
        &mut self,
        title: String,
        kind: TabKind,
        shell: util::shell::Shell,
        cwd: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> u64 {
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
        });
        self.active = Some(id);
        let build = TerminalBuilder::new(
            cwd,
            TerminalMode::interactive(),
            shell,
            HashMap::default(),
            CursorShape::Block,
            AlternateScroll::On,
            None,
            Vec::new(),
            Duration::ZERO,
            false,
            window.window_handle().window_id().as_u64(),
            cx,
            Vec::new(),
            PathStyle::local(),
        );
        cx.spawn_in(window, async move |this, cx| {
            let built = build.await;
            this.update(cx, |this, cx| {
                match built {
                    Ok(builder) => {
                        let terminal = cx.new(|cx| builder.subscribe(cx));
                        cx.subscribe(&terminal, move |this, _, event, cx| match event {
                            TerminalEvent::Wakeup | TerminalEvent::TitleChanged => cx.notify(),
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
        id
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
                .update(cx, |_, window, _| window.remove_window())
                .ok();
        }
        cx.notify();
    }

    fn has_attach_tabs(&self) -> bool {
        self.tabs.iter().any(|t| matches!(t.kind, TabKind::Attach { .. }))
    }

    /// Closing the window with Attach tabs open detaches them first; the window closes when the last
    /// one is gone, or stays open if one fails.
    fn should_close(&mut self, cx: &mut Context<Self>) -> bool {
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

    fn key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let k = &event.keystroke;
        if k.key == "," && k.modifiers.control && !k.modifiers.alt && !k.modifiers.shift {
            self.open_settings(cx);
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
        let Some(terminal) = self
            .active
            .and_then(|id| self.tab(id))
            .and_then(|t| t.terminal.clone())
        else {
            return;
        };
        terminal.update(cx, |terminal, _| {
            if !terminal.try_keystroke(&event.keystroke, false)
                && let Some(text) = &event.keystroke.key_char
            {
                terminal.input(text.clone().into_bytes());
            }
        });
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

/// A small bordered button whose painted bounds are recorded under `key` (the selftest clicks it).
fn button(hits: &HitTargets, key: String, label: impl Into<SharedString>, cx: &gpui::App) -> gpui::Stateful<gpui::Div> {
    let colors = cx.theme().colors();
    div()
        .id(SharedString::from(key.clone()))
        .relative()
        .flex_none()
        .px_2()
        .rounded_sm()
        .border_1()
        .border_color(colors.border)
        .bg(colors.element_background)
        .text_xs()
        .text_color(colors.text)
        .cursor_pointer()
        .hover(|s| s.bg(colors.element_hover))
        .child(label.into())
        .child(hit_target(hits, key))
}

fn needs_color(kind: NeedsKind, cx: &gpui::App) -> Hsla {
    let status = cx.theme().status();
    match kind {
        NeedsKind::Blocked | NeedsKind::TrustNeeded => status.warning,
        NeedsKind::GaveUp | NeedsKind::LaunchFailed => status.error,
        NeedsKind::Duplicate => status.hint,
    }
}

fn needs_label(kind: NeedsKind) -> &'static str {
    match kind {
        NeedsKind::Blocked => "blocked",
        NeedsKind::TrustNeeded => "trust needed",
        NeedsKind::GaveUp => "gave up",
        NeedsKind::LaunchFailed => "launch failed",
        NeedsKind::Duplicate => "duplicate",
    }
}

/// The Inbox button's hit-target key for one need.
pub fn inbox_key(need: &NeedsYou) -> Option<String> {
    let verb = match need.action.as_ref()? {
        Remedy::Attach { .. } => "attach",
        Remedy::Retry => "retry",
        Remedy::RunInTerminal { .. } => "run",
    };
    Some(format!("inbox-{verb}-{}", need.name))
}

fn health_color(health: &Health, cx: &gpui::App) -> Hsla {
    let status = cx.theme().status();
    match health {
        Health::Running => status.success,
        Health::Blocked { .. } => status.warning,
        Health::Idle => status.info,
        Health::Stopped | Health::Orphaned { .. } => cx.theme().colors().text_disabled,
        Health::Unknown => status.hint,
    }
}

impl FleetView {
    fn render_left(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let status = cx.theme().status().clone();
        let tree = self.tree();

        let cli_line: SharedString = match &self.cli {
            Cli::Checking => "checking the claude CLI…".into(),
            Cli::Supported(v) => format!("claude {}", v.raw).into(),
            Cli::Failed(_) => "claude CLI unavailable".into(),
        };
        let refresh_line: SharedString = match self.last_refresh_took {
            Some(took) => format!("refresh every {} s · last {} ms", REFRESH_EVERY.as_secs(), took.as_millis()).into(),
            None => "loading…".into(),
        };
        let stopped_label: SharedString = if self.show_stopped {
            "hide stopped".into()
        } else {
            format!("show stopped ({})", tree.hidden_stopped).into()
        };
        let busy: SharedString = format!("busy ≤ {}", self.max_working).into();

        let header = div()
            .flex()
            .flex_col()
            .gap_1()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(colors.border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .gap_2()
                            .text_xs()
                            .text_color(colors.text_muted)
                            .child(cli_line)
                            .child("·")
                            .child(refresh_line),
                    )
                    .child(
                        div()
                            .id("busy-limit")
                            .flex_none()
                            .px_2()
                            .rounded_sm()
                            .text_xs()
                            .text_color(colors.text_muted)
                            .cursor_pointer()
                            .hover(|s| s.bg(colors.ghost_element_hover))
                            .child(busy)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.open_settings(cx))),
                    )
                    .child(
                        div()
                            .id("toggle-stopped")
                            .flex_none()
                            .px_2()
                            .rounded_sm()
                            .text_xs()
                            .text_color(colors.text_muted)
                            .cursor_pointer()
                            .hover(|s| s.bg(colors.ghost_element_hover))
                            .child(stopped_label)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.show_stopped = !this.show_stopped;
                                cx.notify();
                            })),
                    ),
            );

        let mut notices: Vec<AnyElement> = Vec::new();
        // A tint of the text colour, never the theme's `*_background` fills (unreadable in m1).
        let notice = |text: String, fg: Hsla, bg: Hsla| {
            div()
                .mx_3()
                .my_2()
                .p_2()
                .rounded_md()
                .bg(bg)
                .text_sm()
                .text_color(fg)
                .child(SharedString::from(text))
                .into_any_element()
        };
        if let Cli::Failed(error) = &self.cli {
            notices.push(notice(
                format!("Cannot use the claude CLI, so no sessions are shown: {error}"),
                status.error,
                status.error.opacity(0.12),
            ));
        }
        if let Some(error) = &self.registry_error {
            notices.push(notice(
                format!("Cannot read the workspace registry: {error}"),
                status.error,
                status.error.opacity(0.12),
            ));
        }
        if let Some(error) = &self.sessions_error {
            notices.push(notice(
                format!("Session list is stale, the last read failed (nothing is started or stopped meanwhile): {error}"),
                status.warning,
                status.warning.opacity(0.12),
            ));
        }
        for text in self.setup_notices.iter().chain(&self.supervise_errors).chain(&self.action_error) {
            notices.push(notice(text.clone(), status.warning, status.warning.opacity(0.12)));
        }
        if self.registry_error.is_none() && self.refreshes > 0 && tree.workspaces.is_empty() {
            notices.push(notice(
                "No workspaces registered. Add one with:  fm workspace add <dir>".into(),
                colors.text,
                colors.element_background,
            ));
        }

        let mut body = div()
            .id("fleet")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .pb_4();
        for workspace in &tree.workspaces {
            let pkey = format!("workspace:{}", workspace.name);
            let open = !self.collapsed.contains(&pkey);
            let started = workspace.started == Some(true);
            let (state_text, state_color) = match workspace.started {
                Some(true) => ("started", status.success),
                Some(false) => ("stopped", colors.text_muted),
                None => ("", colors.text_muted),
            };
            let toggle_key = format!("{}-{}", if started { "stop" } else { "start" }, workspace.name);
            let pname = workspace.name.clone();
            let start_stop = button(&self.hits, toggle_key, if started { "Stop" } else { "Start" }, cx).on_click(
                cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    if started {
                        this.stop_workspace(&pname, cx);
                    } else {
                        this.start_workspace(&pname, cx);
                    }
                }),
            );
            // only a workspace whose relay can take it now; otherwise the relay label beside it says why (and
            // a long label plus this button would push Stop past the panel's edge)
            let task = (started && self.relay_refusal(&workspace.name).is_none()).then(|| {
                let w = workspace.name.clone();
                button(&self.hits, format!("task-{}", workspace.name), "Task…", cx).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        this.open_compose(&w, None, cx);
                    },
                ))
            });
            let work_name = workspace.name.clone();
            let work = button(&self.hits, format!("work-{}", workspace.name), "Work", cx).on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.open_work(&work_name, cx);
                },
            ));
            body = body.child(
                div()
                    .id(SharedString::from(pkey.clone()))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_1()
                    .mt_2()
                    .cursor_pointer()
                    .hover(|s| s.bg(colors.ghost_element_hover))
                    .child(if open { "▾" } else { "▸" })
                    .child(
                        div()
                            .font_weight(FontWeight::BOLD)
                            .child(SharedString::from(workspace.name.clone())),
                    )
                    .child(work)
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .text_xs()
                            .text_color(colors.text_muted)
                            .whitespace_nowrap()
                            .child(SharedString::from(workspace.root.display().to_string())),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(colors.text_muted)
                            .child(SharedString::from(format!("{} sessions", workspace.session_count()))),
                    )
                    .child(div().text_xs().text_color(state_color).child(state_text))
                    .children(self.relays.get(&workspace.name).and_then(|r| r.label(now_ms())).map(|(text, ok)| {
                        div()
                            .id(SharedString::from(format!("relay-{}", workspace.name)))
                            // truncates rather than push the buttons after it past the panel's edge
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_xs()
                            .text_color(if ok { status.success } else { status.warning })
                            .child(SharedString::from(text))
                    }))
                    .children(task)
                    .child(start_stop)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle(pkey.clone(), cx))),
            );
            if self.compose.as_ref().is_some_and(|c| c.workspace == workspace.name && c.reply_to.is_none()) {
                body = body.children(self.render_compose(cx));
            }
            if !open {
                continue;
            }
            for row in &workspace.supervised {
                body = body.child(self.render_session(&workspace.name, "supervised", row, px(28.), cx));
            }
            for repo in &workspace.repos {
                let rkey = format!("repo:{}/{}", workspace.name, repo.name);
                let ropen = !self.collapsed.contains(&rkey);
                let empty = repo.sessions.is_empty();
                let branch = repo.branch.as_deref().map(|b| format!("({b})")).unwrap_or_default();
                let new_key = format!("new-claude-{}/{}", workspace.name, repo.name);
                let (new_cwd, new_title) = (repo.path.clone(), format!("{} · {} · claude", workspace.name, repo.name));
                let new_claude = div()
                    .id(SharedString::from(new_key.clone()))
                    .relative()
                    .flex_none()
                    .px_1()
                    .rounded_sm()
                    .text_color(colors.text_muted)
                    .hover(|s| s.bg(colors.element_hover).text_color(colors.text))
                    .child("+")
                    .child(hit_target(&self.hits, new_key))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        this.new_claude(new_cwd.clone(), new_title.clone(), window, cx);
                    }));
                body = body.child(
                    div()
                        .id(SharedString::from(rkey.clone()))
                        .flex()
                        .items_center()
                        .gap_2()
                        .pl(px(28.))
                        .pr_3()
                        .py(px(2.))
                        .text_sm()
                        .cursor_pointer()
                        .hover(|s| s.bg(colors.ghost_element_hover))
                        .when(empty, |d| d.text_color(colors.text_muted))
                        .child(if empty { " " } else if ropen { "▾" } else { "▸" })
                        .child(SharedString::from(format!("./{}", repo.name)))
                        .child(
                            div()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_xs()
                                .text_color(colors.text_muted)
                                .child(SharedString::from(branch)),
                        )
                        .when(!empty, |d| {
                            d.child(
                                div()
                                    .text_xs()
                                    .text_color(colors.text_muted)
                                    .child(SharedString::from(format!("{}", repo.sessions.len()))),
                            )
                        })
                        .child(div().flex_1())
                        .child(new_claude)
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle(rkey.clone(), cx))),
                );
                if !ropen {
                    continue;
                }
                for row in &repo.sessions {
                    body = body.child(self.render_session(&workspace.name, &repo.name, row, px(48.), cx));
                }
            }
        }

        div()
            .flex()
            .flex_col()
            .w(self.left_width)
            .flex_none()
            .h_full()
            .bg(colors.panel_background)
            .child(header)
            .children(self.render_inbox(cx))
            .children(self.render_messages(cx))
            .children(notices)
            .child(body)
            .into_any_element()
    }

    /// One row per [`NeedsYou`] across workspaces, oldest first; nothing when empty.
    fn render_inbox(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.needs.is_empty() {
            return None;
        }
        let colors = cx.theme().colors().clone();
        let now = now_ms();
        let mut needs: Vec<&NeedsYou> = self.needs.iter().collect();
        needs.sort_by_key(|n| (n.since, n.kind));
        let mut inbox = div()
            .id("inbox")
            .flex()
            .flex_col()
            .flex_shrink_0()
            .max_h(relative(INBOX_MAX_SHARE))
            .overflow_y_scroll()
            .gap_1()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(colors.border)
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::BOLD)
                    .text_color(colors.text_muted)
                    .child(SharedString::from(format!("INBOX · {}", needs.len()))),
            );
        for need in needs {
            let color = needs_color(need.kind, cx);
            let remedy = match (&need.action, inbox_key(need)) {
                (Some(action), Some(key)) => {
                    let (label, action, workspace) = match action {
                        Remedy::Attach { .. } => ("Attach", action.clone(), need.workspace.clone()),
                        Remedy::Retry => ("Retry", action.clone(), need.workspace.clone()),
                        Remedy::RunInTerminal { .. } => ("Run in terminal", action.clone(), need.workspace.clone()),
                    };
                    let name = need.name.clone();
                    Some(button(&self.hits, key, label, cx).on_click(cx.listener(
                        move |this, _: &ClickEvent, window, cx| match &action {
                            Remedy::Attach { id } => {
                                this.attach_by_id(&workspace, id, window, cx);
                            }
                            Remedy::Retry => this.retry(&workspace, &name, cx),
                            Remedy::RunInTerminal { cwd, command } => {
                                this.run_in_terminal(cwd.clone(), command.clone(), window, cx);
                            }
                        },
                    )))
                }
                _ => None,
            };
            inbox = inbox.child(
                div()
                    .flex()
                    .items_start()
                    .gap_2()
                    .p_2()
                    .rounded_md()
                    .border_l_2()
                    .border_color(color)
                    .bg(color.opacity(0.10))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap(px(2.))
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .text_sm()
                                    .child(div().font_weight(FontWeight::BOLD).text_color(color).child(needs_label(need.kind)))
                                    .child(SharedString::from(format!("{} · {}", need.workspace, need.name)))
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(colors.text_muted)
                                            .child(SharedString::from(fleet::ago(now - need.since))),
                                    ),
                            )
                            .child(div().text_xs().text_color(colors.text).child(SharedString::from(need.detail.clone()))),
                    )
                    .children(remedy),
            );
        }
        Some(inbox.into_any_element())
    }

    /// What the workspaces' relays received, newest first, across workspaces; nothing when empty. A body
    /// shows its first lines until clicked; Attach opens the sender's session when it is attachable.
    fn render_messages(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let mut all: Vec<(&String, &OutboxMessage)> =
            self.messages.iter().flat_map(|(w, msgs)| msgs.iter().map(move |m| (w, m))).collect();
        if all.is_empty() {
            return None;
        }
        all.sort_by_key(|(_, m)| std::cmp::Reverse((m.at, m.file.clone())));
        let total = all.len();
        all.truncate(MESSAGES_LISTED);
        let colors = cx.theme().colors().clone();
        let status = cx.theme().status().clone();
        let now = now_ms();
        let no_answers = BTreeMap::new();
        let asks: BTreeMap<&String, BTreeMap<String, AskState>> = self
            .messages
            .iter()
            .map(|(w, msgs)| (w, ask_states(msgs, &workspace_session_name(w), self.answered.get(w).unwrap_or(&no_answers))))
            .collect();
        let mut section = div()
            .id("messages")
            .flex()
            .flex_col()
            .flex_shrink_0()
            .max_h(relative(MESSAGES_MAX_SHARE))
            .overflow_y_scroll()
            .gap_1()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(colors.border)
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::BOLD)
                    .text_color(colors.text_muted)
                    .child(SharedString::from(format!("MESSAGES · {total}"))),
            );
        for (workspace, m) in all {
            let key = format!("{workspace}/{}", m.file);
            let color = if m.is_notice() || m.error.is_some() { status.warning } else { status.info };
            let sender = match (&m.error, m.from_name.as_str()) {
                (Some(_), _) => format!("unreadable {}", m.file),
                (None, "") => "?".to_string(),
                (None, name) => name.to_string(),
            };
            let lines: Vec<&str> = m.body.lines().collect();
            let open = self.expanded.contains(&key);
            let shown = if open { lines.len() } else { lines.len().min(MESSAGE_LINES) };
            let mut body = lines[..shown].join("\n");
            if shown < lines.len() {
                body.push_str(&format!("\n… {} more line(s), click to show", lines.len() - shown));
            }
            // every row, not the tree's: a sender in the workspace root but outside its repos has no tree row
            let target = self
                .sessions
                .iter()
                .find(|s| !m.from_name.is_empty() && s.name == m.from_name && fleet::attachable(s, &s.health(now)))
                .map(|s| s.id.clone());
            let attach = target.map(|id| {
                let workspace = workspace.clone();
                button(&self.hits, format!("msg-attach-{key}"), "Attach", cx).on_click(cx.listener(
                    move |this, _: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        this.attach_by_id(&workspace, &id, window, cx);
                    },
                ))
            });
            let answer = asks.get(workspace).and_then(|a| a.get(&m.file)).map(|state| {
                let row = div().flex().items_center().gap_2().text_xs();
                let reply = |cx: &mut Context<Self>| {
                    let (w, file) = (workspace.clone(), m.file.clone());
                    button(&self.hits, format!("msg-reply-{key}"), "Reply…", cx).on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            cx.stop_propagation();
                            this.open_compose(&w, Some(file.clone()), cx);
                        },
                    ))
                };
                match state {
                    AskState::Open(Ask::Approve) => {
                        let (w, file) = (workspace.clone(), m.file.clone());
                        row.child(button(&self.hits, format!("msg-approve-{key}"), "Approve", cx).on_click(cx.listener(
                            move |this, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                this.approve(&w, &file, cx);
                            },
                        )))
                        .child(reply(cx))
                    }
                    AskState::Open(Ask::Decide) => row.child(reply(cx)),
                    AskState::Answered(line) => row.child(
                        div()
                            .relative()
                            .text_color(status.success)
                            .child(SharedString::from(format!("answered: {line}")))
                            .child(hit_target(&self.hits, format!("msg-answered-{key}"))),
                    ),
                    AskState::Superseded => {
                        row.child(div().text_color(colors.text_muted).child("superseded by a later APPROVE?"))
                    }
                }
            });
            let replying = self
                .compose
                .as_ref()
                .is_some_and(|c| &c.workspace == workspace && c.reply_to.as_deref() == Some(m.file.as_str()));
            section = section.child(
                div()
                    .id(SharedString::from(format!("msg-{key}")))
                    .flex()
                    .items_start()
                    .gap_2()
                    .p_2()
                    .rounded_md()
                    .border_l_2()
                    .border_color(color)
                    .bg(color.opacity(0.10))
                    .cursor_pointer()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap(px(2.))
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .text_sm()
                                    .child(div().font_weight(FontWeight::BOLD).text_color(color).child(SharedString::from(sender)))
                                    .child(div().text_color(colors.text_muted).child(SharedString::from(workspace.clone())))
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(colors.text_muted)
                                            .child(SharedString::from(fleet::ago(now - m.at))),
                                    ),
                            )
                            .child(div().text_xs().text_color(colors.text).child(SharedString::from(body)))
                            .children(answer),
                    )
                    .children(attach)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        if !this.expanded.remove(&key) {
                            this.expanded.insert(key.clone());
                        }
                        cx.notify();
                    })),
            );
            if replying {
                section = section.children(self.render_compose(cx));
            }
        }
        Some(section.into_any_element())
    }

    /// The open [`Compose`]: its text with a caret while focused, what Ctrl+Enter does, why the relay
    /// cannot take it now, the last error or the last task sent; Send and Cancel.
    fn render_compose(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let c = self.compose.as_ref()?;
        let colors = cx.theme().colors().clone();
        let status = cx.theme().status().clone();
        let ctrl = workspace_session_name(&c.workspace);
        let title = match &c.reply_to {
            Some(_) => format!("Reply to {ctrl}"),
            None => format!("Task for {ctrl}"),
        };
        let caret = if c.focused { "▏" } else { "" };
        let (before, after) = c.text.split_at(c.cursor());
        let text = if c.text.is_empty() {
            div()
                .text_color(colors.text_muted)
                .child(SharedString::from(format!("{caret}Type here; Enter adds a line.")))
        } else {
            div().text_color(colors.text).child(SharedString::from(format!("{before}{caret}{after}")))
        };
        let refusal = self.relay_refusal(&c.workspace);
        let line = |text: String, color: Hsla| div().text_xs().text_color(color).child(SharedString::from(text));
        Some(
            div()
                .id("compose")
                .relative()
                .flex()
                .flex_col()
                .flex_none()
                .gap_1()
                .mx_3()
                .my_1()
                .p_2()
                .rounded_md()
                .border_1()
                .border_color(if c.focused { colors.border_focused } else { colors.border })
                .bg(colors.editor_background)
                .cursor_text()
                .child(hit_target(&self.hits, "compose-box".into()))
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .text_xs()
                        .text_color(colors.text_muted)
                        .child(SharedString::from(title))
                        .child("Ctrl+Enter sends · Esc cancels"),
                )
                .child(div().min_h(px(48.)).text_sm().child(text))
                .children(refusal.map(|why| line(format!("Cannot send now: {why}."), status.warning)))
                .children(c.error.clone().map(|e| line(e, status.error)))
                .children(c.sent.clone().map(|s| line(format!("sent: {s}"), status.success)))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .justify_end()
                        .child(button(&self.hits, "compose-send".into(), "Send", cx).on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                this.send_compose(cx);
                            },
                        )))
                        .child(button(&self.hits, "compose-cancel".into(), "Cancel", cx).on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                this.compose = None;
                                cx.notify();
                            },
                        ))),
                )
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    if let Some(c) = &mut this.compose {
                        c.focused = true;
                    }
                    cx.notify();
                }))
                .into_any_element(),
        )
    }

    fn render_session(
        &self,
        workspace: &str,
        repo: &str,
        row: &SessionRow,
        indent: Pixels,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let status = cx.theme().status().clone();
        let s = &row.session;
        let dot = health_color(&row.health, cx);
        let blocked = matches!(row.health, Health::Blocked { .. });
        let dimmed = matches!(row.health, Health::Orphaned { .. } | Health::Stopped);
        let waiting_for = match &row.health {
            Health::Blocked { waiting_for: Some(w) } => Some(format!("waiting for: {w}")),
            _ => None,
        };
        let name = if s.name.is_empty() { "(unnamed)".to_string() } else { s.name.clone() };
        let hook = row
            .hook
            .as_ref()
            .filter(|_| s.name.starts_with("fm-"))
            .map(|h| format!("hook: {}", h.label(now_ms())));

        let line = div()
            .flex()
            .items_center()
            .gap_2()
            .child(div().size(px(8.)).flex_none().rounded_full().bg(dot))
            .child(
                div()
                    .w(px(78.))
                    .flex_none()
                    .text_xs()
                    .text_color(dot)
                    .child(SharedString::from(fleet::health_label(&row.health))),
            )
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(SharedString::from(name)),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(colors.text_muted)
                    .child(SharedString::from(format!("{} · {}", s.kind.as_str(), fleet::short_id(s)))),
            )
            .when(!row.attachable && self.resumable(row), |d| {
                let key = format!("resume-{}", s.id);
                let (workspace, repo, session) = (workspace.to_string(), repo.to_string(), s.clone());
                d.child(
                    div()
                        .id(SharedString::from(key.clone()))
                        .relative()
                        .px_2()
                        .rounded_sm()
                        .border_1()
                        .border_color(colors.border)
                        .text_xs()
                        .cursor_pointer()
                        .hover(|s| s.bg(colors.element_hover))
                        .child("resume")
                        .child(hit_target(&self.hits, key))
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.resume(&workspace, &repo, &session, window, cx);
                        })),
                )
            })
            .when(row.attachable, |d| {
                let key = format!("attach-{}", s.id);
                let (workspace, repo, session) = (workspace.to_string(), repo.to_string(), s.clone());
                d.child(
                    div()
                        .id(SharedString::from(key.clone()))
                        .relative()
                        .px_2()
                        .rounded_sm()
                        .border_1()
                        .border_color(colors.border)
                        .text_xs()
                        .cursor_pointer()
                        .hover(|s| s.bg(colors.element_hover))
                        .child("attach")
                        .child(hit_target(&self.hits, key))
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.attach(&workspace, &repo, &session, window, cx);
                        })),
                )
            });

        div()
            .flex()
            .flex_col()
            .pl(indent)
            .pr_3()
            .py(px(3.))
            .text_sm()
            .when(blocked, |d| d.bg(status.warning.opacity(0.12)))
            .when(dimmed, |d| d.opacity(0.5))
            .child(line)
            .when_some(waiting_for, |d, w| {
                d.child(
                    div()
                        .pl(px(16.))
                        .text_xs()
                        .text_color(status.warning)
                        .child(SharedString::from(w)),
                )
            })
            .when_some(hook, |d, h| {
                d.child(
                    div()
                        .pl(px(16.))
                        .text_xs()
                        .text_color(colors.text_muted)
                        .child(SharedString::from(h)),
                )
            })
            .into_any_element()
    }

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
            let mut title = tab.title.clone();
            if tab.exited {
                title.push_str(" (exited)");
            }
            match tab.detach {
                Detach::Pending | Detach::Detaching => title.push_str(" (detaching…)"),
                Detach::Failed => title.push_str(" (detach failed)"),
                Detach::None => {}
            }
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
                    .when(!active, |d| d.text_color(colors.text_muted))
                    .child(SharedString::from(title))
                    .child(
                        div()
                            .id(SharedString::from(close_key.clone()))
                            .relative()
                            .px_1()
                            .rounded_sm()
                            .hover(|s| s.bg(colors.element_hover))
                            .child("×")
                            .child(hit_target(&self.hits, close_key))
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                this.close_tab(id, cx);
                            })),
                    )
                    .child(hit_target(&self.hits, format!("tab-{id}")))
                    .on_mouse_down(
                        MouseButton::Middle,
                        cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            this.close_tab(id, cx);
                        }),
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        if this.tab(id).is_some() {
                            this.active = Some(id);
                            cx.notify();
                        }
                    })),
            );
        }
        bar = bar.child(
            div()
                .id("new-shell")
                .relative()
                .flex()
                .items_center()
                .h_full()
                .px_3()
                .cursor_pointer()
                .hover(|s| s.bg(colors.ghost_element_hover))
                .child("+")
                .child(hit_target(&self.hits, "new-shell".into()))
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.new_shell(window, cx);
                })),
        );

        let active = self.active.and_then(|id| self.tab(id));
        let status = cx.theme().status().clone();
        let content: AnyElement = match active {
            Some(Tab { kind: TabKind::Work(view), .. }) => view.clone().into_any_element(),
            Some(Tab { terminal: Some(terminal), detach, .. }) => div()
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
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        // a click in the terminal gives it the keys back from the task box
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _: &MouseDownEvent, _, cx| {
                                if let Some(c) = &mut this.compose
                                    && c.focused
                                {
                                    c.focused = false;
                                    cx.notify();
                                }
                            }),
                        )
                        .on_scroll({
                            // as Zed's TerminalView::scroll_wheel: the Terminal decides between
                            // scrollback, a mouse report and alternate-scroll arrow keys
                            let terminal = terminal.clone();
                            cx.listener(move |_, e: &ScrollEvent, _, cx| {
                                terminal.update(cx, |terminal, _| {
                                    terminal.scroll_wheel(e, SCROLL_MULTIPLIER, MouseInputMode::ReportToTerminal);
                                });
                                // the queued Scroll is applied by the grid's prepaint sync
                                cx.notify();
                            })
                        })
                        .child(term_element::terminal_grid(terminal.clone())),
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
        self.hits.borrow_mut().clear();
        let colors = cx.theme().colors().clone();
        div()
            .size_full()
            .flex()
            .font_family("Segoe UI")
            .bg(colors.background)
            .text_color(colors.text)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
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
}
