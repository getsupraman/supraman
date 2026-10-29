//! The settings window, as Zed has one: its own OS window with a page list on the left and one
//! row per setting on the right. Every change is written to `settings.toml` at once; the
//! supervisor re-reads it on its next tick.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use foreman_core::{DEFAULT_TERMINAL_FONT_SIZE, DEFAULT_WORKSTATE_URL, MAX_TERMINAL_FONT_SIZE, MIN_TERMINAL_FONT_SIZE, Settings};
use foreman_ui::term_element;
use foreman_workstate::AuthStatus;
use foreman_ui::themes::{self, Catalog, TerminalColors};
use gpui::{
    AnyElement, App, AppContext as _, Bounds, ClickEvent, Context, ElementId, Entity, FocusHandle, FontWeight, KeyDownEvent,
    Pixels, SharedString, Subscription, TitlebarOptions, WeakEntity, Window, WindowBounds, WindowHandle, WindowOptions, div,
    prelude::*, px, size,
};
use theme::ActiveTheme as _;

use crate::text_input::{InputEvent, TextInput};
use crate::view::{Cli, FleetView, REFRESH_EVERY};
use crate::workstate_setup::{RECHECK_EVERY, WsStatus};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Sessions,
    WorkState,
    Plugins,
    /// The theme, the terminal colours and the terminal font.
    Appearance,
    /// Foreman's and the claude CLI's versions, the refresh (m5/13: no longer in the left panel).
    About,
}

impl Page {
    const ALL: [Page; 5] = [Page::Sessions, Page::WorkState, Page::Plugins, Page::Appearance, Page::About];

    fn title(self) -> &'static str {
        match self {
            Page::Sessions => "Sessions",
            Page::WorkState => "WorkState",
            Page::Plugins => "Plugins",
            Page::Appearance => "Appearance",
            Page::About => "About",
        }
    }
}

pub struct SettingsView {
    home: Option<PathBuf>,
    settings: Settings,
    error: Option<String>,
    fleet: WeakEntity<FleetView>,
    page: Page,
    focus: FocusHandle,
    /// The fleet's WorkState status as last shown: a change re-reads settings.toml.
    workstate: WsStatus,
    /// The themes on offer, read with settings.toml.
    catalog: Catalog,
    /// Why the theme settings.toml names is not the one shown.
    theme_problem: Option<String>,
    /// The terminal font size Appearance last painted, for the selftest (step 7q).
    shown_font_size: std::cell::Cell<Option<Pixels>>,
    /// The installed monospace families; `None` while they are being found.
    mono_fonts: Option<Arc<Vec<String>>>,
    font_error: Option<String>,
    /// The WorkState server URL as typed (m5/43); Enter or Apply uses it.
    url_input: Entity<TextInput>,
    url_error: Option<String>,
    /// Whether a grant is held for the configured server, read off the UI thread after settings.toml;
    /// `None` until the first read answers.
    auth: Option<AuthStatus>,
    auth_reads: u64,
    _url_events: Subscription,
}

static MONO_FONTS: OnceLock<Arc<Vec<String>>> = OnceLock::new();

/// Paints the app with the theme and terminal colours `settings.toml` names (at start, and after
/// every change on the Appearance page).
pub fn apply_theme(home: Option<&Path>, cx: &mut App) -> Option<String> {
    let settings = home.and_then(|h| Settings::load(h).ok()).unwrap_or_default();
    let terminal = TerminalColors::from_setting(settings.terminal_colors.as_deref());
    themes::apply(cx, settings.theme.as_deref(), terminal, home).problem
}

pub fn open(home: Option<PathBuf>, fleet: WeakEntity<FleetView>, page: Option<Page>, cx: &mut App) -> Option<WindowHandle<SettingsView>> {
    let bounds = Bounds::centered(None, size(px(760.), px(520.)), cx);
    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some(SharedString::from("Foreman Settings")),
                appears_transparent: true,
                traffic_light_position: None,
            }),
            ..Default::default()
        },
        |window, cx| {
            window.on_window_should_close(cx, |window, cx| {
                crate::tray::close(window, cx);
                false
            });
            let view = cx.new(|cx| SettingsView::new(home, fleet, page.unwrap_or(Page::Sessions), window, cx));
            let focus = view.read(cx).focus.clone();
            window.focus(&focus, cx);
            view
        },
    )
    .ok()
}

