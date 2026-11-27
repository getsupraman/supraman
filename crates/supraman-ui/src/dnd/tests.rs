use std::time::Duration;

use gpui::{App, Axis, Context, IntoElement, Modifiers, MouseButton, Point, Render, ScrollHandle, SharedString, TestAppContext, VisualTestContext, Window, div, point, prelude::*, px};

use super::{DropZone, Edge, Payload, Sortable, carrying, dragging, lands, lit, source};

#[derive(Clone, Debug)]
struct Card(usize);

impl Payload for Card {
    fn title(&self) -> SharedString {
        format!("card {}", self.0).into()
    }
}

#[derive(Clone)]
struct Other;

impl Payload for Other {
    fn title(&self) -> SharedString {
        "other".into()
    }
}

const ITEM: f32 = 100.;
const COUNT: usize = 10;
const VIEWPORT: f32 = 300.;
const ZONE: Point<gpui::Pixels> = point(px(150.), px(150.));
const OTHER: Point<gpui::Pixels> = point(px(50.), px(320.));

#[derive(Default)]
struct Cards {
    moved: Vec<(usize, usize)>,
    zone_drops: Vec<usize>,
    scroll: ScrollHandle,
}

fn sortable() -> Sortable<Card> {
    Sortable::new("cards", Axis::Horizontal)
}

impl Render for Cards {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut row = div().id("row").flex().w(px(VIEWPORT)).h(px(40.)).overflow_x_scroll().track_scroll(&self.scroll);
        for ix in 0..COUNT {
            let key = format!("card-{ix}");
            let item = div().id(SharedString::from(key.clone())).flex_none().w(px(ITEM)).h_full();
            let dropped = cx.listener(move |this, c: &Card, _, cx| {
                this.moved.push((c.0, ix));
                cx.notify();
            });
            let item = sortable().item(item, key.clone(), move |c: &Card, _: &App| lands(c.0, ix), dropped, cx);
            row = row.child(source(item, key, Card(ix), cx));
        }
        let zone = DropZone::new("zone", div().id("zone").absolute().top(px(100.)).left_0().w(px(VIEWPORT)).h(px(100.)))
            .take::<Card>(|c, _, _| c.0 != 2, cx.listener(|this, c: &Card, _, _| this.zone_drops.push(c.0)))
            .element();
        let other = source(div().id("other").absolute().top(px(300.)).left_0().w(px(ITEM)).h(px(40.)), "other", Other, cx);
        div().size_full().relative().child(sortable().scrolls(row, self.scroll.clone())).child(zone).child(other)
    }
}

fn open(cx: &mut TestAppContext) -> (gpui::Entity<Cards>, &mut VisualTestContext) {
    cx.update(|cx| crate::init_globals(cx, "test"));
    let (view, cx) = cx.add_window_view(|_, _| Cards::default());
    cx.run_until_parked();
    (view, cx)
}

fn at(x: f32, y: f32) -> Point<gpui::Pixels> {
    point(px(x), px(y))
}

/// Presses at `from` and moves past the drag threshold, so the drag is under way.
fn press(cx: &mut VisualTestContext, from: Point<gpui::Pixels>) {
    cx.simulate_mouse_move(from, None, Modifiers::none());
    cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(from + at(10., 0.), Some(MouseButton::Left), Modifiers::none());
    cx.run_until_parked();
}

fn to(cx: &mut VisualTestContext, p: Point<gpui::Pixels>) {
    cx.simulate_mouse_move(p, Some(MouseButton::Left), Modifiers::none());
    cx.run_until_parked();
}

