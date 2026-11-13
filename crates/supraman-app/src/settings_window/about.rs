//! The settings window's About page: this build, the update check, the claude CLI, the poll and the home.

use gpui::{AnyElement, App, ClickEvent, Context, Hsla, SharedString, div, prelude::*, px};
use supraman_ui::settings_page::row;
use supraman_ui::{banner, button};
use theme::ActiveTheme as _;

use super::SettingsView;
use crate::view::{Cli, REFRESH_EVERY};

impl SettingsView {
    pub(super) fn render_about(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let colors = cx.theme().colors().clone();
        let (cli, warning, refresh) = self.cli_and_refresh(cx);
        let value = |id: &'static str, text: String| div().id(id).text_sm().child(SharedString::from(text)).into_any_element();
        let home = self.home.as_ref().map_or("not set".to_string(), |h| h.display().to_string());
        let [check, last_check] = self.render_update_rows(cx);
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
            row("Supraman", "This build.", value("about-supraman", env!("CARGO_PKG_VERSION").to_string()), None, cx),
            check,
            last_check,
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
                "How often the session list (`claude agents`) is polled.",
                value("about-refresh", refresh),
                None,
                cx,
            ),
            row(
                "Supraman home",
                "SUPRAMAN_HOME, else %APPDATA%\\Supraman: workspaces, settings.toml, hook events, session token records.",
                div().max_w(px(300.)).text_xs().text_color(colors.text_muted).child(SharedString::from(home)).into_any_element(),
                None,
                cx,
            ),
        ]
    }

    /// The claude CLI's version and any warning in its colour, and the poll's interval and last time.
    fn cli_and_refresh(&self, cx: &App) -> (String, Option<(String, Hsla)>, String) {
        let status = cx.theme().status().clone();
        match self.fleet.upgrade() {
            Some(fleet) => {
                let fleet = fleet.read(cx);
                let (cli, warning) = match &fleet.cli {
                    Cli::Checking => ("checking…".to_string(), None),
                    Cli::Supported(v) => (v.raw.clone(), v.untested().map(|w| (w, status.warning))),
                    Cli::Failed(e) => ("unavailable".to_string(), Some((e.clone(), status.error))),
                };
                let refresh = match fleet.polls.last_refresh_took {
                    Some(took) => format!("every {} s · the last took {} ms", REFRESH_EVERY.as_secs(), took.as_millis()),
                    None => format!("every {} s · loading…", REFRESH_EVERY.as_secs()),
                };
                (cli, warning, refresh)
            }
            None => ("the main window is closed".to_string(), None, "-".to_string()),
        }
    }

    fn render_update_rows(&self, cx: &mut Context<Self>) -> [AnyElement; 2] {
        let on = self.settings.updates.check;
        let check_toggle = button::action(&self.hits, "settings-updates-check", if on { "On" } else { "Off" }, button::Tone::Neutral, button::Fill::Solid, cx)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_update_check(!on, cx)));
        let last_check = div()
            .flex()
            .items_center()
            .gap_2()
            .child(div().id("updates-last").max_w(px(260.)).text_sm().child(SharedString::from(self.updates_line(cx))))
            .children(on.then(|| {
                button::action(&self.hits, "settings-updates-check-now", "Check now", button::Tone::Neutral, button::Fill::Solid, cx).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.fleet.update(cx, |fleet, _| fleet.check_updates_now()).ok();
                }))
            }));
        [
            row(
                "Check for updates",
                "At start and once a day Supraman asks GitHub whether a newer release is published, and shows it above the session list. Off sends no request.",
                check_toggle.into_any_element(),
                None,
                cx,
            ),
            row("Last check", "What the last update check found.", last_check.into_any_element(), None, cx),
        ]
    }

    pub fn set_update_check(&mut self, on: bool, cx: &mut Context<Self>) {
        self.change_then(
            move |s| s.updates.check = on,
            move |this, cx| {
                if on {
                    this.fleet.update(cx, |fleet, _| fleet.check_updates_now()).ok();
                }
            },
            cx,
        );
    }

    pub fn updates_line(&self, cx: &App) -> String {
        let on = self.settings.updates.check;
        self.fleet.upgrade().map_or_else(|| "the main window is closed".into(), |f| f.read(cx).updates.status_line(on))
    }
}
