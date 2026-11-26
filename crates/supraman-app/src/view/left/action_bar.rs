//! The sidebar's action bar: add, expand and collapse, stopped sessions, refresh, the CLI warning, the busy limit, Settings.

use supraman_ui::tokens;
use super::*;
use supraman_ui::controls::stepper::stepper;

impl FleetView {
    pub(super) fn render_action_bar(&self, hidden_stopped: usize, cx: &mut Context<Self>) -> Stateful<Div> {
        let colors = cx.theme().colors().clone();
        let status = cx.theme().status().clone();
        let hits = &self.hits;
        let separator = || div().w(tokens::stroke::HAIRLINE).h(tokens::space::XXL).mx_1().bg(colors.border_variant);
        let warning = match &self.cli {
            Cli::Supported(version) => version.untested(),
            _ => None,
        };
        div()
            .id("action-bar")
            .flex()
            .flex_none()
            .items_center()
            .gap(tokens::space::XS)
            .px_2()
            .py(tokens::space::S)
            .border_b(tokens::stroke::HAIRLINE)
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
                button::Button::toolbar(hits, "refresh").icon(Icon::Refresh.svg(16., colors.text_muted)).turning(self.refreshing()).text_color(colors.text_muted)
                    .tooltip(Tooltip::text("Refresh: read the running sessions and the closed ones' transcripts now (F5 outside a terminal)"))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.refresh_now(cx))),
            )
            .child(div().flex_1())
            .children(warning.map(|w| {
                tinted_button(hits, "cli-warning", Icon::Warning, w, false, status.warning)
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
        self.show_add_menu(position, cx);
        self.dialogs.menu = self.dialogs.menu.take().map(|m| m.anchored("add-folder"));
    }

    /// Add Folder… and Add Group… at `position`: the add button's menu, and a right-click below the tree's rows.
    pub(super) fn show_add_menu(&mut self, position: gpui::Point<Pixels>, cx: &mut Context<Self>) {
        self.dialogs.menu = Some(super::super::menu::ContextMenu::new("Add", position, super::super::menu::add_items()));
        cx.notify();
    }

    fn stopped_toggle(&self, hidden_stopped: usize, cx: &mut Context<Self>) -> button::Button {
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
        let n = self.polled.max_working;
        let busy_tip: SharedString = format!(
            "At most {n} session(s) of this Supraman working at once, across its folders; launches wait while it is reached. 0 pauses them all. Another Supraman's sessions are not counted."
        )
        .into();
        let limit = stepper(&self.hits, "busy", format!("busy ≤ {n}"), None, |this: &mut Self, by, _, cx| {
            let n = this.polled.max_working;
            this.set_max_working(if by < 0 { n.saturating_sub(1) } else { n + 1 }, cx);
        }, cx);
        div().id("busy-limit").flex_none().child(limit).tooltip(Tooltip::text(busy_tip))
    }
}
