//! How the tab gives way in a narrow slice, by its own painted width: the navigator shows its strip, then the
//! detail goes under the run list. The kept widths stay as the user left them.

use gpui::{AnyElement, Context, Div, Pixels, Size, prelude::*};
use supraman_ui::measure::Width;
use supraman_ui::split::{Axis, SplitPane};
use supraman_ui::tokens::stroke::HAIRLINE;

use super::WorkView;
use super::layout::{STACKED_MIN, detail_split, nav_split};
use super::navigator::SHUT_WIDTH;

const STACKED_SHARE: f32 = 0.5;

/// The navigator's strip beside the run list at its least width, over both stacked halves.
pub fn min_size() -> Size<Pixels> {
    Size { width: SHUT_WIDTH + HAIRLINE + detail_split(&Default::default()).min_first, height: STACKED_MIN * 2. + HAIRLINE }
}

/// Below this the navigator shows its strip: its least width beside the list and the detail at theirs.
fn nav_fit() -> Pixels {
    let nav = nav_split(&Default::default());
    nav.min_first + HAIRLINE + nav.min_second
}

/// Below this the detail goes under the list: the strip beside both at their least.
fn stack_below() -> Pixels {
    SHUT_WIDTH + HAIRLINE + nav_split(&Default::default()).min_second
}

pub(crate) struct Narrow {
    width: Width,
    /// The list's share of the height while stacked; not kept in the layout.
    share: f32,
}

impl Default for Narrow {
    fn default() -> Self {
        Narrow { width: Width::default(), share: STACKED_SHARE }
    }
}

impl WorkView {
    pub fn min_size(&self) -> Size<Pixels> {
        min_size()
    }

    pub(crate) fn width_probe(&self, cx: &Context<Self>) -> impl IntoElement {
        self.narrow.width.probe(cx.entity_id())
    }

    /// Too narrow for the navigator, which then shows its strip whatever the user's choice.
    pub(crate) fn nav_narrow(&self) -> bool {
        self.narrow.width.under(nav_fit())
    }

    pub(crate) fn nav_shown_shut(&self) -> bool {
        self.layout.nav_shut() || self.nav_narrow()
    }

    /// The list beside the detail at its kept width, or over it by a share while stacked.
    pub(crate) fn list_and_detail(&mut self, list: Div, detail: Div, cx: &mut Context<Self>) -> AnyElement {
        let split = detail_split(&self.hits);
        if self.narrow.width.under(stack_below()) {
            let split = SplitPane { min_first: STACKED_MIN, min_second: STACKED_MIN, axis: Axis::Rows, ..split };
            return split
                .render_fraction(list.size_full(), detail.w_full().flex_1(), self.narrow.share, STACKED_SHARE, |this: &mut WorkView, share, cx| {
                    this.narrow.share = share;
                    cx.notify();
                }, |_: &mut WorkView, _| {}, cx)
                .size_full()
                .into_any_element();
        }
        let basis = split.min_first;
        split
            .render_second(list.flex_grow(1.).flex_basis(basis).h_full(), detail.w(self.layout.detail()).h_full(), |this: &mut WorkView, width, cx| {
                this.layout.set_detail(width);
                cx.notify();
            }, |this: &mut WorkView, cx| this.layout.keep(cx), cx)
            .size_full()
            .into_any_element()
    }
}

#[cfg(test)]
mod tests;
