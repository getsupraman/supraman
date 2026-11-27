//! Resizable splits, height bars and pane stacks; the owner keeps and persists the width or heights.

use std::{cell::RefCell, rc::Rc};

use gpui::{AnyElement, Bounds, ClickEvent, Context, CursorStyle, Div, DragMoveEvent, MouseButton, Pixels, Point, SharedString, Size, Stateful, Window, canvas, div, prelude::*, px};
use theme::ActiveTheme as _;

use crate::hit::{HitTargets, hit_target};
use crate::tokens;

/// A divider's grab strip, across it, and where it starts before the divider's line.
const GRIP: Pixels = px(7.);
const GRIP_BEFORE: Pixels = px(-3.);

/// Which way a [`SplitPane`] lays its two sides: side by side (`Columns`) or one over the other (`Rows`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Axis {
    #[default]
    Columns,
    Rows,
}

impl Axis {
    fn along(self, point: Point<Pixels>) -> Pixels {
        match self {
            Axis::Columns => point.x,
            Axis::Rows => point.y,
        }
    }

    fn length(self, size: Size<Pixels>) -> Pixels {
        match self {
            Axis::Columns => size.width,
            Axis::Rows => size.height,
        }
    }
}

#[derive(Default)]
pub struct SplitPane {
    /// The divider's id under `hits`' scope; it also tells two splits' drags apart.
    pub id: SharedString,
    pub min_first: Pixels,
    pub min_second: Pixels,
    /// What a double-click on the divider restores in px; [`SplitPane::render_fraction`] takes its own.
    pub default: Pixels,
    pub axis: Axis,
    /// Scopes `id`; the divider's grab strip paints under `<id>-divider`.
    pub hits: HitTargets,
}

/// The drag of a divider: the divider's element id and which divider of its owner it is.
pub(crate) struct DraggedDivider(pub SharedString, pub usize);

impl Render for DraggedDivider {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

impl SplitPane {
    /// `on_resize` gets the first panel's width (height for `Rows`) while dragged and on a double-click; `on_resize_end` follows.
    pub fn render<V: 'static>(
        &self,
        first: impl IntoElement,
        second: impl IntoElement,
        on_resize: impl Fn(&mut V, Pixels, &mut Context<V>) + Clone + 'static,
        on_resize_end: impl Fn(&mut V, &mut Context<V>) + Clone + 'static,
        cx: &mut Context<V>,
    ) -> Div {
        self.divided(first.into_any_element(), second.into_any_element(), self.default, |at, _| at, on_resize, on_resize_end, cx)
    }

