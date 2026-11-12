//! A question before an action that cannot be taken back, or a report with only its OK; a box or a band.

use gpui::{AnyElement, App, ClickEvent, Context, Div, KeyDownEvent, Pixels, SharedString, Window, div, prelude::*, px};
use theme::ActiveTheme as _;

use crate::banner;
use crate::button;
use crate::hit::{HitTargets, hit_target};

/// The height of a band's row of answers: the toolbar's.
pub const ROW: Pixels = px(34.);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirm<A> {
    pub text: String,
    pub yes: String,
    /// What a yes runs; `None` is a report, which has no Cancel.
    pub action: Option<A>,
    /// A second answer beside the yes: its words and what it runs.
    pub alt: Option<(String, A)>,
    /// The yes destroys something: its button is the error colour's and Enter cancels.
    pub danger: bool,
    /// The alt destroys something: its button is the error colour's.
    pub alt_danger: bool,
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
        Self { text: text.into(), yes: yes.into(), action: Some(action), alt: None, danger: false, alt_danger: false }
    }

    pub fn report(text: impl Into<String>) -> Self {
        Self { text: text.into(), yes: "OK".into(), action: None, alt: None, danger: false, alt_danger: false }
    }

    /// A second answer, painted under `<key>-alt` between the yes and Cancel.
    pub fn or(self, words: impl Into<String>, action: A) -> Self {
        Self { alt: Some((words.into(), action)), ..self }
    }

    pub fn danger(self) -> Self {
        Self { danger: true, ..self }
    }

    pub fn danger_alt(self) -> Self {
        Self { alt_danger: true, ..self }
    }

    /// What Enter answers: Cancel when the yes destroys something, else the yes.
    pub fn default_answer(&self) -> Answer {
        if self.danger && self.action.is_some() { Answer::No } else { Answer::Yes }
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
        let default = self.default_answer();
        let fill = |answer| if answer == default { button::Fill::Solid } else { button::Fill::Outline };
        let tone = |danger: bool| if danger { button::Tone::Danger } else { button::Tone::Neutral };
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
                    .child(button::action(hits, format!("{key}-yes"), self.yes.clone(), tone(self.danger), fill(Answer::Yes), cx).on_click(cx.listener(
                        move |this, _: &ClickEvent, window, cx| {
                            cx.stop_propagation();
                            on_yes(this, Answer::Yes, window, cx);
                        },
                    )))
                    .when_some(self.alt.as_ref(), |d, (words, _)| {
                        d.child(button::action(hits, format!("{key}-alt"), words.clone(), tone(self.alt_danger), fill(Answer::Alt), cx).on_click(cx.listener(
                            move |this, _: &ClickEvent, window, cx| {
                                cx.stop_propagation();
                                on_alt(this, Answer::Alt, window, cx);
                            },
                        )))
                    })
                    .when(self.action.is_some(), |d| {
                        d.child(button::action(hits, format!("{key}-cancel"), "Cancel", button::Tone::Neutral, fill(Answer::No), cx).on_click(cx.listener(
                            move |this, _: &ClickEvent, window, cx| {
                                cx.stop_propagation();
                                on_answer(this, Answer::No, window, cx);
                            },
                        )))
                    }),
            )
    }

    /// [`Confirm::render`] as a [`band`]; Enter takes [`Confirm::default_answer`], Esc cancels, while it holds the owner's focus.
    pub fn render_band<V: 'static>(
        &self,
        hits: &HitTargets,
        key: &str,
        on_answer: impl Fn(&mut V, bool, &mut Window, &mut Context<V>) + Clone + 'static,
        cx: &mut Context<V>,
    ) -> Div {
        self.render_band_answer(hits, key, move |this, answer, window, cx| on_answer(this, answer == Answer::Yes, window, cx), cx)
    }

    /// [`Confirm::render_band`] with each answer's [`Answer`] to `on_answer`.
    pub fn render_band_answer<V: 'static>(
        &self,
        hits: &HitTargets,
        key: &str,
        on_answer: impl Fn(&mut V, Answer, &mut Window, &mut Context<V>) + Clone + 'static,
        cx: &mut Context<V>,
    ) -> Div {
        let (default, escape) = (self.default_answer(), if self.action.is_some() { Answer::No } else { Answer::Yes });
        let look = |danger: bool, answer: Answer| {
            let tone = if danger { button::Tone::Danger } else if answer == Answer::No { button::Tone::Neutral } else { button::Tone::Success };
            (tone, if answer == default { button::Fill::Solid } else { button::Fill::Outline })
        };
        let mut items = vec![answer_button(hits, format!("{key}-yes"), self.yes.clone(), look(self.danger, Answer::Yes), Answer::Yes, on_answer.clone(), cx)];
        if let Some((words, _)) = &self.alt {
            items.push(answer_button(hits, format!("{key}-alt"), words.clone(), look(self.alt_danger, Answer::Alt), Answer::Alt, on_answer.clone(), cx));
        }
        if self.action.is_some() {
            items.push(answer_button(hits, format!("{key}-cancel"), "Cancel".into(), look(false, Answer::No), Answer::No, on_answer.clone(), cx));
        }
        band(hits, key, SharedString::from(self.text.clone()), items, cx).on_key_down(cx.listener(move |this, e: &KeyDownEvent, window, cx| {
            let answer = match e.keystroke.key.as_str() {
                "enter" => default,
                "escape" => escape,
                _ => return,
            };
            cx.stop_propagation();
            on_answer(this, answer, window, cx);
        }))
    }
}

