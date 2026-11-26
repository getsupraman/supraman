use gpui::{Context, Entity, FocusHandle, Modifiers, Render, TestAppContext, VisualTestContext, Window, div};

use super::*;

struct Form {
    hits: HitTargets,
    on: bool,
    toggles: usize,
    focus: FocusHandle,
    disabled: bool,
}

impl Render for Form {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let check = Checkbox::new(&self.hits, "check", self.on)
            .label("Check out after creating")
            .focus(&self.focus)
            .disabled(self.disabled.then_some("Not now"))
            .on_toggle(cx.listener(|this, on: &bool, _, cx| {
                this.on = *on;
                this.toggles += 1;
                cx.notify();
            }));
        crate::focus::root(div().size_full().p(tokens::space::XL)).child(check)
    }
}

fn form(cx: &mut TestAppContext) -> (Entity<Form>, &mut VisualTestContext) {
    cx.update(|cx| {
        crate::init_globals(cx, "test");
        crate::focus_order::bind_keys(cx);
    });
    let (view, cx) = cx.add_window_view(|_, cx| Form { hits: HitTargets::default(), on: false, toggles: 0, focus: cx.focus_handle(), disabled: false });
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    (view, cx)
}

fn click(view: &Entity<Form>, key: &str, cx: &mut VisualTestContext) {
    let at = view.read_with(cx, |v, _| v.hits.borrow().get(key).map(|h| h.bounds.center())).unwrap_or_else(|| panic!("{key} paints"));
    cx.simulate_click(at, Modifiers::none());
    cx.run_until_parked();
}

#[gpui::test]
fn a_click_on_its_label_or_square_toggles_it(cx: &mut TestAppContext) {
    let (view, cx) = form(cx);
    let (whole, square, label) = view.read_with(cx, |v, _| {
        let hits = v.hits.borrow();
        (hits["check"].bounds, hits["check-square"].bounds, hits["check-label"].bounds)
    });
    assert!(whole.contains(&square.center()) && whole.contains(&label.center()), "one hit target holds both: {whole:?} {square:?} {label:?}");
    click(&view, "check-label", cx);
    assert!(view.read_with(cx, |v, _| v.on), "the label's click ticks it");
    click(&view, "check-square", cx);
    assert!(!view.read_with(cx, |v, _| v.on), "the square's click unticks it");
    view.update(cx, |v, cx| {
        v.disabled = true;
        cx.notify();
    });
    cx.run_until_parked();
    click(&view, "check-label", cx);
    assert_eq!(view.read_with(cx, |v, _| (v.on, v.toggles)), (false, 2), "a disabled one does not toggle");
}

#[gpui::test]
fn space_and_enter_toggle_it_and_it_rings_for_keyboard_focus_only(cx: &mut TestAppContext) {
    let (view, cx) = form(cx);
    let focus = view.read_with(cx, |v, _| v.focus.clone());
    cx.update(|window, cx| window.focus(&focus, cx));
    cx.run_until_parked();
    assert!(!cx.update(|window, cx| crate::focus::visible(&focus, window, cx)), "no ring before a key");
    cx.simulate_keystrokes("space");
    cx.run_until_parked();
    assert!(view.read_with(cx, |v, _| v.on), "Space ticks it");
    assert!(cx.update(|window, cx| crate::focus::visible(&focus, window, cx)), "the ring once a key came");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(!view.read_with(cx, |v, _| v.on), "Enter unticks it");
    click(&view, "check-label", cx);
    assert!(!cx.update(|window, cx| crate::focus::visible(&focus, window, cx)), "no ring after a click");
}
