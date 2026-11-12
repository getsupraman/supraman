//! The settings window's Sessions page: the busy limit, elevation and the reopened tabs, `[sessions]`.

use gpui::{AnyElement, ClickEvent, Context, FontWeight, SharedString, div, prelude::*, px};
use supraman_ui::button;
use supraman_ui::settings_page::row;
use theme::ActiveTheme as _;

use super::SettingsView;

impl SettingsView {
    pub(super) fn set_max_working(&mut self, n: usize, cx: &mut Context<Self>) {
        // The fleet window saves it (one path for the app and its selftest) and shows any error.
        if let Ok(shown) = self.fleet.update(cx, |fleet, cx| {
            fleet.set_max_working(n, cx);
            fleet.supervising.max_working
        }) {
            self.settings.sessions.max_working = shown;
        }
        cx.notify();
    }

    pub(super) fn render_sessions(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let n = self.settings.sessions.max_working;
        let stepper = div()
            .flex()
            .items_center()
            .gap_1()
            .child(button::action(&self.hits, "settings-max-dec", "−", button::Tone::Neutral, button::Fill::Solid, cx).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
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
            .child(button::action(&self.hits, "settings-max-inc", "+", button::Tone::Neutral, button::Fill::Solid, cx).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.set_max_working(n + 1, cx);
            })));
        let admin = self.settings.sessions.run_as_admin;
        let toggle = button::action(&self.hits, "settings-run-as-admin", if admin { "On" } else { "Off" }, button::Tone::Neutral, button::Fill::Solid, cx)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.change(move |s| s.sessions.run_as_admin = !admin, cx)));
        let now = self.elevated.map(|elevated| {
            let now: SharedString =
                if elevated { "Supraman runs as administrator now.".into() } else { "Supraman runs unelevated now.".into() };
            div().text_xs().text_color(cx.theme().colors().text_muted).child(now).into_any_element()
        });
        let reopen = self.settings.sessions.reopen_tabs;
        let reopen_toggle = button::action(&self.hits, "settings-reopen-tabs", if reopen { "On" } else { "Off" }, button::Tone::Neutral, button::Fill::Solid, cx)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.change(move |s| s.sessions.reopen_tabs = !reopen, cx)));
        let on_click = self.settings.sessions.resume_tabs_on_click;
        let resume_toggle = button::action(&self.hits, "settings-resume-tabs-on-click", if on_click { "On a click" } else { "At once" }, button::Tone::Neutral, button::Fill::Solid, cx)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.change(move |s| s.sessions.resume_tabs_on_click = !on_click, cx)));
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
                now,
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
}
