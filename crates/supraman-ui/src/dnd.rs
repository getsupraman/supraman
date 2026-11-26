//! Drag and drop (UI-KIT §4.7): a dragged [`Payload`] under its [`chip`], its source dimmed while it is dragged, Esc
//! cancelling it, [`Sortable`] (a reorder: its insertion line, edge auto-scroll) and [`DropZone`] (a typed target in
//! the `drop_over` look). A resize is `split`'s divider.

use std::any::TypeId;
use std::marker::PhantomData;
use std::rc::Rc;

use gpui::{
    AnyWeakEntity, App, AppContext as _, Axis, Context, Div, DragMoveEvent, Global, InteractiveElement, IntoElement, ParentElement, Pixels, Point, Render, SharedString,
    StatefulInteractiveElement, StyleRefinement, Styled, Subscription, Window, div, point, prelude::FluentBuilder, px,
};
use theme::ActiveTheme as _;

use crate::fonts;
use crate::scroll::ScrollbarHandle;
use crate::state::{Fills, State, resolve};
use crate::tokens;

/// What a drag carries; its title is its chip's words.
pub trait Payload: Clone + 'static {
    fn title(&self) -> SharedString;
}

/// How near an edge a drag starts scrolling its area.
pub const EDGE: Pixels = px(40.);
/// The most a drag scrolls per move or tick, at the very edge.
pub const EDGE_STEP: Pixels = px(24.);

/// How far a drag at `at` (from the viewport's start) scrolls: towards the edge it is within [`EDGE`] of, more the nearer it is.
pub fn edge_scroll(at: Pixels, viewport: Pixels) -> Pixels {
    let depth = |into: Pixels| EDGE_STEP * ((EDGE - into.max(Pixels::ZERO)) / EDGE).clamp(0., 1.);
    if at < EDGE {
        -depth(at)
    } else if at > viewport - EDGE {
        depth(viewport - at)
    } else {
        Pixels::ZERO
    }
}

/// Where an item's insertion line is drawn: its leading or trailing edge along the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Before,
    After,
}

/// Where the item at `from`, dropped on the one at `to`, lands: before it when it comes from after it; none onto itself.
pub fn lands(from: usize, to: usize) -> Option<Edge> {
    match from.cmp(&to) {
        std::cmp::Ordering::Greater => Some(Edge::Before),
        std::cmp::Ordering::Less => Some(Edge::After),
        std::cmp::Ordering::Equal => None,
    }
}

/// A dragged thing's chip under the pointer.
pub fn chip(title: SharedString, cx: &App) -> Div {
    let colors = cx.theme().colors();
    div()
        .font_family(fonts::UI)
        .text_size(tokens::text::BODY)
        .px(tokens::space::XL)
        .py(tokens::space::S)
        .max_w(px(280.))
        .overflow_hidden()
        .whitespace_nowrap()
        .rounded(tokens::radius::S)
        .border(tokens::stroke::HAIRLINE)
        .border_color(colors.border_focused)
        .bg(colors.tab_active_background)
        .text_color(colors.text)
        .opacity(0.9)
        .child(title)
}

struct Chip<P>(P);

impl<P: Payload> Render for Chip<P> {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        chip(self.0.title(), cx)
    }
}

struct Scroll {
    by: Point<Pixels>,
    handle: Rc<dyn ScrollbarHandle>,
    ticking: bool,
}

/// The one drag under way: its source, the chip that lives exactly as long as it, the item it is over and its scroll.
#[derive(Default)]
struct Drag {
    source: Option<(SharedString, TypeId, AnyWeakEntity)>,
    over: Option<(SharedString, SharedString, Option<Edge>)>,
    /// The drop zones painted `drop_over` since the pointer last moved.
    lit: Vec<SharedString>,
    scroll: Option<Scroll>,
    _esc: Option<Subscription>,
}

impl Global for Drag {}

/// Esc cancels a drag: nothing is dropped and nothing changes. Once, at start.
pub fn init(cx: &mut App) {
    let esc = cx.intercept_keystrokes(|e, window, cx| {
        if e.keystroke.key == "escape" && under_way(cx).is_some() {
            cancel(window, cx);
            cx.stop_propagation();
        }
    });
    cx.default_global::<Drag>()._esc = Some(esc);
}

/// Ends the drag under way with no drop.
pub fn cancel(window: &mut Window, cx: &mut App) {
    if cx.has_global::<Drag>() {
        let drag = cx.global_mut::<Drag>();
        drag.over = None;
        drag.scroll = None;
    }
    cx.stop_active_drag(window);
}

fn under_way(cx: &App) -> Option<(&SharedString, TypeId)> {
    let (key, kind, chip) = cx.try_global::<Drag>()?.source.as_ref()?;
    (cx.has_active_drag() && chip.upgrade().is_some()).then_some((key, *kind))
}

