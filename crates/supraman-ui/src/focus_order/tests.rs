use gpui::{Context, Entity, FocusHandle, Focusable, KeyDownEvent, TestAppContext, VisualTestContext, Window, div, prelude::*};

use super::*;
use crate::text_input::TextInput;

struct Form {
    fields: [Entity<TextInput>; 2],
    button: FocusHandle,
    pressed: usize,
    outside: FocusHandle,
    outside_keys: Vec<String>,
}

impl Form {
    fn order(&self, cx: &App) -> FocusOrder {
        FocusOrder::new(self.fields.iter().map(|f| f.focus_handle(cx)).chain([self.button.clone()]))
    }
}

impl Render for Form {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let hits = crate::hit::HitTargets::default();
        let button = crate::button::focusable(crate::button::bordered(&hits, "ok", "OK", cx), &self.button, cx.listener(|this, _: &Press, _, _| this.pressed += 1), cx);
        let form = self.order(cx).on(div().id("form").children(self.fields.iter().cloned()).child(button));
        let outside = div().id("outside").track_focus(&self.outside).on_key_down(cx.listener(|this, e: &KeyDownEvent, _, _| this.outside_keys.push(e.keystroke.key.clone())));
        div().size_full().child(form).child(outside)
    }
}

fn form(cx: &mut TestAppContext) -> (Entity<Form>, &mut VisualTestContext) {
    cx.update(|cx| {
        crate::init_globals(cx, "test");
        crate::text_input::bind_keys(cx);
    });
    let (view, cx) = cx.add_window_view(|window, cx| Form {
        fields: [cx.new(|cx| TextInput::new("one", None, window, cx)), cx.new(|cx| TextInput::new("two", None, window, cx).multi_line())],
        button: cx.focus_handle(),
        pressed: 0,
        outside: cx.focus_handle(),
        outside_keys: Vec::new(),
    });
    cx.run_until_parked();
    (view, cx)
}

fn focused(view: &Entity<Form>, cx: &mut VisualTestContext) -> Option<usize> {
    cx.update(|window, cx| view.read(cx).order(cx).focused(window, cx))
}

#[gpui::test]
fn tab_and_shift_tab_walk_the_form_and_wrap_and_no_field_types_a_tab(cx: &mut TestAppContext) {
    let (view, cx) = form(cx);
    let first = view.read_with(cx, |f, _| f.fields[0].clone());
    cx.update(|window, cx| first.update(cx, |i, cx| i.focus(window, cx)));
    let mut walked = vec![focused(&view, cx)];
    for _ in 0..3 {
        cx.simulate_keystrokes("tab");
        walked.push(focused(&view, cx));
    }
    assert_eq!(walked, [Some(0), Some(1), Some(2), Some(0)], "Tab: each field, the button, then back to the first");
    cx.simulate_keystrokes("shift-tab");
    assert_eq!(focused(&view, cx), Some(2), "Shift+Tab from the first wraps to the last");
    cx.simulate_keystrokes("space");
    cx.simulate_keystrokes("enter");
    assert_eq!(view.read_with(cx, |f, _| f.pressed), 2, "Space and Enter press the focused button");
    cx.simulate_keystrokes("shift-tab");
    assert_eq!(focused(&view, cx), Some(1));
    let texts = view.read_with(cx, |f, cx| f.fields.iter().map(|i| i.read(cx).text().to_string()).collect::<Vec<_>>());
    assert_eq!(texts, ["", ""], "no field typed a tab");
}

#[gpui::test]
fn outside_the_form_tab_reaches_the_view_holding_the_focus(cx: &mut TestAppContext) {
    let (view, cx) = form(cx);
    let outside = view.read_with(cx, |f, _| f.outside.clone());
    cx.update(|window, cx| outside.focus(window, cx));
    cx.simulate_keystrokes("tab shift-tab");
    assert_eq!(focused(&view, cx), None, "the form took no focus");
    assert_eq!(view.read_with(cx, |f, _| f.outside_keys.clone()), ["tab", "tab"]);
}
