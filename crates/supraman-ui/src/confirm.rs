//! A question before an action that cannot be taken back, or a report with only its OK; a band here, a dialog in [`crate::dialog`].

use gpui::{AnyElement, App, ClickEvent, Context, Div, KeyDownEvent, Pixels, SharedString, Stateful, Window, div, prelude::*, px};
use theme::ActiveTheme as _;

use crate::button;
use crate::hit::{HitTargets, hit_target};
use crate::themes;

/// The height of a band's row of answers: the toolbar's.
pub const ROW: Pixels = px(34.);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirm<A> {
    /// The dialog's title line.
    pub title: Option<String>,
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
        Self { title: None, text: text.into(), yes: yes.into(), action: Some(action), alt: None, danger: false, alt_danger: false }
    }

    pub fn report(text: impl Into<String>) -> Self {
        Self { title: None, text: text.into(), yes: "OK".into(), action: None, alt: None, danger: false, alt_danger: false }
    }

    /// A second answer, painted under `<key>-alt` between the yes and Cancel.
    pub fn or(self, words: impl Into<String>, action: A) -> Self {
        Self { alt: Some((words.into(), action)), ..self }
    }

    pub fn titled(self, title: impl Into<String>) -> Self {
        Self { title: Some(title.into()), ..self }
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

    /// The question as a [`band`]: its text, then the yes, the alt and Cancel under `<key>-yes`, `<key>-alt` and `<key>-cancel`; Enter takes [`Confirm::default_answer`], Esc cancels, while it holds the owner's focus.
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
        let items = self.answers(hits, key, button::Tone::Success, on_answer.clone(), cx).into_iter().map(IntoElement::into_any_element);
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

    /// The yes in `yes_tone` (Danger when it destroys), the alt, then Cancel: each an [`answer`], the default one ringed.
    fn answers<V: 'static>(
        &self,
        hits: &HitTargets,
        key: &str,
        yes_tone: button::Tone,
        on_answer: impl Fn(&mut V, Answer, &mut Window, &mut Context<V>) + Clone + 'static,
        cx: &mut Context<V>,
    ) -> Vec<Stateful<Div>> {
        let default = self.default_answer();
        let tone = |danger: bool| if danger { button::Tone::Danger } else { yes_tone };
        let mut asked = vec![(format!("{key}-yes"), self.yes.clone(), tone(self.danger), Answer::Yes)];
        if let Some((words, _)) = &self.alt {
            asked.push((format!("{key}-alt"), words.clone(), tone(self.alt_danger), Answer::Alt));
        }
        if self.action.is_some() {
            asked.push((format!("{key}-cancel"), "Cancel".into(), button::Tone::Neutral, Answer::No));
        }
        asked
            .into_iter()
            .map(|(key, words, tone, answer)| {
                let on_answer = on_answer.clone();
                self::answer(hits, key, words, tone, answer == default, cx)
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        on_answer(this, answer, window, cx);
                    }))
            })
            .collect()
    }
}

/// Every answer of a question (a band's, a dialog's): `button::action` Solid at `button::Size::Medium`, the message's
/// text size; the `default` one, Enter's, `button::ring`ed in the theme's text colour, which reads on the band.
pub fn answer(hits: &HitTargets, key: impl Into<SharedString>, words: impl Into<SharedString>, tone: button::Tone, default: bool, cx: &App) -> Stateful<Div> {
    let ring = button::ring(cx.theme().colors().text);
    button::sized(button::action(hits, key, words, tone, button::Fill::Solid, cx), button::Size::Medium).when(default, |b| b.shadow(vec![ring]))
}

/// A prompt band on the [`themes::attention`] fill: `message`, then `items` (fields, then answers) centred in a [`ROW`]; paints `<key>-band` and `<key>-text`.
pub fn band(hits: &HitTargets, key: &str, message: impl IntoElement, items: impl IntoIterator<Item = AnyElement>, cx: &App) -> Div {
    let text = div().relative().min_w_0().mr_4().py_1().text_sm().text_color(cx.theme().colors().text).child(message).child(hit_target(hits, format!("{key}-text")));
    let row = div().flex().flex_none().items_center().gap(button::GAP).h(ROW).pr_2().children(items);
    div()
        .relative()
        .w_full()
        .flex()
        .flex_wrap()
        .justify_center()
        .items_center()
        .content_center()
        .px(px(10.))
        .bg(themes::attention::attention(cx.theme()))
        .child(text)
        .child(row)
        .child(hit_target(hits, format!("{key}-band")))
}

/// `band` over the top of its owner (a `relative` element) at least `height` tall, its content centred, nothing under it reachable.
pub fn cover(band: Div, height: Pixels) -> Div {
    div().absolute().top_0().left_0().right_0().occlude().child(band.min_h(height))
}

#[cfg(test)]
mod tests {
    use gpui::{Bounds, FocusHandle, TestAppContext, VisualTestContext};

    use super::*;
    use crate::ink;

    struct Asking {
        hits: HitTargets,
        confirm: Confirm<u8>,
        answers: Vec<Answer>,
        focus: FocusHandle,
        width: Pixels,
        cover: Option<Pixels>,
        painted: Vec<(Option<gpui::Hsla>, bool)>,
        sizes: Vec<Option<gpui::AbsoluteLength>>,
    }

