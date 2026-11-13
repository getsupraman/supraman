//! The right pane: the tab strip with its new-shell split button, and the active tab's content.

pub(in crate::view) mod tab_menu;

use gpui::Focusable as _;

use super::*;

impl FleetView {
    pub(super) fn render_right(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        self.show_git_tabs(cx);
        let _stall = supraman_ui::stall::section("render: right");
        let colors = cx.theme().colors().clone();
        let headers = self
            .tabs
            .iter()
            .map(|tab| {
                let title = tab.label();
                let card = self.tab_card(tab, title.clone());
                let clipped = supraman_ui::clipped::clipped_with(format!("tab-title-{}", tab.id), title.clone(), false, false);
                TabHeader {
                    id: tab.id,
                    label: match tab.kind {
                        TabKind::Work(_) => div().flex().min_w_0().items_center().gap_1p5().child(crate::brand::workman(gpui::rems(1.), if self.active == Some(tab.id) { colors.text } else { colors.text_muted })).child(clipped).into_any_element(),
                        TabKind::Git(_) => div().flex().min_w_0().items_center().gap_1p5().child(Icon::GitGraph.svg(14., if self.active == Some(tab.id) { colors.text } else { colors.text_muted })).child(clipped).into_any_element(),
                        _ => clipped.into_any_element(),
                    },
                    title: title.into(),
                    tooltip: Some(Box::new(hover_card::tooltip(move || card.hover_card()))),
                }
            })
            .collect();
        let muted = colors.text_muted;
        let plus = button::toolbar(&self.hits, "new-shell", Icon::Plus.svg(16., muted), false, muted, cx)
            .tooltip(Tooltip::text("New shell"))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.new_shell(window, cx);
            }));
        let shells = button::toolbar(&self.hits, "new-shell-menu", Icon::ChevronDown.svg(12., muted), false, muted, cx)
            .tooltip(Tooltip::text("New shell of…"))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.open_shell_menu(cx)));
        let new_tab = button::split(plus, shells, cx).into_any_element();
        let bar = tab_strip(
            &self.hits,
            headers,
            Some(new_tab),
            self.active,
            || Icon::Close.svg(14., muted),
            |this: &mut Self, id, cx| this.activate_tab(id, cx),
            |this: &mut Self, id, cx| this.close_tab(id, cx),
            |this: &mut Self, from, to, cx| {
                supraman_ui::tabs::reorder(&mut this.tabs, from, to);
                cx.notify();
            },
            |this: &mut Self, id, at, cx| this.open_tab_menu(id, at, cx),
            window,
            cx,
        );

        let content = self.render_tab_content(cx);
        div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .min_w_0()
            .child(bar)
            .child(self.content_area(content, cx))
            .into_any_element()
    }

    /// The tab's content under [`FleetView::focus`]: a pane of its own unless its tab lays out panes.
    fn content_area(&mut self, content: AnyElement, cx: &mut Context<Self>) -> gpui::Div {
        let tab = self.active.and_then(|id| self.tab(id));
        let pane = match tab.map(|t| (&t.kind, t.terminal.is_some())) {
            None | Some((TabKind::Git(_) | TabKind::Work(_), _)) => None,
            Some((TabKind::Details(_), _)) => Some("details"),
            Some((TabKind::Schedule(_), _)) => Some("schedule"),
            Some((TabKind::Start(_), _)) => Some("start"),
            Some((_, true)) => Some("terminal"),
            Some(_) => Some("tab"),
        };
        let area = div().relative().flex_1().min_h_0().track_focus(&self.focus).child(content).children(self.pane_toast.render(&self.hits, "pane-toast", cx));
        let area = area.children(pane.map(|key| supraman_ui::focus::Pane::new(key, &self.focus).paint(&self.hits)));
        area.children(tab.map(|t| self.tab_keys(t, cx).paint()))
    }

    /// The shown tab takes the keys: what it held when last shown, else its view's own (the Git and Work tabs
    /// hand them to the graph and the list), else the content area's.
    fn tab_keys(&self, tab: &Tab, cx: &Context<Self>) -> supraman_ui::focus::Group {
        let entry = match &tab.kind {
            TabKind::Git(view) => view.focus_handle(cx),
            TabKind::Work(view) => view.focus_handle(cx),
            _ => self.focus.clone(),
        };
        let (fleet, id) = (cx.weak_entity(), tab.id);
        supraman_ui::focus::Group::new(format!("tab-{id}"), &self.focus, &entry).while_shown(move |cx| fleet.upgrade().is_some_and(|f| f.read(cx).active == Some(id)))
    }

    fn render_tab_content(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let active = self.active.and_then(|id| self.tab(id));
        let status = cx.theme().status().clone();
        match active {
            Some(Tab { kind: TabKind::Work(view), .. }) => view.clone().into_any_element(),
            Some(Tab { kind: TabKind::Git(view), .. }) => view.clone().into_any_element(),
            Some(Tab { kind: TabKind::Schedule(page), .. }) => self
                .render_schedule_page(page, cx)
                .unwrap_or_else(|| placeholder(format!("Schedule {} is gone.", page.id), cx)),
            Some(Tab { kind: TabKind::Details(page), .. }) => self.render_details_page(page, cx).unwrap_or_else(|| placeholder("Its group or folder is gone.".into(), cx)),
            Some(Tab { id, kind: TabKind::Start(page), .. }) => self.render_start_page(*id, page, cx).unwrap_or_else(|| placeholder(format!("{} is no longer listed.", page.entry), cx)),
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
        }
    }
}

impl FleetView {
    /// Each tab's branch for its hover card, read off the UI thread.
    pub(super) fn read_tab_branches(&mut self, cx: &mut Context<Self>) {
        let mut cwds: Vec<PathBuf> = self.tabs.iter().filter_map(|t| t.cwd.clone()).collect();
        cwds.sort();
        cwds.dedup();
        if cwds.is_empty() {
            return;
        }
        cx.spawn(async move |this, cx| {
            let read: Vec<(Option<String>, PathBuf)> =
                cx.background_spawn(async move { cwds.into_iter().map(|cwd| (supraman_core::branch_at(&cwd), cwd)).collect() }).await;
            this.update(cx, |this, _| {
                for tab in &mut this.tabs {
                    if let Some((branch, _)) = read.iter().find(|(_, cwd)| tab.cwd.as_ref() == Some(cwd)) {
                        tab.branch = branch.clone();
                    }
                }
            })
            .ok();
        })
        .detach();
    }
}