    /// [`SplitPane::render`] for an owner that keeps the second panel's width, `default` included.
    pub fn render_second<V: 'static>(
        &self,
        first: impl IntoElement,
        second: impl IntoElement,
        on_resize: impl Fn(&mut V, Pixels, &mut Context<V>) + Clone + 'static,
        on_resize_end: impl Fn(&mut V, &mut Context<V>) + Clone + 'static,
        cx: &mut Context<V>,
    ) -> Div {
        let second_of = |at: Pixels, total: Pixels| total - at - tokens::stroke::HAIRLINE;
        self.divided(first.into_any_element(), second.into_any_element(), self.default, second_of, on_resize, on_resize_end, cx)
    }

    /// The split sized by the first side's share, `fraction` (0..1), which the owner keeps; the mins hold both
    /// sides as it resizes. `on_resize` gets the share while dragged and `default` on a double-click.
    #[allow(clippy::too_many_arguments)]
    pub fn render_fraction<V: 'static>(
        &self,
        first: impl IntoElement,
        second: impl IntoElement,
        fraction: f32,
        default: f32,
        on_resize: impl Fn(&mut V, f32, &mut Context<V>) + Clone + 'static,
        on_resize_end: impl Fn(&mut V, &mut Context<V>) + Clone + 'static,
        cx: &mut Context<V>,
    ) -> Div {
        let fraction = fraction.clamp(0., 1.);
        let (first, second) = (self.share(first, fraction, self.min_first), self.share(second, 1. - fraction, self.min_second));
        let share_of = |at: Pixels, total: Pixels| {
            let room = total - tokens::stroke::HAIRLINE;
            if room > Pixels::ZERO { (at / room).clamp(0., 1.) } else { 0. }
        };
        self.divided(first, second, default.clamp(0., 1.), share_of, on_resize, on_resize_end, cx)
    }

    /// One side of a fraction split: `grow` of the room, never under `min`.
    fn share(&self, side: impl IntoElement, grow: f32, min: Pixels) -> AnyElement {
        let mut sized = div().relative().flex().flex_col().flex_shrink(1.).flex_basis(Pixels::ZERO).overflow_hidden();
        sized.style().flex_grow = Some(grow);
        let sized = match self.axis {
            Axis::Columns => sized.min_w(min).min_h_0(),
            Axis::Rows => sized.min_h(min).min_w_0(),
        };
        sized.child(side).into_any_element()
    }

    /// The two sides and the divider between them; `value` turns the divider's offset and the split's length into what the owner keeps.
    #[allow(clippy::too_many_arguments)]
    fn divided<V: 'static, T: Copy + 'static>(
        &self,
        first: AnyElement,
        second: AnyElement,
        reset_to: T,
        value: impl Fn(Pixels, Pixels) -> T + 'static,
        on_resize: impl Fn(&mut V, T, &mut Context<V>) + Clone + 'static,
        on_resize_end: impl Fn(&mut V, &mut Context<V>) + Clone + 'static,
        cx: &mut Context<V>,
    ) -> Div {
        let (axis, min_first, min_second) = (self.axis, self.min_first, self.min_second);
        let id: SharedString = self.hits.key(&self.id).into();
        let border = cx.theme().colors().border;
        let (drag, reset, end, reset_end) = (on_resize.clone(), on_resize, on_resize_end.clone(), on_resize_end);
        let dragged = id.clone();
        let grip = handle_on(axis, id, move |this, cx| end(this, cx), move |this, cx| {
            reset(this, reset_to, cx);
            reset_end(this, cx);
        }, cx)
        .child(hit_target(&self.hits, format!("{}-divider", self.id)));
        let (split, line) = match axis {
            Axis::Columns => (div().flex(), div().w(tokens::stroke::HAIRLINE).h_full()),
            Axis::Rows => (div().flex().flex_col(), div().h(tokens::stroke::HAIRLINE).w_full()),
        };
        split
            .on_drag_move(cx.listener(move |this, e: &DragMoveEvent<DraggedDivider>, _, cx| {
                if e.drag(cx).0 != dragged {
                    return;
                }
                let total = axis.length(e.bounds.size);
                let max = (total - min_second).max(min_first);
                let at = (axis.along(e.event.position) - axis.along(e.bounds.origin)).clamp(min_first, max);
                drag(this, value(at, total), cx);
            }))
            .child(first)
            .child(line.relative().flex_none().bg(border).child(grip))
            .child(second)
    }
}

/// A split divider's grab strip across `axis`'s line, absolute in its parent.
fn handle_on<V: 'static>(
    axis: Axis,
    id: SharedString,
    on_end: impl Fn(&mut V, &mut Context<V>) + 'static,
    on_reset: impl Fn(&mut V, &mut Context<V>) + 'static,
    cx: &mut Context<V>,
) -> Stateful<Div> {
    match axis {
        Axis::Columns => handle(id, 0, on_end, on_reset, cx).left(GRIP_BEFORE),
        Axis::Rows => grip(id, 0, CursorStyle::ResizeUpDown, on_end, on_reset, cx).absolute().left_0().top(GRIP_BEFORE).w_full().h(GRIP),
    }
}

/// A divider's 7 px grab strip, absolute in its parent (the caller places it); see `grip`.
pub(crate) fn handle<V: 'static>(
    id: SharedString,
    index: usize,
    on_end: impl Fn(&mut V, &mut Context<V>) + 'static,
    on_reset: impl Fn(&mut V, &mut Context<V>) + 'static,
    cx: &mut Context<V>,
) -> Stateful<Div> {
    grip(id, index, CursorStyle::ResizeLeftRight, on_end, on_reset, cx).absolute().top_0().w(GRIP).h_full()
}

/// A box over a bar that drags the box's height, the bar a short rounded line centred under it.
pub struct HeightBar {
    /// The bar's element id and hit key; it also tells two bars' drags apart.
    pub id: SharedString,
    pub min: Pixels,
    pub max: Pixels,
    /// What a double-click on the bar restores.
    pub default: Pixels,
}

