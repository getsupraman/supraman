//! The one modal prompt: a scrim over its region (the whole window, or a tab's content) that takes every click and
//! key, and a panel centred on it; a question's ([`Confirm`]) or a form's.

use std::cell::{Cell, OnceCell, RefCell};
use std::rc::Rc;

use gpui::{AnyElement, BoxShadow, Context, Div, FocusHandle, KeyDownEvent, Pixels, SharedString, Window, canvas, deferred, div, point, prelude::*, px};
use theme::ActiveTheme as _;

use crate::controls::button;
use crate::surfaces::confirm::{self, Answer, Confirm};
use crate::focus::{self, Pane};
use crate::focus_order::{Dismiss, Members};
use crate::hit::{HitTargets, hit_target};
use crate::icon::Icon;
use crate::tokens;

/// The panel's width, or the region's less [`MARGIN`] each side when narrower.
pub const WIDTH: Pixels = px(520.);
/// The least room between the panel and the region's edges.
pub const MARGIN: Pixels = tokens::space::PAGE;
/// The scrim's black on a dark theme: enough to set the panel apart on a near-black ground.
pub const SCRIM_DARK: f32 = 0.6;
/// The scrim's black on a light theme: [`SCRIM_DARK`] there turns the window a muddy grey.
pub const SCRIM_LIGHT: f32 = 0.3;
/// A tab's prompt's pane: F6's way into it while it shows.
pub const PANE: &str = "prompt";

type Act<V> = Rc<dyn Fn(&mut V, &mut Window, &mut Context<V>)>;

/// One of the panel's buttons, under its hit key.
pub struct Button<V> {
    key: SharedString,
    words: SharedString,
    tone: button::Tone,
    why: Option<SharedString>,
    running: bool,
    focus: Option<FocusHandle>,
    act: Act<V>,
}

impl<V: 'static> Button<V> {
    pub fn new(key: impl Into<SharedString>, words: impl Into<SharedString>, tone: button::Tone, act: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static) -> Self {
        Self { key: key.into(), words: words.into(), tone, why: None, running: false, focus: None, act: Rc::new(act) }
    }

    /// Dimmed with `why` as its tooltip: a click does nothing, Enter or Space still run it (the owner says why not).
    pub fn dimmed(self, why: Option<impl Into<SharedString>>) -> Self {
        Self { why: why.map(Into::into), ..self }
    }

    /// Its icon turning while the owner's write of it runs.
    pub fn turning(self, running: bool) -> Self {
        Self { running, ..self }
    }

    /// Under the owner's focus handle (one its own Tab order names), else one the dialog keeps.
    pub fn focus(self, focus: Option<&FocusHandle>) -> Self {
        Self { focus: focus.cloned(), ..self }
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
    /// Enter's button, by key: ringed while no other button holds the focus.
    pub default: Option<SharedString>,
    /// The title line's × under this hit key, outside the Tab order: a form's close.
    pub close: Option<SharedString>,
    /// Esc, from anywhere in the panel.
    pub cancel: Act<V>,
}

/// A view's prompt: its buttons' focus handles, kept while it shows; the owner keeps one and renders it while asking.
#[derive(Default)]
pub struct Dialog {
    members: Members,
    region: OnceCell<FocusHandle>,
    /// A tab's: the element tracking this handle, which it lies over; `None`, the window's.
    tab: Option<FocusHandle>,
    /// What the panel last showed: another takes the focus afresh.
    shown: RefCell<String>,
    fresh: Rc<Cell<bool>>,
}

impl Dialog {
    /// A tab's prompt, laid with [`over`] beside the element tracking `tab`: it covers only that element, takes the
    /// keys as it opens and whenever they would land in what it covers, and is F6's [`PANE`] there; the rest of the
    /// window keeps its keys and clicks.
    pub fn in_tab(tab: FocusHandle) -> Dialog {
        Dialog { tab: Some(tab), ..Dialog::default() }
    }

    /// The hit key of the button holding the focus.
    pub fn focused(&self, window: &Window, cx: &gpui::App) -> Option<SharedString> {
        self.members.focused(window, cx)
    }

    /// Whether anything in its panel holds the focus.
    pub fn holds_focus(&self, window: &Window, cx: &gpui::App) -> bool {
        self.region.get().is_some_and(|r| r.contains_focused(window, cx))
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
        let first = SharedString::from(match confirm.default_answer() {
            Answer::No => format!("{key}-cancel"),
            _ => format!("{key}-yes"),
        });
        let escape = if confirm.action.is_some() { Answer::No } else { Answer::Yes };
        let text = div().relative().child(SharedString::from(confirm.text.clone())).child(hit_target(hits, format!("{key}-text")));
        let panel = Panel {
            key: key.to_string(),
            title: confirm.title.clone().map(SharedString::from),
            body: text.into_any_element(),
            fields: Vec::new(),
            buttons,
            first: Some(first.clone()),
            default: Some(first),
            close: None,
            cancel: Rc::new(move |this, window, cx| on_answer(this, escape, window, cx)),
        };
        self.render(hits, panel, cx)
    }

