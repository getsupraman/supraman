//! The one modal dialog: a scrim over the whole window that takes every click and key, and a panel centred on it;
//! a question's ([`Confirm`]) or a form's.

use std::rc::Rc;

use gpui::{AnyElement, BoxShadow, Context, Div, FocusHandle, KeyDownEvent, Pixels, SharedString, Window, canvas, deferred, div, point, prelude::*, px};
use theme::ActiveTheme as _;

use crate::button;
use crate::confirm::{self, Answer, Confirm};
use crate::focus_order::{Dismiss, Members};
use crate::hit::{HitTargets, hit_target};
use crate::tokens;

/// The panel's width, or the window's less [`MARGIN`] each side when narrower.
pub const WIDTH: Pixels = px(520.);
/// The least room between the panel and the window's edges.
pub const MARGIN: Pixels = tokens::space::PAGE;
/// The scrim's black on a dark theme: enough to set the panel apart on a near-black ground.
pub const SCRIM_DARK: f32 = 0.6;
/// The scrim's black on a light theme: [`SCRIM_DARK`] there turns the window a muddy grey.
pub const SCRIM_LIGHT: f32 = 0.3;

type Act<V> = Rc<dyn Fn(&mut V, &mut Window, &mut Context<V>)>;

/// One of the panel's buttons, under its hit key.
pub struct Button<V> {
    key: SharedString,
    words: SharedString,
    tone: button::Tone,
    act: Act<V>,
}

impl<V: 'static> Button<V> {
    pub fn new(key: impl Into<SharedString>, words: impl Into<SharedString>, tone: button::Tone, act: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static) -> Self {
        Self { key: key.into(), words: words.into(), tone, act: Rc::new(act) }
    }
}

/// What a dialog shows: a title, its body, the body's fields (in Tab order, before the buttons) and its buttons.
pub struct Panel<V> {
    pub key: String,
    pub title: Option<SharedString>,
    pub body: AnyElement,
    pub fields: Vec<FocusHandle>,
    pub buttons: Vec<Button<V>>,
    /// The button that takes the focus as it opens, by key; else the first field, else the first button.
    pub first: Option<SharedString>,
    /// Esc, from anywhere in the panel.
    pub cancel: Act<V>,
}

/// A view's dialog: its buttons' focus handles, kept while it shows; the owner keeps one and renders it while asking.
#[derive(Default)]
pub struct Dialog {
    members: Members,
}

impl Dialog {
    /// The hit key of the button holding the focus.
    pub fn focused(&self, window: &Window, cx: &gpui::App) -> Option<SharedString> {
        self.members.focused(window, cx)
    }

