use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use supraman_core::{DEFAULT_TERMINAL_FONT_SIZE, DEFAULT_WORKMAN_URL, MAX_TERMINAL_FONT_SIZE, MIN_TERMINAL_FONT_SIZE, Settings};
use supraman_ui::settings_page::{SettingsPage, SettingsPages, row};
use supraman_ui::{banner, button, term_element};
use supraman_workman::AuthStatus;
use supraman_ui::themes::{self, Catalog, TerminalColors};
use gpui::{
    AnyElement, App, AppContext as _, Bounds, ClickEvent, Context, Entity, FocusHandle, FontWeight, KeyDownEvent,
    Pixels, SharedString, Subscription, TitlebarOptions, WeakEntity, Window, WindowBounds, WindowHandle, WindowOptions, div,
    prelude::*, px, size,
};
use theme::ActiveTheme as _;

use supraman_ui::text_input::{InputEvent, TextInput};
use crate::view::{Cli, FleetView, REFRESH_EVERY};
use crate::workstate_setup::{RECHECK_EVERY, WsStatus};

pub mod page {
    pub const SESSIONS: &str = "sessions";
    pub const WORKSTATE: &str = "workstate";
    pub const PLUGINS: &str = "plugins";
    pub const APPEARANCE: &str = "appearance";
    pub const ABOUT: &str = "about";
}

fn pages() -> SettingsPages<SettingsView> {
    let page = |id, title, icon, render| SettingsPage { id, title, icon, mark: None, render };
    let workman = |color| crate::brand::workman(gpui::rems(1.), color).into_any_element();
    SettingsPages::default()
        .register(page(page::SESSIONS, "Sessions", "\u{E7C4}", SettingsView::render_sessions))
        .register(SettingsPage { mark: Some(workman), ..page(page::WORKSTATE, "Workman", "\u{E753}", SettingsView::render_workstate) })
        .register(page(page::PLUGINS, "Plugins", "\u{EA86}", SettingsView::render_plugins))
        .register(page(page::APPEARANCE, "Appearance", "\u{E790}", SettingsView::render_appearance))
        .register(page(page::ABOUT, "About", "\u{E946}", SettingsView::render_about))
}

pub struct SettingsView {
    home: Option<PathBuf>,
    settings: Settings,
    error: Option<String>,
    fleet: WeakEntity<FleetView>,
    pages: SettingsPages<SettingsView>,
    page: &'static str,
    focus: FocusHandle,
    /// As last shown: a change re-reads settings.toml.
    workstate: WsStatus,
    catalog: Catalog,
    theme_problem: Option<String>,
    theme_picks: u64,
    /// The terminal font size Appearance last painted, for the selftest (step 7q).
    shown_font_size: std::cell::Cell<Option<Pixels>>,
    /// `None` while they are being found.
    mono_fonts: Option<Arc<Vec<String>>>,
    font_error: Option<String>,
    url_input: Entity<TextInput>,
    url_error: Option<String>,
    auth: Option<AuthStatus>,
    auth_reads: u64,
    _url_events: Subscription,
}

static MONO_FONTS: OnceLock<Arc<Vec<String>>> = OnceLock::new();

pub fn apply_theme(home: Option<&Path>, cx: &mut App) -> Option<String> {
    let settings = home.and_then(|h| Settings::load(h).ok()).unwrap_or_default();
    themes::apply_resolved(cx, saved_theme(&settings, home)).problem
}

fn saved_theme(settings: &Settings, home: Option<&Path>) -> themes::Resolved {
    let terminal = TerminalColors::from_setting(settings.appearance.terminal_colors.as_deref());
    themes::resolve(settings.appearance.theme.as_deref(), terminal, home)
}

fn choose_theme(settings: &mut Settings, name: &str, terminal: TerminalColors) {
    settings.appearance.theme = Some(name.to_string());
    settings.appearance.terminal_colors = terminal.setting();
}

