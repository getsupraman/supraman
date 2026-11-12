use std::cell::Cell;
use std::rc::Rc;

use gpui::{Context, Entity, Modifiers, Render, TestAppContext, VisualTestContext};

use super::*;
use crate::hit::HitTargets;
use crate::tooltip::LAST_SHOWN;

struct Holder {
    hits: HitTargets,
    opened: Rc<Cell<u32>>,
}

impl Render for Holder {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let runs = Opening { icon: Icon::Attach, hint: "Attach in a new tab".into(), runs: true };
        let dimmed = Opening { icon: Icon::Attach, hint: "shown now".into(), runs: false };
        let (a, b) = (self.opened.clone(), self.opened.clone());
        div()
            .flex()
            .gap_2()
            .p_2()
            .child(open(&self.hits, "s-open", &runs, Fill::Solid, move |_, _| a.set(a.get() + 1), cx))
            .child(open(&self.hits, "s-shown", &dimmed, Fill::Outline, move |_, _| b.set(b.get() + 10), cx))
    }
}

fn holder(cx: &mut TestAppContext) -> (Entity<Holder>, &mut VisualTestContext) {
    cx.update(|cx| crate::init_globals(cx, "test"));
    let (view, cx) = cx.add_window_view(|_, _| Holder { hits: HitTargets::default(), opened: Rc::default() });
    cx.run_until_parked();
    (view, cx)
}

fn tooltip_at(view: &Entity<Holder>, key: &str, cx: &mut VisualTestContext) -> Option<SharedString> {
    let at = view.read_with(cx, |h, _| h.hits.borrow()[key].bounds.center());
    LAST_SHOWN.with(|l| l.borrow_mut().take());
    cx.simulate_mouse_move(at, None, Modifiers::none());
    cx.executor().advance_clock(std::time::Duration::from_secs(2));
    cx.run_until_parked();
    LAST_SHOWN.with(|l| l.borrow().clone())
}

#[gpui::test]
fn open_runs_with_its_hint_as_tooltip_and_a_dimmed_one_says_why_and_does_nothing(cx: &mut TestAppContext) {
    let (view, cx) = holder(cx);
    assert_eq!(tooltip_at(&view, "s-open", cx).as_deref(), Some("Attach in a new tab"));
    let at = view.read_with(cx, |h, _| h.hits.borrow()["s-open"].bounds.center());
    cx.simulate_click(at, Modifiers::none());
    assert_eq!(view.read_with(cx, |h, _| h.opened.get()), 1, "its click runs the owner's open");

    assert_eq!(tooltip_at(&view, "s-shown", cx).as_deref(), Some("shown now"), "dimmed, with why");
    let at = view.read_with(cx, |h, _| h.hits.borrow()["s-shown"].bounds.center());
    cx.simulate_click(at, Modifiers::none());
    assert_eq!(view.read_with(cx, |h, _| h.opened.get()), 1, "a dimmed Open's click is a no-op");
}
