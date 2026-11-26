use gpui::{
    Context, InteractiveElement as _, Entity, IntoElement, Modifiers, MouseButton, ParentElement as _, Pixels, Point, Render, ScrollDelta, ScrollHandle, ScrollEvent, Styled as _, TestAppContext, TouchPhase,
    VisualTestContext, Window, div, point, px,
};

use super::{Scrollbars, Thumb, area};
use crate::hit::HitTargets;

const ROW: f32 = 20.;

#[test]
fn the_thumb_is_the_viewports_share_of_the_track_at_least_its_minimum() {
    let thumb = Thumb::new(px(0.), px(300.), px(300.), px(600.), px(0.)).expect("overflows");
    assert_eq!(thumb.length, px(150.));
    let long = Thumb::new(px(0.), px(200.), px(200.), px(20_000.), px(0.)).expect("overflows");
    assert_eq!(long.length, px(32.), "a long content keeps a thumb that can be grabbed");
    assert!(Thumb::new(px(0.), px(200.), px(200.), px(200.), px(0.)).is_none(), "nothing to scroll, no thumb");
}

#[test]
fn the_thumb_travels_the_track_as_the_content_scrolls() {
    let at = |offset: f32| Thumb::new(px(10.), px(300.), px(300.), px(600.), px(offset)).unwrap().top;
    assert_eq!(at(0.), px(10.));
    assert_eq!(at(-150.), px(85.), "half way");
    assert_eq!(at(-300.), px(160.), "the end: the track's end less the thumb");
    assert_eq!(at(-900.), px(160.), "past the end, clamped");
}

#[test]
fn a_drag_keeps_the_grab_and_a_track_press_pages() {
    let thumb = Thumb::new(px(0.), px(300.), px(300.), px(600.), px(-150.)).unwrap();
    assert_eq!(thumb.offset_at(thumb.top + px(40.), px(40.)), px(-150.), "the point grabbed stays under the pointer");
    assert_eq!(thumb.offset_at(thumb.top + px(70.), px(40.)), px(-210.), "30 px of track is 60 px of content");
    assert_eq!(thumb.paged(px(-150.), px(290.)), px(-300.), "below the thumb: a page on, clamped to the end");
    assert_eq!(thumb.paged(px(-150.), px(10.)), px(0.), "above it: a page back, clamped to the top");
}

struct Rows {
    hits: HitTargets,
    outer: ScrollHandle,
    inner: ScrollHandle,
    nested: bool,
    mode: Scrollbars,
    focus: gpui::FocusHandle,
}

fn rows(n: usize) -> Vec<gpui::Div> {
    (0..n).map(|i| div().h(px(ROW)).flex_none().child(format!("row {i}"))).collect()
}

impl Render for Rows {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let mut outer = area("outer").track(&self.outer).hit(&self.hits, "outer").mode(self.mode).w(px(300.)).h(px(200.)).flex().flex_col();
        if self.nested {
            outer = outer.child(area("inner").track(&self.inner).hit(&self.hits, "inner").flex_none().h(px(100.)).flex().flex_col().children(rows(10)));
        }
        let first = div().h(px(ROW)).flex_none().track_focus(&self.focus).child("focusable");
        div().size_full().child(outer.child(first).children(rows(49)))
    }
}

fn open(nested: bool, mode: Scrollbars, cx: &mut TestAppContext) -> (Entity<Rows>, &mut VisualTestContext) {
    cx.update(|cx| crate::init_globals(cx, "test"));
    let (view, cx) = cx.add_window_view(|_, cx| Rows { hits: HitTargets::default(), outer: ScrollHandle::new(), inner: ScrollHandle::new(), nested, mode, focus: cx.focus_handle() });
    cx.run_until_parked();
    (view, cx)
}

fn wheel(at: Point<Pixels>, dy: f32, cx: &mut VisualTestContext) {
    cx.simulate_event(ScrollEvent { position: at, delta: ScrollDelta::Pixels(point(px(0.), px(dy))), modifiers: Modifiers::none(), touch_phase: TouchPhase::Moved, source: Default::default() });
    cx.run_until_parked();
}

fn offset(view: &Entity<Rows>, inner: bool, cx: &mut VisualTestContext) -> Pixels {
    view.read_with(cx, |v, _| if inner { v.inner.offset().y } else { v.outer.offset().y })
}

fn shown(view: &Entity<Rows>, key: &str, cx: &mut VisualTestContext) -> bool {
    view.read_with(cx, |v, _| v.hits.borrow().contains_key(&format!("{key}-thumb-shown")))
}

fn thumb(view: &Entity<Rows>, key: &str, cx: &mut VisualTestContext) -> gpui::Bounds<Pixels> {
    view.read_with(cx, |v, _| v.hits.borrow().get(&format!("{key}-thumb")).copied()).expect("the thumb is recorded").bounds
}

#[gpui::test]
fn the_bar_shows_while_scrolling_and_hides_at_rest(cx: &mut TestAppContext) {
    let (view, cx) = open(false, Scrollbars::Auto, cx);
    assert!(!shown(&view, "outer", cx), "at rest, no bar");
    wheel(point(px(150.), px(100.)), -60., cx);
    assert_eq!(offset(&view, false, cx), px(-60.));
    assert!(shown(&view, "outer", cx), "scrolled, the bar shows");
    cx.simulate_mouse_move(point(px(600.), px(500.)), None, Modifiers::none());
    cx.executor().advance_clock(super::IDLE + super::FADE * 4);
    for _ in 0..12 {
        cx.executor().advance_clock(super::FADE);
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
    }
    assert!(!shown(&view, "outer", cx), "idle, it fades");
}