pub fn open(home: Option<PathBuf>, fleet: WeakEntity<FleetView>, page: Option<&str>, cx: &mut App) -> Option<WindowHandle<SettingsView>> {
    let bounds = Bounds::centered(None, size(px(760.), px(520.)), cx);
    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some(SharedString::from("Supraman Settings")),
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
            let view = cx.new(|cx| SettingsView::new(home, fleet, page, window, cx));
            let focus = view.read(cx).focus.clone();
            window.focus(&focus, cx);
            view
        },
    )
    .ok()
}

impl SettingsView {
    fn new(home: Option<PathBuf>, fleet: WeakEntity<FleetView>, page: Option<&str>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        if let Some(fleet) = fleet.upgrade() {
            cx.observe(&fleet, |this, fleet, cx| {
                let workstate = fleet.read(cx).workstate.clone();
                if this.workstate != workstate {
                    this.workstate = workstate;
                    this.reload(cx);
                    cx.notify();
                } else if [page::ABOUT, page::APPEARANCE].contains(&this.page) {
                    cx.notify();
                }
            })
            .detach();
        }
        let workstate = fleet.upgrade().map_or(WsStatus::Checking, |f| f.read(cx).workstate.clone());
        let url_input = cx.new(|cx| TextInput::new(DEFAULT_WORKMAN_URL, None, window, cx));
        let url_events = cx.subscribe_in(&url_input, window, |this: &mut Self, _, event: &InputEvent, _, cx| match event {
            InputEvent::Confirm => this.apply_url(cx),
            InputEvent::Edited => {
                this.url_error = None;
                cx.notify();
            }
            _ => {}
        });
        let pages = pages();
        let page = pages.resolve(page);
        let mut this = Self {
            home,
            settings: Settings::default(),
            error: None,
            fleet,
            pages,
            page,
            focus: cx.focus_handle(),
            workstate,
            catalog: Catalog::default(),
            theme_problem: None,
            theme_picks: 0,
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
        let url = this.settings.workman.origin();
        this.url_input.update(cx, |input, cx| input.set_text(&url, cx));
        this
    }

    pub fn page(&self) -> &'static str {
        self.page
    }