    /// `confirm` as a dialog: its text, then its yes, alt and Cancel under `<key>-yes`, `<key>-alt`, `<key>-cancel`;
    /// the default answer focused as it opens, Esc Cancel (a report's OK).
    pub fn ask<V: 'static, A>(
        &self,
        hits: &HitTargets,
        key: &str,
        confirm: &Confirm<A>,
        on_answer: impl Fn(&mut V, Answer, &mut Window, &mut Context<V>) + Clone + 'static,
        cx: &mut Context<V>,
    ) -> AnyElement {
        let tone = |danger: bool| if danger { button::Tone::Danger } else { button::Tone::Neutral };
        let button = |suffix: &str, words: String, tone: button::Tone, answer: Answer| {
            let on_answer = on_answer.clone();
            Button::new(format!("{key}-{suffix}"), words, tone, move |this, window, cx| on_answer(this, answer, window, cx))
        };
        let mut buttons = vec![button("yes", confirm.yes.clone(), tone(confirm.danger), Answer::Yes)];
        if let Some((words, _)) = &confirm.alt {
            buttons.push(button("alt", words.clone(), tone(confirm.alt_danger), Answer::Alt));
        }
        if confirm.action.is_some() {
            buttons.push(button("cancel", "Cancel".into(), button::Tone::Neutral, Answer::No));
        }
        let first = match confirm.default_answer() {
            Answer::No => "cancel",
            _ => "yes",
        };
        let escape = if confirm.action.is_some() { Answer::No } else { Answer::Yes };
        let text = div().relative().child(SharedString::from(confirm.text.clone())).child(hit_target(hits, format!("{key}-text")));
        let panel = Panel {
            key: key.to_string(),
            title: confirm.title.clone().map(SharedString::from),
            body: text.into_any_element(),
            fields: Vec::new(),
            buttons,
            first: Some(format!("{key}-{first}").into()),
            cancel: Rc::new(move |this, window, cx| on_answer(this, escape, window, cx)),
        };
        self.render(hits, panel, cx)
    }

    /// `panel` centred on a scrim over the whole window ([`over`]'s), at [`tokens::Layer::Dialog`]; Tab and Shift+Tab walk its
    /// fields and buttons, Enter or Space press the focused button; paints `<key>-scrim`, `<key>-dialog`, `<key>-title`.
    pub fn render<V: 'static>(&self, hits: &HitTargets, panel: Panel<V>, cx: &mut Context<V>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        self.members.begin();
        for field in &panel.fields {
            self.members.add(field.clone());
        }
        let buttons: Vec<_> = panel
            .buttons
            .into_iter()
            .map(|b| {
                let focus = self.members.handle(b.key.clone(), cx);
                let act = b.act.clone();
                confirm::answer_button(hits, b.key.clone(), b.words, b.tone, false, cx).acting(&focus, move |this: &mut V, window, cx| act(this, window, cx), cx)
            })
            .collect();
        let first = panel.first.as_ref().and_then(|k| self.members.get(k)).or_else(|| panel.fields.first().cloned()).or_else(|| self.members.order().first());
        let order = self.members.order();
        // Focused as it prepaints, before its members, so the frame that first shows it holds the focus (closed unseen, a focus lost all the same).
        let take_focus = {
            let order = order.clone();
            canvas(
                move |_, window, cx| {
                    if let Some(first) = first.clone().filter(|_| order.focused(window, cx).is_none()) {
                        window.focus(&first, cx);
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .size_0()
        };
        let (key, cancel, escape) = (panel.key, panel.cancel.clone(), panel.cancel);
        let title = panel.title.map(|t| div().relative().text_size(tokens::text::TITLE).font_weight(gpui::FontWeight::SEMIBOLD).child(t).child(hit_target(hits, format!("{key}-title"))));
        let body = crate::scroll::area(SharedString::from(format!("{key}-body"))).flex_1().min_h_0().child(panel.body);
        let card = order
            .on(div().id(SharedString::from(format!("{key}-dialog"))))
            .relative()
            .flex()
            .flex_col()
            .gap(tokens::space::XL)
            .w(WIDTH)
            .max_w_full()
            .max_h_full()
            .p(tokens::space::XXXL)
            .rounded(tokens::radius::L)
            .border(tokens::stroke::HAIRLINE)
            .border_color(colors.border)
            .bg(colors.elevated_surface_background)
            .shadow(vec![shade(tokens::space::S, tokens::space::XL), shade(tokens::space::XXL, px(40.))])
            .text_size(tokens::text::BODY)
            .text_color(colors.text)
            .child(take_focus)
            .on_action(cx.listener(move |this, _: &Dismiss, window, cx| cancel(this, window, cx)))
            .on_key_down(cx.listener(move |this, e: &KeyDownEvent, window, cx| {
                if e.keystroke.key == "escape" {
                    cx.stop_propagation();
                    escape(this, window, cx);
                }
            }))
            .children(title)
            .child(body)
            .child(button::Button::line().justify_end().flex_wrap().pt(tokens::space::L).children(buttons))
            .child(hit_target(hits, format!("{key}-dialog")));
        let shade = if cx.theme().appearance.is_light() { SCRIM_LIGHT } else { SCRIM_DARK };
        deferred(scrim(hits, &key, shade).child(card)).with_priority(tokens::Layer::Dialog.priority()).into_any_element()
    }
}

/// `content` (the window's root element, holding its key context) with `dialog` beside it: a dialog's focus lies
/// outside `content`, so none of its keys, actions or chords reach what lies behind.
pub fn over(content: impl IntoElement, dialog: Option<AnyElement>) -> Div {
    div().relative().size_full().child(content).children(dialog)
}

fn scrim(hits: &HitTargets, key: &str, shade: f32) -> gpui::Stateful<Div> {
    div()
        .id(SharedString::from(format!("{key}-scrim")))
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .p(MARGIN)
        .bg(gpui::black().opacity(shade))
        .occlude()
        .child(hit_target(hits, format!("{key}-scrim")))
}

fn shade(offset: Pixels, blur: Pixels) -> BoxShadow {
    BoxShadow { color: gpui::black().opacity(0.28), offset: point(Pixels::ZERO, offset), blur_radius: blur, spread_radius: Pixels::ZERO, inset: false }
}

#[cfg(test)]
mod tests;