impl SettingsView {
    fn new(home: Option<PathBuf>, fleet: WeakEntity<FleetView>, page: Page, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // About shows the fleet's CLI check and refresh as they change, WorkState its status, Appearance
        // the terminal font size a zoom in a terminal changed
        if let Some(fleet) = fleet.upgrade() {
            cx.observe(&fleet, |this, fleet, cx| {
                let workstate = fleet.read(cx).workstate.clone();
                if this.workstate != workstate {
                    this.workstate = workstate;
                    this.reload(cx);
                    cx.notify();
                } else if matches!(this.page, Page::About | Page::Appearance) {
                    cx.notify();
                }
            })
            .detach();
        }
        let workstate = fleet.upgrade().map_or(WsStatus::Checking, |f| f.read(cx).workstate.clone());
        let url_input = cx.new(|cx| TextInput::new(DEFAULT_WORKSTATE_URL, None, window, cx));
        let url_events = cx.subscribe_in(&url_input, window, |this: &mut Self, _, event: &InputEvent, _, cx| match event {
            InputEvent::Confirm => this.apply_url(cx),
            InputEvent::Edited => {
                this.url_error = None;
                cx.notify();
            }
            _ => {}
        });
        let mut this = Self {
            home,
            settings: Settings::default(),
            error: None,
            fleet,
            page,
            focus: cx.focus_handle(),
            workstate,
            catalog: Catalog::default(),
            theme_problem: None,
            shown_font_size: Default::default(),
            mono_fonts: MONO_FONTS.get().cloned(),
            font_error: None,
            url_input,
            url_error: None,
            auth: None,
            auth_reads: 0,
            _url_events: url_events,
        };
        if this.mono_fonts.is_none() {
            let text_system = cx.text_system().clone();
            let installed = this.fleet.upgrade().map(|f| f.read(cx).installed_fonts.clone()).unwrap_or_default();
            cx.spawn(async move |this, cx| {
                let found = cx
                    .background_spawn(async move { installed.iter().filter(|f| term_element::is_monospace(&text_system, f)).cloned().collect::<Vec<_>>() })
                    .await;
                let found = MONO_FONTS.get_or_init(|| Arc::new(found)).clone();
                this.update(cx, |this, cx| {
                    this.mono_fonts = Some(found);
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
        this.reload(cx);
        let url = this.settings.workstate_origin();
        this.url_input.update(cx, |input, cx| input.set_text(&url, cx));
        this
    }

    pub fn page(&self) -> Page {
        self.page
    }

    pub fn show(&mut self, page: Page, cx: &mut Context<Self>) {
        self.page = page;
        cx.notify();
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(home) = self.home.clone() else {
            self.error = Some("No Foreman home (set FOREMAN_HOME or APPDATA): settings cannot be saved.".into());
            return;
        };
        match Settings::load_with_notices(&home) {
            Ok((settings, notices)) => {
                self.settings = settings;
                if !notices.is_empty() {
                    self.error = Some(notices.join("; "));
                }
            }
            Err(e) => self.error = Some(format!("Cannot read settings.toml: {e}")),
        }
        self.catalog = themes::catalog(Some(&home));
        self.read_auth(home, cx);
    }

    /// `Auth::status` reads the servers file and decrypts the store: never on the UI thread.
    fn read_auth(&mut self, home: PathBuf, cx: &mut Context<Self>) {
        self.auth_reads += 1;
        let (read, origin) = (self.auth_reads, self.settings.workstate_origin());
        cx.spawn(async move |this, cx| {
            let auth = cx.background_spawn(async move { crate::signin::auth(&home).status(&origin) }).await;
            this.update(cx, |this, cx| {
                if this.auth_reads == read {
                    this.auth = Some(auth);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// The selftest types into the URL field.
    pub fn set_url_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.url_input.update(cx, |input, cx| input.set_text(text, cx));
    }

    pub fn url_error(&self) -> Option<String> {
        self.url_error.clone()
    }

    /// The URL field's Enter and Apply: a valid URL goes to the fleet, which switches servers through a
    /// sign-in when one is held ([`FleetView::set_workstate_url`]).
    pub fn apply_url(&mut self, cx: &mut Context<Self>) {
        let text = self.url_input.read(cx).text().to_string();
        let result = self.fleet.update(cx, |fleet, cx| fleet.set_workstate_url(&text, cx)).unwrap_or(Err("the main window is closed".into()));
        match result {
            Ok(()) => {
                self.url_error = None;
                self.reload(cx);
                if let Ok(origin) = foreman_core::validate_workstate_url(&text) {
                    self.url_input.update(cx, |input, cx| input.set_text(&origin, cx));
                }
            }
            Err(e) => self.url_error = Some(e),
        }
        cx.notify();
    }

    pub fn set_theme(&mut self, name: String, terminal: TerminalColors, cx: &mut Context<Self>) {
        self.change(
            |s| {
                s.theme = Some(name.clone());
                s.terminal_colors = terminal.setting();
            },
            cx,
        );
        self.theme_problem = apply_theme(self.home.as_deref(), cx);
    }

    /// The fleet window owns the size (one path for Settings, the keys and the wheel) and saves it.
    pub fn set_term_font_size(&mut self, size: Pixels, cx: &mut Context<Self>) {
        self.fleet.update(cx, |fleet, cx| fleet.set_term_font_size(size, cx)).ok();
        cx.notify();
    }

    pub fn shown_font_size(&self) -> Option<Pixels> {
        self.shown_font_size.get()
    }

    /// The terminal font family (`None`: the default) and ligatures, applied by the fleet window.
    pub fn set_term_font(&mut self, family: Option<&str>, ligatures: bool, cx: &mut Context<Self>) {
        let result = self.fleet.update(cx, |fleet, cx| fleet.set_term_font(family, ligatures, cx));
        self.font_error = match result {
            Ok(Ok(())) => None,
            Ok(Err(e)) => Some(e),
            Err(_) => Some("the main window is closed".into()),
        };
        self.reload(cx);
        cx.notify();
    }

    pub fn mono_fonts(&self) -> Option<Arc<Vec<String>>> {
        self.mono_fonts.clone()
    }

    fn open_themes_folder(&mut self, cx: &mut Context<Self>) {
        let Some(home) = &self.home else {
            return;
        };
        let dir = themes::themes_dir(home);
        if let Err(e) = std::fs::create_dir_all(&dir) {
            self.error = Some(format!("Cannot create {}: {e}", dir.display()));
            cx.notify();
            return;
        }
        cx.open_with_system(&dir);
    }

    fn change(&mut self, edit: impl FnMut(&mut Settings), cx: &mut Context<Self>) {
        let Some(home) = self.home.clone() else {
            return;
        };
        match Settings::update(&home, edit) {
            Ok((settings, ())) => {
                self.settings = settings;
                self.error = None;
            }
            Err(e) => self.error = Some(format!("Cannot save settings.toml: {e}")),
        }
        self.fleet.update(cx, |fleet, _| fleet.recheck_workstate()).ok();
        cx.notify();
    }

    fn set_max_working(&mut self, n: usize, cx: &mut Context<Self>) {
        // The fleet window saves it (one path for the app and its selftest) and shows any error.
        self.fleet.update(cx, |fleet, cx| fleet.set_max_working(n, cx)).ok();
        self.reload(cx);
        cx.notify();
    }

    fn open_file(&mut self, cx: &mut Context<Self>) {
        let Some(home) = self.home.clone() else {
            return;
        };
        let path = Settings::path(&home);
        if !path.exists() {
            self.change(|_| {}, cx);
        }
        cx.open_with_system(&path);
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key == "escape" {
            crate::tray::close(window, cx);
        }
    }

    fn render_sessions(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let n = self.settings.max_working;
        let stepper = div()
            .flex()
            .items_center()
            .gap_1()
            .child(small_button("max-dec", "−", cx).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.set_max_working(n.saturating_sub(1), cx);
            })))
            .child(
                div()
                    .w(px(28.))
                    .flex()
                    .justify_center()
                    .font_weight(FontWeight::BOLD)
                    .child(SharedString::from(n.to_string())),
            )
            .child(small_button("max-inc", "+", cx).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.set_max_working(n + 1, cx);
            })));
        let admin = self.settings.run_as_admin;
        let toggle = small_button("run-as-admin", if admin { "On" } else { "Off" }, cx)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.change(|s| s.run_as_admin = !admin, cx)));
        let now: SharedString = if foreman_claude::is_elevated() {
            "Foreman runs as administrator now.".into()
        } else {
            "Foreman runs unelevated now.".into()
        };
        let now = div().text_xs().text_color(cx.theme().colors().text_muted).child(now).into_any_element();
        vec![
            row(
                "Busy sessions at most",
                "How many sessions of this Foreman may be working (mid-turn) at the same time, counted across \
                 its workspaces. Launches and resumes wait while the limit is reached. 0 pauses them all. \
                 Your own sessions and another Foreman's are not counted.",
                stepper.into_any_element(),
                None,
                cx,
            ),
            row(
                "Run Foreman as administrator",
                "Turn this on when your terminals run as administrator: the claude background service they \
                 start then refuses an unelevated Foreman, and its sessions cannot launch or resume. Each \
                 start asks Windows (UAC) once. Takes effect the next time Foreman starts.",
                toggle.into_any_element(),
                Some(now),
                cx,
            ),
        ]
    }

    fn render_workstate(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let status = cx.theme().status().clone();
        let colors = cx.theme().colors().clone();
        let field = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .id("ws-url")
                    .w(px(280.))
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .border_1()
                    .border_color(colors.border)
                    .bg(colors.editor_background)
                    .text_sm()
                    .child(self.url_input.clone()),
            )
            .child(small_button("ws-url-apply", "Apply", cx).on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.apply_url(cx))));
        let below = div()
            .flex()
            .flex_col()
            .gap_1()
            .pt_2()
            .children(self.url_error.clone().map(|e| div().id("ws-url-error").text_xs().text_color(status.error).child(SharedString::from(e))))
            .child(div().text_xs().text_color(colors.text_muted).child(SharedString::from(format!(
                "Now {}{}.",
                self.settings.workstate_origin(),
                if self.settings.workstate_url.is_none() { " (the default)" } else { "" }
            ))));
        vec![
            self.render_workstate_status(cx),
            row(
                "Server URL",
                "The WorkState server Foreman signs in to, e.g. https://workstate.dikaia.ai (http only for this \
                 machine). While signed in, a new server is used only once you have signed in there: Foreman then \
                 restarts its sessions with tokens from it, asks you to restart open claude tabs, and signs out of \
                 the old one.",
                field.into_any_element(),
                Some(below.into_any_element()),
                cx,
            ),
        ]
    }

    /// Signed in or not, on which server and as whom; Sign in, Sign out and Check now.
    fn render_workstate_status(&self, cx: &mut Context<Self>) -> AnyElement {
        let status = cx.theme().status().clone();
        let colors = cx.theme().colors().clone();
        let (signing_in, sign_in_error) = self
            .fleet
            .upgrade()
            .map(|f| {
                let f = f.read(cx);
                (f.signing_in.clone(), f.sign_in_error.clone())
            })
            .unwrap_or_default();
        let url = self.settings.workstate_origin();
        let (text, color) = match (&self.workstate, &self.auth) {
            (WsStatus::Checking, _) => ("checking…".to_string(), colors.text_muted),
            (WsStatus::Ready { url, .. }, Some(AuthStatus::SignedIn { unsaved: Some(why), .. })) => {
                (format!("Signed in on {url}, but {why}."), status.warning)
            }
            (WsStatus::Ready { url, who }, _) => (
                match who {
                    Some(who) => format!("Signed in on {url} as {who}."),
                    None => format!("Signed in on {url}."),
                },
                status.success,
            ),
            (WsStatus::NotSignedIn { .. }, _) => ("Not signed in.".to_string(), status.warning),
            (WsStatus::SignInAgain { why, .. }, _) => (format!("Sign in again: {why}."), status.warning),
            (WsStatus::Unreachable { url, error }, Some(AuthStatus::SignedIn { .. })) => {
                (format!("Signed in on {url}, but it is unreachable ({error})."), status.error)
            }
            (ws, _) => (ws.problem().unwrap_or_default(), status.error),
        };
        let held = self.auth.as_ref().is_some_and(|a| !matches!(a, AuthStatus::NotSignedIn));
        let mut controls = div().flex().flex_col().items_end().gap_1();
        controls = match &signing_in {
            Some(_) => controls.child(small_button("ws-cancel", "Cancel sign-in", cx).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.fleet.update(cx, |fleet, _| fleet.cancel_sign_in()).ok();
            }))),
            None if self.auth.is_some() && (!held || matches!(self.workstate, WsStatus::SignInAgain { .. })) => {
                controls.child(small_button("ws-signin", "Sign in", cx).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.fleet.update(cx, |fleet, cx| fleet.sign_in(cx)).ok();
                    cx.notify();
                })))
            }
            None => controls,
        };
        if held && signing_in.is_none() {
            controls = controls.child(small_button("ws-signout", "Sign out", cx).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.fleet.update(cx, |fleet, cx| fleet.sign_out(cx)).ok();
                cx.notify();
            })));
        }
        controls = controls.child(small_button("ws-check", "Check now", cx).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            this.fleet.update(cx, |fleet, _| fleet.recheck_workstate()).ok();
        })));
        row_owned(
            "Status",
            format!(
                "Foreman signs in to WorkState in your browser (OAuth). The grant is kept in the Foreman home, \
                 encrypted for your Windows account; Foreman uses it to read WorkState and to give each session it \
                 starts a token of its own. Sign out revokes the grant, and with it every session token minted \
                 under it. Checked every {} s and after every change here.",
                RECHECK_EVERY.as_secs()
            ),
            controls.into_any_element(),
            Some(
                div()
                    .id("ws-status")
                    .flex()
                    .flex_col()
                    .gap_1()
                    .pt_2()
                    .child(div().text_sm().text_color(color).child(SharedString::from(text)))
                    .children(signing_in.map(|to| {
                        div().text_xs().child(SharedString::from(format!("Waiting for the sign-in to {to} in your browser…")))
                    }))
                    .children(sign_in_error.map(|e| div().text_xs().text_color(status.error).child(SharedString::from(e))))
                    .children(
                        matches!(self.workstate, WsStatus::Unreachable { .. })
                            .then(|| self.workstate.fix())
                            .flatten()
                            .map(|fix| div().text_xs().child(SharedString::from(fix))),
                    )
                    .children((url != self.workstate_url()).then(|| div().text_xs().child(SharedString::from(format!("Checking {url}…")))))
                    .into_any_element(),
            ),
            cx,
        )
    }

    /// The URL the last status is about.
    fn workstate_url(&self) -> String {
        match &self.workstate {
            WsStatus::Ready { url, .. } | WsStatus::NotSignedIn { url } | WsStatus::SignInAgain { url, .. } | WsStatus::Unreachable { url, .. } => {
                url.clone()
            }
            WsStatus::Checking => self.settings.workstate_origin(),
        }
    }

    pub fn workstate(&self) -> &WsStatus {
        &self.workstate
    }

    fn render_about(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let status = cx.theme().status().clone();
        let colors = cx.theme().colors().clone();
        let (cli, warning, refresh) = match self.fleet.upgrade() {
            Some(fleet) => {
                let fleet = fleet.read(cx);
                let (cli, warning) = match &fleet.cli {
                    Cli::Checking => ("checking…".to_string(), None),
                    Cli::Supported(v) => (v.raw.clone(), v.untested().map(|w| (w, status.warning))),
                    Cli::Failed(e) => ("unavailable".to_string(), Some((e.clone(), status.error))),
                };
                let refresh = match fleet.last_refresh_took {
                    Some(took) => format!("every {} s · the last took {} ms", REFRESH_EVERY.as_secs(), took.as_millis()),
                    None => format!("every {} s · loading…", REFRESH_EVERY.as_secs()),
                };
                (cli, warning, refresh)
            }
            None => ("the main window is closed".to_string(), None, "-".to_string()),
        };
        let value = |id: &'static str, text: String| div().id(id).text_sm().child(SharedString::from(text)).into_any_element();
        let home = self.home.as_ref().map_or("not set".to_string(), |h| h.display().to_string());
        vec![
            row("Foreman", "This build.", value("about-foreman", env!("CARGO_PKG_VERSION").to_string()), None, cx),
            row(
                "claude CLI",
                "The Claude Code CLI Foreman lists, launches and attaches sessions with (`claude --version`).",
                value("about-cli", cli),
                warning.map(|(text, color)| {
                    div()
                        .id("about-cli-warning")
                        .mt_2()
                        .p_2()
                        .rounded_md()
                        .bg(color.opacity(0.12))
                        .text_xs()
                        .text_color(color)
                        .child(SharedString::from(text))
                        .into_any_element()
                }),
                cx,
            ),
            row(
                "Refresh",
                "How often the session list (`claude agents`) is polled and the supervisor ticks.",
                value("about-refresh", refresh),
                None,
                cx,
            ),
            row(
                "Foreman home",
                "FOREMAN_HOME, else %APPDATA%\\Foreman: workspaces, desired state, settings.toml, hook events.",
                div().max_w(px(300.)).text_xs().text_color(colors.text_muted).child(SharedString::from(home)).into_any_element(),
                None,
                cx,
            ),
        ]
    }

    fn render_appearance(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let colors = cx.theme().colors().clone();
        let status = cx.theme().status().clone();
        let current = self.settings.theme.clone().unwrap_or_else(|| themes::DEFAULT_THEME.into());
        let terminal = TerminalColors::from_setting(self.settings.terminal_colors.as_deref());
        let mut list = div().flex().flex_col().gap_1().pt_2();
        for (ix, entry) in self.catalog.entries.iter().enumerate() {
            let selected = entry.name == current;
            let name = entry.name.clone();
            let from: SharedString = match &entry.file {
                None if entry.name == themes::DEFAULT_THEME => "bundled · default".into(),
                None => "bundled".into(),
                Some(file) => file.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned()).into(),
            };
            list = list.child(
                choice(("theme", ix), SharedString::from(entry.name.clone()), selected, cx)
                    .child(div().text_xs().text_color(colors.text_muted).child(from))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_theme(name.clone(), terminal, cx))),
            );
        }
        let notes = self
            .theme_problem
            .iter()
            .chain(&self.catalog.errors)
            .map(|e| div().text_xs().text_color(status.warning).child(SharedString::from(e.clone())).into_any_element())
            .collect::<Vec<_>>();
        list = list.children(notes).child(
            div()
                .pt_1()
                .text_xs()
                .text_color(colors.text_muted)
                .child("VS Code Dark Modern: kevcamel/vscode_dark_modern.zed, © 2024 Kevin Camellini, MIT licence (THIRD_PARTY.md)."),
        );
        let folder = small_button("themes-folder", "Open themes folder", cx)
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.open_themes_folder(cx)));

        let mut terminal_choices = div().flex().gap_1();
        for (ix, (label, value)) in [("Theme", TerminalColors::Theme), ("Campbell", TerminalColors::Campbell)].into_iter().enumerate() {
            let name = current.clone();
            terminal_choices = terminal_choices.child(
                choice(("terminal-colors", ix), label.into(), terminal == value, cx)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_theme(name.clone(), value, cx))),
            );
        }
        let size = self.fleet.upgrade().map(|f| f.read(cx).term_font_size);
        self.shown_font_size.set(size);
        let font_size = match size {
            Some(size) => div()
                .flex()
                .items_center()
                .gap_1()
                .child(small_button("term-font-dec", "−", cx).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.set_term_font_size(size - px(1.), cx);
                })))
                .child(
                    div()
                        .id("term-font-size")
                        .w(px(28.))
                        .flex()
                        .justify_center()
                        .font_weight(FontWeight::BOLD)
                        .child(SharedString::from(format!("{}", f32::from(size)))),
                )
                .child(small_button("term-font-inc", "+", cx).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.set_term_font_size(size + px(1.), cx);
                })))
                .child(small_button("term-font-reset", "Reset", cx).ml_1().on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.set_term_font_size(term_element::FONT_SIZE, cx);
                })))
                .into_any_element(),
            None => div().text_sm().text_color(colors.text_muted).child("the main window is closed").into_any_element(),
        };

        let (family, ligatures, installed) = self
            .fleet
            .upgrade()
            .map(|f| {
                let f = f.read(cx);
                (f.term_font_family.clone(), f.term_ligatures, f.installed_fonts.clone())
            })
            .unwrap_or_else(|| (SharedString::default(), self.settings.terminal_ligatures(), Arc::default()));
        let mut fonts = div().flex().flex_wrap().gap_1().pt_2();
        match &self.mono_fonts {
            Some(mono) => {
                for (ix, name) in mono.iter().enumerate() {
                    let pick = name.clone();
                    fonts = fonts.child(
                        choice(("term-font", ix), SharedString::from(name.clone()), *name == family, cx)
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_term_font(Some(&pick), ligatures, cx))),
                    );
                }
            }
            None => fonts = fonts.child(div().text_xs().text_color(colors.text_muted).child("finding the monospace fonts…")),
        }
        let font_notes = self
            .settings
            .missing_terminal_font(&installed)
            .map(|missing| format!("{missing} (settings.toml) is not installed: {family} is shown."))
            .into_iter()
            .chain(self.font_error.clone())
            .map(|e| div().w_full().text_xs().text_color(status.warning).child(SharedString::from(e)).into_any_element())
            .collect::<Vec<_>>();
        fonts = fonts.children(font_notes);
        let default = foreman_core::default_terminal_font_family(&installed);
        let font_reset = small_button("term-font-default", "Default", cx)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_term_font(None, ligatures, cx)));
        let mut ligature_choices = div().flex().gap_1();
        for (ix, (label, on)) in [("On", true), ("Off", false)].into_iter().enumerate() {
            let family = self.settings.terminal_font_family.clone();
            ligature_choices = ligature_choices.child(
                choice(("ligatures", ix), label.into(), ligatures == on, cx)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_term_font(family.as_deref(), on, cx))),
            );
        }
        vec![
            row(
                "Theme",
                "The colours of every Foreman window. Zed theme files (*.json) in the themes folder of the \
                 Foreman home are listed too; a key a file leaves out keeps One Dark's. Applied at once.",
                folder.into_any_element(),
                Some(list.into_any_element()),
                cx,
            ),
            row(
                "Terminal colours",
                "The 16 ANSI colours in terminal tabs: the theme's, or Windows Terminal's Campbell. Background, \
                 text and cursor stay the theme's; the selection stays white, as in Windows Terminal.",
                terminal_choices.into_any_element(),
                None,
                cx,
            ),
            row_owned(
                "Terminal font size",
                format!(
                    "The text size in every terminal tab, in pixels ({MIN_TERMINAL_FONT_SIZE} to {MAX_TERMINAL_FONT_SIZE}, \
                     default {DEFAULT_TERMINAL_FONT_SIZE}). Ctrl+= and Ctrl+- or Ctrl+wheel in any terminal change it \
                     too, Ctrl+0 resets it. Applied to open terminals at once and kept in settings.toml \
                     (terminal_font_size)."
                ),
                font_size,
                None,
                cx,
            ),
            row_owned(
                "Terminal font",
                format!(
                    "The font of every terminal tab, from the monospace fonts installed. Default: Cascadia Code, else \
                     Cascadia Mono, else Consolas (here {default}). A character the font lacks is drawn from a fallback \
                     font, shrunk to fit its cell. Applied to open terminals at once and kept in settings.toml \
                     (terminal_font_family)."
                ),
                font_reset.into_any_element(),
                Some(fonts.into_any_element()),
                cx,
            ),
            row(
                "Ligatures",
                "Character pairs such as -> and != drawn as one symbol, when the font has them (Cascadia Code does), \
                 each character still in its own cell. Kept in settings.toml (terminal_ligatures).",
                ligature_choices.into_any_element(),
                None,
                cx,
            ),
        ]
    }

    fn render_plugins(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let colors = cx.theme().colors().clone();
        let dirs = &self.settings.extra_plugin_dirs;
        let lines: Vec<AnyElement> = if dirs.is_empty() {
            vec![div().text_sm().text_color(colors.text_muted).child("none").into_any_element()]
        } else {
            dirs.iter()
                .map(|dir| {
                    let missing = !dir.join(".claude-plugin").join("plugin.json").is_file();
                    let text = format!("{}{}", dir.display(), if missing { "  (no .claude-plugin/plugin.json here)" } else { "" });
                    div()
                        .text_sm()
                        .when(missing, |d| d.text_color(cx.theme().status().warning))
                        .child(SharedString::from(text))
                        .into_any_element()
                })
                .collect()
        };
        vec![row(
            "Extra plugin dirs",
            "Passed as --plugin-dir to every session Foreman launches, after Foreman's own plugin. A plugin \
             installed for your user, such as rlarch, loads without being listed here. Change them in \
             settings.toml (extra_plugin_dirs = [\"…\"]); a running session keeps the plugins it was launched with.",
            div().into_any_element(),
            Some(div().flex().flex_col().gap_1().pt_2().children(lines).into_any_element()),
            cx,
        )]
    }
}