    pub fn show(&mut self, page: &str, cx: &mut Context<Self>) {
        self.page = self.pages.resolve(Some(page));
        cx.notify();
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(home) = self.home.clone() else {
            self.error = Some("No Supraman home (set SUPRAMAN_HOME or APPDATA): settings cannot be saved.".into());
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
        self.read_catalog(home.clone(), cx);
        self.read_auth(home, cx);
    }

    /// The themes folder is listed and its files parsed off the UI thread.
    fn read_catalog(&mut self, home: PathBuf, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let catalog = cx.background_spawn(async move { themes::catalog(Some(&home)) }).await;
            this.update(cx, |this, cx| {
                this.catalog = catalog;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// `Auth::status` reads the servers file and decrypts the store: never on the UI thread.
    fn read_auth(&mut self, home: PathBuf, cx: &mut Context<Self>) {
        self.auth_reads += 1;
        let (read, origin) = (self.auth_reads, self.settings.workman.origin());
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

    pub fn apply_url(&mut self, cx: &mut Context<Self>) {
        let text = self.url_input.read(cx).text().to_string();
        let result = self.fleet.update(cx, |fleet, cx| fleet.set_workstate_url(&text, cx)).unwrap_or(Err("the main window is closed".into()));
        match result {
            Ok(()) => {
                self.url_error = None;
                self.reload(cx);
                if let Ok(origin) = supraman_core::validate_workman_url(&text) {
                    self.url_input.update(cx, |input, cx| input.set_text(&origin, cx));
                }
            }
            Err(e) => self.url_error = Some(e),
        }
        cx.notify();
    }

    fn set_theme(&mut self, name: String, terminal: TerminalColors, cx: &mut Context<Self>) {
        self.change(|s| choose_theme(s, &name, terminal), cx);
        self.theme_picks += 1;
        let (pick, home) = (self.theme_picks, self.home.clone());
        let resolving = cx.background_spawn(async move { themes::resolve(Some(&name), terminal, home.as_deref()) });
        cx.spawn(async move |this, cx| {
            let resolved = resolving.await;
            this.update(cx, |this, cx| {
                if this.theme_picks == pick {
                    this.theme_problem = themes::apply_resolved(cx, resolved).problem;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// The fleet window owns and saves the size: one path for Settings, the keys and the wheel.
    pub fn set_term_font_size(&mut self, size: Pixels, cx: &mut Context<Self>) {
        self.fleet.update(cx, |fleet, cx| fleet.set_term_font_size(size, cx)).ok();
        cx.notify();
    }

    pub fn shown_font_size(&self) -> Option<Pixels> {
        self.shown_font_size.get()
    }

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
        supraman_ui::open::open_off_thread(dir, cx).detach();
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
        supraman_ui::open::open_off_thread(path, cx).detach();
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key == "escape" {
            crate::tray::close(window, cx);
        }
    }

    fn render_sessions(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let n = self.settings.sessions.max_working;
        let stepper = div()
            .flex()
            .items_center()
            .gap_1()
            .child(button::small("max-dec", "−", cx).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
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
            .child(button::small("max-inc", "+", cx).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.set_max_working(n + 1, cx);
            })));
        let admin = self.settings.sessions.run_as_admin;
        let toggle = button::small("run-as-admin", if admin { "On" } else { "Off" }, cx)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.change(|s| s.sessions.run_as_admin = !admin, cx)));
        let now: SharedString = if supraman_claude::is_elevated() {
            "Supraman runs as administrator now.".into()
        } else {
            "Supraman runs unelevated now.".into()
        };
        let now = div().text_xs().text_color(cx.theme().colors().text_muted).child(now).into_any_element();
        let reopen = self.settings.sessions.reopen_tabs;
        let reopen_toggle = button::small("reopen-tabs", if reopen { "On" } else { "Off" }, cx)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.change(|s| s.sessions.reopen_tabs = !reopen, cx)));
        let on_click = self.settings.sessions.resume_tabs_on_click;
        let resume_toggle = button::small("resume-tabs-on-click", if on_click { "On a click" } else { "At once" }, cx)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.change(|s| s.sessions.resume_tabs_on_click = !on_click, cx)));
        vec![
            row(
                "Busy sessions at most",
                "How many sessions of this Supraman may be working (mid-turn) at the same time, counted across \
                 its workspaces. Launches and resumes wait while the limit is reached. 0 pauses them all. \
                 Your own sessions and another Supraman's are not counted.",
                stepper.into_any_element(),
                None,
                cx,
            ),
            row(
                "Run Supraman as administrator",
                "Turn this on when your terminals run as administrator: the claude background service they \
                 start then refuses an unelevated Supraman, and its sessions cannot launch or resume. Each \
                 start asks Windows (UAC) once. Takes effect the next time Supraman starts.",
                toggle.into_any_element(),
                Some(now),
                cx,
            ),
            row(
                "Reopen last tabs on start",
                "Supraman keeps the open tabs, in their order, and reopens them when it starts again: shells in                  their folder, attached background sessions attached again while they run (a stopped one waits for                  Resume; nothing is restarted unasked), Work tabs, and claude tabs resumed with a fresh Workman                  token. A tab whose folder or session is gone is left out with one notice.",
                reopen_toggle.into_any_element(),
                None,
                cx,
            ),
            row(
                "Resume reopened claude tabs",
                "At once: each claude tab of the last run resumes its conversation when Supraman starts. On a click:                  it opens with a Resume button and nothing runs until you click it.",
                resume_toggle.into_any_element(),
                None,
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
            .child(div().id("ws-url").w(px(280.)).child(self.url_input.clone()))
            .child(button::small("ws-url-apply", "Apply", cx).on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.apply_url(cx))));
        let below = div()
            .flex()
            .flex_col()
            .gap_1()
            .pt_2()
            .children(self.url_error.clone().map(|e| div().id("ws-url-error").text_xs().text_color(status.error).child(SharedString::from(e))))
            .child(div().text_xs().text_color(colors.text_muted).child(SharedString::from(format!(
                "Now {}{}.",
                self.settings.workman.origin(),
                if self.settings.workman.url.is_none() { " (the default)" } else { "" }
            ))));
        vec![
            self.render_workstate_status(cx),
            row(
                "Server URL",
                format!(
                    "The Workman server Supraman signs in to, e.g. {DEFAULT_WORKMAN_URL} (http only for this \
                     machine). While signed in, a new server is used only once you have signed in there: Supraman then \
                     restarts its sessions with tokens from it, asks you to restart open claude tabs, and signs out of \
                     the old one."
                ),
                field.into_any_element(),
                Some(below.into_any_element()),
                cx,
            ),
        ]
    }

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
        let url = self.settings.workman.origin();
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
            Some(_) => controls.child(button::small("ws-cancel", "Cancel sign-in", cx).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.fleet.update(cx, |fleet, _| fleet.cancel_sign_in()).ok();
            }))),
            None if self.auth.is_some() && (!held || matches!(self.workstate, WsStatus::SignInAgain { .. })) => {
                controls.child(button::small("ws-signin", "Sign in", cx).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.fleet.update(cx, |fleet, cx| fleet.sign_in(cx)).ok();
                    cx.notify();
                })))
            }
            None => controls,
        };
        if held && signing_in.is_none() {
            controls = controls.child(button::small("ws-signout", "Sign out", cx).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.fleet.update(cx, |fleet, cx| fleet.sign_out(cx)).ok();
                cx.notify();
            })));
        }
        controls = controls.child(button::small("ws-check", "Check now", cx).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            this.fleet.update(cx, |fleet, _| fleet.recheck_workstate()).ok();
        })));
        row(
            "Status",
            format!(
                "Supraman signs in to Workman in your browser (OAuth). The grant is kept in the Supraman home, \
                 encrypted for your Windows account; Supraman uses it to read Workman and to give each session it \
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

    fn workstate_url(&self) -> String {
        match &self.workstate {
            WsStatus::Ready { url, .. } | WsStatus::NotSignedIn { url } | WsStatus::SignInAgain { url, .. } | WsStatus::Unreachable { url, .. } => {
                url.clone()
            }
            WsStatus::Checking => self.settings.workman.origin(),
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
        let on = self.settings.updates.check;
        let check_toggle = button::small("updates-check", if on { "On" } else { "Off" }, cx)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_update_check(!on, cx)));
        let last_check = div()
            .flex()
            .items_center()
            .gap_2()
            .child(div().id("updates-last").max_w(px(260.)).text_sm().child(SharedString::from(self.updates_line(cx))))
            .children(on.then(|| {
                button::small("updates-check-now", "Check now", cx).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.fleet.update(cx, |fleet, _| fleet.check_updates_now()).ok();
                }))
            }));
        vec![
            div()
                .id("about-mark")
                .flex()
                .items_center()
                .gap_3()
                .py_3()
                .border_b_1()
                .border_color(colors.border_variant)
                .child(crate::brand::supraman(40.))
                .child(crate::brand::wordmark(22., cx))
                .into_any_element(),
            row("Supraman", "This build.", value("about-foreman", env!("CARGO_PKG_VERSION").to_string()), None, cx),
            row(
                "Check for updates",
                "At start and once a day Supraman asks GitHub whether a newer release is published, and shows it above the session list. Off sends no request.",
                check_toggle.into_any_element(),
                None,
                cx,
            ),
            row("Last check", "What the last update check found.", last_check.into_any_element(), None, cx),
            row(
                "claude CLI",
                "The Claude Code CLI Supraman lists, launches and attaches sessions with (`claude --version`).",
                value("about-cli", cli),
                warning.map(|(text, color)| {
                    banner::tinted(color)
                        .id("about-cli-warning")
                        .mt_2()
                        .text_xs()
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
                "Supraman home",
                "SUPRAMAN_HOME, else FOREMAN_HOME, else %APPDATA%\\Supraman: workspaces, desired state, settings.toml, hook events.",
                div().max_w(px(300.)).text_xs().text_color(colors.text_muted).child(SharedString::from(home)).into_any_element(),
                None,
                cx,
            ),
        ]
    }

    pub fn set_update_check(&mut self, on: bool, cx: &mut Context<Self>) {
        self.change(|s| s.updates.check = on, cx);
        if on {
            self.fleet.update(cx, |fleet, _| fleet.check_updates_now()).ok();
        }
    }

    pub fn updates_line(&self, cx: &App) -> String {
        let on = self.settings.updates.check;
        self.fleet.upgrade().map_or_else(|| "the main window is closed".into(), |f| f.read(cx).updates.status_line(on))
    }

    fn render_appearance(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let colors = cx.theme().colors().clone();
        let status = cx.theme().status().clone();
        let current = themes::canonical(self.settings.appearance.theme.as_deref().unwrap_or(themes::DEFAULT_THEME)).to_string();
        let terminal = TerminalColors::from_setting(self.settings.appearance.terminal_colors.as_deref());
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
                button::choice(("theme", ix), SharedString::from(entry.name.clone()), selected, cx)
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
        let folder = button::small("themes-folder", "Open themes folder", cx)
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.open_themes_folder(cx)));

        let mut terminal_choices = div().flex().gap_1();
        for (ix, (label, value)) in [("Theme", TerminalColors::Theme), ("Campbell", TerminalColors::Campbell)].into_iter().enumerate() {
            let name = current.clone();
            terminal_choices = terminal_choices.child(
                button::choice(("terminal-colors", ix), label, terminal == value, cx)
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
                .child(button::small("term-font-dec", "−", cx).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
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
                .child(button::small("term-font-inc", "+", cx).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.set_term_font_size(size + px(1.), cx);
                })))
                .child(button::small("term-font-reset", "Reset", cx).ml_1().on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
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
            .unwrap_or_else(|| (SharedString::default(), self.settings.terminal.ligatures(), Arc::default()));
        let mut fonts = div().flex().flex_wrap().gap_1().pt_2();
        match &self.mono_fonts {
            Some(mono) => {
                for (ix, name) in mono.iter().enumerate() {
                    let pick = name.clone();
                    fonts = fonts.child(
                        button::choice(("term-font", ix), SharedString::from(name.clone()), *name == family, cx)
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_term_font(Some(&pick), ligatures, cx))),
                    );
                }
            }
            None => fonts = fonts.child(div().text_xs().text_color(colors.text_muted).child("finding the monospace fonts…")),
        }
        let font_notes = self
            .settings
            .terminal.missing_font(&installed)
            .map(|missing| format!("{missing} (settings.toml) is not installed: {family} is shown."))
            .into_iter()
            .chain(self.font_error.clone())
            .map(|e| div().w_full().text_xs().text_color(status.warning).child(SharedString::from(e)).into_any_element())
            .collect::<Vec<_>>();
        fonts = fonts.children(font_notes);
        let default = supraman_core::default_terminal_font_family(&installed);
        let font_reset = button::small("term-font-default", "Default", cx)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_term_font(None, ligatures, cx)));
        let mut ligature_choices = div().flex().gap_1();
        for (ix, (label, on)) in [("On", true), ("Off", false)].into_iter().enumerate() {
            let family = self.settings.terminal.font_family.clone();
            ligature_choices = ligature_choices.child(
                button::choice(("ligatures", ix), label, ligatures == on, cx)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_term_font(family.as_deref(), on, cx))),
            );
        }
        vec![
            row(
                "Theme",
                "The colours of every Supraman window. Zed theme files (*.json) in the themes folder of the \
                 Supraman home are listed too; a key a file leaves out keeps One Dark's. Applied at once.",
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
            row(
                "Terminal font size",
                format!(
                    "The text size in every terminal tab, in pixels ({MIN_TERMINAL_FONT_SIZE} to {MAX_TERMINAL_FONT_SIZE}, \
                     default {DEFAULT_TERMINAL_FONT_SIZE}). Ctrl+= and Ctrl+- or Ctrl+wheel in any terminal change it \
                     too, Ctrl+0 resets it. Applied to open terminals at once and kept in settings.toml \
                     ([terminal] font_size)."
                ),
                font_size,
                None,
                cx,
            ),
            row(
                "Terminal font",
                format!(
                    "The font of every terminal tab, from the monospace fonts installed. Default: Cascadia Code, else \
                     Cascadia Mono, else Consolas (here {default}). A character the font lacks is drawn from a fallback \
                     font, shrunk to fit its cell. Applied to open terminals at once and kept in settings.toml \
                     ([terminal] font_family)."
                ),
                font_reset.into_any_element(),
                Some(fonts.into_any_element()),
                cx,
            ),
            row(
                "Ligatures",
                "Character pairs such as -> and != drawn as one symbol, when the font has them (Cascadia Code does), \
                 each character still in its own cell. Kept in settings.toml ([terminal] ligatures).",
                ligature_choices.into_any_element(),
                None,
                cx,
            ),
        ]
    }

    fn render_plugins(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let colors = cx.theme().colors().clone();
        let dirs = &self.settings.sessions.extra_plugin_dirs;
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
            "Passed as --plugin-dir to every session Supraman launches, after Supraman's own plugin. A plugin \
             installed for your user, such as workman, loads without being listed here. Change them in \
             settings.toml ([sessions] extra_plugin_dirs = [\"…\"]); a running session keeps the plugins it was launched with.",
            div().into_any_element(),
            Some(div().flex().flex_col().gap_1().pt_2().children(lines).into_any_element()),
            cx,
        )]
    }
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
        nav = nav.children(self.pages.nav(self.page, |this, page, cx| this.show(page, cx), cx));
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

        let (title, rows) = match self.pages.get(self.page) {
            Some(page) => (page.title, (page.render)(self, cx)),
            None => ("", Vec::new()),
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
            .child(div().text_lg().font_weight(FontWeight::BOLD).pb_2().child(title))
            .when_some(self.error.clone(), |d, e| {
                d.child(
                    banner::tinted(status.error)
                        .my_2()
                        .child(SharedString::from(e)),
                )
            })
            .children(rows);

        div()
            .size_full()
            .flex()
            .flex_col()
            .font_family(supraman_ui::fonts::UI)
            .bg(colors.background)
            .text_color(colors.text)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .child(crate::titlebar::render("Settings", None, window, cx))
            .child(div().flex().flex_1().min_h_0().child(nav).child(content))
    }
}

