//! A question before an action that cannot be taken back, or a report with only its OK, asked in a [`crate::surfaces::dialog`].

use gpui::SharedString;

use crate::controls::button;
use crate::hit::HitTargets;

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
}

/// Every answer of a question and every button of a [`crate::surfaces::dialog`]: a `button::Button` at `button::Size::Medium`, the
/// message's text size; the dialog makes the default one Solid, the rest Outline.
pub fn answer(hits: &HitTargets, key: impl Into<SharedString>, words: impl Into<SharedString>, tone: button::Tone) -> button::Button {
    button::Button::new(hits, key).label(words).tone(tone).size(button::Size::Medium)
}