impl HeightBar {
    /// `content` sized to `height`; `on_resize` gets the height, held within min and max; `on_resize_end` follows.
    pub fn render<V: 'static>(
        &self,
        content: impl IntoElement,
        height: Pixels,
        hits: &HitTargets,
        on_resize: impl Fn(&mut V, Pixels, &mut Context<V>) + Clone + 'static,
        on_resize_end: impl Fn(&mut V, &mut Context<V>) + Clone + 'static,
        cx: &mut Context<V>,
    ) -> Div {
        let (id, min, max, default) = (self.id.clone(), self.min, self.max.max(self.min), self.default);
        let (line, lit) = (cx.theme().colors().border, cx.theme().colors().border_selected);
        let (drag, reset, end, reset_end) = (on_resize.clone(), on_resize, on_resize_end.clone(), on_resize_end);
        let dragged: SharedString = hits.key(&id).into();
        let bar = grip(dragged.clone(), 0, CursorStyle::ResizeUpDown, move |this, cx| end(this, cx), move |this, cx| {
            reset(this, default.clamp(min, max), cx);
            reset_end(this, cx);
        }, cx)
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .w_full()
        .h(px(8.))
        .child(div().w(px(48.)).h(px(4.)).rounded(tokens::radius::FULL).bg(line).hover(|s| s.bg(lit)))
        .child(hit_target(hits, id.to_string()));
        div()
            .flex()
            .flex_col()
            .on_drag_move(cx.listener(move |this, e: &DragMoveEvent<DraggedDivider>, _, cx| {
                if e.drag(cx).0 != dragged {
                    return;
                }
                drag(this, (e.event.position.y - e.bounds.top() - px(4.)).clamp(min, max), cx);
            }))
            .child(div().flex_none().h(height).min_h_0().overflow_hidden().child(content))
            .child(bar)
    }
}

/// Panes in a column whose borders drag height between the open neighbours (see `dragged_heights`); a closed one is its header.
pub struct Stack {
    /// The column's element id; its borders paint under `<id>-divider-<pane>`.
    pub id: SharedString,
    /// An open pane's least height, its header included.
    pub min: Pixels,
}

/// One pane of a [`Stack`]: `body` `None` while it is closed; `height` the owner's, zero for an even share.
pub struct Pane {
    pub header: AnyElement,
    pub body: Option<AnyElement>,
    pub height: Pixels,
}

/// The panes' bounds as last laid out, which a drag measures from; the owner keeps it between frames.
#[derive(Clone, Default)]
pub struct StackState(Rc<RefCell<Vec<Option<Bounds<Pixels>>>>>);

impl Stack {
    /// `on_resize` gets every pane's height while a border is dragged, all zero on a double-click; `on_resize_end` follows.
    pub fn render<V: 'static>(
        &self,
        state: &StackState,
        panes: Vec<Pane>,
        hits: &HitTargets,
        on_resize: impl Fn(&mut V, Vec<Pixels>, &mut Context<V>) + Clone + 'static,
        on_resize_end: impl Fn(&mut V, &mut Context<V>) + Clone + 'static,
        cx: &mut Context<V>,
    ) -> Div {
        let given: Vec<(bool, Pixels)> = panes.iter().map(|p| (p.body.is_some(), p.height)).collect();
        state.0.borrow_mut().resize(panes.len(), None);
        let weights = weights(&given);
        let (id, min, measured) = (self.id.clone(), self.min, state.clone());
        let dragged = format!("{}-divider-", hits.key(&id));
        let drag = on_resize.clone();
        let column = div().flex().flex_col().size_full().min_h_0().overflow_hidden().on_drag_move(cx.listener(move |this, e: &DragMoveEvent<DraggedDivider>, _, cx| {
            let divider = e.drag(cx);
            if !divider.0.starts_with(&dragged) {
                return;
            }
            if let Some(heights) = dragged_heights(&measured.0.borrow(), &given, divider.1, e.event.position.y, min) {
                drag(this, heights, cx);
            }
        }));
        let count = panes.len();
        let children = panes.into_iter().enumerate().map(|(i, pane)| {
            let open = pane.body.is_some();
            let border = (i > 0).then(|| {
                let (reset, end, reset_end) = (on_resize.clone(), on_resize_end.clone(), on_resize_end.clone());
                let key = format!("{id}-divider-{i}");
                grip(hits.key(&key).into(), i, CursorStyle::ResizeUpDown, move |this, cx| end(this, cx), move |this, cx| {
                    reset(this, vec![Pixels::ZERO; count], cx);
                    reset_end(this, cx);
                }, cx)
                .absolute()
                .left_0()
                .top(GRIP_BEFORE)
                .w_full()
                .h(GRIP)
                .child(hit_target(hits, key))
            });
            let probe = state.clone();
            let record = canvas(move |bounds, _, _| probe.0.borrow_mut()[i] = Some(bounds), |_, _, _, _| {}).absolute().top_0().left_0().size_full();
            let mut pane_div = div().relative().flex().flex_col().min_h_0().child(record).child(div().relative().flex_none().child(pane.header).children(border));
            if open {
                pane_div.style().flex_grow = Some(weights[i]);
                pane_div = pane_div.flex_shrink(1.).flex_basis(Pixels::ZERO).min_h(min).children(pane.body.map(|b| div().flex().flex_col().flex_1().min_h_0().child(b)));
            } else {
                pane_div = pane_div.flex_none();
            }
            pane_div
        });
        column.children(children)
    }
}

