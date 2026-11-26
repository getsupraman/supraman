//! A count beside a list's heading or row, a pill of its own.

use gpui::{App, Div, Hsla, SharedString, div, prelude::*, px};
use theme::ActiveTheme as _;

use crate::tokens;

/// A result group's count badge.
pub fn count(n: usize, cx: &App) -> Div {
    let colors = cx.theme().colors();
    badge(n, colors.element_background, colors.text)
}

/// A count of what waits for the user (the Work navigator's Needs you): [`count`] on the error colour.
pub fn alert_count(n: usize, cx: &App) -> Div {
    let error = cx.theme().status().error;
    badge(n, error, crate::themes::on(error))
}

fn badge(n: usize, bg: Hsla, fg: Hsla) -> Div {
    div()
        .flex_none()
        .min_w(px(18.))
        .px(tokens::space::S)
        .rounded(tokens::radius::FULL)
        .bg(bg)
        .text_size(tokens::text::SMALL)
        .text_center()
        .text_color(fg)
        .child(SharedString::from(n.to_string()))
}
