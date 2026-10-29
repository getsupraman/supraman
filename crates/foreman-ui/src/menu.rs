//! A right-click menu of items carrying the owner's action type. One menu at a time; a click outside it or
//! choosing an item closes it.

use gpui::{AnyElement, ClickEvent, Context, MouseDownEvent, Pixels, Point, SharedString, Window, anchored, deferred, div, prelude::*, px};
use theme::ActiveTheme as _;

use crate::hit::{HitTargets, hit_target};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem<A> {
    /// Unique within the menu; painted under `menu-<key>`.
    pub key: &'static str,
    pub label: String,
    pub action: A,
}

pub fn item<A>(key: &'static str, label: impl Into<String>, action: A) -> MenuItem<A> {
    MenuItem { key, label: label.into(), action }
}

pub struct ContextMenu<A> {
    pub title: String,
    pub position: Point<Pixels>,
    pub items: Vec<MenuItem<A>>,
}

impl<A: Clone + 'static> ContextMenu<A> {
    /// Over everything, kept inside the window. `on_pick` gets the chosen item's action; `on_dismiss` a click
    /// outside. Both are expected to close the menu.
    pub fn render<V: 'static>(
        &self,
        hits: &HitTargets,
        on_pick: impl Fn(&mut V, A, &mut Window, &mut Context<V>) + Clone + 'static,
        on_dismiss: impl Fn(&mut V, &mut Context<V>) + 'static,
        cx: &mut Context<V>,
    ) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let mut list = div()
            .id("context-menu")
            .flex()
            .flex_col()
            .min_w(px(180.))
            .py_1()
            .rounded_md()
            .border_1()
            .border_color(colors.border)
            .bg(colors.elevated_surface_background)
            .shadow_md()
            .text_sm()
            .occlude()
            .on_mouse_down_out(cx.listener(move |this, _: &MouseDownEvent, _, cx| on_dismiss(this, cx)))
            .child(
                div()
                    .px_3()
                    .pb_1()
                    .text_size(px(10.))
                    .text_color(colors.text_placeholder)
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(SharedString::from(self.title.clone())),
            );
        for entry in &self.items {
            let key = format!("menu-{}", entry.key);
            let action = entry.action.clone();
            let on_pick = on_pick.clone();
            list = list.child(
                div()
                    .id(SharedString::from(key.clone()))
                    .relative()
                    .px_3()
                    .py(px(3.))
                    .cursor_pointer()
                    .hover(|s| s.bg(colors.element_hover))
                    .child(SharedString::from(entry.label.clone()))
                    .child(hit_target(hits, key))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        on_pick(this, action.clone(), window, cx);
                    })),
            );
        }
        deferred(anchored().position(self.position).snap_to_window_with_margin(px(8.)).child(list)).with_priority(1).into_any_element()
    }
}
