//! How the tab gives way in a narrow slice, by its own painted width: the ref panel shows its strip, then the
//! commit panel goes under the graph. The kept widths stay as the user left them.

use gpui::{AnyElement, Context, Div, Pixels, Size, prelude::*};
use supraman_ui::measure::Width;
use supraman_ui::split::{Axis, SplitPane};
use supraman_ui::tokens::stroke::HAIRLINE;

use super::{GitView, PANEL_WIDTH, STACKED_MIN, TOOLBAR, TOOLBAR_FULL, graph_split, ref_panel};

/// The graph's share while stacked: the commit panel under it needs its details, message and files' toolbar.
const STACKED_SHARE: f32 = 0.4;

/// The ref panel's strip beside a graph at its least width, over the toolbar and both stacked halves.
pub fn min_size() -> Size<Pixels> {
    let beside = graph_split(&Default::default());
    Size { width: ref_panel::STRIP + HAIRLINE + beside.min_first, height: TOOLBAR + STACKED_MIN * 2. + HAIRLINE }
}

/// Below this the ref panel shows its strip: its least width beside the graph and the commit panel at theirs.
fn refs_fit() -> Pixels {
    let beside = graph_split(&Default::default());
    ref_panel::MIN_WIDTH + HAIRLINE + beside.min_first + HAIRLINE + beside.min_second
}

/// Below this the commit panel goes under the graph: the strip beside a graph at its least and the panel at its default width.
fn stack_below() -> Pixels {
    ref_panel::STRIP + HAIRLINE + graph_split(&Default::default()).min_first + HAIRLINE + PANEL_WIDTH
}

pub(crate) struct Narrow {
    width: Width,
    /// The graph's share of the height while stacked; not kept in the layout.
    share: f32,
}

impl Default for Narrow {
    fn default() -> Self {
        Narrow { width: Width::default(), share: STACKED_SHARE }
    }
}

impl GitView {
    pub fn min_size(&self) -> Size<Pixels> {
        min_size()
    }

    pub(crate) fn width_probe(&self, cx: &Context<Self>) -> impl IntoElement {
        self.narrow.width.probe(cx.entity_id())
    }

    pub(crate) fn refs_fit(&self) -> bool {
        !self.narrow.width.under(refs_fit())
    }

    /// Stash, Pop, Pull and Push show their icons alone, their words in their tooltips.
    pub(crate) fn compact_toolbar(&self) -> bool {
        self.narrow.width.under(TOOLBAR_FULL)
    }

    pub(crate) fn stacked(&self) -> bool {
        self.narrow.width.under(stack_below())
    }

    /// The graph beside the commit panel at its kept width, or over it by a share while stacked.
    pub(crate) fn graph_and_panel(&mut self, table: Div, panel: Div, cx: &mut Context<Self>) -> AnyElement {
        let split = graph_split(&self.hits);
        if self.stacked() {
            let split = SplitPane { min_first: STACKED_MIN, min_second: STACKED_MIN, axis: Axis::Rows, ..split };
            return split
                .render_fraction(table.size_full(), panel.w_full().flex_1(), self.narrow.share, STACKED_SHARE, |this: &mut GitView, share, cx| {
                    this.narrow.share = share;
                    cx.notify();
                }, |_: &mut GitView, _| {}, cx)
                .size_full()
                .into_any_element();
        }
        split
            .render_second(table.flex_1().h_full(), panel.w(self.panel_width).h_full(), |this: &mut GitView, width, cx| {
                this.panel_width = width;
                cx.notify();
            }, |this: &mut GitView, cx| this.keep_layout(cx), cx)
            .size_full()
            .into_any_element()
    }
}

#[cfg(test)]
mod tests;
