//! Every side form: a title with its icon and ×, labelled fields, and its OK (`button::action` Success Solid, [`BUTTON_HEIGHT`]) dimmed with why,
//! turning while the owner's write runs;
//! Tab and Shift+Tab through its `FocusOrder`, Enter in a field its OK, Esc its close; hit keys `<key>`, `<key>-close`, `<key>-ok`.

use gpui::{AnyElement, AppContext as _, ClickEvent, Context, Div, Entity, FocusHandle, FontWeight, Pixels, SharedString, Stateful, Subscription, Window, div, prelude::*, px};
use theme::ActiveTheme as _;

use crate::button;
use crate::focus_order::{Dismiss, FocusOrder, Press};
use crate::hit::{HitTargets, hit_target};
use crate::icon::Icon;
use crate::text_input::{InputEvent, SINGLE_HEIGHT, TextInput};
use crate::themes::secondary;
use crate::tooltip::Tooltip;

/// A form's buttons stand as high as its one-line fields.
pub const BUTTON_HEIGHT: Pixels = SINGLE_HEIGHT;

/// `button` (a `button::action`) at [`BUTTON_HEIGHT`]: a form's OK, a button beside a field (Browse) and the WIP panel's Commit.
pub fn sized(button: Stateful<Div>) -> Stateful<Div> {
    button.h(BUTTON_HEIGHT).px_3()
}

/// The owner's OK and close, what the form's button, ×, Enter and Esc run.
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
    pub icon: Icon,
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
    /// The form over its owner's place, `fields` as `(label, field)` in order; `ok_focus` makes OK a member of `order`'s Tab.
    pub fn render(self, hits: &HitTargets, fields: Vec<(SharedString, AnyElement)>, order: FocusOrder, ok_focus: Option<&FocusHandle>, cx: &mut Context<V>) -> AnyElement {
        let colors = cx.theme().colors();
        let (muted, border, background) = (secondary(colors), colors.border, colors.panel_background);
        let Answers { ok: on_ok, close: on_close } = self.answers;
        let close = button::icon(hits, format!("{}-close", self.key), Icon::Close.svg(14., muted), px(22.), cx)
            .tooltip(Tooltip::text("Close (Esc)"))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| on_close(this, cx)));
        let title = div()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(border)
            .child(self.icon.svg(16., muted))
            .child(div().flex_1().min_w_0().font_weight(FontWeight::SEMIBOLD).child(self.title))
            .child(close);
        let fields = fields.into_iter().map(|(label, field)| div().flex().flex_col().gap_1().child(div().text_xs().text_color(muted).child(label)).child(field));
        let ok_key = format!("{}-ok", self.key);
        let ok = if self.running {
            button::action_turning(hits, ok_key, Icon::Refresh, self.ok, button::Tone::Success, button::Fill::Solid, cx)
        } else {
            button::action(hits, ok_key, self.ok, button::Tone::Success, button::Fill::Solid, cx)
        };
        let ok = match self.why {
            Some(why) => button::dimmed(ok).tooltip(Tooltip::text(why)),
            None => ok.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| on_ok(this, cx))),
        };
        let ok = match ok_focus {
            Some(focus) => button::focusable(ok, focus, cx.listener(move |this, _: &Press, _, cx| on_ok(this, cx)), cx),
            None => ok,
        };
        let body = div().flex().flex_col().gap_3().p_3().children(fields).child(div().flex().child(sized(ok)));
        order
            .on(div())
            .on_action(cx.listener(move |this, _: &Dismiss, _, cx| on_close(this, cx)))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(background)
            .child(title)
            .child(body)
            .child(hit_target(hits, self.key.to_string()))
            .into_any_element()
    }
}
