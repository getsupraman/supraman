//! A card: a box of rows on a page (the panel's fill in a hairline at [`radius::M`], its rows [`ruled`] apart), or one
//! [`floating`] over a pane, raised on the elevated surface with a shadow.

use gpui::{App, Div, FontWeight, Pixels, SharedString, Styled, div, prelude::*, px};
use theme::ActiveTheme as _;

use crate::tokens::{self, radius, space, stroke};

/// A card's row's padding above and below its text.
pub const ROW_Y: Pixels = px(5.);

/// An empty card, a column; its rows follow as children.
pub fn card(cx: &App) -> Div {
    let colors = cx.theme().colors();
    div().flex().flex_col().rounded(radius::M).border(stroke::HAIRLINE).border_color(colors.border_variant).bg(colors.panel_background)
}

/// A card's title line, with its count muted after it.
pub fn header(title: impl Into<SharedString>, count: Option<usize>, cx: &App) -> Div {
    let muted = cx.theme().colors().text_muted;
    div()
        .flex()
        .items_center()
        .gap(space::L)
        .px(space::XL)
        .py(space::L)
        .text_size(tokens::text::BODY)
        .font_weight(FontWeight::SEMIBOLD)
        .child(title.into())
        .children(count.map(|n| div().text_size(tokens::text::SMALL).font_weight(FontWeight::NORMAL).text_color(muted).child(SharedString::from(n.to_string()))))
}

/// `row` with the card's rule above it, which parts it from the row or header before.
pub fn ruled<E: Styled>(row: E, cx: &App) -> E {
    row.border_t(stroke::HAIRLINE).border_color(cx.theme().colors().border_variant)
}

/// A card over a pane (the ref panel's section, the palette): the elevated surface in the border's hairline, with a shadow.
pub fn floating(cx: &App) -> Div {
    let colors = cx.theme().colors();
    div().rounded(radius::M).border(stroke::HAIRLINE).border_color(colors.border).bg(colors.elevated_surface_background).shadow_lg()
}
