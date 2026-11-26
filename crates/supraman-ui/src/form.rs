//! Every small form, in its owner's [`Dialog`]: a title with its ×, labelled fields, then its OK (Enter's, dimmed with
//! why and turning while the owner's write runs); Tab and Shift+Tab through its fields and OK, Enter in a field its OK,
//! Esc its close; hit keys `<key>` (its fields), `<key>-close`, `<key>-ok`.

use std::rc::Rc;

use gpui::{AnyElement, AppContext as _, Context, Entity, FocusHandle, SharedString, Subscription, Window, div, prelude::*};

use crate::button;
use crate::dialog::{Button, Dialog, Panel};
use crate::hit::{HitTargets, hit_target};
use crate::text_input::{InputEvent, TextInput};
use crate::themes::secondary;
use crate::tokens;
use theme::ActiveTheme as _;

/// The owner's OK and close, what the form's buttons, Enter and Esc run.
pub struct Answers<V> {
    pub ok: fn(&mut V, &mut Context<V>),
    pub close: fn(&mut V, &mut Context<V>),
}

impl<V> Clone for Answers<V> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<V> Copy for Answers<V> {}

pub struct Form<V> {
    pub key: SharedString,
    pub title: SharedString,
    /// The OK button's words.
    pub ok: SharedString,
    /// Why OK cannot act now; the button dimmed with it as tooltip.
    pub why: Option<SharedString>,
    /// The owner's write of this form runs: OK turns, dimmed with `why`.
    pub running: bool,
    pub answers: Answers<V>,
}

/// A one-line field holding `text`, its Enter `answers.ok`, its Esc `answers.close`, each edit `edited`.
pub fn field<V: 'static>(placeholder: &str, text: &str, answers: Answers<V>, edited: fn(&mut V, &mut Context<V>), window: &mut Window, cx: &mut Context<V>) -> (Entity<TextInput>, Subscription) {
    field_with(placeholder, text, None, answers, edited, window, cx)
}

/// [`field`] taking only the characters `filter` keeps (`supraman_core::name_char` for a name).
pub fn field_with<V: 'static>(
    placeholder: &str,
    text: &str,
    filter: Option<fn(char) -> Option<char>>,
    answers: Answers<V>,
    edited: fn(&mut V, &mut Context<V>),
    window: &mut Window,
    cx: &mut Context<V>,
) -> (Entity<TextInput>, Subscription) {
    let input = cx.new(|cx| TextInput::new(placeholder.to_string(), filter, window, cx));
    if !text.is_empty() {
        input.update(cx, |i, cx| i.set_text(text, cx));
    }
    let sub = cx.subscribe(&input, move |this: &mut V, _, e: &InputEvent, cx| match e {
        InputEvent::Confirm => (answers.ok)(this, cx),
        InputEvent::Cancel => (answers.close)(this, cx),
        InputEvent::Edited => edited(this, cx),
        _ => cx.notify(),
    });
    (input, sub)
}

/// A field's input under its hit key, taking the row's width.
pub fn input(hits: &HitTargets, key: impl Into<SharedString>, input: &Entity<TextInput>) -> AnyElement {
    div().relative().flex_1().min_w_0().child(input.clone()).child(hit_target(hits, key.into().to_string())).into_any_element()
}

impl<V: 'static> Form<V> {
    /// The form in `dialog`, `fields` as `(label, field)` in order, `members` their focus handles in Tab order before OK
    /// (under `ok_focus` when the owner keeps its handle); the first member takes the focus as it opens.
    pub fn render(self, dialog: &Dialog, hits: &HitTargets, fields: Vec<(SharedString, AnyElement)>, members: Vec<FocusHandle>, ok_focus: Option<&FocusHandle>, cx: &mut Context<V>) -> AnyElement {
        let muted = secondary(cx.theme().colors());
        let Answers { ok: on_ok, close: on_close } = self.answers;
        let rows = fields.into_iter().map(|(label, field)| div().flex().flex_col().gap(tokens::space::S).child(div().text_size(tokens::text::SMALL).text_color(muted).child(label)).child(field));
        let body = div().relative().flex().flex_col().gap(tokens::space::XL).children(rows).child(hit_target(hits, self.key.to_string()));
        let ok_key = SharedString::from(format!("{}-ok", self.key));
        let ok = Button::new(ok_key.clone(), self.ok, button::Tone::Neutral, move |this: &mut V, _, cx| on_ok(this, cx)).dimmed(self.why).turning(self.running).focus(ok_focus);
        let panel = Panel {
            key: self.key.to_string(),
            title: Some(self.title),
            body: body.into_any_element(),
            fields: members,
            buttons: vec![ok],
            first: None,
            default: Some(ok_key),
            close: Some(format!("{}-close", self.key).into()),
            cancel: Rc::new(move |this: &mut V, _, cx| on_close(this, cx)),
        };
        dialog.render(hits, panel, cx)
    }
}
