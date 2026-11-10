use std::path::{Path, PathBuf};
use std::sync::Arc;

use supraman_core::Settings;
use supraman_ui::settings_page::{SettingsPage, SettingsPages};
use supraman_ui::{banner, button};
use supraman_workman::AuthStatus;
use supraman_ui::themes::{self, Catalog, TerminalColors};
use gpui::{
    AnyElement, App, AppContext as _, Bounds, ClickEvent, Context, Entity, FocusHandle, FontWeight, KeyDownEvent,
    Pixels, SharedString, Subscription, TitlebarOptions, WeakEntity, Window, WindowBounds, WindowHandle, WindowOptions, div,
    prelude::*, px, size,
};
use theme::ActiveTheme as _;

use supraman_ui::hit::HitTargets;
use supraman_ui::text_input::{InputEvent, TextInput};
use crate::view::FleetView;
use crate::workman_setup::ServerCheck;
use supraman_workman::status::WorkmanStatus;

mod about;
mod appearance;
pub mod gitman;
mod plugins;
mod sessions;
mod workman;

pub mod page {
    pub const SESSIONS: &str = "sessions";
    pub const WORKMAN: &str = "workman";
    pub const PLUGINS: &str = "plugins";
    pub const GITMAN: &str = "gitman";
    pub const APPEARANCE: &str = "appearance";
    pub const ABOUT: &str = "about";
}

fn pages() -> SettingsPages<SettingsView> {
    let page = |id, title, icon, render| SettingsPage { id, title, icon, mark: None, render };
    let workman = |color| crate::brand::workman(gpui::rems(1.), color).into_any_element();
    SettingsPages::default()
        .register(page(page::SESSIONS, "Sessions", "\u{E7C4}", SettingsView::render_sessions))
        .register(SettingsPage { mark: Some(workman), ..page(page::WORKMAN, "Workman", "\u{E753}", SettingsView::render_workman) })
        .register(page(page::PLUGINS, "Plugins", "\u{EA86}", SettingsView::render_plugins))
        .register(page(page::GITMAN, "GitMan", "\u{E895}", SettingsView::render_gitman))
        .register(page(page::APPEARANCE, "Appearance", "\u{E790}", SettingsView::render_appearance))
        .register(page(page::ABOUT, "About", "\u{E946}", SettingsView::render_about))
}