/// Whether the source under `key` is being dragged.
pub fn dragging(key: &str, cx: &App) -> bool {
    under_way(cx).is_some_and(|(k, _)| k == key)
}

/// Whether a drag of `P` is under way.
pub fn carrying<P: 'static>(cx: &App) -> bool {
    under_way(cx).is_some_and(|(_, kind)| kind == TypeId::of::<P>())
}

/// `el`, under `key`, dragged as `payload` with its chip, and dimmed while it is.
pub fn source<E, P>(el: E, key: impl Into<SharedString>, payload: P, cx: &App) -> E
where
    E: StatefulInteractiveElement + Styled + FluentBuilder,
    P: Payload,
{
    let key = key.into();
    let opacity = resolve(State { dragging: dragging(&key, cx), ..State::default() }, Fills::default()).opacity;
    el.when(opacity < 1., |el| el.opacity(opacity)).on_drag(payload, move |p, _, _, cx| {
        let chip = cx.new(|_| Chip(p.clone()));
        let drag = cx.default_global::<Drag>();
        drag.source = Some((key.clone(), TypeId::of::<P>(), chip.downgrade().into()));
        drag.over = None;
        drag.lit.clear();
        drag.scroll = None;
        chip
    })
}

/// The `drop_over` look: [`tokens::Roles`]' `drop_over` fill inside its `drop_border`.
fn over_look(style: StyleRefinement, cx: &App) -> StyleRefinement {
    let roles = tokens::roles(cx);
    let look = resolve(State { drop_over: true, ..State::default() }, Fills { drop_over: Some(roles.drop_over), ..Fills::default() });
    let style = match look.fill {
        Some(fill) => style.bg(fill),
        None => style,
    };
    style.border(tokens::stroke::HAIRLINE).border_color(roles.drop_border)
}

/// A typed drop target: each payload it [`take`](DropZone::take)s paints it `drop_over` while it accepts it hovering, and
/// calls its owner once when let go there.
pub struct DropZone<E> {
    el: E,
    key: Option<SharedString>,
}

/// Whether the drop zone under `key` painted `drop_over` for the drag under way since the pointer last moved.
pub fn lit(key: &str, cx: &App) -> bool {
    under_way(cx).is_some() && cx.try_global::<Drag>().is_some_and(|d| d.lit.iter().any(|k| k == key))
}

impl<E: InteractiveElement + Styled> DropZone<E> {
    pub fn new(key: impl Into<SharedString>, el: E) -> Self {
        Self { el, key: Some(key.into()) }
    }

    /// A target with no look of its own (the room around rows that are targets themselves).
    pub fn unmarked(el: E) -> Self {
        Self { el, key: None }
    }

    pub fn take<P: 'static>(self, accepts: impl Fn(&P, &Window, &App) -> bool + 'static, on_drop: impl Fn(&P, &mut Window, &mut App) + 'static) -> Self {
        let accepts = Rc::new(accepts);
        let el = match self.key.clone() {
            Some(key) => {
                let (shown, moved, lights) = (key.clone(), key, accepts.clone());
                self.el
                    .on_drag_move::<P>(move |_, _, cx| cx.default_global::<Drag>().lit.retain(|k| *k != moved))
                    .drag_over::<P>(move |s, p, window, cx| {
                        if !lights(p, window, cx) {
                            return s;
                        }
                        let drag = cx.default_global::<Drag>();
                        if !drag.lit.contains(&shown) {
                            drag.lit.push(shown.clone());
                        }
                        over_look(s, cx)
                    })
            }
            None => self.el,
        };
        let el = el.on_drop::<P>(move |p, window, cx| {
            if accepts(p, window, cx) {
                on_drop(p, window, cx);
            }
        });
        Self { el, key: self.key }
    }

    pub fn element(self) -> E {
        self.el
    }
}

/// A reorder over a list's or a strip's items: a payload over an item draws the insertion line on the [`Edge`] its
/// owner says it lands, dropped there it calls the owner; near its area's edge it scrolls, also while the pointer rests.
pub struct Sortable<P> {
    id: SharedString,
    axis: Axis,
    payload: PhantomData<fn() -> P>,
}

impl<P: Payload> Sortable<P> {
    pub fn new(id: impl Into<SharedString>, axis: Axis) -> Self {
        Self { id: id.into(), axis, payload: PhantomData }
    }

    /// The item and the edge a payload of the drag under way would land on, from the pointer.
    pub fn over(&self, cx: &App) -> Option<(SharedString, Option<Edge>)> {
        let (id, key, edge) = cx.try_global::<Drag>()?.over.clone()?;
        (id == self.id && carrying::<P>(cx)).then_some((key, edge))
    }

