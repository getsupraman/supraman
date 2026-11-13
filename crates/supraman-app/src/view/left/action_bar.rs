//! The sidebar's action bar: add, expand and collapse, stopped sessions, refresh, the CLI warning, the busy limit, Settings.

use super::*;

impl FleetView {
    pub(super) fn render_action_bar(&self, hidden_stopped: usize, cx: &mut Context<Self>) -> Stateful<Div> {
        let colors = cx.theme().colors().clone();
        let status = cx.theme().status().clone();
        let hits = &self.hits;
        let separator = || div().w(px(1.)).h(px(16.)).mx_1().bg(colors.border_variant);
        let warning = match &self.cli {
            Cli::Supported(version) => version.untested(),
            _ => None,
        };
        div()
            .id("action-bar")
            .flex()
            .flex_none()
            .items_center()
            .gap(px(2.))
            .px_2()
            .py(px(3.))
            .border_b_1()
            .border_color(colors.border)
            .child(
                icon_button(hits, "add-folder", Icon::FolderAdd, "Add Folder or Group…", false, cx)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.open_add_menu(cx))),
            )
            .child(separator())
            .child(
                icon_button(hits, "expand-all", Icon::ExpandAll, "Expand all", false, cx)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.collapse_all(false, cx))),
            )
            .child(
                icon_button(hits, "collapse-all", Icon::CollapseAll, "Collapse all", false, cx)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.collapse_all(true, cx))),
            )
            .child(self.stopped_toggle(hidden_stopped, cx))
            .child(
                button::toolbar_turning(hits, "refresh", Icon::Refresh.svg(16., colors.text_muted), false, self.refreshing(), colors.text_muted, cx)
                    .tooltip(Tooltip::text("Refresh: read the running sessions and the closed ones' transcripts now (F5 outside a terminal)"))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.refresh_now(cx))),
            )
            .child(div().flex_1())
            .children(warning.map(|w| {
                tinted_button(hits, "cli-warning", Icon::Warning, w, false, status.warning, cx)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.open_settings_at(Some(page::ABOUT), cx)))
            }))
            .child(self.busy_limit(cx))
            .child(
                icon_button(hits, "bar-settings", Icon::Settings, "Settings (Ctrl+,)", false, cx)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.open_settings(cx))),
            )
    }

    /// The add button's menu, under the button.
    fn open_add_menu(&mut self, cx: &mut Context<Self>) {
        let position = self.hits.borrow().get("add-folder").map(|h| h.bounds.bottom_left()).unwrap_or_default();
        self.dialogs.menu = Some(super::super::menu::ContextMenu::new("Add", position, super::super::menu::add_items()));
        cx.notify();
    }

    fn stopped_toggle(&self, hidden_stopped: usize, cx: &mut Context<Self>) -> Stateful<Div> {
        let stopped_tip: SharedString = if self.show_stopped {
            "Hide stopped sessions".into()
        } else {
            format!("Show stopped sessions ({hidden_stopped} hidden)").into()
        };
        icon_button(
            &self.hits,
            "toggle-stopped",
            if self.show_stopped { Icon::Eye } else { Icon::EyeOff },
            stopped_tip,
            self.show_stopped,
            cx,
        )
        .when(!self.show_stopped && hidden_stopped > 0, |d| d.child(SharedString::from(hidden_stopped.to_string())))
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            this.show_stopped = !this.show_stopped;
            cx.notify();
        }))
    }

    fn busy_limit(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let colors = cx.theme().colors().clone();
        let hits = &self.hits;
        let n = self.polled.max_working;
        let busy_tip: SharedString = format!(
            "At most {n} session(s) of this Supraman working at once, across its folders; launches wait while it is reached. 0 pauses them all. Another Supraman's sessions are not counted."
        )
        .into();
        div()
            .id("busy-limit")
            .flex()
            .flex_none()
            .items_center()
            .h(px(26.))
            .rounded_md()
            .border_1()
            .border_color(colors.border_variant)
            .child(
                icon_button(hits, "busy-dec", Icon::Minus, "One fewer busy session", false, cx)
                    .h(px(24.))
                    .min_w(px(22.))
                    .px(px(3.))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.set_max_working(this.polled.max_working.saturating_sub(1), cx))),
            )
            .child(
                div()
                    .id("busy-value")
                    .px_1()
                    .text_xs()
                    .text_color(colors.text_muted)
                    .child(SharedString::from(format!("busy ≤ {n}")))
                    .tooltip(Tooltip::text(busy_tip)),
            )
            .child(
                icon_button(hits, "busy-inc", Icon::Plus, "One more busy session", false, cx)
                    .h(px(24.))
                    .min_w(px(22.))
                    .px(px(3.))
                    .when(n >= MAX_WORKING_LIMIT, |d| d.opacity(0.4))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.set_max_working(this.polled.max_working + 1, cx))),
            )
    }
}
