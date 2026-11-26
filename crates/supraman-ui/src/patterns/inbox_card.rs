//! Every Inbox card: its colour's tint and badge, a head line, the title, its lines, the owner's main remedy as a word and its other actions as hover icons.

use std::rc::Rc;

use gpui::{AnyElement, App, ClickEvent, Div, FontWeight, Hsla, SharedString, Stateful, Svg, Window, div, prelude::*, px};
use supraman_core::time;
use theme::ActiveTheme as _;

use crate::controls::button::{self, Button};
use crate::clipped::{clamped, clipped, clipped_with};
use crate::hit::{HitTargets, hit_target};
use crate::controls::tooltip::Tooltip;
use crate::label::{Label, LabelMark};
use crate::tokens;

pub const TINT: f32 = 0.06;
pub const TINT_HOVER: f32 = 0.14;
const BADGE_TINT: f32 = 0.16;
pub const ICON: f32 = 14.;
const BUTTON: f32 = 20.;

/// `at` as `time::short` under `key`, its whole date as the tooltip; the Inbox's and the Overview's one time.
pub fn when(key: impl Into<SharedString>, at: i64, now: i64, cx: &App) -> Option<Stateful<Div>> {
    let shown = time::short(at, now)?;
    Some(
        div()
            .id(key.into())
            .flex_none()
            .text_color(crate::themes::secondary(cx.theme().colors()))
            .child(SharedString::from(shown))
            .tooltip(Tooltip::text(time::date_words(at))),
    )
}

/// A card's hover icon under `key`, shown while the card `card_key` is hovered; the owner adds its click.
pub fn action(hits: &HitTargets, key: impl Into<SharedString>, card_key: impl Into<SharedString>, icon: Svg, tip: impl Into<SharedString>, cx: &App) -> Stateful<Div> {
    Button::new(hits, key).icon(icon).icon_only(px(BUTTON)).reveal(card_key, false).tooltip(Tooltip::text(tip)).build(cx)
}

/// A card's main remedy under `key`: its word, shown without hover; the owner adds its click.
pub fn primary(hits: &HitTargets, key: impl Into<SharedString>, label: impl Into<SharedString>, cx: &App) -> Stateful<Div> {
    Button::new(hits, key).label(label).fill(button::Fill::Outline).build(cx)
}

pub fn icon_color(cx: &App) -> Hsla {
    cx.theme().colors().text_muted
}

type OnClick = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// One card under `key` (its id, hover group and hit key); its parts paint under `part(name)`, `<key>-<name>` unless the owner names them.
pub struct InboxCard {
    key: String,
    color: Hsla,
    badge: SharedString,
    title: SharedString,
    part: Rc<dyn Fn(&str) -> String>,
    mark: Option<AnyElement>,
    context: Option<SharedString>,
    context_tip: Option<SharedString>,
    when: Option<(i64, i64)>,
    lines: Vec<(String, SharedString, bool)>,
    actions: Vec<Stateful<Div>>,
    primary: Option<Stateful<Div>>,
    on_click: Option<OnClick>,
}

impl InboxCard {
    /// `color` is the owner's colour of the card's kind; `badge` is its word.
    pub fn new(key: impl Into<String>, color: Hsla, badge: impl Into<SharedString>, title: impl Into<SharedString>) -> Self {
        let key = key.into();
        let prefix = key.clone();
        Self {
            key,
            color,
            badge: badge.into(),
            title: title.into(),
            part: Rc::new(move |part| format!("{prefix}-{part}")),
            mark: None,
            context: None,
            context_tip: None,
            when: None,
            lines: Vec::new(),
            actions: Vec::new(),
            primary: None,
            on_click: None,
        }
    }

    pub fn parts(mut self, part: impl Fn(&str) -> String + 'static) -> Self {
        self.part = Rc::new(part);
        self
    }

    pub fn mark(mut self, mark: impl Fn(Hsla) -> AnyElement) -> Self {
        self.mark = Some(mark(self.color));
        self
    }

