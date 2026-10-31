//! The notice box: text in a status colour on a tint of that colour, never the theme's status
//! `*_background` fill.

use gpui::{App, Div, Hsla, IntoElement, div, prelude::*};
use theme::ActiveTheme as _;

pub const TINT: f32 = 0.12;

pub fn tint(color: Hsla) -> Hsla {
    color.opacity(TINT)
}

/// `color` text on its [`tint`]; the caller places it and adds the body.
pub fn tinted(color: Hsla) -> Div {
    filled(color, tint(color))
}

/// The text colour on the element fill: a notice that is not a problem.
pub fn plain(cx: &App) -> Div {
    let colors = cx.theme().colors();
    filled(colors.text, colors.element_background)
}

fn filled(fg: Hsla, bg: Hsla) -> Div {
    div().p_2().rounded_md().bg(bg).text_sm().text_color(fg)
}

/// `body` beside a dismiss button at the top right.
pub fn dismissable(body: impl IntoElement, dismiss: impl IntoElement) -> Div {
    div().flex().items_start().gap_2().child(div().flex_1().min_w_0().child(body)).child(dismiss)
}
