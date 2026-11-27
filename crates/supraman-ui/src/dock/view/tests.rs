use gpui::{Bounds, Context, IntoElement, Modifiers, MouseButton, Pixels, Point, Render, SharedString, TestAppContext, VisualTestContext, Window, div, point, prelude::*, px};

use super::{Dock, GHOST, Panel};
use crate::dnd::{Payload, source};
use crate::dock::layout::{Axis, Dropped, Layout, Zone};
use crate::hit::{HitTargets, hit_target};
use crate::icon::Icon;
use crate::tokens;

const W: f32 = 600.;
const H: f32 = 400.;
/// How far a slot's painted edge may lie from the model's rect: the dividers' hairlines.
const SLACK: f32 = 2.;

#[derive(Clone)]
struct Moving(u8);

impl Payload for Moving {
    fn title(&self) -> SharedString {
        format!("panel {}", self.0).into()
    }
}

struct Host {
    hits: HitTargets,
    layout: Layout<u8>,
    laid: Vec<Layout<u8>>,
    closed: Vec<u8>,
    centred: Vec<(u8, u8)>,
}

fn panel(p: u8, hits: &HitTargets) -> (u8, Panel<Moving>) {
    let element = div().size_full().child(hit_target(hits, format!("panel-{p}"))).into_any_element();
    (p, Panel { element, icon: Icon::Terminal, words: format!("Panel {p}").into(), payload: Moving(p) })
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panels = (1..=3).map(|p| panel(p, &self.hits)).collect();
        let dock = Dock::new(&self.hits, "dock", |m: &Moving, _| Some(m.0))
            .focused(Some(1))
            .on_layout(|h: &mut Host, laid, cx| {
                h.laid.push(laid.clone());
                h.layout = laid;
                cx.notify();
            })
            .on_close(|h: &mut Host, p, _| h.closed.push(p))
            .on_centre(|h: &mut Host, target, m: &Moving, _, _| h.centred.push((target, m.0)))
            .render(&self.layout, panels, cx);
        let outside = div().id("outside").absolute().top(px(H + 10.)).left_0().w(px(100.)).h(px(20.)).child(hit_target(&self.hits, "outside".into()));
        div()
            .size_full()
            .relative()
            .child(div().absolute().top_0().left_0().w(px(W)).h(px(H)).child(dock))
            .child(source(outside, "outside", Moving(3), cx))
    }
}

fn cols(a: Layout<u8>, b: Layout<u8>) -> Layout<u8> {
    Layout::Split { axis: Axis::Columns, ratio: 0.5, first: Box::new(a), second: Box::new(b) }
}

fn two() -> Layout<u8> {
    cols(Layout::Leaf(1), Layout::Leaf(2))
}

fn open(layout: Layout<u8>, cx: &mut TestAppContext) -> (gpui::Entity<Host>, &mut VisualTestContext) {
    cx.update(|cx| crate::init_globals(cx, "test"));
    let (host, cx) = cx.add_window_view(|_, _| Host { hits: HitTargets::default(), layout, laid: Vec::new(), closed: Vec::new(), centred: Vec::new() });
    cx.run_until_parked();
    (host, cx)
}