    /// The muted words before the time: the project or folder the card is of.
    pub fn context(mut self, text: impl Into<SharedString>) -> Self {
        self.context = Some(text.into()).filter(|t| !t.is_empty());
        self
    }

    /// The context's tooltip in place of its own whole text (a project's title under its ref).
    pub fn context_tip(mut self, tip: Option<impl Into<SharedString>>) -> Self {
        self.context_tip = tip.map(Into::into).filter(|t| !t.is_empty());
        self
    }

    pub fn when(mut self, at: Option<i64>, now: i64) -> Self {
        self.when = at.map(|at| (at, now));
        self
    }

    /// A one-line text under the title, painted under `part(part)`; `muted` for a reference, else the text colour.
    pub fn line(mut self, part: &str, text: impl Into<SharedString>, muted: bool) -> Self {
        let text = text.into();
        if !text.is_empty() {
            self.lines.push((part.to_string(), text, muted));
        }
        self
    }

    /// A button made by [`action`] with this card's key, in order at the head's right.
    pub fn action(mut self, button: Stateful<Div>) -> Self {
        self.actions.push(button);
        self
    }

    /// A button made by [`primary`], at the head's right after the hover icons.
    pub fn primary(mut self, button: Stateful<Div>) -> Self {
        self.primary = Some(button);
        self
    }

    pub fn on_click(mut self, on: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(on));
        self
    }

    pub fn render(self, hits: &HitTargets, cx: &App) -> AnyElement {
        let colors = cx.theme().colors();
        let color = self.color;
        let muted = crate::themes::secondary(colors);
        let part = self.part.clone();
        let context = self.context.map(|text| {
            let context = match self.context_tip {
                Some(tip) => clipped_with(part("context"), text, false, false).tooltip(Tooltip::text(tip)),
                None => clipped(part("context"), text, false),
            };
            context.flex_shrink(1.).text_color(muted)
        });
        let lead = match context {
            Some(context) => Label::element(context),
            None => Label::none(),
        };
        let lead = lead.mark_if(self.mark.map(LabelMark::Element)).gap(tokens::space::L).flex_shrink(1.);
        let head = div()
            .flex()
            .items_center()
            .gap(tokens::space::L)
            .text_size(tokens::text::SMALL)
            .child(lead)
            .children(self.when.and_then(|(at, now)| when(part("when"), at, now, cx)))
            .child(div().flex_none().px(tokens::space::S).rounded(tokens::radius::S).bg(color.opacity(BADGE_TINT)).text_color(color).font_weight(FontWeight::BOLD).child(self.badge))
            .child(div().flex_1())
            .child(div().flex().flex_none().items_center().gap(tokens::space::S).children(self.actions).children(self.primary))
            .child(hit_target(hits, part("head")));
        let title = clamped(part("title"), self.title, 2).text_size(tokens::text::BODY).text_color(colors.text).child(hit_target(hits, part("title")));
        let lines = self.lines.into_iter().map(|(name, text, quiet)| {
            clipped(part(&name), text, false).text_size(tokens::text::SMALL).text_color(if quiet { muted } else { colors.text }).child(hit_target(hits, part(&name)))
        });
        let key = SharedString::from(self.key);
        let on_click = self.on_click;
        div()
            .id(key.clone())
            .group(key.clone())
            .relative()
            .flex()
            .flex_col()
            .gap(tokens::space::XS)
            .p(tokens::space::L)
            .rounded(tokens::radius::M)
            .border_l(tokens::stroke::FOCUS)
            .border_color(color)
            .bg(color.opacity(TINT))
            .hover(|s| s.bg(color.opacity(TINT_HOVER)))
            .when(on_click.is_some(), |d| d.cursor_pointer())
            .child(head)
            .child(title)
            .children(lines)
            .child(hit_target(hits, key.to_string()))
            .when_some(on_click, |d, on| d.on_click(move |e, window, cx| on(e, window, cx)))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests;
