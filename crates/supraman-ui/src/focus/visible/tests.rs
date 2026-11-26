use gpui::{Context, Entity, Modifiers, Render, TestAppContext, VisualTestContext, Window, prelude::*};

use crate::button::{Button, Size};
use crate::focus::{root, visible};
use crate::focus_order::Members;
use crate::hit::HitTargets;

struct Two {
    hits: HitTargets,
    members: Members,
}

impl Render for Two {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.members.begin();
        let mut row = Button::line();
        for n in 0..2 {
            let key = format!("b-{n}");
            let focus = self.members.handle(key.clone(), cx);
            row = row.child(Button::new(&self.hits, key).label(format!("B {n}")).size(Size::Medium).acting(&focus, |_: &mut Two, _, _| {}, cx));
        }
        root(gpui::div().size_full()).child(self.members.order().on(row.id("two")))
    }
}

fn ringed(view: &Entity<Two>, cx: &mut VisualTestContext) -> Vec<usize> {
    cx.run_until_parked();
    let handles: Vec<_> = (0..2).filter_map(|n| view.read_with(cx, |v, _| v.members.get(&format!("b-{n}")))).collect();
    cx.update(|window, cx| (0..2).filter(|n| visible(&handles[*n], window, cx)).collect())
}

#[gpui::test]
fn a_click_on_a_button_shows_no_ring_and_tab_does(cx: &mut TestAppContext) {
    cx.update(|cx| {
        crate::init_globals(cx, "test");
        crate::focus_order::bind_keys(cx);
    });
    let (view, cx) = cx.add_window_view(|_, _| Two { hits: HitTargets::default(), members: Members::default() });
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    let at = view.read_with(cx, |v, _| v.hits.borrow().get("b-0").map(|h| h.bounds.center())).expect("b-0 paints");
    cx.simulate_click(at, Modifiers::none());
    let first = view.read_with(cx, |v, _| v.members.get("b-0")).expect("b-0's handle");
    assert!(cx.update(|window, _| first.is_focused(window)), "the click focuses it");
    assert_eq!(ringed(&view, cx), Vec::<usize>::new(), "a click shows no ring");
    cx.simulate_keystrokes("tab");
    assert_eq!(ringed(&view, cx), [1], "Tab rings the next button");
    cx.simulate_click(at, Modifiers::none());
    assert_eq!(ringed(&view, cx), Vec::<usize>::new(), "a click takes the ring away again");
}
