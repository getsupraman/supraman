use std::cell::Cell;
use std::rc::Rc;

use gpui::{Context, Entity, FocusHandle, Render, Subscription, TestAppContext, VisualTestContext, Window, div, prelude::*};

use super::*;

struct Owner {
    focus: FocusHandle,
    blink: Entity<Blink>,
    _follow: [Subscription; 4],
}

impl Render for Owner {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().track_focus(&self.focus)
    }
}

fn open(cx: &mut TestAppContext) -> (Entity<Owner>, &mut VisualTestContext) {
    let (view, cx) = cx.add_window_view(|window, cx| {
        let focus = cx.focus_handle();
        let (blink, follow) = Blink::follow(&focus, window, cx);
        Owner { focus, blink, _follow: follow }
    });
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    (view, cx)
}

fn focus(view: &Entity<Owner>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| view.read(cx).focus.focus(window, cx));
    cx.run_until_parked();
}

fn blink(view: &Entity<Owner>, cx: &mut VisualTestContext) -> (bool, bool) {
    cx.run_until_parked();
    view.read_with(cx, |v, cx| (v.blink.read(cx).running(), v.blink.read(cx).visible()))
}

/// How often the blink notifies over `time`.
fn notices(view: &Entity<Owner>, time: std::time::Duration, cx: &mut VisualTestContext) -> usize {
    let blink = view.read_with(cx, |v, _| v.blink.clone());
    let count = Rc::new(Cell::new(0usize));
    let seen = count.clone();
    let _observe = cx.update(|_, cx| cx.observe(&blink, move |_, _| seen.set(seen.get() + 1)));
    cx.run_until_parked();
    cx.executor().advance_clock(time);
    cx.run_until_parked();
    count.get()
}

#[gpui::test]
fn no_timer_runs_while_the_owner_does_not_hold_the_keys(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    assert_eq!(blink(&view, cx), (false, false), "unfocused: stopped and hidden");
    assert_eq!(notices(&view, CARET_BLINK * 6, cx), 0, "no timer while unfocused");
    focus(&view, cx);
    assert_eq!(blink(&view, cx), (true, true), "focused: lit");
    assert!(notices(&view, CARET_BLINK * 2, cx) >= 2, "it blinks while it holds the keys");
    cx.update(|window, cx| window.blur(cx));
    assert_eq!(blink(&view, cx), (false, false), "blurred: hidden");
    assert_eq!(notices(&view, CARET_BLINK * 6, cx), 0, "no timer once blurred");
}

#[gpui::test]
fn no_timer_runs_while_the_window_is_inactive(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    focus(&view, cx);
    assert_eq!(blink(&view, cx).0, true);
    cx.deactivate_window();
    assert_eq!(blink(&view, cx), (false, false), "an inactive window's caret is hidden");
    assert_eq!(notices(&view, CARET_BLINK * 6, cx), 0, "and no timer runs");
    cx.update(|window, _| window.activate_window());
    assert_eq!(blink(&view, cx), (true, true), "active again: lit");
}

#[gpui::test]
fn a_keystroke_keeps_it_lit_for_a_whole_interval(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    focus(&view, cx);
    let b = view.read_with(cx, |v, _| v.blink.clone());
    cx.executor().advance_clock(CARET_BLINK);
    assert_eq!(blink(&view, cx).1, false, "off after one interval");
    b.update(cx, |b, cx| b.pause(cx));
    assert_eq!(blink(&view, cx).1, true, "a key lights it");
    cx.executor().advance_clock(CARET_BLINK - std::time::Duration::from_millis(1));
    assert_eq!(blink(&view, cx).1, true, "still lit just short of an interval");
    cx.executor().advance_clock(std::time::Duration::from_millis(1));
    assert_eq!(blink(&view, cx).1, false, "then it blinks off");
}

#[gpui::test]
fn a_pause_does_not_start_a_stopped_blink(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    let b = view.read_with(cx, |v, _| v.blink.clone());
    b.update(cx, |b, cx| b.pause(cx));
    assert_eq!(blink(&view, cx), (false, false));
    assert_eq!(notices(&view, CARET_BLINK * 6, cx), 0);
}
