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
    /// A second answer beside the yes: its words and what it runs.
    pub alt: Option<(String, A)>,
}

/// Which button answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Yes,
    Alt,
    No,
}

impl<A> Confirm<A> {
    pub fn ask(text: impl Into<String>, yes: impl Into<String>, action: A) -> Self {
        Self { text: text.into(), yes: yes.into(), action: Some(action), alt: None }
    }

    pub fn report(text: impl Into<String>) -> Self {
        Self { text: text.into(), yes: "OK".into(), action: None, alt: None }
    }

    /// A second answer, painted under `<key>-alt` between the yes and Cancel.
    pub fn or(self, words: impl Into<String>, action: A) -> Self {
        Self { alt: Some((words.into(), action)), ..self }
    }

    /// What `answer` runs: the yes's action, the alt's, or nothing.
    pub fn take(self, answer: Answer) -> Option<A> {
        match answer {
            Answer::Yes => self.action,
            Answer::Alt => self.alt.map(|(_, a)| a),
            Answer::No => None,
        }
    }

    /// The text, then the yes button and Cancel, under the hit keys `<key>-yes` and `<key>-cancel`.
    pub fn render<V: 'static>(
        &self,
        hits: &HitTargets,
        key: &str,
        on_answer: impl Fn(&mut V, bool, &mut Window, &mut Context<V>) + Clone + 'static,
        cx: &mut Context<V>,
    ) -> Div {
        self.render_answer(hits, key, move |this, answer, window, cx| on_answer(this, answer == Answer::Yes, window, cx), cx)
    }

    /// [`Confirm::render`] with the alt's button under `<key>-alt`, each button's [`Answer`] to `on_answer`.
    pub fn render_answer<V: 'static>(
        &self,
        hits: &HitTargets,
        key: &str,
        on_answer: impl Fn(&mut V, Answer, &mut Window, &mut Context<V>) + Clone + 'static,
        cx: &mut Context<V>,
    ) -> Div {
        let status = cx.theme().status().clone();
        let (on_yes, on_alt) = (on_answer.clone(), on_answer.clone());
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
                            on_yes(this, Answer::Yes, window, cx);
                        },
                    )))
                    .when_some(self.alt.as_ref(), |d, (words, _)| {
                        d.child(button::bordered(hits, format!("{key}-alt"), words.clone(), cx).on_click(cx.listener(
                            move |this, _: &ClickEvent, window, cx| {
                                cx.stop_propagation();
                                on_alt(this, Answer::Alt, window, cx);
                            },
                        )))
                    })
                    .when(self.action.is_some(), |d| {
                        d.child(button::bordered(hits, format!("{key}-cancel"), "Cancel", cx).on_click(cx.listener(
                            move |this, _: &ClickEvent, window, cx| {
                                cx.stop_propagation();
                                on_answer(this, Answer::No, window, cx);
                            },
                        )))
                    }),
            )
    }
}
