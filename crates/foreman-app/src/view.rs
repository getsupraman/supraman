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
use foreman_core::{DEFAULT_MAX_WORKING, Event, EventReader, Platform, Registry};
use foreman_supervisor::{NeedsKind, NeedsYou, Remedy};
use foreman_term::{
    Event as TerminalEvent, Terminal, TerminalBuilder, TerminalMode,
    terminal_settings::{AlternateScroll, CursorShape},
};
use foreman_ui::{FrameProbe, term_element};
use gpui::{
    AnyElement, AnyWindowHandle, Bounds, ClickEvent, Context, Entity, FocusHandle, FontWeight, Hsla,
    KeyDownEvent, Pixels, Render, SharedString, Window, canvas, div, prelude::*, px,
};
use theme::ActiveTheme as _;
use util::paths::PathStyle;

use crate::fleet::{self, HookSignal, SessionRow, Supervised, Tree};
use crate::supervise::{self, Report, Supervision};

pub const REFRESH_EVERY: Duration = Duration::from_secs(2);
/// What closing an Attach tab types into `claude attach`: Ctrl+Z, its own "drop back to your
/// shell", which ends the attach process and leaves the session running (LEARNINGS m1/7).
pub const DETACH_INPUT: &[u8] = b"\x1a";
pub const DETACH_WITHIN: Duration = Duration::from_secs(5);
const LEFT_WIDTH: Pixels = px(460.);
/// The concurrency stepper's range; 0 pauses every launch and resume.
pub const MAX_WORKING_LIMIT: usize = 20;

/// Window-space bounds of clickable things, keyed by the same string as their element id, as of
/// the last painted frame. The selftest clicks through these.
pub type HitTargets = Rc<RefCell<StdHashMap<String, Bounds<Pixels>>>>;

pub enum Cli {
    Checking,
    Supported(ClaudeVersion),
    Failed(String),
}

pub enum TabKind {
    Attach { session_id: String },
    Shell,
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
    pub platforms: Vec<Platform>,
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
    /// The Inbox: every platform's [`NeedsYou`] from the last tick.
    pub needs: Vec<NeedsYou>,
    /// Per platform with a desired-state file: started, wanted session names.
    pub desired: BTreeMap<String, (bool, Vec<String>)>,
    pub max_working: usize,
    pub supervise_errors: Vec<String>,
    pub action_error: Option<String>,
    pub last_attempt: StdHashMap<String, i64>,
    pub hooks: StdHashMap<String, HookSignal>,
    /// When the shown `claude agents` snapshot was taken.
    pub polled_at: i64,
    pub ticks: u64,
    /// Retries (`(platform, name)`) for the next tick.
    retries: Arc<Mutex<Vec<(String, String)>>>,
}

