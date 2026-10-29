//! A question before an action that cannot be taken back, or a report with only its OK. The owner places it.

use gpui::{ClickEvent, Context, Div, SharedString, Window, div, prelude::*};
use theme::ActiveTheme as _;

use crate::banner;
use crate::button;
use crate::hit::HitTargets;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirm<A> {
    pub text: String,
    pub yes: String,
    /// What a yes runs; `None` is a report, which has no Cancel.
    pub action: Option<A>,
}

impl<A> Confirm<A> {
    pub fn ask(text: impl Into<String>, yes: impl Into<String>, action: A) -> Self {
        Self { text: text.into(), yes: yes.into(), action: Some(action) }
    }

    pub fn report(text: impl Into<String>) -> Self {
        Self { text: text.into(), yes: "OK".into(), action: None }
    }

    /// The text, then the yes button and Cancel, under the hit keys `<key>-yes` and `<key>-cancel`.
    pub fn render<V: 'static>(
        &self,
        hits: &HitTargets,
        key: &str,
        on_answer: impl Fn(&mut V, bool, &mut Window, &mut Context<V>) + Clone + 'static,
        cx: &mut Context<V>,
    ) -> Div {
        let status = cx.theme().status().clone();
        let on_yes = on_answer.clone();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_2()
            .rounded_md()
            .border_1()
            .border_color(status.warning)
            .bg(banner::tint(status.warning))
            .text_sm()
            .child(SharedString::from(self.text.clone()))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .justify_end()
                    .child(button::bordered(hits, format!("{key}-yes"), self.yes.clone(), cx).on_click(cx.listener(
                        move |this, _: &ClickEvent, window, cx| {
                            cx.stop_propagation();
                            on_yes(this, true, window, cx);
                        },
                    )))
                    .when(self.action.is_some(), |d| {
                        d.child(button::bordered(hits, format!("{key}-cancel"), "Cancel", cx).on_click(cx.listener(
                            move |this, _: &ClickEvent, window, cx| {
                                cx.stop_propagation();
                                on_answer(this, false, window, cx);
                            },
                        )))
                    }),
            )
    }
}
