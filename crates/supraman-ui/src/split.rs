//! Two panels side by side with a draggable divider between them, and a box over a bar that drags its
//! height. The owner keeps the width or height, sizes the panel or box by it, and persists it.

use gpui::{ClickEvent, Context, CursorStyle, Div, DragMoveEvent, MouseButton, Pixels, SharedString, Stateful, Window, div, prelude::*, px};
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
    /// `on_resize` gets the first panel's new width while it is dragged and on a double-click;
    /// `on_resize_end` follows when the button is let go and after a double-click.
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

    /// [`SplitPane::render`] for an owner that keeps the second panel's width: `on_resize` gets that width,
    /// and `default` is the second panel's.
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
                let max = (e.bounds.right() - min_second).max(min_first);
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

/// A divider's 7 px grab strip, absolute in its parent (the caller places it): it starts a
/// [`DraggedDivider`] drag that the parent's `on_drag_move` follows, calls `on_end` when the button is
/// let go, and `on_reset` on a double-click.
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
    /// `content` sized to `height`; `on_resize` gets the new height, held within min and max, while the bar
    /// is dragged and on a double-click; `on_resize_end` follows when the button is let go and after a
    /// double-click.
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
}
