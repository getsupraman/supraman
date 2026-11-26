use std::ops::Range;

use gpui::{
    Bounds, Context, Entity, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, TestAppContext, VisualTestContext, Window, div, point,
    px,
};

use super::*;
use crate::hit::{HitTargets, hit_target};
use crate::text::input::{InputEvent, TextInput};

const TEXT: &str = "alpha beta gamma\nsecond line here";
const WRAPPING: &str = "fix(dolomiti): skipass error codes are camelCase on the wire and in the widget, so errors show translated";

struct Holder {
    input: Entity<TextInput>,
    hits: HitTargets,
    pressed: usize,
    _pressed: gpui::Subscription,
}

impl Render for Holder {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let view = div().relative().child(TextView::new("tv", TEXT)).child(hit_target(&self.hits, "tv".into()));
        div().w(px(300.)).flex().flex_col().gap(px(10.)).text_size(px(14.)).child(self.input.clone()).child(view)
    }
}

fn holder<'a>(text: &str, cx: &'a mut TestAppContext) -> (Entity<Holder>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        crate::init_globals(cx, "test");
        crate::text::input::bind_keys(cx);
    });
    let text = text.to_string();
    let (view, cx) = cx.add_window_view(move |window, cx| {
        let input = cx.new(|cx| {
            let mut i = TextInput::new("", None, window, cx).plain().read_only().multi_line();
            i.set_text(&text, cx);
            i
        });
        let pressed = cx.subscribe(&input, |this: &mut Holder, _, e: &InputEvent, _| {
            if *e == InputEvent::Pressed {
                this.pressed += 1;
            }
        });
        Holder { input, hits: HitTargets::default(), pressed: 0, _pressed: pressed }
    });
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    (view, cx)
}

fn input(view: &Entity<Holder>, cx: &mut VisualTestContext) -> Entity<TextInput> {
    view.read_with(cx, |h, _| h.input.clone())
}

fn redraw(view: &Entity<Holder>, cx: &mut VisualTestContext) {
    view.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
}

fn spot(input: &Entity<TextInput>, offset: usize, cx: &mut VisualTestContext) -> Point<Pixels> {
    input.read_with(cx, |i, _| i.painted_at(offset)).expect("painted")
}

fn down(at: Point<Pixels>, clicks: usize, shift: bool, cx: &mut VisualTestContext) {
    let modifiers = Modifiers { shift, ..Modifiers::none() };
    cx.simulate_event(MouseDownEvent { position: at, button: MouseButton::Left, modifiers, click_count: clicks, first_mouse: false });
    cx.simulate_event(MouseUpEvent { position: at, button: MouseButton::Left, modifiers, click_count: clicks });
}

fn drag(input: &Entity<TextInput>, from: usize, to: usize, cx: &mut VisualTestContext) {
    let (a, b) = (spot(input, from, cx), spot(input, to, cx));
    let button = MouseButton::Left;
    cx.simulate_event(MouseDownEvent { position: a, button, modifiers: Modifiers::none(), click_count: 1, first_mouse: false });
    cx.simulate_event(MouseMoveEvent { position: b, pressed_button: Some(button), modifiers: Modifiers::none() });
    cx.simulate_event(MouseUpEvent { position: b, button, modifiers: Modifiers::none(), click_count: 1 });
}

fn selection(input: &Entity<TextInput>, cx: &mut VisualTestContext) -> Range<usize> {
    input.read_with(cx, |i, _| i.line.selection())
}

fn copied(cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|_, cx| crate::clipboard::text(cx))
}

#[gpui::test]
fn a_drag_selects_and_ctrl_c_copies_exactly_the_selection(cx: &mut TestAppContext) {
    let (view, cx) = holder(TEXT, cx);
    let input = input(&view, cx);
    drag(&input, 6, 10, cx);
    assert_eq!(selection(&input, cx), 6..10);
    cx.simulate_keystrokes("ctrl-c");
    assert_eq!(copied(cx).as_deref(), Some("beta"), "the selection, not the whole text");
    drag(&input, 13, 23, cx);
    cx.simulate_keystrokes("ctrl-insert");
    assert_eq!(copied(cx).as_deref(), Some("mma\nsecond"), "across the line break");
    cx.simulate_keystrokes("ctrl-a ctrl-c");
    assert_eq!(copied(cx).as_deref(), Some(TEXT));
}

#[gpui::test]
fn a_double_click_selects_the_word_and_a_triple_click_the_line(cx: &mut TestAppContext) {
    let (view, cx) = holder(TEXT, cx);
    let input = input(&view, cx);
    down(spot(&input, 8, cx), 2, false, cx);
    assert_eq!(selection(&input, cx), 6..10);
    down(spot(&input, 22, cx), 3, false, cx);
    assert_eq!(selection(&input, cx), 17..TEXT.len(), "the second line, without the first");
    down(spot(&input, 2, cx), 3, false, cx);
    assert_eq!(selection(&input, cx), 0..16, "the first line, without its newline");
}

