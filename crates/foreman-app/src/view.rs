//! The window: fleet tree on the left, terminal tabs on the right.

use std::{
    cell::RefCell,
    collections::{HashMap as StdHashMap, HashSet},
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

use collections::HashMap;
use foreman_claude::{ClaudeVersion, Health, Session, check_supported, list_sessions, now_ms};
use foreman_core::{Platform, Registry};
use foreman_term::{
    Event as TerminalEvent, Terminal, TerminalBuilder, TerminalMode,
    terminal_settings::{AlternateScroll, CursorShape},
};
use foreman_ui::{FrameProbe, term_element};
use gpui::{
    AnyElement, Bounds, ClickEvent, Context, Entity, FocusHandle, FontWeight, Hsla, KeyDownEvent,
    Pixels, Render, SharedString, Window, canvas, div, prelude::*, px,
};
use theme::ActiveTheme as _;
use util::paths::PathStyle;

use crate::fleet::{self, SessionRow, Tree};

pub const REFRESH_EVERY: Duration = Duration::from_secs(2);
const LEFT_WIDTH: Pixels = px(460.);

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

pub struct Tab {
    pub id: u64,
    pub title: String,
    pub kind: TabKind,
    pub terminal: Option<Entity<Terminal>>,
    pub error: Option<String>,
    pub exited: bool,
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
}

struct Refresh {
    platforms: Result<Vec<Platform>, String>,
    sessions: Option<Result<Vec<Session>, String>>,
    took: Duration,
}

/// Runs on a background thread: the registry read and the `claude agents` process both block.
fn load(with_sessions: bool) -> Refresh {
    let started = Instant::now();
    let platforms = Registry::from_env()
        .and_then(|r| r.list())
        .map_err(|e| e.to_string());
    let sessions = with_sessions.then(|| list_sessions().map_err(|e| e.to_string()));
    Refresh {
        platforms,
        sessions,
        took: started.elapsed(),
    }
}

impl FleetView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        cx.spawn(async move |this, cx| {
            let executor = cx.background_executor().clone();
            let checked = executor.spawn(async { check_supported() }).await;
            let supported = checked.is_ok();
            if this
                .update(cx, |this, cx| {
                    this.cli = match checked {
                        Ok(version) => Cli::Supported(version),
                        Err(error) => Cli::Failed(error.to_string()),
                    };
                    cx.notify();
                })
                .is_err()
            {
                return;
            }
            loop {
                let refresh = executor.spawn(async move { load(supported) }).await;
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
            }
            Some(Err(error)) => self.sessions_error = Some(error),
            None => {}
        }
        self.last_refresh_took = Some(refresh.took);
        self.refreshes += 1;
    }

    pub fn tree(&self) -> Tree {
        fleet::build(&self.platforms, &self.sessions, now_ms(), self.show_stopped)
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
                            Some(tab) => tab.terminal = Some(terminal),
                            // Closed while it was starting.
                            None => terminal.update(cx, |t, _| t.release_pty_resources()),
                        }
                    }
                    Err(error) => {
                        if let Some(tab) = this.tab_mut(id) {
                            tab.error = Some(format!("{error:#}"));
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

    /// Ends the tab's process only. For an attach tab that detaches: the session keeps running.
    pub fn close_tab(&mut self, id: u64, cx: &mut Context<Self>) {
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
        cx.notify();
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
                    .child(div().text_lg().font_weight(FontWeight::BOLD).child("Foreman"))
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
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .text_xs()
                    .text_color(colors.text_muted)
                    .child(cli_line)
                    .child("·")
                    .child(refresh_line),
            );

        let mut notices: Vec<AnyElement> = Vec::new();
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
                status.error_background,
            ));
        }
        if let Some(error) = &self.registry_error {
            notices.push(notice(
                format!("Cannot read the platform registry: {error}"),
                status.error,
                status.error_background,
            ));
        }
        if let Some(error) = &self.sessions_error {
            notices.push(notice(
                format!("Session list is stale, the last read failed: {error}"),
                status.warning,
                status.warning_background,
            ));
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
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle(pkey.clone(), cx))),
            );
            if !open {
                continue;
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
                    body = body.child(self.render_session(&platform.name, &repo.name, row, cx));
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
            .children(notices)
            .child(body)
            .into_any_element()
    }

    fn render_session(
        &self,
        platform: &str,
        repo: &str,
        row: &SessionRow,
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
            .pl(px(48.))
            .pr_3()
            .py(px(3.))
            .text_sm()
            .when(blocked, |d| d.bg(status.warning_background))
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
        let content: AnyElement = match active {
            Some(Tab { terminal: Some(terminal), .. }) => div()
                .size_full()
                .bg(colors.terminal_background)
                .child(term_element::terminal_grid(terminal.clone()))
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
            .child(self.render_left(cx))
            .child(self.render_right(cx))
    }
}
