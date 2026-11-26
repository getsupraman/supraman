//! The left column: Overview, Needs you and Activity, then each bound project with its active runs and its closed ones.

use gpui::{ClickEvent, FontWeight};
use supraman_ui::clipped::clipped;
use supraman_ui::icon::Icon;
use supraman_ui::marks::progress_line;
use supraman_ui::{controls::button, hit::hit_target};
use supraman_workman::{OverviewRun, Rollup};

use super::projects::{active, closed_runs};
use super::*;
use supraman_ui::label::{Label, LabelMark};
use supraman_ui::tokens;

/// The shut navigator's strip.
const SHUT_WIDTH: gpui::Pixels = gpui::px(28.);

/// A navigator row's chrome: selected while the navigator's selection is on it, active while the navigator holds the keys.
pub(crate) fn nav_chrome(selected: bool, focused: bool) -> supraman_ui::list::Chrome {
    supraman_ui::list::Chrome::new().selected(selected).focused(focused).radius(tokens::radius::S)
}

pub(crate) fn done_total(r: &Rollup) -> String {
    format!("{}/{}", r.done, r.counted())
}

impl WorkView {
    /// A navigator row on the list core, selected while the navigator's selection is on it; a click clears the keys' move.
    fn nav_row(&self, key: &str, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let chrome = nav_chrome(self.nav_selected().as_deref() == Some(key), self.keys_in.0);
        supraman_ui::list::row(&self.hits, button::RowKeys::one(key.to_string().into()), chrome, cx).on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
            this.nav_cursor = None;
            cx.notify();
        }))
    }

    fn nav_entry(&self, key: &str, icon: Icon, label: &str, badge: Option<usize>, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let colors = cx.theme().colors().clone();
        self.nav_row(key, cx)
            .flex()
            .items_center()
            .gap_2()
            .mx_2()
            .px_2()
            .py(tokens::space::S)
            .rounded(tokens::radius::S)
            .text_sm()
            .child(Label::new(label.to_string()).mark(LabelMark::Svg(icon.svg(14., colors.icon_muted))).gap(tokens::space::L).flex_1())
            .children(badge.filter(|n| *n > 0).map(|n| supraman_ui::controls::badge::alert_count(n, cx)))
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
            .child(clipped("work-nav-name", self.name(), false).flex_1().text_sm().font_weight(FontWeight::SEMIBOLD))
            .child(
                button::Button::new(&self.hits, "work-nav-shut").icon(Icon::ChevronLeft.svg(14., colors.icon_muted)).icon_only(tokens::control::SMALL.height)
                    .tooltip(supraman_ui::controls::tooltip::Tooltip::text("Hide the navigator"))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_nav(cx))),
            );
        let overview = self.nav_entry("work-nav-overview", Icon::Home, "Overview", None, cx)
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.show_page(Page::Overview, cx)));
        let inbox = self
            .nav_entry("work-nav-inbox", Icon::Chat, "Needs you", Some(needs), cx)
            .child(hit_target(&self.hits, format!("work-nav-inbox={needs}")))
            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(WorkEvent::ShowInbox)));
        let activity = self.nav_entry("work-nav-activity", Icon::Clock, "Activity", None, cx)
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.show_page(Page::Activity, cx)));
        let mut list = supraman_ui::scroll::area("work-nav")
            .track(&self.nav_scroll)
            .size_full()
            .flex()
            .flex_col()
            .gap(tokens::space::XS)
            .bg(colors.panel_background)
            .child(head)
            .child(overview)
            .child(inbox)
            .child(activity)
            .child(div().px_3().pt_3().pb_1().text_xs().font_weight(FontWeight::BOLD).text_color(colors.text_muted).child("PROJECTS"));
        for project in &self.projects {
            list = list.child(self.render_project(project, cx));
        }
        list.child(div().h(tokens::space::XXL).flex_none()).into_any_element()
    }

    fn render_project(&self, project: &OverviewProject, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let qref = project.item.qualified_ref.clone();
        let mut out = div().flex().flex_col().gap(tokens::space::XS).pb_1().child(
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
                self.nav_row(&format!("work-closed-{qref}"), cx)
                    .flex()
                    .items_center()
                    .gap_1()
                    .mx_2()
                    .px_2()
                    .rounded(tokens::radius::S)
                    .text_xs()
                    .text_color(colors.text_muted)
                    .child(Label::new(format!("{} closed", closed.len())).mark(LabelMark::Svg((if open { Icon::ChevronDown } else { Icon::ChevronRight }).svg(12., colors.icon_muted))).gap(tokens::space::S))
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
        self.nav_row(&format!("work-nav-run-{qref}"), cx)
            .flex()
            .flex_col()
            .gap(tokens::space::S)
            .mx_2()
            .px_2()
            .py(tokens::space::S)
            .rounded(tokens::radius::S)
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
            .w(SHUT_WIDTH)
            .flex()
            .flex_col()
            .items_center()
            .pt_2()
            .bg(colors.panel_background)
            .child(
                button::Button::new(&self.hits, "work-nav-open").icon(Icon::ChevronRight.svg(14., colors.icon_muted)).icon_only(tokens::control::SMALL.height)
                    .tooltip(supraman_ui::controls::tooltip::Tooltip::text("Show the navigator"))
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

    struct Nav {
        hits: supraman_ui::hit::HitTargets,
        popup: supraman_ui::popup::Popup,
    }

    impl gpui::Render for Nav {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            self.popup.shown(window, cx);
            let chromes = [nav_chrome(true, true), nav_chrome(true, false), nav_chrome(false, true), nav_chrome(false, true)];
            div().w(px(200.)).flex().flex_col().children(chromes.into_iter().enumerate().map(|(i, c)| supraman_ui::list::row(&self.hits, button::RowKeys::one(format!("n-{i}").into()), c, cx).h(px(20.)).child("entry")))
        }
    }

    fn fills_at(view: &gpui::Entity<Nav>, hit: &str, hover: bool, cx: &mut gpui::VisualTestContext) -> Vec<gpui::Background> {
        let bounds = view.read_with(cx, |v, _| v.hits.borrow().get(hit).map(|h| h.bounds)).unwrap_or_else(|| panic!("{hit} paints"));
        let at = if hover { bounds.center() } else { gpui::point(px(500.), px(500.)) };
        cx.simulate_mouse_move(at, None, gpui::Modifiers::none());
        view.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        cx.update(|window, _| {
            let middle = bounds.center().scale(window.scale_factor());
            window.painted_quads().into_iter().filter(|q| q.bounds.contains(&middle) && q.content_mask.bounds.contains(&middle)).map(|q| q.background).collect()
        })
    }

    #[gpui::test]
    fn navigator_rows_take_their_hover_selection_and_open_fills_from_the_roles(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| supraman_ui::init_globals(cx, "test"));
        let popup = supraman_ui::popup::Popup::open(Some("n-3".into()), supraman_ui::tokens::Layer::Menu);
        let (view, cx) = cx.add_window_view(|_, _| Nav { hits: Default::default(), popup });
        cx.run_until_parked();
        let (roles, hover) = cx.update(|_, cx| (supraman_ui::tokens::roles(cx), cx.theme().colors().ghost_element_hover));
        let [active, inactive, open, hover] = [roles.selection, roles.selection_inactive, roles.open, hover].map(gpui::Background::from);
        assert!(fills_at(&view, "n-0", false, cx).contains(&active), "the selected entry while the navigator has the keys");
        assert!(fills_at(&view, "n-1", false, cx).contains(&inactive), "without the keys: the inactive role");
        assert!(fills_at(&view, "n-2", true, cx).contains(&hover), "an entry under the pointer: the ghost hover");
        assert!(fills_at(&view, "n-3", false, cx).contains(&open), "an entry under its open menu: the open role");
    }
}
