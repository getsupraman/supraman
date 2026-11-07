//! The left column: Overview, Needs you and Activity, then each bound project with its active runs and its closed ones.

use gpui::{ClickEvent, FontWeight};
use supraman_ui::clipped::clipped;
use supraman_ui::icon::Icon;
use supraman_ui::marks::progress_line;
use supraman_ui::{button, hit::hit_target};
use supraman_workman::{OverviewRun, Rollup};

use super::projects::{active, closed_runs};
use super::*;

pub fn done_total(r: &Rollup) -> String {
    format!("{}/{}", r.done, r.counted())
}

impl WorkView {
    fn nav_entry(&self, key: &str, icon: Icon, label: &str, on: bool, badge: Option<usize>, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let colors = cx.theme().colors().clone();
        let error = cx.theme().status().error;
        button::ghost(&self.hits, key.to_string(), cx)
            .flex()
            .items_center()
            .gap_2()
            .mx_2()
            .px_2()
            .py(px(3.))
            .rounded_sm()
            .text_sm()
            .when(on, |d| d.bg(colors.element_selected))
            .child(icon.svg(14., colors.icon_muted))
            .child(div().flex_1().min_w_0().child(SharedString::from(label.to_string())))
            .children(badge.filter(|n| *n > 0).map(|n| {
                div().flex_none().px(px(6.)).rounded_full().bg(error).text_xs().text_color(colors.background).child(SharedString::from(n.to_string()))
            }))
    }

    pub(super) fn render_navigator(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let needs = self.needs_you(cx);
        let head = div()
            .flex()
            .items_center()
            .gap_1()
            .px_3()
            .pt_2()
            .pb_1()
            .child(clipped("work-nav-workspace", self.workspace.clone(), false).flex_1().text_sm().font_weight(FontWeight::SEMIBOLD))
            .child(
                button::icon(&self.hits, "work-nav-shut", Icon::ChevronLeft.svg(14., colors.icon_muted), px(20.), cx)
                    .tooltip(supraman_ui::tooltip::Tooltip::text("Hide the navigator"))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_nav(cx))),
            );
        let overview = self.nav_entry("work-nav-overview", Icon::Home, "Overview", self.page == Page::Overview, None, cx)
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.show_page(Page::Overview, cx)));
        let inbox = self
            .nav_entry("work-nav-inbox", Icon::Chat, "Needs you", false, Some(needs), cx)
            .child(hit_target(&self.hits, format!("work-nav-inbox={needs}")))
            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(WorkEvent::ShowInbox)));
        let activity = self.nav_entry("work-nav-activity", Icon::Clock, "Activity", self.page == Page::Activity, None, cx)
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.show_page(Page::Activity, cx)));
        let mut list = div()
            .id("work-nav")
            .size_full()
            .flex()
            .flex_col()
            .gap(px(2.))
            .overflow_y_scroll()
            .bg(colors.panel_background)
            .child(head)
            .child(overview)
            .child(inbox)
            .child(activity)
            .child(div().px_3().pt_3().pb_1().text_xs().font_weight(FontWeight::BOLD).text_color(colors.text_muted).child("PROJECTS"));
        for project in &self.projects {
            list = list.child(self.render_project(project, cx));
        }
        list.child(div().h(px(16.)).flex_none()).into_any_element()
    }

    fn render_project(&self, project: &OverviewProject, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let qref = project.item.qualified_ref.clone();
        let mut out = div().flex().flex_col().gap(px(1.)).pb_1().child(
            div()
                .relative()
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .pt_1()
                .child(clipped(format!("work-project-title-{qref}"), project.item.title.clone(), false).flex_1().text_sm().font_weight(FontWeight::SEMIBOLD))
                .child(div().flex_none().font_family(supraman_ui::fonts::MONO).text_xs().text_color(colors.text_muted).child(SharedString::from(qref.clone())))
                .child(hit_target(&self.hits, format!("work-project-{qref}"))),
        );
        for run in active(project) {
            out = out.child(self.render_nav_run(run, cx));
        }
        let closed: Vec<&OverviewRun> = closed_runs(project).collect();
        if !closed.is_empty() {
            let open = self.closed_open.contains(&project.item.id);
            let id = project.item.id.clone();
            out = out.child(
                button::ghost(&self.hits, format!("work-closed-{qref}"), cx)
                    .flex()
                    .items_center()
                    .gap_1()
                    .mx_2()
                    .px_2()
                    .rounded_sm()
                    .text_xs()
                    .text_color(colors.text_muted)
                    .child((if open { Icon::ChevronDown } else { Icon::ChevronRight }).svg(12., colors.icon_muted))
                    .child(SharedString::from(format!("{} closed", closed.len())))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle_closed(&id, cx))),
            );
            if open {
                for run in closed {
                    out = out.child(self.render_nav_run(run, cx));
                }
            }
        }
        out.into_any_element()
    }

    fn render_nav_run(&self, run: &OverviewRun, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let status = cx.theme().status().clone();
        let qref = run.item.qualified_ref.clone();
        let on = self.shown_run() == Some(run.item.id.as_str());
        let r = &run.rollup;
        let parts = [(r.done, status.success), (r.in_progress + r.review, status.info), (r.blocked, status.error)];
        let id = run.item.id.clone();
        button::ghost(&self.hits, format!("work-nav-run-{qref}"), cx)
            .flex()
            .flex_col()
            .gap(px(3.))
            .mx_2()
            .px_2()
            .py(px(3.))
            .rounded_sm()
            .when(on, |d| d.bg(colors.element_selected))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_sm()
                    .child(clipped(format!("work-nav-run-title-{qref}"), run.item.title.clone(), false).flex_1().when(on, |d| d.text_color(colors.text_accent)))
                    .child(supraman_ui::fonts::figures(div().flex_none().text_xs().text_color(colors.text_muted)).child(SharedString::from(done_total(r)))),
            )
            .child(progress_line(&parts, r.counted(), colors.border))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.open_run(id.clone(), cx)))
            .into_any_element()
    }

    pub(super) fn render_nav_strip(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        div()
            .h_full()
            .w(px(28.))
            .flex()
            .flex_col()
            .items_center()
            .pt_2()
            .bg(colors.panel_background)
            .child(
                button::icon(&self.hits, "work-nav-open", Icon::ChevronRight.svg(14., colors.icon_muted), px(20.), cx)
                    .tooltip(supraman_ui::tooltip::Tooltip::text("Show the navigator"))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_nav(cx))),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_runs_count_leaves_the_cancelled_out() {
        let r = Rollup { total: 14, done: 10, cancelled: 1, ..Default::default() };
        assert_eq!(done_total(&r), "10/13");
    }
}