pub struct SettingsView {
    home: Option<PathBuf>,
    settings: Settings,
    /// A change waits for the first read: its toggles flip what was read, never a default.
    loaded: bool,
    settings_reads: u64,
    error: Option<String>,
    fleet: WeakEntity<FleetView>,
    pages: SettingsPages<SettingsView>,
    page: &'static str,
    focus: FocusHandle,
    hits: HitTargets,
    /// As last shown: a change re-reads settings.toml.
    workman: WorkmanStatus,
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
    /// `None` until the first look at server.json and claude's entry for it.
    server_check: Option<ServerCheck>,
    server_checks: u64,
    _url_events: Subscription,
    saves: crate::view::writes::Serial,
    gitman: Entity<gitman::GitmanPage>,
    /// The extra plugin dirs without a `.claude-plugin/plugin.json`, found with the settings they came with.
    missing_plugins: Vec<PathBuf>,
}

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
        Self::observe_fleet(&fleet, cx);
        let workman = fleet.upgrade().map_or(WorkmanStatus::Checking, |f| f.read(cx).workman.clone());
        let url_input = cx.new(|cx| TextInput::new("https://work.example", None, window, cx));
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
        let hits = HitTargets::default();
        let gitman = cx.new(|cx| gitman::GitmanPage::new(home.clone(), hits.clone(), window, cx));
        let mut this = Self {
            home,
            settings: Settings::default(),
            loaded: false,
            settings_reads: 0,
            error: None,
            fleet,
            pages,
            page,
            focus: cx.focus_handle(),
            hits,
            workman,
            catalog: Catalog::default(),
            theme_problem: None,
            theme_picks: 0,
            shown_font_size: Default::default(),
            mono_fonts: appearance::MONO_FONTS.get().cloned(),
            font_error: None,
            url_input,
            url_error: None,
            auth: None,
            auth_reads: 0,
            server_check: None,
            server_checks: 0,
            _url_events: url_events,
            saves: Default::default(),
            gitman,
            missing_plugins: Vec::new(),
        };
        if this.mono_fonts.is_none() {
            this.find_mono_fonts(cx);
        }
        this.reload(cx);
        if let Some(url) = this.workman.checked_url().map(str::to_string) {
            this.url_input.update(cx, |input, cx| input.set_text(&url, cx));
        }
        this
    }

    fn observe_fleet(fleet: &WeakEntity<FleetView>, cx: &mut Context<Self>) {
        let Some(fleet) = fleet.upgrade() else {
            return;
        };
        cx.observe(&fleet, |this, fleet, cx| {
            let workman = fleet.read(cx).workman.clone();
            if this.workman != workman {
                if let Some(url) = workman.checked_url().filter(|u| this.workman.checked_url() != Some(*u)) {
                    this.url_input.update(cx, |input, cx| input.set_text(url, cx));
                }
                this.workman = workman;
                this.reload(cx);
                cx.notify();
            } else if [page::ABOUT, page::APPEARANCE].contains(&this.page) {
                cx.notify();
            }
        })
        .detach();
    }

    #[cfg(feature = "selftest")]
    pub fn page(&self) -> &'static str {
        self.page
    }

    #[cfg(feature = "selftest")]
    pub fn loaded(&self) -> bool {
        self.loaded
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
        self.read_settings(home.clone(), cx);
        self.read_catalog(home.clone(), cx);
        self.read_auth(home.clone(), cx);
        self.check_server(cx);
    }

    fn read_settings(&mut self, home: PathBuf, cx: &mut Context<Self>) {
        self.settings_reads += 1;
        let read = self.settings_reads;
        cx.spawn(async move |this, cx| {
            let loaded = cx
                .background_spawn(async move {
                    Settings::load_with_notices(&home).map(|(settings, notices)| {
                        let missing = plugins::missing(&settings);
                        (settings, notices, missing)
                    })
                })
                .await;
            this.update(cx, |this, cx| {
                if this.settings_reads != read {
                    return;
                }
                this.loaded = true;
                match loaded {
                    Ok((settings, notices, missing)) => {
                        this.settings = settings;
                        this.missing_plugins = missing;
                        if !notices.is_empty() {
                            this.error = Some(notices.join("; "));
                        }
                    }
                    Err(e) => this.error = Some(format!("Cannot read settings.toml: {e}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
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
        let read = self.auth_reads;
        let Some(origin) = self.workman.checked_url().map(str::to_string) else {
            self.auth = None;
            return;
        };
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

    fn change(&mut self, edit: impl FnOnce(&mut Settings) + Send + 'static, cx: &mut Context<Self>) {
        self.change_then(edit, |_, _| {}, cx);
    }

    /// Saved off the UI thread after the changes before it; `then` runs once it is applied.
    fn change_then(
        &mut self,
        edit: impl FnOnce(&mut Settings) + Send + 'static,
        then: impl FnOnce(&mut Self, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(home) = self.home.clone().filter(|_| self.loaded) else {
            return;
        };
        let previous = self.saves.previous();
        let task = cx.spawn(async move |this, cx| {
            if let Some(previous) = previous {
                previous.await;
            }
            let saved = cx
                .background_spawn(async move {
                    Settings::update(&home, edit).map(|(settings, ())| {
                        let missing = plugins::missing(&settings);
                        (settings, missing)
                    })
                })
                .await;
            this.update(cx, |this, cx| {
                match saved {
                    Ok((settings, missing)) => {
                        this.settings = settings;
                        this.missing_plugins = missing;
                        this.settings_reads += 1;
                        this.error = None;
                    }
                    Err(e) => this.error = Some(format!("Cannot save settings.toml: {e}")),
                }
                let terminal = this.settings.terminal.clone();
                this.fleet
                    .update(cx, |fleet, _| {
                        fleet.term_look.shell_default.read(terminal);
                        fleet.recheck_workman();
                    })
                    .ok();
                then(this, cx);
                cx.notify();
            })
            .ok();
        });
        self.saves.push(task);
    }

    fn open_file(&mut self, cx: &mut Context<Self>) {
        let Some(home) = self.home.clone() else {
            return;
        };
        let path = Settings::path(&home);
        cx.spawn(async move |this, cx| {
            let exists = cx.background_spawn({
                let path = path.clone();
                async move { path.exists() }
            });
            if exists.await {
                cx.update(|cx| supraman_ui::open::open_off_thread(path, cx).detach());
            } else {
                this.update(cx, |this, cx| this.change_then(|_| {}, move |_, cx| supraman_ui::open::open_off_thread(path, cx).detach(), cx)).ok();
            }
        })
        .detach();
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key == "escape" {
            crate::tray::close(window, cx);
        }
    }

    fn render_gitman(&self, _: &mut Context<Self>) -> Vec<AnyElement> {
        vec![self.gitman.clone().into_any_element()]
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
            button::ghost(&self.hits, "open-file", cx)
                .mx_2()
                .px_2()
                .py_1()
                .rounded_sm()
                .text_xs()
                .text_color(colors.text_muted)
                .hover(|s| s.bg(colors.ghost_element_hover).text_color(colors.text))
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
            .child(crate::titlebar::render("Settings", None, &self.hits, window, cx))
            .child(div().flex().flex_1().min_h_0().child(nav).child(content))
    }
}

#[cfg(test)]
mod tests {
    use supraman_term::terminal_settings::CAMPBELL;
    use supraman_ui::themes::{DARK_MODERN, SUPRAMAN_DARK, hex, palette};

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

        Settings::update(home.path(), |s| choose_theme(s, SUPRAMAN_DARK, TerminalColors::Campbell)).unwrap();
        let saved = Settings::load(home.path()).unwrap();
        assert_eq!((saved.appearance.theme.as_deref(), saved.appearance.terminal_colors.as_deref()), (Some(SUPRAMAN_DARK), Some("campbell")));
        let dark = painted(&saved, home.path());
        assert_eq!((dark.0.as_str(), dark.2.clone()), (SUPRAMAN_DARK, hex(gpui::rgb(CAMPBELL[8]).into())));
        assert_ne!(dark.1, start.1, "Supraman Dark's panel is not Dark Modern's");

        Settings::update(home.path(), |s| choose_theme(s, DARK_MODERN, TerminalColors::Theme)).unwrap();
        let back = Settings::load(home.path()).unwrap();
        assert_eq!((back.appearance.theme.as_deref(), back.appearance.terminal_colors.as_deref()), (Some(DARK_MODERN), None));
        assert_eq!(painted(&back, home.path()), start);
    }
}