#[gpui::test]
fn always_shows_the_bar_at_rest(cx: &mut TestAppContext) {
    let (view, cx) = open(false, Scrollbars::Always, cx);
    assert!(shown(&view, "outer", cx));
}

#[gpui::test]
fn the_pointer_over_the_area_shows_the_bar_and_over_the_thumb_widens_it(cx: &mut TestAppContext) {
    let (view, cx) = open(false, Scrollbars::Auto, cx);
    cx.simulate_mouse_move(point(px(100.), px(100.)), None, Modifiers::none());
    cx.run_until_parked();
    assert!(shown(&view, "outer", cx), "hovered, the bar shows");
    let rest = view.read_with(cx, |v, _| v.hits.borrow()["outer-thumb-shown"].bounds.size.width);
    let over = thumb(&view, "outer", cx).center();
    cx.simulate_mouse_move(over, None, Modifiers::none());
    cx.run_until_parked();
    let wide = view.read_with(cx, |v, _| v.hits.borrow()["outer-thumb-shown"].bounds.size.width);
    assert_eq!((rest, wide), (px(6.), px(8.)));
}

#[gpui::test]
fn dragging_the_thumb_moves_the_content_and_keeps_the_grab(cx: &mut TestAppContext) {
    let (view, cx) = open(false, Scrollbars::Always, cx);
    let start = thumb(&view, "outer", cx);
    let grab = start.top() + px(5.);
    let at = point(start.center().x, grab);
    cx.simulate_mouse_down(at, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(at + point(px(0.), px(39.2)), MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(at + point(px(0.), px(39.2)), MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    // 1000 px of rows in 200: the track is 196, the thumb 39.2, so 39.2 px of travel is 200 px of content.
    let moved = offset(&view, false, cx);
    assert!((moved - px(-200.)).abs() < px(0.5), "moved {moved:?}");
    let after = thumb(&view, "outer", cx);
    assert!((after.top() - (start.top() + px(39.2))).abs() < px(0.5), "the grabbed point stayed under the pointer: {after:?}");
}

#[gpui::test]
fn a_press_on_the_track_pages_toward_it(cx: &mut TestAppContext) {
    let (view, cx) = open(false, Scrollbars::Always, cx);
    let t = thumb(&view, "outer", cx);
    cx.simulate_mouse_down(point(t.center().x, px(190.)), MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(point(t.center().x, px(190.)), MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    assert_eq!(offset(&view, false, cx), px(-200.), "one viewport down");
}

#[gpui::test]
fn the_inner_area_takes_the_wheel_until_its_edge(cx: &mut TestAppContext) {
    let (view, cx) = open(true, Scrollbars::Auto, cx);
    let over_inner = point(px(100.), px(50.));
    wheel(over_inner, -60., cx);
    assert_eq!((offset(&view, true, cx), offset(&view, false, cx)), (px(-60.), px(0.)), "the inner one scrolls alone");
    wheel(over_inner, -60., cx);
    assert_eq!((offset(&view, true, cx), offset(&view, false, cx)), (px(-100.), px(0.)), "to its end, the outer still");
    wheel(over_inner, -60., cx);
    assert_eq!((offset(&view, true, cx), offset(&view, false, cx)), (px(-100.), px(-60.)), "at its edge, the outer takes the wheel");
    wheel(point(px(100.), px(150.)), 30., cx);
    assert_eq!(offset(&view, false, cx), px(-30.), "outside the inner one, the outer scrolls");
}

#[gpui::test]
fn page_keys_scroll_the_area_holding_the_focus(cx: &mut TestAppContext) {
    let (view, cx) = open(false, Scrollbars::Auto, cx);
    let focus = view.read_with(cx, |v, _| v.focus.clone());
    cx.update(|window, cx| window.focus(&focus, cx));
    cx.simulate_keystrokes("pagedown");
    assert_eq!(offset(&view, false, cx), px(-200.), "a viewport down");
    assert!(shown(&view, "outer", cx), "a key's scroll shows the bar");
    cx.simulate_keystrokes("end");
    assert_eq!(offset(&view, false, cx), px(-800.));
    cx.simulate_keystrokes("pageup home");
    assert_eq!(offset(&view, false, cx), px(0.));
}

struct Plain {
    handle: ScrollHandle,
}

impl Render for Plain {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(area("plain").track(&self.handle).w(px(300.)).h(px(200.)).flex().flex_col().children(rows(50)))
    }
}

#[gpui::test]
fn a_click_gives_an_area_with_nothing_focusable_in_it_the_keys_and_its_page_keys_work(cx: &mut TestAppContext) {
    cx.update(|cx| crate::init_globals(cx, "test"));
    let (view, cx) = cx.add_window_view(|_, _| Plain { handle: ScrollHandle::new() });
    cx.run_until_parked();
    assert!(cx.update(|window, cx| window.focused(cx).is_none()));
    cx.simulate_click(point(px(100.), px(50.)), Modifiers::none());
    assert!(cx.update(|window, cx| window.focused(cx).is_some()), "the click gives the area the keys");
    cx.simulate_keystrokes("pagedown");
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |v, _| v.handle.offset().y), px(-200.), "Page Down scrolls it a viewport");
}