/// The window painted afresh, with only this frame's hits.
fn repaint(host: &gpui::Entity<Host>, cx: &mut VisualTestContext) {
    host.read_with(cx, |h, _| h.hits.borrow_mut().clear());
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn hit(host: &gpui::Entity<Host>, key: &str, cx: &mut VisualTestContext) -> Option<Bounds<Pixels>> {
    host.read_with(cx, |h, _| h.hits.get(key).map(|hit| hit.bounds))
}

fn press(cx: &mut VisualTestContext, from: Point<Pixels>) {
    cx.simulate_mouse_move(from, None, Modifiers::none());
    cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(from + point(px(10.), px(0.)), Some(MouseButton::Left), Modifiers::none());
    cx.run_until_parked();
}

fn to(cx: &mut VisualTestContext, at: Point<Pixels>) {
    cx.simulate_mouse_move(at, Some(MouseButton::Left), Modifiers::none());
    cx.run_until_parked();
}

fn release(cx: &mut VisualTestContext, at: Point<Pixels>) {
    cx.simulate_mouse_up(at, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
}

/// A point inside `zone`'s band of `b`, a tenth of its size in from that edge.
fn inside(b: Bounds<Pixels>, zone: Zone) -> Point<Pixels> {
    let (w, h, c) = (b.size.width, b.size.height, b.center());
    match zone {
        Zone::Left => point(b.left() + w * 0.1, c.y),
        Zone::Right => point(b.right() - w * 0.1, c.y),
        Zone::Top => point(c.x, b.top() + h * 0.1),
        Zone::Bottom => point(c.x, b.bottom() - h * 0.1),
        Zone::Centre => c,
    }
}

fn near(a: Bounds<Pixels>, b: Bounds<Pixels>) -> bool {
    let d = |x: Pixels, y: Pixels| (f32::from(x) - f32::from(y)).abs() <= SLACK;
    d(a.left(), b.left()) && d(a.top(), b.top()) && d(a.right(), b.right()) && d(a.bottom(), b.bottom())
}

fn expected(layout: &Layout<u8>, p: u8, target: u8, zone: Zone) -> Option<Dropped<u8>> {
    if zone == Zone::Centre {
        return Some(Dropped::Owner);
    }
    if layout.contains(p) { layout.moved(p, target, zone) } else { layout.drop(target, zone, p) }
}

#[gpui::test]
fn a_payload_over_each_zone_paints_the_dropped_layouts_ghost_and_lands_there(cx: &mut TestAppContext) {
    let (host, cx) = open(two(), cx);
    let mut ghosts = 0;
    for (from, p) in [("dock-title-1", 2u8), ("outside", 3u8)] {
        for (n, target) in [(0usize, 1u8), (1, 2)] {
            for zone in Zone::ALL {
                host.update(cx, |h, cx| {
                    h.layout = two();
                    h.laid.clear();
                    h.centred.clear();
                    cx.notify();
                });
                repaint(&host, cx);
                let start = hit(&host, from, cx).expect("the source painted").center();
                press(cx, start);
                repaint(&host, cx);
                let aim = hit(&host, &format!("dock-aim-{n}"), cx).expect("the slots are targets while a panel is dragged");
                let at = inside(aim, zone);
                to(cx, at);
                repaint(&host, cx);
                let ghost = hit(&host, &format!("dock-{GHOST}"), cx);
                let case = format!("panel {p} over panel {target}'s {zone:?}");
                match expected(&two(), p, target, zone) {
                    Some(Dropped::Layout(landed)) => {
                        let (_, rect) = landed.rects(W, H).into_iter().find(|(q, _)| *q == p).unwrap();
                        let want = Bounds::new(point(px(rect.x), px(rect.y)), gpui::size(px(rect.width), px(rect.height)));
                        let ghost = ghost.unwrap_or_else(|| panic!("{case}: a ghost"));
                        assert!(near(ghost, want), "{case}: the ghost at {ghost:?}, the drop's slot {want:?}");
                        release(cx, at);
                        assert_eq!(host.read_with(cx, |h, _| h.laid.clone()), [landed], "{case}: the owner gets that layout once");
                        ghosts += 1;
                    }
                    Some(Dropped::Owner) => {
                        assert_eq!(ghost, None, "{case}: the centre paints no ghost");
                        release(cx, at);
                        assert!(host.read_with(cx, |h, _| h.laid.is_empty()), "{case}");
                        assert_eq!(host.read_with(cx, |h, _| h.centred.clone()), [(target, p)], "{case}: the owner's own drop");
                    }
                    None => {
                        assert_eq!(ghost, None, "{case}: a refused zone paints the current layout");
                        release(cx, at);
                        assert!(host.read_with(cx, |h, _| h.laid.is_empty() && h.centred.is_empty()), "{case}: and calls nothing");
                    }
                }
            }
        }
    }
    assert!(ghosts >= 6, "the zones exercise moves and joins: {ghosts}");
}

#[gpui::test]
fn esc_over_a_zone_paints_the_current_layout_and_changes_nothing(cx: &mut TestAppContext) {
    let (host, cx) = open(two(), cx);
    let start = hit(&host, "dock-title-1", cx).unwrap().center();
    press(cx, start);
    repaint(&host, cx);
    let at = inside(hit(&host, "dock-aim-0", cx).unwrap(), Zone::Left);
    to(cx, at);
    repaint(&host, cx);
    assert!(hit(&host, "dock-ghost", cx).is_some(), "the move's ghost");
    cx.simulate_keystrokes("escape");
    repaint(&host, cx);
    assert_eq!(hit(&host, "dock-ghost", cx), None, "Esc takes the ghost away");
    assert!(near(hit(&host, "dock-slot-1", cx).unwrap(), Bounds::new(point(px(W / 2.), px(0.)), gpui::size(px(W / 2.), px(H)))), "panel 2 back in its slot");
    release(cx, at);
    assert!(host.read_with(cx, |h, _| h.laid.is_empty() && h.layout == two()), "nothing changed");
}

#[gpui::test]
fn a_dock_of_one_paints_its_element_alone(cx: &mut TestAppContext) {
    let (host, cx) = open(Layout::Leaf(1), cx);
    repaint(&host, cx);
    assert_eq!(hit(&host, "dock-title-0", cx), None, "no title bar");
    assert_eq!(hit(&host, "dock-slot-0", cx), None, "no slot around it");
    assert_eq!(hit(&host, "panel-1", cx), Some(Bounds::new(point(px(0.), px(0.)), gpui::size(px(W), px(H)))), "the whole dock");
}

#[gpui::test]
fn two_panels_paint_one_divider_between_them_and_no_border(cx: &mut TestAppContext) {
    let (host, cx) = open(two(), cx);
    repaint(&host, cx);
    let [s0, s1, t0, p1, p2] = ["dock-slot-0", "dock-slot-1", "dock-title-0", "panel-1", "panel-2"].map(|k| hit(&host, k, cx).unwrap_or_else(|| panic!("{k} painted")));
    let f = f32::from;
    assert_eq!(f(s0.left()), 0.);
    assert_eq!(f(s1.right()), W);
    assert_eq!(f(s1.left()) - f(s0.right()), f(tokens::stroke::HAIRLINE), "one hairline between the slots");
    assert_eq!((t0.top(), t0.size.height), (s0.top(), tokens::bar::HEADER), "the title bar on top, bar::HEADER high");
    for (panel, slot) in [(p1, s0), (p2, s1)] {
        assert_eq!((panel.left(), panel.right(), panel.bottom()), (slot.left(), slot.right(), slot.bottom()), "no border inside the slot");
        assert_eq!(panel.top(), slot.top() + tokens::bar::HEADER, "no rule under the title bar");
    }
    let dividers = host.read_with(cx, |h, _| h.hits.borrow().keys().filter(|k| k.ends_with("-divider")).count());
    assert_eq!(dividers, 1, "the one divider");
}

#[gpui::test]
fn a_divider_drag_and_a_close_reach_the_owner(cx: &mut TestAppContext) {
    let (host, cx) = open(two(), cx);
    repaint(&host, cx);
    let grip = hit(&host, "dock-split-divider", cx).expect("the split's divider").center();
    press(cx, grip);
    to(cx, point(px(W * 0.3), grip.y));
    release(cx, point(px(W * 0.3), grip.y));
    let ratio = host.read_with(cx, |h, _| match &h.layout {
        Layout::Split { ratio, .. } => *ratio,
        Layout::Leaf(_) => 0.,
    });
    assert!((ratio - 0.3).abs() < 0.01, "the new share: {ratio}");
    repaint(&host, cx);
    let close = hit(&host, "dock-close-1", cx).expect("panel 2's ×").center();
    cx.simulate_click(close, Modifiers::none());
    assert_eq!(host.read_with(cx, |h, _| h.closed.clone()), [2]);
}
