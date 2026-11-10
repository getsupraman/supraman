//! Resizable splits, height bars and pane stacks; the owner keeps and persists the width or heights.

use std::{cell::RefCell, rc::Rc};

use gpui::{AnyElement, Bounds, ClickEvent, Context, CursorStyle, Div, DragMoveEvent, MouseButton, Pixels, SharedString, Stateful, Window, canvas, div, prelude::*, px};
use theme::ActiveTheme as _;

use crate::hit::{HitTargets, hit_target};

pub struct SplitPane {
    /// The divider's element id; it also tells two splits' drags apart.
    pub id: SharedString,
    pub min_first: Pixels,
    pub min_second: Pixels,
    /// What a double-click on the divider restores.
    pub default: Pixels,
}

/// The drag of a divider: the divider's element id and which divider of its owner it is.
pub(crate) struct DraggedDivider(pub SharedString, pub usize);

impl Render for DraggedDivider {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

impl SplitPane {
    /// `on_resize` gets the first panel's width while dragged and on a double-click; `on_resize_end` follows.
    pub fn render<V: 'static>(
        &self,
        first: impl IntoElement,
        second: impl IntoElement,
        on_resize: impl Fn(&mut V, Pixels, &mut Context<V>) + Clone + 'static,
        on_resize_end: impl Fn(&mut V, &mut Context<V>) + Clone + 'static,
        cx: &mut Context<V>,
    ) -> Div {
        self.split(first, second, false, on_resize, on_resize_end, cx)
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
        self.split(first, second, true, on_resize, on_resize_end, cx)
    }

    fn split<V: 'static>(
        &self,
        first: impl IntoElement,
        second: impl IntoElement,
        keeps_second: bool,
        on_resize: impl Fn(&mut V, Pixels, &mut Context<V>) + Clone + 'static,
        on_resize_end: impl Fn(&mut V, &mut Context<V>) + Clone + 'static,
        cx: &mut Context<V>,
    ) -> Div {
        let (id, min_first, min_second, default) = (self.id.clone(), self.min_first, self.min_second, self.default);
        let border = cx.theme().colors().border;
        let (drag, reset, end, reset_end) = (on_resize.clone(), on_resize, on_resize_end.clone(), on_resize_end);
        let dragged = id.clone();
        div()
            .flex()
            .on_drag_move(cx.listener(move |this, e: &DragMoveEvent<DraggedDivider>, _, cx| {
                if e.drag(cx).0 != dragged {
                    return;
                }
                let max = (e.bounds.size.width - min_second).max(min_first);
                let x = (e.event.position.x - e.bounds.left()).clamp(min_first, max);
                drag(this, if keeps_second { e.bounds.size.width - x - px(1.) } else { x }, cx);
            }))
            .child(first)
            .child(
                div().relative().w(px(1.)).h_full().flex_none().bg(border).child(
                    handle(id, 0, move |this, cx| end(this, cx), move |this, cx| {
                        reset(this, default, cx);
                        reset_end(this, cx);
                    }, cx)
                    .left(px(-3.)),
                ),
            )
            .child(second)
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
    grip(id, index, CursorStyle::ResizeLeftRight, on_end, on_reset, cx).absolute().top_0().w(px(7.)).h_full()
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
        let dragged = id.clone();
        let bar = grip(id.clone(), 0, CursorStyle::ResizeUpDown, move |this, cx| end(this, cx), move |this, cx| {
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
        .child(div().w(px(48.)).h(px(4.)).rounded_full().bg(line).hover(|s| s.bg(lit)))
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
        let dragged = format!("{id}-divider-");
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
                grip(key.clone().into(), i, CursorStyle::ResizeUpDown, move |this, cx| end(this, cx), move |this, cx| {
                    reset(this, vec![px(0.); count], cx);
                    reset_end(this, cx);
                }, cx)
                .absolute()
                .left_0()
                .top(px(-3.))
                .w_full()
                .h(px(7.))
                .child(hit_target(hits, key))
            });
            let probe = state.clone();
            let record = canvas(move |bounds, _, _| probe.0.borrow_mut()[i] = Some(bounds), |_, _, _, _| {}).absolute().top_0().left_0().size_full();
            let mut pane_div = div().relative().flex().flex_col().min_h_0().child(record).child(div().relative().flex_none().child(pane.header).children(border));
            if open {
                pane_div.style().flex_grow = Some(weights[i]);
                pane_div = pane_div.flex_shrink(1.).flex_basis(px(0.)).min_h(min).children(pane.body.map(|b| div().flex().flex_col().flex_1().min_h_0().child(b)));
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
    let set: Vec<f32> = given.iter().filter(|(open, h)| *open && *h > px(0.)).map(|(_, h)| f32::from(*h)).collect();
    let fill = if set.is_empty() { 1. } else { set.iter().sum::<f32>() / set.len() as f32 };
    given.iter().map(|(_, h)| if *h > px(0.) { f32::from(*h) } else { fill }).collect()
}

/// Every pane's height with the border above pane `border` at `y`: the nearest open pane above it and the nearest
/// open one at or below it trade height, each held at `min`; `None` without one on either side or unmeasured.
fn dragged_heights(bounds: &[Option<Bounds<Pixels>>], given: &[(bool, Pixels)], border: usize, y: Pixels, min: Pixels) -> Option<Vec<Pixels>> {
    let above = (0..border).rev().find(|i| given[*i].0)?;
    let below = (border..given.len()).find(|i| given[*i].0)?;
    let at = |i: usize| bounds.get(i).copied().flatten();
    let (top, bottom) = (at(above)?.top(), at(below)?.bottom());
    let fixed = |range: std::ops::Range<usize>| range.map(|i| at(i).map_or(px(0.), |b| b.size.height)).fold(px(0.), |s, h| s + h);
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
            let pane = SplitPane { id: "pane".into(), min_first: px(200.), min_second: px(260.), default: px(380.) };
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

}
