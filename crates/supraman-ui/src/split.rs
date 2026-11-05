//! Two panels side by side with a draggable divider between them. The owner keeps the first panel's
//! width, sizes the first panel by it, and persists it.

use gpui::{ClickEvent, Context, CursorStyle, Div, DragMoveEvent, MouseButton, Pixels, SharedString, Stateful, Window, div, prelude::*, px};
use theme::ActiveTheme as _;

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
                drag(this, (e.event.position.x - e.bounds.left()).clamp(min_first, max), cx);
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
    div()
        .id(id.clone())
        .absolute()
        .top_0()
        .w(px(7.))
        .h_full()
        .occlude()
        .cursor(CursorStyle::ResizeLeftRight)
        .on_drag(DraggedDivider(id, index), |d, _, _, cx| cx.new(|_| DraggedDivider(d.0.clone(), d.1)))
        .on_mouse_up(MouseButton::Left, cx.listener(move |this, _, _, cx| on_end(this, cx)))
        .on_click(cx.listener(move |this, e: &ClickEvent, _, cx| {
            if e.click_count() == 2 {
                on_reset(this, cx);
            }
        }))
}