/// Each open pane's flex weight: its height, or the mean of the set ones (one apiece when none is set).
fn weights(given: &[(bool, Pixels)]) -> Vec<f32> {
    let set: Vec<f32> = given.iter().filter(|(open, h)| *open && *h > Pixels::ZERO).map(|(_, h)| f32::from(*h)).collect();
    let fill = if set.is_empty() { 1. } else { set.iter().sum::<f32>() / set.len() as f32 };
    given.iter().map(|(_, h)| if *h > Pixels::ZERO { f32::from(*h) } else { fill }).collect()
}

/// Every pane's height with the border above pane `border` at `y`: the nearest open pane above it and the nearest
/// open one at or below it trade height, each held at `min`; `None` without one on either side or unmeasured.
fn dragged_heights(bounds: &[Option<Bounds<Pixels>>], given: &[(bool, Pixels)], border: usize, y: Pixels, min: Pixels) -> Option<Vec<Pixels>> {
    let above = (0..border).rev().find(|i| given[*i].0)?;
    let below = (border..given.len()).find(|i| given[*i].0)?;
    let at = |i: usize| bounds.get(i).copied().flatten();
    let (top, bottom) = (at(above)?.top(), at(below)?.bottom());
    let fixed = |range: std::ops::Range<usize>| range.map(|i| at(i).map_or(Pixels::ZERO, |b| b.size.height)).fold(Pixels::ZERO, |s, h| s + h);
    let (over, under) = (fixed(above + 1..border), fixed(border..below));
    let shared = bottom - top - over - under;
    if shared < min * 2. {
        return None;
    }
    let first = (y - top - over).clamp(min, shared - min);
    let mut heights: Vec<Pixels> = given.iter().enumerate().map(|(i, (open, h))| if *open { at(i).map_or(*h, |b| b.size.height) } else { *h }).collect();
    heights[above] = first;
    heights[below] = shared - first;
    Some(heights)
}

/// The drag plumbing every divider and bar shares: a [`DraggedDivider`] drag the parent's `on_drag_move`
/// follows, `on_end` when the button is let go, and `on_reset` on a double-click.
fn grip<V: 'static>(
    id: SharedString,
    index: usize,
    cursor: CursorStyle,
    on_end: impl Fn(&mut V, &mut Context<V>) + 'static,
    on_reset: impl Fn(&mut V, &mut Context<V>) + 'static,
    cx: &mut Context<V>,
) -> Stateful<Div> {
    div()
        .id(id.clone())
        .occlude()
        .cursor(cursor)
        .on_drag(DraggedDivider(id, index), |d, _, _, cx| cx.new(|_| DraggedDivider(d.0.clone(), d.1)))
        .on_mouse_up(MouseButton::Left, cx.listener(move |this, _, _, cx| on_end(this, cx)))
        .on_click(cx.listener(move |this, e: &ClickEvent, _, cx| {
            if e.click_count() == 2 {
                on_reset(this, cx);
            }
        }))
}

#[cfg(test)]
mod tests {
    use gpui::{Entity, Modifiers, MouseButton, TestAppContext, VisualTestContext, point};

    use super::*;

    struct Message {
        hits: HitTargets,
        height: Pixels,
        heights: Vec<Pixels>,
        ends: usize,
    }