fn row(
    title: &'static str,
    description: &'static str,
    control: AnyElement,
    below: Option<AnyElement>,
    cx: &App,
) -> AnyElement {
    row_owned(title, description.into(), control, below, cx)
}

fn row_owned(
    title: &'static str,
    description: String,
    control: AnyElement,
    below: Option<AnyElement>,
    cx: &App,
) -> AnyElement {
    let colors = cx.theme().colors();
    div()
        .flex()
        .items_start()
        .justify_between()
        .gap_6()
        .py_3()
        .border_b_1()
        .border_color(colors.border_variant)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(title))
                .child(div().text_xs().text_color(colors.text_muted).child(SharedString::from(description)))
                .children(below),
        )
        .child(div().flex_none().child(control))
        .into_any_element()
}

fn small_button(id: &'static str, label: &'static str, cx: &App) -> gpui::Stateful<gpui::Div> {
    let colors = cx.theme().colors();
    div()
        .id(id)
        .px_2()
        .rounded_sm()
        .border_1()
        .border_color(colors.border)
        .bg(colors.element_background)
        .text_sm()
        .cursor_pointer()
        .hover(|s| s.bg(colors.element_hover))
        .active(|s| s.bg(colors.element_active))
        .child(label)
}

/// One option of a choice; the selected one is drawn as a selected list row.
fn choice(id: impl Into<ElementId>, label: SharedString, selected: bool, cx: &App) -> gpui::Stateful<gpui::Div> {
    let colors = cx.theme().colors();
    div()
        .id(id)
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .py_0p5()
        .rounded_sm()
        .border_1()
        .border_color(if selected { colors.border_focused } else { colors.border })
        .when(selected, |d| d.bg(colors.element_selected))
        .when(!selected, |d| d.bg(colors.element_background).hover(|s| s.bg(colors.element_hover)))
        .text_sm()
        .cursor_pointer()
        .child(div().flex_1().child(label))
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors().clone();
        let status = cx.theme().status().clone();
        let mut nav = div()
            .flex()
            .flex_col()
            .w(px(180.))
            .flex_none()
            .h_full()
            .py_2()
            .bg(colors.panel_background)
            .border_r_1()
            .border_color(colors.border);
        for page in Page::ALL {
            let active = self.page == page;
            nav = nav.child(
                div()
                    .id(page.title())
                    .mx_2()
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .text_sm()
                    .cursor_pointer()
                    .when(active, |d| d.bg(colors.element_selected))
                    .when(!active, |d| d.text_color(colors.text_muted))
                    .hover(|s| s.bg(colors.ghost_element_hover))
                    .active(|s| s.bg(colors.ghost_element_active))
                    .child(page.title())
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.page = page;
                        cx.notify();
                    })),
            );
        }
        nav = nav.child(div().flex_1()).child(
            div()
                .id("open-file")
                .mx_2()
                .px_2()
                .py_1()
                .rounded_sm()
                .text_xs()
                .text_color(colors.text_muted)
                .cursor_pointer()
                .hover(|s| s.bg(colors.ghost_element_hover).text_color(colors.text))
                .active(|s| s.bg(colors.ghost_element_active))
                .child("Open settings.toml")
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.open_file(cx))),
        );

        let rows = match self.page {
            Page::Sessions => self.render_sessions(cx),
            Page::WorkState => self.render_workstate(cx),
            Page::Plugins => self.render_plugins(cx),
            Page::Appearance => self.render_appearance(cx),
            Page::About => self.render_about(cx),
        };
        let content = div()
            .id("settings-page")
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .overflow_y_scroll()
            .px_6()
            .py_4()
            .child(div().text_lg().font_weight(FontWeight::BOLD).pb_2().child(self.page.title()))
            .when_some(self.error.clone(), |d, e| {
                d.child(
                    div()
                        .my_2()
                        .p_2()
                        .rounded_md()
                        .bg(status.error.opacity(0.12))
                        .text_sm()
                        .text_color(status.error)
                        .child(SharedString::from(e)),
                )
            })
            .children(rows);

        div()
            .size_full()
            .flex()
            .flex_col()
            .font_family("Segoe UI")
            .bg(colors.background)
            .text_color(colors.text)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .child(crate::titlebar::render("Settings", None, window, cx))
            .child(div().flex().flex_1().min_h_0().child(nav).child(content))
    }
}