    /// `panel` centred on a scrim over its region ([`over`]'s), at [`tokens::Layer::Dialog`] (a tab's at
    /// [`tokens::Layer::Prompt`]); Tab and Shift+Tab walk its fields and buttons, Enter or Space press the focused
    /// button; paints `<key>-scrim`, `<key>-dialog`, `<key>-title`.
    pub fn render<V: 'static>(&self, hits: &HitTargets, panel: Panel<V>, cx: &mut Context<V>) -> AnyElement {
        self.members.begin();
        for field in &panel.fields {
            self.members.add(field.clone());
        }
        self.note_shown(&panel);
        let title = title_line(hits, &panel, cx);
        let handles: Vec<FocusHandle> = panel.buttons.iter().map(|b| self.members.handle_or(b.key.clone(), b.focus.as_ref(), cx)).collect();
        let others = Rc::new(handles.clone());
        let default = panel.default.clone();
        let buttons: Vec<_> = panel.buttons.into_iter().zip(handles).map(|(b, focus)| answer(hits, b, &focus, default.as_ref(), &others, cx)).collect();
        let first = panel.first.as_ref().and_then(|k| self.members.get(k)).or_else(|| panel.fields.first().cloned()).or_else(|| self.members.order().first());
        let order = self.members.order();
        let region = self.region.get_or_init(|| cx.focus_handle()).clone();
        let take_focus = self.take_focus(&region, first.clone());
        let (key, cancel, escape) = (panel.key, panel.cancel.clone(), panel.cancel);
        let body = crate::scroll::area(SharedString::from(format!("{key}-body"))).flex_1().min_h_0().child(panel.body);
        let marker = focus::popup(std::iter::once(region.clone()).chain(order.handles().to_vec()));
        let pane = self.tab.as_ref().map(|tab| Pane::new(PANE, &region).entry(first.as_ref().unwrap_or(&region)).covering(tab).paint(hits));
        let colors = cx.theme().colors().clone();
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
            .track_focus(&region)
            .child(marker)
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
            .child(hit_target(hits, format!("{key}-dialog")))
            .children(pane);
        let shade = if cx.theme().appearance.is_light() { SCRIM_LIGHT } else { SCRIM_DARK };
        let layer = if self.tab.is_some() { tokens::Layer::Prompt } else { tokens::Layer::Dialog };
        deferred(scrim(hits, &key, shade).child(card)).with_priority(layer.priority()).into_any_element()
    }

    /// Another panel than the last one shown takes the focus afresh as it paints.
    fn note_shown<V>(&self, panel: &Panel<V>) {
        let shown = std::iter::once(panel.key.as_str()).chain(panel.title.as_deref()).chain(panel.buttons.iter().map(|b| b.words.as_ref())).collect::<Vec<_>>().join("\n");
        if *self.shown.borrow() != shown {
            *self.shown.borrow_mut() = shown;
            self.fresh.set(true);
        }
    }

    /// Focused as it prepaints, before its members, so the frame that first shows it holds the focus (closed unseen, a
    /// focus lost all the same): `first` as it opens; then a window's whenever it lost the focus, a tab's whenever the
    /// focus lies in what it covers.
    fn take_focus(&self, region: &FocusHandle, first: Option<FocusHandle>) -> impl IntoElement {
        let (region, fresh, tab) = (region.clone(), self.fresh.clone(), self.tab.clone());
        canvas(
            move |_, window, cx| {
                let lost = match &tab {
                    Some(tab) => tab.contains_focused(window, cx),
                    None => !region.contains_focused(window, cx),
                };
                if let Some(first) = first.clone().filter(|_| fresh.take() || lost) {
                    window.focus(&first, cx);
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_0()
    }
}

/// One answer: a [`confirm::answer`] in the panel's Tab order; the default one Solid and ringed while no other holds the focus, the rest Outline.
fn answer<V: 'static>(hits: &HitTargets, b: Button<V>, focus: &FocusHandle, default: Option<&SharedString>, others: &Rc<Vec<FocusHandle>>, cx: &mut Context<V>) -> gpui::Stateful<Div> {
    let act = b.act.clone();
    let fill = if default == Some(&b.key) { button::Fill::Solid } else { button::Fill::Outline };
    let ring = (default == Some(&b.key)).then(|| {
        let (mine, others) = (focus.clone(), others.clone());
        focus::ring_while(tokens::radius::S, move |window, _| mine.is_focused(window) || !others.iter().any(|o| o.is_focused(window)))
    });
    confirm::answer(hits, b.key, b.words, b.tone)
        .fill(fill)
        .when(b.running, |button| button.icon(Icon::Refresh).turning(true))
        .dimmed(b.why)
        .acting(focus, move |this: &mut V, window, cx| act(this, window, cx), cx)
        .children(ring)
        .build(cx)
}

/// The title, and a form's × after it.
fn title_line<V: 'static>(hits: &HitTargets, panel: &Panel<V>, cx: &mut Context<V>) -> Option<Div> {
    let key = &panel.key;
    let title = panel.title.clone().map(|t| div().relative().flex_1().min_w_0().text_size(tokens::text::TITLE).font_weight(gpui::FontWeight::SEMIBOLD).child(t).child(hit_target(hits, format!("{key}-title"))));
    let close = panel.close.clone().map(|close| {
        let cancel = panel.cancel.clone();
        let muted = crate::themes::secondary(cx.theme().colors());
        button::Button::new(hits, close).icon(Icon::Close.svg(tokens::icon::M, muted)).icon_only(tokens::control::MEDIUM.height).tooltip(crate::controls::tooltip::Tooltip::text("Close (Esc)")).on_click(cx.listener(move |this, _: &gpui::ClickEvent, window, cx| cancel(this, window, cx)))
    });
    (title.is_some() || close.is_some()).then(|| div().flex().items_center().gap(tokens::space::L).children(title).children(close.map(|c| div().ml_auto().child(c))))
}

/// `content` (the window's root element, or a tab's, holding its key context) with `dialog` beside it: a dialog's
/// focus lies outside `content`, so none of its keys, actions or chords reach what lies behind.
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