    impl Render for Asking {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let probe = HitTargets::default();
            let mut answers = self.confirm.answers(&probe, "probe", button::Tone::Success, |_: &mut Asking, _, _, _| {}, cx);
            self.sizes = answers.iter_mut().map(|b| b.style().text.font_size).collect();
            self.painted = answers.iter_mut().map(|b| {
                let style = b.style();
                (style.background.as_ref().and_then(|f| f.color()).and_then(|c| c.as_solid()), style.box_shadow.as_ref().is_some_and(|s| !s.is_empty()))
            }).collect();
            let band = self.confirm.render_band_answer(&self.hits, "ask", |this: &mut Asking, answer, _, cx| {
                this.answers.push(answer);
                cx.notify();
            }, cx);
            let band = band.track_focus(&self.focus);
            let placed = match self.cover {
                Some(height) => cover(band, height),
                None => band,
            };
            div().relative().w(self.width).h(px(400.)).child(div().h(px(50.)).child(hit_target(&self.hits, "under".into()))).child(placed)
        }
    }

    fn open(confirm: Confirm<u8>, width: f32, cx: &mut TestAppContext) -> (gpui::Entity<Asking>, &mut VisualTestContext) {
        cx.update(|cx| crate::init_globals(cx, "test"));
        let (view, cx) = cx.add_window_view(|_, cx| Asking { hits: HitTargets::default(), confirm, answers: Vec::new(), focus: cx.focus_handle(), width: px(width), cover: None, painted: Vec::new(), sizes: Vec::new() });
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
        let below = letters_below(cx);
        let sizes = view.read_with(cx, |v, _| v.sizes.clone());
        assert_eq!(sizes, [Some(gpui::rems(0.875).into()); 3], "the answers in the message's text size, so one offset puts both's letters");
        for (b, k) in [(yes, "yes"), (alt, "alt"), (cancel, "cancel")] {
            assert!(ink::level(b.center().y + below, text.center().y + below), "{k}'s letters on the text's: {b:?} by {text:?}");
        }
        assert!(text.right() <= yes.left() && yes.right() < alt.left() && alt.right() < cancel.left(), "the text, the yes, the alt, Cancel");
        assert_eq!((alt.left() - yes.right(), cancel.left() - alt.right()), (button::GAP + px(2.), button::GAP + px(2.)), "one gap between answers, border to border");
        let (left, right) = (text.left() - band.left(), band.right() - cancel.right());
        assert!((left - right).abs() < px(24.), "centred as one group: {left:?} left, {right:?} right");
        assert_eq!(band.size.height, ROW, "one row");
    }

    fn letters_below(cx: &mut VisualTestContext) -> Pixels {
        cx.update(|window, _| {
            let mut style = window.text_style();
            style.font_size = gpui::rems(0.875).into();
            ink::offset(&style, window)
        })
    }

    fn painted(view: &gpui::Entity<Asking>, cx: &mut VisualTestContext) -> Vec<(Option<gpui::Hsla>, bool)> {
        view.read_with(cx, |v, _| v.painted.clone())
    }

    #[gpui::test]
    fn every_answer_of_a_question_paints_the_same_fill_at_the_forms_button_height_and_only_the_default_is_ringed(cx: &mut TestAppContext) {
        let (view, cx) = open(Confirm::ask("Stash 2 files?", "Stash", 1).or("Stash staged", 2), 1000., cx);
        let [success, neutral, danger] = cx.update(|_, cx| [button::Tone::Success, button::Tone::Neutral, button::Tone::Danger].map(|tone| button::look(tone, button::Fill::Solid, cx).fill));
        assert_eq!(painted(&view, cx), [(Some(success), true), (Some(success), false), (Some(neutral), false)], "all Solid, the yes ringed");
        swap(&view, deleting(), cx);
        assert_eq!(painted(&view, cx), [(Some(danger), false), (Some(success), false), (Some(neutral), true)], "all Solid, Cancel ringed as Enter's");
        swap(&view, Confirm::ask("Delete the tag v1?", "Delete", 1).danger(), cx);
        assert_eq!(painted(&view, cx), [(Some(danger), false), (Some(neutral), true)], "a two-answer question");
        for k in ["ask-yes", "ask-cancel"] {
            assert_eq!(at(&view, k, cx).size.height + px(2.), button::Size::Medium.height(), "{k} and its border at the medium height");
        }
    }

    #[gpui::test]
    fn a_cover_band_lies_over_the_top_of_its_owner_at_its_height_with_its_content_centred(cx: &mut TestAppContext) {
        let (view, cx) = open(deleting(), 1000., cx);
        view.update(cx, |v, cx| {
            v.cover = Some(px(70.));
            cx.notify();
        });
        cx.run_until_parked();
        let [band, under, text, yes] = ["ask-band", "under", "ask-text", "ask-yes"].map(|k| at(&view, k, cx));
        assert_eq!((band.origin, band.size.width, band.size.height), (under.origin, px(1000.), px(70.)), "over the owner's top, the height asked");
        assert!(band.bottom() >= under.bottom(), "covers what lies there");
        for (b, k) in [(text, "text"), (yes, "yes")] {
            assert!((b.center().y - band.center().y).abs() < px(1.), "the {k} centred vertically: {b:?} in {band:?}");
        }
        view.update(cx, |v, cx| {
            v.width = px(320.);
            cx.notify();
        });
        cx.run_until_parked();
        let [band, text, yes] = ["ask-band", "ask-text", "ask-yes"].map(|k| at(&view, k, cx));
        assert!(band.size.height > px(70.) && yes.top() >= text.bottom(), "too narrow: it grows by the wrapped row");
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