#[cfg(test)]
mod tests {
    use supraman_term::terminal_settings::CAMPBELL;
    use supraman_ui::themes::{DARK_MODERN, FOREMAN_DARK, hex, palette};

    use super::*;

    fn painted(settings: &Settings, home: &Path) -> (String, String, String) {
        let theme = saved_theme(settings, Some(home)).theme;
        (theme.name.to_string(), hex(theme.colors().panel_background), hex(palette(&theme).ansi[8]))
    }

    #[test]
    fn without_a_saved_theme_the_app_paints_vs_code_dark_modern() {
        let home = tempfile::tempdir().unwrap();
        let fresh = Settings::load(home.path()).unwrap();
        assert_eq!((fresh.appearance.theme.as_deref(), fresh.appearance.terminal_colors.as_deref()), (None, None));
        assert_eq!(painted(&fresh, home.path()), (DARK_MODERN.to_string(), "#181818".to_string(), "#666666".to_string()));
    }

    #[test]
    fn a_theme_choice_is_saved_to_settings_toml_and_painted_from_it() {
        let home = tempfile::tempdir().unwrap();
        let start = painted(&Settings::default(), home.path());

        Settings::update(home.path(), |s| choose_theme(s, FOREMAN_DARK, TerminalColors::Campbell)).unwrap();
        let saved = Settings::load(home.path()).unwrap();
        assert_eq!((saved.appearance.theme.as_deref(), saved.appearance.terminal_colors.as_deref()), (Some(FOREMAN_DARK), Some("campbell")));
        let dark = painted(&saved, home.path());
        assert_eq!((dark.0.as_str(), dark.2.clone()), (FOREMAN_DARK, hex(gpui::rgb(CAMPBELL[8]).into())));
        assert_ne!(dark.1, start.1, "Supraman Dark's panel is not Dark Modern's");

        Settings::update(home.path(), |s| choose_theme(s, DARK_MODERN, TerminalColors::Theme)).unwrap();
        let back = Settings::load(home.path()).unwrap();
        assert_eq!((back.appearance.theme.as_deref(), back.appearance.terminal_colors.as_deref()), (Some(DARK_MODERN), None));
        assert_eq!(painted(&back, home.path()), start);
    }
}