struct Refresh {
    platforms: Result<Vec<Platform>, String>,
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
        let platforms = Registry::from_env()
            .and_then(|r| r.list())
            .map_err(|e| e.to_string());
        let polled_at = now_ms();
        let sessions = with_sessions.then(|| list_sessions().map_err(|e| e.to_string()));
        let events = self.events.as_mut().map(EventReader::poll).unwrap_or_default();
        let took = started.elapsed();
        let report = match (&platforms, &sessions, self.supervision.as_mut()) {
            (Ok(platforms), Some(Ok(sessions)), Some(supervision)) => {
                let retries = std::mem::take(&mut *self.retries.lock().unwrap_or_else(|e| e.into_inner()));
                Some(supervision.tick(platforms, sessions, retries, now_ms()))
            }
            _ => None,
        };
        Refresh {
            platforms,
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
            platforms: Vec::new(),
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
        }
    }

    fn apply(&mut self, refresh: Refresh) {
        match refresh.platforms {
            Ok(platforms) => {
                self.platforms = platforms;
                self.registry_error = None;
            }
            Err(error) => self.registry_error = Some(error),
        }
        // A failed read keeps the last good list on screen, flagged as stale.
        match refresh.sessions {
            Some(Ok(sessions)) => {
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
        fleet::build(&self.platforms, &self.sessions, now_ms(), self.show_stopped, &sup)
    }

    pub fn start_platform(&mut self, name: &str, cx: &mut Context<Self>) {
        let result = match (&self.home, self.platforms.iter().find(|p| p.name == name)) {
            (Some(home), Some(platform)) => supervise::start_platform(home, self.plugin_dir.as_deref(), platform),
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

    pub fn stop_platform(&mut self, name: &str, cx: &mut Context<Self>) {
        let result = match &self.home {
            Some(home) => supervise::stop_platform(home, name),
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

    pub fn retry(&mut self, platform: &str, name: &str, cx: &mut Context<Self>) {
        self.retries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((platform.to_string(), name.to_string()));
        self.needs.retain(|n| !(n.platform == platform && n.name == name));
        cx.notify();
    }

    /// The Inbox's Attach: the existing attach-tab path, by the row's 8-hex id.
    pub fn attach_by_id(&mut self, platform: &str, id: &str, window: &mut Window, cx: &mut Context<Self>) -> Option<u64> {
        let session = self.sessions.iter().find(|s| s.id == id)?.clone();
        Some(self.attach(platform, "supervised", &session, window, cx))
    }

    /// A shell tab in `cwd` with `command` typed at its prompt and NOT run: the human presses Enter
    /// and answers whatever it asks (the trust prompt is theirs; Foreman never answers it).
    pub fn run_in_terminal(&mut self, cwd: PathBuf, command: String, window: &mut Window, cx: &mut Context<Self>) -> u64 {
        let shell = util::shell::Shell::WithArguments {
            program: "pwsh".into(),
            args: self.shell_args.clone(),
            title_override: None,
        };
        let title = format!("trust · {}", cwd.display());
        let id = self.open_tab(title, TabKind::Shell, shell, Some(cwd), window, cx);
        cx.spawn(async move |this, cx| {
            let started = Instant::now();
            while started.elapsed() < Duration::from_secs(15) {
                let Ok(terminal) = this.read_with(cx, |this, _| this.tab(id).and_then(|t| t.terminal.clone())) else {
                    return;
                };
                if let Some(terminal) = terminal
                    && terminal.read_with(cx, |t, _| t.get_content().contains("PS "))
                {
                    terminal.update(cx, |t, _| t.input(command.into_bytes()));
                    return;
                }
                cx.background_executor().timer(Duration::from_millis(50)).await;
            }
        })
        .detach();
        id
    }

    pub fn tab(&self, id: u64) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == id)
    }

    fn tab_mut(&mut self, id: u64) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    pub fn attach(
        &mut self,
        platform: &str,
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
            format!("{platform} · {repo} · {name}"),
            TabKind::Attach {
                session_id: session.id.clone(),
            },
            shell,
            Some(session.cwd.clone()),
            window,
            cx,
        )
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

/// An invisible overlay that records its parent's painted bounds under `key`.
fn hit_target(hits: &HitTargets, key: String) -> impl IntoElement {
    let hits = hits.clone();
    canvas(
        move |bounds, _, _| {
            hits.borrow_mut().insert(key, bounds);
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
        let max_working = self.max_working;
        let stepper = div()
            .flex()
            .items_center()
            .gap_1()
            .text_xs()
            .text_color(colors.text_muted)
            .child("busy ≤")
            .child(
                button(&self.hits, "budget-dec".into(), "−", cx).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.set_max_working(max_working.saturating_sub(1), cx);
                })),
            )
            .child(
                div()
                    .w(px(18.))
                    .flex()
                    .justify_center()
                    .text_color(colors.text)
                    .font_weight(FontWeight::BOLD)
                    .child(SharedString::from(max_working.to_string())),
            )
            .child(
                button(&self.hits, "budget-inc".into(), "+", cx).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.set_max_working(max_working + 1, cx);
                })),
            );

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
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .text_xs()
                            .text_color(colors.text_muted)
                            .child(cli_line)
                            .child("·")
                            .child(refresh_line),
                    )
                    .child(stepper)
                    .child(
                        div()
                            .id("toggle-stopped")
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
                format!("Cannot read the platform registry: {error}"),
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
        if self.registry_error.is_none() && self.refreshes > 0 && tree.platforms.is_empty() {
            notices.push(notice(
                "No platforms registered. Add one with:  fm platform add <dir>".into(),
                colors.text,
                colors.element_background,
            ));
        }

        let mut body = div()
            .id("fleet")
            .flex()
            .flex_col()
            .flex_1()
            .overflow_y_scroll()
            .pb_4();
        for platform in &tree.platforms {
            let pkey = format!("platform:{}", platform.name);
            let open = !self.collapsed.contains(&pkey);
            let started = platform.started == Some(true);
            let (state_text, state_color) = match platform.started {
                Some(true) => ("started", status.success),
                Some(false) => ("stopped", colors.text_muted),
                None => ("", colors.text_muted),
            };
            let toggle_key = format!("{}-{}", if started { "stop" } else { "start" }, platform.name);
            let pname = platform.name.clone();
            let start_stop = button(&self.hits, toggle_key, if started { "Stop" } else { "Start" }, cx).on_click(
                cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    if started {
                        this.stop_platform(&pname, cx);
                    } else {
                        this.start_platform(&pname, cx);
                    }
                }),
            );
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
                            .child(SharedString::from(platform.name.clone())),
                    )
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .text_xs()
                            .text_color(colors.text_muted)
                            .whitespace_nowrap()
                            .child(SharedString::from(platform.root.display().to_string())),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(colors.text_muted)
                            .child(SharedString::from(format!("{} sessions", platform.session_count()))),
                    )
                    .child(div().text_xs().text_color(state_color).child(state_text))
                    .child(start_stop)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle(pkey.clone(), cx))),
            );
            if !open {
                continue;
            }
            for row in &platform.supervised {
                body = body.child(self.render_session(&platform.name, "supervised", row, px(28.), cx));
            }
            for repo in &platform.repos {
                let rkey = format!("repo:{}/{}", platform.name, repo.name);
                let ropen = !self.collapsed.contains(&rkey);
                let empty = repo.sessions.is_empty();
                let branch = repo.branch.as_deref().map(|b| format!("({b})")).unwrap_or_default();
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
                        .child(div().text_xs().text_color(colors.text_muted).child(SharedString::from(branch)))
                        .when(!empty, |d| {
                            d.child(
                                div()
                                    .text_xs()
                                    .text_color(colors.text_muted)
                                    .child(SharedString::from(format!("{}", repo.sessions.len()))),
                            )
                        })
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle(rkey.clone(), cx))),
                );
                if !ropen {
                    continue;
                }
                for row in &repo.sessions {
                    body = body.child(self.render_session(&platform.name, &repo.name, row, px(48.), cx));
                }
            }
        }

        div()
            .flex()
            .flex_col()
            .w(LEFT_WIDTH)
            .flex_none()
            .h_full()
            .bg(colors.panel_background)
            .border_r_1()
            .border_color(colors.border)
            .child(header)
            .children(self.render_inbox(cx))
            .children(notices)
            .child(body)
            .into_any_element()
    }

    /// One row per [`NeedsYou`] across platforms, oldest first; nothing when empty.
    fn render_inbox(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.needs.is_empty() {
            return None;
        }
        let colors = cx.theme().colors().clone();
        let now = now_ms();
        let mut needs: Vec<&NeedsYou> = self.needs.iter().collect();
        needs.sort_by_key(|n| (n.since, n.kind));
        let mut inbox = div()
            .flex()
            .flex_col()
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
                    let (label, action, platform) = match action {
                        Remedy::Attach { .. } => ("Attach", action.clone(), need.platform.clone()),
                        Remedy::Retry => ("Retry", action.clone(), need.platform.clone()),
                        Remedy::RunInTerminal { .. } => ("Run in terminal", action.clone(), need.platform.clone()),
                    };
                    let name = need.name.clone();
                    Some(button(&self.hits, key, label, cx).on_click(cx.listener(
                        move |this, _: &ClickEvent, window, cx| match &action {
                            Remedy::Attach { id } => {
                                this.attach_by_id(&platform, id, window, cx);
                            }
                            Remedy::Retry => this.retry(&platform, &name, cx),
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
                                    .child(SharedString::from(format!("{} · {}", need.platform, need.name)))
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

    fn render_session(
        &self,
        platform: &str,
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
            .when(row.attachable, |d| {
                let key = format!("attach-{}", s.id);
                let (platform, repo, session) = (platform.to_string(), repo.to_string(), s.clone());
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
                            this.attach(&platform, &repo, &session, window, cx);
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
                .child(div().flex_1().min_h_0().child(term_element::terminal_grid(terminal.clone())))
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
            .child(crate::titlebar::render(window, cx))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_left(cx))
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