    impl Render for Message {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let bar = HeightBar { id: "message-bar".into(), min: px(40.), max: px(200.), default: px(100.) };
            let column = bar.render(
                div().size_full().child("message"),
                self.height,
                &self.hits,
                |this: &mut Message, height, cx| {
                    this.height = height;
                    this.heights.push(height);
                    cx.notify();
                },
                |this: &mut Message, _| this.ends += 1,
                cx,
            );
            div().size_full().child(column)
        }
    }

    fn message(cx: &mut TestAppContext) -> (Entity<Message>, &mut VisualTestContext) {
        cx.update(|cx| crate::init_globals(cx, "test"));
        let (message, cx) = cx.add_window_view(|_, _| Message { hits: HitTargets::default(), height: px(100.), heights: Vec::new(), ends: 0 });
        cx.run_until_parked();
        (message, cx)
    }

    #[gpui::test]
    fn dragging_the_bar_reports_the_height_within_its_bounds_and_its_end(cx: &mut TestAppContext) {
        let (message, cx) = message(cx);
        let hit = message.read_with(cx, |m, _| m.hits.borrow().get("message-bar").copied()).expect("the bar paints under its key");
        let start = hit.bounds.center();
        cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(start + point(px(0.), px(20.)), MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(start + point(px(0.), px(30.)), MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(start + point(px(0.), px(30.)), MouseButton::Left, Modifiers::none());
        message.read_with(cx, |m, _| {
            assert!(m.height > px(126.) && m.height < px(134.), "follows the pointer 30 px down: {:?}", m.heights);
            assert_eq!(m.ends, 1);
        });
        cx.run_until_parked();
        let start = message.read_with(cx, |m, _| m.hits.borrow().get("message-bar").copied()).expect("the bar paints").bounds.center();
        let top = point(start.x, px(2.));
        cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(start - point(px(0.), px(10.)), MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(top, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(top, MouseButton::Left, Modifiers::none());
        message.read_with(cx, |m, _| assert_eq!(m.height, px(40.), "held at its min"));
        cx.run_until_parked();
        let hit = message.read_with(cx, |m, _| m.hits.borrow().get("message-bar").copied()).expect("the bar paints");
        cx.simulate_event(gpui::MouseDownEvent { position: hit.bounds.center(), button: MouseButton::Left, modifiers: Modifiers::none(), click_count: 1, first_mouse: false });
        cx.simulate_event(gpui::MouseUpEvent { position: hit.bounds.center(), button: MouseButton::Left, modifiers: Modifiers::none(), click_count: 1 });
        cx.simulate_event(gpui::MouseDownEvent { position: hit.bounds.center(), button: MouseButton::Left, modifiers: Modifiers::none(), click_count: 2, first_mouse: false });
        cx.simulate_event(gpui::MouseUpEvent { position: hit.bounds.center(), button: MouseButton::Left, modifiers: Modifiers::none(), click_count: 2 });
        message.read_with(cx, |m, _| assert_eq!(m.height, px(100.), "a double-click restores the default"));
    }

    struct Offset {
        hits: HitTargets,
        width: Pixels,
    }

    impl Render for Offset {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let pane = SplitPane { id: "pane".into(), min_first: px(200.), min_second: px(260.), default: px(380.), ..Default::default() };
            let second = div().relative().w(self.width).h_full().child(hit_target(&self.hits, "second".into()));
            let split = pane.render_second(div().flex_1().min_w_0().h_full(), second, |this: &mut Offset, width, cx| {
                this.width = width;
                cx.notify();
            }, |_: &mut Offset, _| {}, cx);
            div().flex().size_full().child(div().flex_none().w(px(300.))).child(split.flex_1().h_full())
        }
    }

    #[gpui::test]
    fn a_split_right_of_the_window_edge_holds_its_second_panel_at_its_min(cx: &mut TestAppContext) {
        cx.update(|cx| crate::init_globals(cx, "test"));
        let (view, cx) = cx.add_window_view(|_, _| Offset { hits: HitTargets::default(), width: px(380.) });
        cx.run_until_parked();
        let second = view.read_with(cx, |o, _| o.hits.borrow().get("second").copied()).expect("the second panel paints").bounds;
        let start = point(second.left() - px(1.), second.center().y);
        let end = point(second.right() - px(4.), start.y);
        cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(start + point(px(20.), px(0.)), MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(end, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::none());
        let width = view.read_with(cx, |o, _| o.width);
        assert!(width >= px(259.) && width <= px(261.), "held at the second panel's min: {width:?}");
    }
    struct Panes {
        hits: HitTargets,
        state: StackState,
        heights: Vec<Pixels>,
        ends: usize,
    }

    impl Render for Panes {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let panes = ["a", "b", "c"]
                .iter()
                .enumerate()
                .map(|(i, name)| Pane {
                    header: div().h(px(20.)).child(*name).into_any_element(),
                    body: (i != 1).then(|| div().size_full().into_any_element()),
                    height: self.heights[i],
                })
                .collect();
            let stack = Stack { id: "stack".into(), min: px(60.) };
            let column = stack.render(&self.state, panes, &self.hits, |this: &mut Panes, heights, cx| {
                this.heights = heights;
                cx.notify();
            }, |this: &mut Panes, _| this.ends += 1, cx);
            div().h(px(320.)).w(px(200.)).child(column)
        }
    }

    #[gpui::test]
    fn a_stacks_border_trades_height_between_the_open_panes_beside_it(cx: &mut TestAppContext) {
        cx.update(|cx| crate::init_globals(cx, "test"));
        let (view, cx) = cx.add_window_view(|_, _| Panes { hits: HitTargets::default(), state: StackState::default(), heights: vec![px(0.); 3], ends: 0 });
        cx.run_until_parked();
        let bounds = |cx: &mut VisualTestContext| view.read_with(cx, |p, _| p.state.0.borrow().iter().map(|b| b.unwrap().size.height).collect::<Vec<_>>());
        assert_eq!(bounds(cx), [px(150.), px(20.), px(150.)], "the open panes share what the closed header leaves");
        let border = |key: &str, cx: &mut VisualTestContext| view.read_with(cx, |p, _| p.hits.borrow().get(key).copied()).expect("a border paints").bounds.center();
        let start = border("stack-divider-2", cx);
        cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(start + point(px(0.), px(10.)), MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(start + point(px(0.), px(30.)), MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(start + point(px(0.), px(30.)), MouseButton::Left, Modifiers::none());
        cx.run_until_parked();
        let heights = view.read_with(cx, |p, _| p.heights.clone());
        assert!((heights[0] - px(180.)).abs() < px(1.) && (heights[2] - px(120.)).abs() < px(1.) && heights[1] == px(0.), "below the closed pane, A and C trade: {heights:?}");
        let laid = bounds(cx);
        assert!((laid[0] - px(180.)).abs() < px(1.) && (laid[2] - px(120.)).abs() < px(1.), "laid out so: {laid:?}");
        assert_eq!(view.read_with(cx, |p, _| p.ends), 1);
        let start = border("stack-divider-1", cx);
        cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(start - point(px(0.), px(20.)), MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(point(start.x, px(1.)), MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(point(start.x, px(1.)), MouseButton::Left, Modifiers::none());
        cx.run_until_parked();
        let heights = view.read_with(cx, |p, _| p.heights.clone());
        assert!(heights[0] == px(60.) && (heights[2] - px(240.)).abs() < px(1.), "the border above the closed pane too, A held at its min: {heights:?}");
    }

    struct Nested {
        hits: HitTargets,
        width: Pixels,
        height: Pixels,
    }

    impl Render for Nested {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let rows = SplitPane { id: "rows".into(), min_first: px(80.), min_second: px(100.), axis: Axis::Rows, hits: self.hits.clone(), ..Default::default() };
            let top = div().flex_none().h(self.height).w_full();
            let inner = rows.render(top, div().flex_1().min_h_0().w_full(), |this: &mut Nested, height, cx| {
                this.height = height;
                cx.notify();
            }, |_: &mut Nested, _| {}, cx);
            let columns = SplitPane { id: "columns".into(), min_first: px(120.), min_second: px(150.), hits: self.hits.clone(), ..Default::default() };
            let left = div().flex_none().w(self.width).h_full();
            let split = columns.render(left, inner.flex_1().min_w_0().h_full(), |this: &mut Nested, width, cx| {
                this.width = width;
                cx.notify();
            }, |_: &mut Nested, _| {}, cx);
            div().w(px(600.)).h(px(400.)).child(split.size_full())
        }
    }

    fn drag(cx: &mut VisualTestContext, from: gpui::Point<Pixels>, to: gpui::Point<Pixels>) {
        cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(from + (to - from) * 0.5, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(to, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
        cx.run_until_parked();
    }

    #[gpui::test]
    fn a_rows_split_inside_a_columns_split_drags_only_its_own_divider_within_its_mins(cx: &mut TestAppContext) {
        cx.update(|cx| crate::init_globals(cx, "test"));
        let (view, cx) = cx.add_window_view(|_, _| Nested { hits: HitTargets::default(), width: px(250.), height: px(150.) });
        cx.run_until_parked();
        let divider = |key: &str, cx: &mut VisualTestContext| view.read_with(cx, |n, _| n.hits.borrow().get(key).copied()).expect("each divider paints under <id>-divider").bounds.center();
        let state = |cx: &mut VisualTestContext| view.read_with(cx, |n, _| (n.width, n.height));
        let start = divider("rows-divider", cx);
        drag(cx, start, start + point(px(0.), px(40.)));
        let (width, height) = state(cx);
        assert_eq!(width, px(250.), "the columns' divider stays");
        assert!((height - px(190.)).abs() < px(2.), "the rows follow the pointer 40 px down: {height:?}");
        let start = divider("rows-divider", cx);
        drag(cx, start, point(start.x, px(395.)));
        assert_eq!(state(cx).1, px(400.) - px(100.), "held at the second row's min");
        let start = divider("rows-divider", cx);
        drag(cx, start, point(start.x, px(2.)));
        assert_eq!(state(cx).1, px(80.), "held at the first row's min");
        let start = divider("columns-divider", cx);
        drag(cx, start, start + point(px(30.), px(0.)));
        let (width, height) = state(cx);
        assert_eq!(height, px(80.), "the rows' divider stays");
        assert!((width - px(280.)).abs() < px(2.), "the columns follow the pointer 30 px right: {width:?}");
        let start = divider("columns-divider", cx);
        drag(cx, start, point(px(598.), start.y));
        assert_eq!(state(cx).0, px(600.) - px(150.), "held at the second column's min");
    }

    struct Shares {
        hits: HitTargets,
        fraction: f32,
    }

    impl Render for Shares {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let pane = SplitPane { id: "shares".into(), min_first: px(100.), min_second: px(100.), hits: self.hits.clone(), ..Default::default() };
            let first = div().size_full().child(hit_target(&self.hits, "first".into()));
            let split = pane.render_fraction(first, div().size_full(), self.fraction, 0.5, |this: &mut Shares, fraction, cx| {
                this.fraction = fraction;
                cx.notify();
            }, |_: &mut Shares, _| {}, cx);
            div().size_full().child(split.size_full())
        }
    }

    #[gpui::test]
    fn a_split_by_fraction_keeps_its_proportions_as_the_window_resizes(cx: &mut TestAppContext) {
        cx.update(|cx| crate::init_globals(cx, "test"));
        let (view, cx) = cx.add_window_view(|_, _| Shares { hits: HitTargets::default(), fraction: 0.5 });
        cx.simulate_resize(gpui::size(px(801.), px(300.)));
        cx.run_until_parked();
        let first = |cx: &mut VisualTestContext| view.read_with(cx, |s, _| s.hits.borrow().get("first").copied()).expect("the first side paints").bounds.size.width;
        assert!((first(cx) - px(400.)).abs() < px(1.), "an even share: {:?}", first(cx));
        let start = view.read_with(cx, |s, _| s.hits.borrow().get("shares-divider").copied()).expect("the divider paints").bounds.center();
        drag(cx, start, point(px(200.), start.y));
        let fraction = view.read_with(cx, |s, _| s.fraction);
        assert!((fraction - 0.25).abs() < 0.01, "the share the pointer left: {fraction}");
        assert!((first(cx) - px(200.)).abs() < px(1.), "laid out so: {:?}", first(cx));
        cx.simulate_resize(gpui::size(px(1601.), px(300.)));
        cx.run_until_parked();
        assert_eq!(view.read_with(cx, |s, _| s.fraction), fraction, "a resize keeps the owner's share");
        assert!((first(cx) - px(400.)).abs() < px(1.), "the same share of the wider window: {:?}", first(cx));
        cx.simulate_resize(gpui::size(px(301.), px(300.)));
        cx.run_until_parked();
        assert!((first(cx) - px(100.)).abs() < px(1.), "a narrow window holds the first side at its min: {:?}", first(cx));
    }
}