fn release(cx: &mut VisualTestContext, p: Point<gpui::Pixels>) {
    cx.simulate_mouse_up(p, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
}

#[gpui::test]
fn the_insertion_line_follows_the_pointer_and_the_drop_lands_there(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    press(cx, at(50., 20.));
    assert!(cx.update(|_, cx| dragging("card-0", cx) && carrying::<Card>(cx)), "the source is dragged");
    let over = |cx: &mut VisualTestContext| cx.update(|_, cx| sortable().over(cx));
    to(cx, at(250., 20.));
    assert_eq!(over(cx), Some(("card-2".into(), Some(Edge::After))), "past the third card: after it");
    to(cx, at(150., 20.));
    assert_eq!(over(cx), Some(("card-1".into(), Some(Edge::After))));
    to(cx, at(50., 20.));
    assert_eq!(over(cx), Some(("card-0".into(), None)), "over itself: no line");
    to(cx, at(250., 20.));
    release(cx, at(250., 20.));
    assert_eq!(view.read_with(cx, |v, _| v.moved.clone()), [(0, 2)]);
    assert!(!cx.update(|_, cx| dragging("card-0", cx)), "the drag is over");

    press(cx, at(250., 20.));
    to(cx, at(50., 20.));
    assert_eq!(over(cx), Some(("card-0".into(), Some(Edge::Before))), "from the right: before it");
    release(cx, at(50., 20.));
    assert_eq!(view.read_with(cx, |v, _| v.moved.clone()), [(0, 2), (2, 0)]);
}

#[gpui::test]
fn esc_cancels_a_drag_and_nothing_changes(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    press(cx, at(50., 20.));
    to(cx, at(250., 20.));
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(!cx.update(|_, cx| carrying::<Card>(cx)), "Esc ends the drag");
    assert_eq!(cx.update(|_, cx| sortable().over(cx)), None, "and its line");
    release(cx, at(250., 20.));
    assert!(view.read_with(cx, |v, _| v.moved.is_empty()), "the drop after Esc moves nothing");

    press(cx, at(50., 20.));
    to(cx, ZONE);
    cx.simulate_keystrokes("escape");
    release(cx, ZONE);
    assert!(view.read_with(cx, |v, _| v.zone_drops.is_empty()), "nor drops on a zone");
}

#[gpui::test]
fn a_drag_resting_at_an_edge_scrolls_on(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    let scrolled = |cx: &mut VisualTestContext| view.read_with(cx, |v, _| -v.scroll.offset().x);
    press(cx, at(50., 20.));
    to(cx, at(VIEWPORT - 5., 20.));
    let first = scrolled(cx);
    assert!(first > px(0.), "a move at the right edge scrolls by its step");
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    let rested = scrolled(cx);
    assert!(rested > first, "resting there it scrolls on: {first:?} then {rested:?}");
    to(cx, at(VIEWPORT / 2., 20.));
    let left = scrolled(cx);
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    assert_eq!(scrolled(cx), left, "away from the edge it stops");
    release(cx, at(VIEWPORT / 2., 20.));
}

#[gpui::test]
fn a_drop_zone_lights_only_for_a_payload_it_takes_and_calls_its_owner_once(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    let lit_now = |cx: &mut VisualTestContext| cx.update(|_, cx| lit("zone", cx));
    press(cx, OTHER);
    to(cx, ZONE);
    assert!(!lit_now(cx), "another payload type");
    release(cx, ZONE);

    press(cx, at(250., 20.));
    to(cx, ZONE);
    assert!(!lit_now(cx), "a card it refuses");
    release(cx, ZONE);
    assert!(view.read_with(cx, |v, _| v.zone_drops.is_empty()));

    press(cx, at(50., 20.));
    to(cx, ZONE);
    assert!(lit_now(cx), "a card it takes");
    to(cx, at(50., 450.));
    assert!(!lit_now(cx), "off the zone");
    to(cx, ZONE);
    release(cx, ZONE);
    assert_eq!(view.read_with(cx, |v, _| v.zone_drops.clone()), [0], "one drop, one call");
}

#[test]
fn zone_in_is_the_nearest_edge_band_a_quarter_deep_else_the_centre() {
    use super::{Zone, zone_in};
    let b = gpui::Bounds::new(at(100., 100.), gpui::size(px(400.), px(200.)));
    assert_eq!(zone_in(b, at(120., 200.)), Zone::Left);
    assert_eq!(zone_in(b, at(480., 200.)), Zone::Right);
    assert_eq!(zone_in(b, at(300., 110.)), Zone::Top);
    assert_eq!(zone_in(b, at(300., 290.)), Zone::Bottom);
    assert_eq!(zone_in(b, at(300., 200.)), Zone::Centre);
    assert_eq!(zone_in(b, at(199., 200.)), Zone::Left, "just inside the band");
    assert_eq!(zone_in(b, at(201., 200.)), Zone::Centre, "just past it");
    assert_eq!(zone_in(b, at(110., 140.)), Zone::Left, "a corner goes to the nearer edge by share");
}