#[gpui::test]
fn shift_arrows_and_shift_click_extend_once_focused(cx: &mut TestAppContext) {
    let (view, cx) = holder(TEXT, cx);
    let input = input(&view, cx);
    down(spot(&input, 6, cx), 1, false, cx);
    assert!(cx.update(|window, cx| input.read(cx).is_focused(window)), "a click gives it the keys");
    cx.simulate_keystrokes("shift-right shift-right shift-right shift-right");
    assert_eq!(selection(&input, cx), 6..10);
    cx.simulate_keystrokes("ctrl-shift-right");
    assert_eq!(selection(&input, cx), 6..16);
    cx.simulate_keystrokes("shift-down");
    assert_eq!(selection(&input, cx).start, 6);
    assert!(selection(&input, cx).end > 17, "Shift+Down extends onto the next line");
    down(spot(&input, 2, cx), 1, false, cx);
    down(spot(&input, 13, cx), 1, true, cx);
    assert_eq!(selection(&input, cx), 2..13, "Shift+click extends from where the click left it");
}

#[gpui::test]
fn read_only_text_takes_the_keys_but_no_caret_and_no_edit_and_says_it_was_pressed(cx: &mut TestAppContext) {
    let (view, cx) = holder(TEXT, cx);
    let input = input(&view, cx);
    down(spot(&input, 3, cx), 1, false, cx);
    redraw(&view, cx);
    assert!(!input.read_with(cx, |i, _| i.caret_painted.get()), "no caret");
    cx.simulate_input("x");
    cx.simulate_keystrokes("backspace ctrl-x ctrl-v enter");
    assert_eq!(input.read_with(cx, |i, _| i.text().to_string()), TEXT, "nothing edits it");
    assert_eq!(view.read_with(cx, |h, _| h.pressed), 1);
}

fn laid_out(input: &Entity<TextInput>, cx: &mut VisualTestContext) -> (Bounds<Pixels>, Vec<(Range<usize>, Pixels)>) {
    input.read_with(cx, |i, _| i.painted_rows()).expect("painted")
}

#[gpui::test]
fn switching_to_editable_keeps_the_box_and_the_layout_and_only_lights_the_caret(cx: &mut TestAppContext) {
    let (view, cx) = holder(WRAPPING, cx);
    let input = input(&view, cx);
    redraw(&view, cx);
    let before = laid_out(&input, cx);
    assert!(before.1.len() > 1, "the text wraps");
    down(spot(&input, 20, cx), 1, false, cx);
    cx.update(|window, cx| input.update(cx, |i, cx| i.set_read_only(false, window, cx)));
    redraw(&view, cx);
    assert_eq!(laid_out(&input, cx), before, "the same box and rows");
    assert_eq!(selection(&input, cx), 20..20, "the caret where the click was");
    assert!(input.read_with(cx, |i, _| i.caret_painted.get()), "the caret lit");
    cx.simulate_input("X");
    assert_eq!(input.read_with(cx, |i, _| i.text().to_string()), format!("{}X{}", &WRAPPING[..20], &WRAPPING[20..]), "typed at the click");
}

#[gpui::test]
fn the_selection_paints_in_the_selection_role_and_in_the_inactive_one_once_the_keys_go(cx: &mut TestAppContext) {
    let (view, cx) = holder(TEXT, cx);
    let input = input(&view, cx);
    drag(&input, 0, 5, cx);
    redraw(&view, cx);
    let fills = |cx: &mut VisualTestContext| {
        let at = spot(&input, 2, cx);
        cx.update(|window, _| {
            let at = at.scale(window.scale_factor());
            window.painted_quads().into_iter().filter(|q| q.bounds.contains(&at)).map(|q| q.background).collect::<Vec<_>>()
        })
    };
    let roles = cx.update(|_, cx| crate::tokens::roles(cx));
    assert!(fills(cx).contains(&roles.selection.into()), "active");
    cx.update(|window, cx| window.blur(cx));
    redraw(&view, cx);
    assert!(fills(cx).contains(&roles.selection_inactive.into()), "inactive");
}

#[gpui::test]
fn a_text_view_is_selected_and_copied_by_the_mouse(cx: &mut TestAppContext) {
    let (view, cx) = holder(TEXT, cx);
    let bounds = view.read_with(cx, |h, _| h.hits.borrow().get("tv").map(|h| h.bounds)).expect("the text view paints");
    down(bounds.origin + point(px(2.), px(4.)), 2, false, cx);
    cx.simulate_keystrokes("ctrl-c");
    assert_eq!(copied(cx).as_deref(), Some("alpha"));
}