    /// `el`, the item `key`: `lands` its edge for a payload over it (none: no line and no drop), `on_drop` its drop.
    pub fn item<E>(&self, el: E, key: impl Into<SharedString>, lands: impl Fn(&P, &App) -> Option<Edge> + 'static, on_drop: impl Fn(&P, &mut Window, &mut App) + 'static, cx: &App) -> E
    where
        E: InteractiveElement + ParentElement + Styled + FluentBuilder,
    {
        let key = key.into();
        let line = self.over(cx).filter(|(k, _)| *k == key).and_then(|(_, edge)| edge).map(|edge| self.line(edge, cx));
        let (id, moved) = (self.id.clone(), key.clone());
        let lands = Rc::new(lands);
        let dropped = lands.clone();
        el.relative()
            .children(line)
            .on_drag_move::<P>(move |e: &DragMoveEvent<P>, window, cx| {
                let inside = e.bounds.contains(&e.event.position);
                let now = inside.then(|| (id.clone(), moved.clone(), lands(e.drag(cx), cx)));
                let drag = cx.default_global::<Drag>();
                let was = drag.over.as_ref().is_some_and(|(i, k, _)| *i == id && *k == moved);
                if (inside || was) && drag.over != now {
                    drag.over = now;
                    window.refresh();
                }
            })
            .on_drop::<P>(move |p, window, cx| {
                cx.default_global::<Drag>().over = None;
                if dropped(p, cx).is_some() {
                    on_drop(p, window, cx);
                }
            })
    }

    fn line(&self, edge: Edge, cx: &App) -> Div {
        let colour = tokens::roles(cx).drop_border;
        let line = div().absolute().bg(colour);
        match (self.axis, edge) {
            (Axis::Horizontal, Edge::Before) => line.top_0().bottom_0().left_0().w(tokens::stroke::FOCUS),
            (Axis::Horizontal, Edge::After) => line.top_0().bottom_0().right_0().w(tokens::stroke::FOCUS),
            (Axis::Vertical, Edge::Before) => line.left_0().right_0().top_0().h(tokens::stroke::FOCUS),
            (Axis::Vertical, Edge::After) => line.left_0().right_0().bottom_0().h(tokens::stroke::FOCUS),
        }
    }

    /// `el`, the area scrolled by `handle`: a drag of `P` near its edge scrolls it each move, and on while it rests there.
    pub fn scrolls<E: InteractiveElement>(&self, el: E, handle: impl ScrollbarHandle + Clone) -> E {
        let axis = self.axis;
        el.on_drag_move::<P>(move |e: &DragMoveEvent<P>, window, cx| {
            let (b, at) = (e.bounds, e.event.position);
            let (along, start, len, across) = match axis {
                Axis::Horizontal => (at.x, b.left(), b.size.width, at.y >= b.top() && at.y <= b.bottom()),
                Axis::Vertical => (at.y, b.top(), b.size.height, at.x >= b.left() && at.x <= b.right()),
            };
            let step = if across { edge_scroll(along - start, len) } else { Pixels::ZERO };
            let by = match axis {
                Axis::Horizontal => point(step, Pixels::ZERO),
                Axis::Vertical => point(Pixels::ZERO, step),
            };
            scroll_by(&handle, by);
            let drag = cx.default_global::<Drag>();
            let ticking = drag.scroll.as_ref().is_some_and(|s| s.ticking);
            drag.scroll = Some(Scroll { by, handle: Rc::new(handle.clone()), ticking: ticking || step != Pixels::ZERO });
            if step != Pixels::ZERO {
                window.refresh();
                if !ticking {
                    tick(window, cx);
                }
            }
        })
    }
}

/// Scrolls the drag's area by its last step each [`tokens::motion::EDGE_SCROLL`] until the drag ends or leaves the edge.
fn tick(window: &mut Window, cx: &mut App) {
    window
        .spawn(cx, async move |cx| {
            loop {
                cx.background_executor().timer(tokens::motion::EDGE_SCROLL).await;
                let go = cx.update(|window, cx| {
                    let live = under_way(cx).is_some();
                    let drag = cx.default_global::<Drag>();
                    let Some(s) = drag.scroll.as_mut().filter(|s| live && s.by != Point::default()) else {
                        if let Some(s) = drag.scroll.as_mut() {
                            s.ticking = false;
                        }
                        return false;
                    };
                    scroll_by(s.handle.as_ref(), s.by);
                    window.refresh();
                    true
                });
                if !matches!(go, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
}

fn scroll_by(handle: &dyn ScrollbarHandle, by: Point<Pixels>) {
    if by == Point::default() {
        return;
    }
    let (viewport, content, offset) = (handle.viewport_bounds().size, handle.content_size(), handle.offset());
    let max = point((content.width - viewport.width).max(Pixels::ZERO), (content.height - viewport.height).max(Pixels::ZERO));
    let to = point((-offset.x + by.x).clamp(Pixels::ZERO, max.x), (-offset.y + by.y).clamp(Pixels::ZERO, max.y));
    handle.set_offset(point(-to.x, -to.y));
}

#[cfg(test)]
mod tests;
