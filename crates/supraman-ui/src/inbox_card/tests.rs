use std::cell::Cell;

use gpui::{Context, Entity, Modifiers, Render, TestAppContext, Window};

use super::*;
use crate::icon::Icon;

const NOW: i64 = 1_762_430_400_000;

struct Holder {
    hits: HitTargets,
    clicked: Rc<Cell<u32>>,
    acted: Rc<Cell<u32>>,
}

impl Render for Holder {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (clicked, acted) = (self.clicked.clone(), self.acted.clone());
        let act = action(&self.hits, "c-act", "c", Icon::Close.svg(ICON, icon_color(cx)), "Dismiss", cx).on_click(move |_, _, cx| {
            cx.stop_propagation();
            acted.set(acted.get() + 1);
        });
        let card = InboxCard::new("c", "blocked", "blocked", "A title")
            .mark(|color| div().size(px(12.)).bg(color).into_any_element(), cx)
            .context("ws")
            .when(Some(NOW - 60_000), NOW)
            .line("ref", "r1/t1", true)
            .line("detail", "", false)
            .line("why", "the reason", false)
            .action(act)
            .on_click(move |_, _, _| clicked.set(clicked.get() + 1));
        div().w(px(320.)).child(card.render(&self.hits, cx))
    }
}

fn open(cx: &mut TestAppContext) -> (Entity<Holder>, &mut gpui::VisualTestContext) {
    cx.update(|cx| crate::init_globals(cx, "test"));
    let (view, cx) = cx.add_window_view(|_, _| Holder { hits: HitTargets::default(), clicked: Rc::default(), acted: Rc::default() });
    cx.run_until_parked();
    (view, cx)
}

#[gpui::test]
fn the_head_then_the_title_then_each_non_empty_line(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    let hits = view.read_with(cx, |h, _| h.hits.clone());
    let b = |key: &str| hits.borrow().get(key).unwrap_or_else(|| panic!("{key} is painted")).bounds;
    let (head, title, ref_, why) = (b("c-head"), b("c-title"), b("c-ref"), b("c-why"));
    assert!(head.bottom() <= title.top() + px(1.) && title.bottom() <= ref_.top() + px(1.) && ref_.bottom() <= why.top() + px(1.));
    assert!(hits.borrow().get("c-detail").is_none(), "an empty line is not painted");
    let act = b("c-act");
    assert!(act.top() >= head.top() - px(1.) && act.bottom() <= head.bottom() + px(1.), "the action sits in the head");
}

#[gpui::test]
fn a_click_on_the_card_is_the_owners_and_an_action_stops_it(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    let at = |key: &str, cx: &mut gpui::VisualTestContext| view.read_with(cx, |h, _| h.hits.borrow()[key].bounds.center());
    let title = at("c-title", cx);
    cx.simulate_click(title, Modifiers::none());
    let act = at("c-act", cx);
    cx.simulate_click(act, Modifiers::none());
    view.read_with(cx, |h, _| assert_eq!((h.clicked.get(), h.acted.get()), (1, 1)));
}

#[gpui::test]
fn blocked_is_the_warning_colour_whoever_raised_it(cx: &mut TestAppContext) {
    cx.update(|cx| {
        crate::init_globals(cx, "test");
        let status = cx.theme().status().clone();
        assert_eq!(kind_color("blocked", cx), status.warning);
        assert_eq!(kind_color("blocker_finding", cx), status.error);
        assert_eq!(kind_color("run failed", cx), status.error);
        assert_eq!(kind_color("duplicate", cx), status.hint);
        assert_eq!(kind_color("stale_in_progress", cx), status.info);
    });
}
