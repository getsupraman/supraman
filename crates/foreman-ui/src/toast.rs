//! A short note floating over the bottom of a panel, gone after [`TOAST_FOR`]; it moves nothing.

use std::time::{Duration, Instant};

use gpui::{Context, Div, SharedString, Stateful, div, prelude::*};
use theme::ActiveTheme as _;

use crate::hit::{HitTargets, hit_target};

pub const TOAST_FOR: Duration = Duration::from_millis(2500);

struct Toast {
    text: String,
    shown_at: Instant,
    done: bool,
}

#[derive(Default)]
pub struct ToastSlot(Option<Toast>);

impl ToastSlot {
    pub fn text(&self) -> Option<&str> {
        self.0.as_ref().map(|t| t.text.as_str())
    }

    /// Shows `text` in place of any toast before; `slot` finds this slot in the view when the timer ends.
    pub fn show<V: 'static>(&mut self, text: String, slot: fn(&mut V) -> &mut ToastSlot, cx: &mut Context<V>) {
        self.put(text, true, slot, cx);
    }

    /// As [`Self::show`], without the ✓: why nothing was done.
    pub fn show_why<V: 'static>(&mut self, text: String, slot: fn(&mut V) -> &mut ToastSlot, cx: &mut Context<V>) {
        self.put(text, false, slot, cx);
    }

    fn put<V: 'static>(&mut self, text: String, done: bool, slot: fn(&mut V) -> &mut ToastSlot, cx: &mut Context<V>) {
        let shown_at = Instant::now();
        self.0 = Some(Toast { text, shown_at, done });
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TOAST_FOR).await;
            this.update(cx, |this, cx| {
                let slot = slot(this);
                if slot.0.as_ref().is_some_and(|t| t.shown_at == shown_at) {
                    slot.0 = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Absolute: the caller's container must be `relative`. `key` is the element id and the hit key.
    pub fn render<V>(&self, hits: &HitTargets, key: &str, cx: &Context<V>) -> Option<Stateful<Div>> {
        let toast = self.0.as_ref()?;
        let colors = cx.theme().colors();
        let success = cx.theme().status().success;
        Some(
            div()
                .id(SharedString::from(key.to_string()))
                .absolute()
                .bottom_3()
                .left_3()
                .right_3()
                .flex()
                .justify_center()
                .child(
                    div()
                        .relative()
                        .max_w_full()
                        .px_3()
                        .py_1()
                        .rounded_md()
                        .border_1()
                        .border_color(if toast.done { success.opacity(0.5) } else { colors.border })
                        .bg(colors.elevated_surface_background)
                        .shadow_md()
                        .text_sm()
                        .text_color(colors.text)
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(SharedString::from(if toast.done { format!("✓ {}", toast.text) } else { toast.text.clone() }))
                        .child(hit_target(hits, key.to_string())),
                ),
        )
    }
}