fn answer_button<V: 'static>(
    hits: &HitTargets,
    key: String,
    words: String,
    (tone, fill): (button::Tone, button::Fill),
    answer: Answer,
    on_answer: impl Fn(&mut V, Answer, &mut Window, &mut Context<V>) + 'static,
    cx: &mut Context<V>,
) -> AnyElement {
    button::action(hits, key, words, tone, fill, cx)
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            cx.stop_propagation();
            on_answer(this, answer, window, cx);
        }))
        .into_any_element()
}

/// A prompt band: `message`, then `items` (fields, then answers) centred in a [`ROW`]; paints `<key>-band` and `<key>-text`.
pub fn band(hits: &HitTargets, key: &str, message: impl IntoElement, items: impl IntoIterator<Item = AnyElement>, cx: &App) -> Div {
    let colors = cx.theme().colors();
    let text = div().relative().min_w_0().mr_4().py_1().text_sm().text_color(colors.text).child(message).child(hit_target(hits, format!("{key}-text")));
    let row = div().flex().flex_none().items_center().gap_3().h(ROW).pr_2().children(items);
    let tinted = div().flex().flex_wrap().justify_center().items_center().px(px(10.)).bg(banner::tint(cx.theme().status().info)).child(text).child(row);
    div().relative().w_full().bg(colors.editor_background).child(tinted).child(hit_target(hits, format!("{key}-band")))
}

#[cfg(test)]
mod tests {
    use gpui::{Bounds, FocusHandle, TestAppContext, VisualTestContext};

    use super::*;

    struct Asking {
        hits: HitTargets,
        confirm: Confirm<u8>,
        answers: Vec<Answer>,
        focus: FocusHandle,
        width: Pixels,
    }

