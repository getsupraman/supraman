//! The right pane: the tab strip with its new-shell buttons, and the active tab's content.

use super::*;

impl FleetView {
    pub(super) fn render_right(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let _stall = supraman_ui::stall::section("render: right");
        let colors = cx.theme().colors().clone();
        let headers = self
            .tabs
            .iter()
            .map(|tab| {
                let title = tab.label();
                let card = self.tab_card(tab, title.clone());
                let cwd = tab.cwd.clone();
                let clipped = supraman_ui::clipped::clipped_with(format!("tab-title-{}", tab.id), title.clone(), false, false).max_w(px(280.));
                TabHeader {
                    id: tab.id,
                    label: match tab.kind {
                        TabKind::Work(_) => div().flex().items_center().gap_1p5().child(crate::brand::workman(gpui::rems(1.), if self.active == Some(tab.id) { colors.text } else { colors.text_muted })).child(clipped).into_any_element(),
                        _ => clipped.into_any_element(),
                    },
                    title: title.into(),
                    tooltip: Some(Box::new(hover_card::tooltip(move || {
                        tab_card::Card { branch: cwd.as_deref().and_then(supraman_core::branch_at), ..card.clone() }.hover_card()
                    }))),
                }
            })
            .collect();
        let muted = colors.text_muted;
        let mut bar = tab_strip(
            &self.hits,
            headers,
            self.active,
            || Icon::Close.svg(14., muted),
            |this: &mut Self, id, cx| this.activate_tab(id, cx),
            |this: &mut Self, id, cx| this.close_tab(id, cx),
            |this: &mut Self, from, to, cx| {
                supraman_ui::tabs::reorder(&mut this.tabs, from, to);
                cx.notify();
            },
            cx,
        );
        bar = bar.child(
            button::ghost(&self.hits, "new-shell", cx)
                .flex()
                .items_center()
                .justify_center()
                .h_full()
                .w(px(36.))
                .child(Icon::Plus.svg(16., colors.text_muted).group_hover("new-shell", |s| s.text_color(colors.text)))
                .tooltip(Tooltip::text("New shell"))
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.new_shell(window, cx);
                })),
        );
        bar = bar.child(
            button::ghost(&self.hits, "new-shell-menu", cx)
                .flex()
                .items_center()
                .justify_center()
                .h_full()
                .w(px(22.))
                .child(Icon::ChevronDown.svg(14., colors.text_muted).group_hover("new-shell-menu", |s| s.text_color(colors.text)))
                .tooltip(Tooltip::text("New shell of…"))
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.open_shell_menu(cx))),
        );

        let active = self.active.and_then(|id| self.tab(id));
        let status = cx.theme().status().clone();
        let content: AnyElement = match active {
            Some(Tab { kind: TabKind::Work(view), .. }) => view.clone().into_any_element(),
            Some(Tab { kind: TabKind::Schedule(page), .. }) => self
                .render_schedule_page(page, cx)
                .unwrap_or_else(|| placeholder(format!("Schedule {} of {} is gone.", page.id, page.workspace), cx)),
            Some(Tab { kind: TabKind::Details(page), .. }) => self.render_details_page(page, cx).unwrap_or_else(|| placeholder("Its workspace is gone.".into(), cx)),
            Some(Tab { id, kind: TabKind::Start(page), .. }) => self.render_start_page(*id, page, cx).unwrap_or_else(|| placeholder(format!("Workspace {} is gone.", page.workspace), cx)),
            Some(Tab { id: tab_id, terminal: Some(terminal), detach, find, .. }) => div()
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
                .children(self.render_schedule_ask(*tab_id, cx))
                .when_some(find.as_ref(), |d, find| d.child(self.find_bar(*tab_id, find, cx)))
                .child(
                    div()
                        .relative()
                        .flex_1()
                        .min_h_0()
                        .child(hit_target(&self.hits, format!("terminal-{tab_id}")))
                        .on_scroll({
                            let terminal = terminal.clone();
                            cx.listener(move |this, e: &ScrollEvent, _, cx| {
                                if e.modifiers.control {
                                    this.wheel_font_size(e.delta, cx);
                                    return;
                                }
                                terminal.update(cx, |terminal, _| {
                                    terminal.scroll_wheel(e, SCROLL_MULTIPLIER, MouseInputMode::ReportToTerminal);
                                });
                                // the queued Scroll is applied by the grid's prepaint sync
                                cx.notify();
                            })
                        })
                        .child(term_element::terminal_grid(terminal.clone(), self.term_font()))
                        .children(self.link_hint(*tab_id, terminal, cx))
                        .child(self.terminal_drop(*tab_id, cx)),
                )
                .into_any_element(),
            Some(Tab { id, kind: TabKind::Saved(saved), .. }) => self.saved_placeholder(*id, saved, cx),
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
            .child(div().relative().flex_1().min_h_0().child(content).children(self.pane_toast.render(&self.hits, "pane-toast", cx)))
            .into_any_element()
    }
}
