//! The words a pane shows while it has nothing else: a title and its muted details, centred.

use gpui::{App, Div, SharedString, div, prelude::*};
use theme::ActiveTheme as _;

/// The caller sizes and places it, and adds anything below the details (a Retry, a hit target).
pub fn placeholder<D: Into<SharedString>>(title: impl Into<SharedString>, details: impl IntoIterator<Item = D>, cx: &App) -> Div {
    let colors = cx.theme().colors();
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_1()
        .child(div().text_color(colors.text).child(title.into()))
        .children(details.into_iter().map(|detail| div().text_xs().text_color(colors.text_muted).child(detail.into())))
}