    impl Render for Asking {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let band = self.confirm.render_band_answer(&self.hits, "ask", |this: &mut Asking, answer, _, cx| {
                this.answers.push(answer);
                cx.notify();
            }, cx);
            div().w(self.width).h(px(400.)).child(band.track_focus(&self.focus))
        }
    }

    fn open(confirm: Confirm<u8>, width: f32, cx: &mut TestAppContext) -> (gpui::Entity<Asking>, &mut VisualTestContext) {
        cx.update(|cx| crate::init_globals(cx, "test"));
        let (view, cx) = cx.add_window_view(|_, cx| Asking { hits: HitTargets::default(), confirm, answers: Vec::new(), focus: cx.focus_handle(), width: px(width) });
        cx.run_until_parked();
        (view, cx)
    }

    fn at(view: &gpui::Entity<Asking>, key: &str, cx: &mut VisualTestContext) -> Bounds<Pixels> {
        view.read_with(cx, |v, _| v.hits.borrow().get(key).map(|h| h.bounds)).unwrap_or_else(|| panic!("{key} paints"))
    }

    fn deleting() -> Confirm<u8> {
        Confirm::ask("Delete the branch topic? Undo can bring it back.", "Delete", 1).or("Keep it", 2).danger()
    }

    #[gpui::test]
    fn a_band_spans_its_width_with_the_text_and_answers_on_one_centred_line(cx: &mut TestAppContext) {
        let (view, cx) = open(deleting(), 1000., cx);
        let band = at(&view, "ask-band", cx);
        assert_eq!((band.origin.x, band.size.width), (px(0.), px(1000.)), "full width");
        let [text, yes, alt, cancel] = ["ask-text", "ask-yes", "ask-alt", "ask-cancel"].map(|k| at(&view, k, cx));
        for (b, k) in [(yes, "yes"), (alt, "alt"), (cancel, "cancel")] {
            assert!((b.center().y - text.center().y).abs() < px(1.), "{k} on the text's line");
        }
        assert!(text.right() <= yes.left() && yes.right() < alt.left() && alt.right() < cancel.left(), "the text, the yes, the alt, Cancel");
        let (left, right) = (text.left() - band.left(), band.right() - cancel.right());
        assert!((left - right).abs() < px(24.), "centred as one group: {left:?} left, {right:?} right");
        assert_eq!(band.size.height, ROW, "one row");
    }

    #[gpui::test]
    fn a_band_too_narrow_for_its_text_puts_the_answers_on_a_second_row(cx: &mut TestAppContext) {
        let (view, cx) = open(deleting(), 320., cx);
        let [band, text, yes, cancel] = ["ask-band", "ask-text", "ask-yes", "ask-cancel"].map(|k| at(&view, k, cx));
        assert!(yes.top() >= text.bottom(), "the answers wrap under the text");
        assert!(text.right() <= band.right() && cancel.right() <= band.right(), "nothing past the band");
        assert!((yes.center().y - cancel.center().y).abs() < px(1.), "the answers stay on one row");
    }

    fn key(view: &gpui::Entity<Asking>, keys: &str, cx: &mut VisualTestContext) -> Vec<Answer> {
        cx.update(|window, cx| {
            let focus = view.read(cx).focus.clone();
            window.focus(&focus, cx);
        });
        cx.simulate_keystrokes(keys);
        cx.run_until_parked();
        view.update(cx, |v, _| std::mem::take(&mut v.answers))
    }

    fn swap(view: &gpui::Entity<Asking>, confirm: Confirm<u8>, cx: &mut VisualTestContext) {
        view.update(cx, |v, cx| {
            v.confirm = confirm;
            cx.notify();
        });
        cx.run_until_parked();
    }

    #[gpui::test]
    fn enter_takes_the_yes_and_esc_cancels(cx: &mut TestAppContext) {
        let (view, cx) = open(Confirm::ask("Stash 2 files?", "Stash", 1), 1000., cx);
        assert_eq!(key(&view, "enter", cx), [Answer::Yes]);
        assert_eq!(key(&view, "escape", cx), [Answer::No]);
        swap(&view, deleting(), cx);
        assert_eq!(key(&view, "enter", cx), [Answer::No], "Enter cancels a question that destroys something");
        swap(&view, Confirm::report("Nothing to decide"), cx);
        assert_eq!(key(&view, "escape", cx), [Answer::Yes], "a report's only answer");
    }
}
