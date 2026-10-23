//! The settings window, as Zed has one: its own OS window with a page list on the left and one
//! row per setting on the right. Every change is written to `settings.toml` at once; the
//! supervisor re-reads it on its next tick.

use std::path::PathBuf;

use foreman_core::Settings;
use gpui::{
    AnyElement, App, AppContext as _, Bounds, ClickEvent, Context, FocusHandle, FontWeight, KeyDownEvent,
    PathPromptOptions, SharedString, TitlebarOptions, WeakEntity, Window, WindowBounds, WindowHandle,
    WindowOptions, div, prelude::*, px, size,
};
use theme::ActiveTheme as _;

use crate::tokens::{self, DEFAULT_WORKSTATE_URL};
use crate::view::FleetView;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Sessions,
    WorkState,
}

impl Page {
    const ALL: [Page; 2] = [Page::Sessions, Page::WorkState];

    fn title(self) -> &'static str {
        match self {
            Page::Sessions => "Sessions",
            Page::WorkState => "WorkState",
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
}

pub fn open(home: Option<PathBuf>, fleet: WeakEntity<FleetView>, cx: &mut App) -> Option<WindowHandle<SettingsView>> {
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
            let view = cx.new(|cx| SettingsView::new(home, fleet, cx));
            let focus = view.read(cx).focus.clone();
            window.focus(&focus, cx);
            view
        },
    )
    .ok()
}

impl SettingsView {
    fn new(home: Option<PathBuf>, fleet: WeakEntity<FleetView>, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            home,
            settings: Settings::default(),
            error: None,
            fleet,
            page: Page::Sessions,
            focus: cx.focus_handle(),
        };
        this.reload();
        this
    }

    fn reload(&mut self) {
        let Some(home) = &self.home else {
            self.error = Some("No Foreman home (set FOREMAN_HOME or APPDATA): settings cannot be saved.".into());
            return;
        };
        match Settings::load(home) {
            Ok(settings) => self.settings = settings,
            Err(e) => self.error = Some(format!("Cannot read settings.toml: {e}")),
        }
    }

    /// Load, change, save: the file may have been edited by hand or by the fleet window meanwhile.
    fn change(&mut self, edit: impl FnOnce(&mut Settings), cx: &mut Context<Self>) {
        let Some(home) = self.home.clone() else {
            return;
        };
        let result = Settings::load(&home).and_then(|mut settings| {
            edit(&mut settings);
            settings.save(&home)?;
            Ok(settings)
        });
        match result {
            Ok(settings) => {
                self.settings = settings;
                self.error = None;
            }
            Err(e) => self.error = Some(format!("Cannot save settings.toml: {e}")),
        }
        cx.notify();
    }

    fn set_max_working(&mut self, n: usize, cx: &mut Context<Self>) {
        // The fleet window saves it (one path for the app and its selftest) and shows any error.
        self.fleet.update(cx, |fleet, cx| fleet.set_max_working(n, cx)).ok();
        self.reload();
        cx.notify();
    }

    fn choose_env(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Use as WorkState token file".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            this.update(cx, |this, cx| this.change(|s| s.workstate_env = Some(path), cx)).ok();
        })
        .detach();
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

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, _cx: &mut Context<Self>) {
        if event.keystroke.key == "escape" {
            window.remove_window();
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
        vec![row(
            "Busy sessions at most",
            "How many Foreman sessions may be working (mid-turn) at the same time, counted across all \
             workspaces. Launches and resumes wait while the limit is reached. 0 pauses them all. \
             Your own sessions are not counted.",
            stepper.into_any_element(),
            None,
            cx,
        )]
    }

    fn render_workstate(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let colors = cx.theme().colors().clone();
        let env_text: SharedString = match &self.settings.workstate_env {
            Some(path) => path.display().to_string().into(),
            None => "not set".into(),
        };
        let env_control = div()
            .flex()
            .items_center()
            .gap_2()
            .child(small_button("env-choose", "Choose…", cx).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.choose_env(cx);
            })))
            .when(self.settings.workstate_env.is_some(), |d| {
                d.child(small_button("env-clear", "Clear", cx).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.change(|s| s.workstate_env = None, cx);
                })))
            });

        // Which line each role would use; the token itself is never shown.
        let found = ["workspace", "repo"].map(|role| {
            let own = tokens::token_key(role);
            let via = tokens::workstate_role(role).map(tokens::token_key);
            let via = via.as_deref();
            let text = match &self.settings.workstate_env {
                None => format!("{role} sessions: no WorkState"),
                Some(env) => {
                    let has = |key: &str| tokens::read_env_value(env, key).ok().flatten().is_some();
                    if has(&own) {
                        format!("{role} sessions: {own} ✓")
                    } else if let Some(via) = via.filter(|k| has(k)) {
                        format!("{role} sessions: {via} ✓")
                    } else {
                        format!("{role} sessions: no {own} or {} line, so no WorkState", via.unwrap_or("-"))
                    }
                }
            };
            div().text_xs().text_color(colors.text_muted).child(SharedString::from(text)).into_any_element()
        });

        let url = self.settings.workstate_url.clone();
        vec![
            row(
                "Token file",
                "A .env file with WORKSTATE_<ROLE>_TOKEN=… lines, e.g. the WorkState server's own \
                 mcp/.env. A workspace session uses WORKSTATE_WORKSPACE_TOKEN, else \
                 WORKSTATE_CONTROLLER_TOKEN; a repo session WORKSTATE_REPO_TOKEN, else \
                 WORKSTATE_ORCHESTRATOR_TOKEN. Read when a session launches; tokens are never shown \
                 or logged. Not set: Foreman's sessions run without WorkState.",
                env_control.into_any_element(),
                Some(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .pt_2()
                        .child(div().text_sm().child(env_text))
                        .children(found)
                        .into_any_element(),
                ),
                cx,
            ),
            row(
                "Server URL",
                "The WorkState MCP endpoint. Change it in settings.toml (workstate_url).",
                div()
                    .text_sm()
                    .child(SharedString::from(match url {
                        Some(url) => url,
                        None => format!("{DEFAULT_WORKSTATE_URL} (default)"),
                    }))
                    .into_any_element(),
                None,
                cx,
            ),
        ]
    }
}

fn row(
    title: &'static str,
    description: &'static str,
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
                .child(div().text_xs().text_color(colors.text_muted).child(description))
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
        .child(label)
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
                .child("Open settings.toml")
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.open_file(cx))),
        );

        let rows = match self.page {
            Page::Sessions => self.render_sessions(cx),
            Page::WorkState => self.render_workstate(cx),
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
