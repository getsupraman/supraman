use gpui::{App, Div, Hsla, SharedString, Stateful, div, prelude::*};
use theme::ActiveTheme as _;

use crate::controls::button::base;
use crate::hit::{HitTargets, hit_target};
use crate::label::Label;
use crate::tokens::{self, radius, space, stroke};

/// A quiet outlined pill in `fg`, lit on hover; its label's letters centred, painted under `<key>-label`.
pub fn pill(hits: &HitTargets, key: impl Into<SharedString>, label: impl Into<SharedString>, fg: Hsla, cx: &App) -> Stateful<Div> {
    let colors = cx.theme().colors();
    let key = key.into();
    let label = Label::element(div().relative().child(label.into()).child(hit_target(hits, format!("{key}-label")))).centred();
    base(hits, key, |s| s.bg(colors.element_hover).border_color(colors.border).text_color(colors.text), colors.element_active)
        .flex()
        .flex_none()
        .items_center()
        .h(tokens::control::SMALL.height)
        .px(space::L)
        .rounded(radius::S)
        .border(stroke::HAIRLINE)
        .border_color(colors.border_variant)
        .text_size(tokens::text::SMALL)
        .text_color(fg)
        .child(label)
}

/// A still tag: a word in a hairline box at the small text's size, muted, with no hover or click (the diff's side, a terminal's link).
pub fn tag(cx: &App) -> Div {
    let colors = cx.theme().colors();
    div()
        .flex_none()
        .px(space::M)
        .rounded(radius::S)
        .border(stroke::HAIRLINE)
        .border_color(colors.border)
        .text_size(tokens::text::SMALL)
        .text_color(colors.text_muted)
}
