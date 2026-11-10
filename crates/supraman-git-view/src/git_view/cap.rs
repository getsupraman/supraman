//! The loaded rows' cap: while the view is far above the last loaded row, the rows below a page's end are
//! dropped and that page's layout kept, so scrolling back down pages them in again from there.

use gpui::{AppContext as _, Context, px};
use std::rc::Rc;

use supraman_gitman::{FIRST_PAGE, Layout, NEXT_PAGE};

use super::load::GraphRow;
use super::pages::Pages;
use super::{GitView, graph};

/// The most rows kept loaded once the view is far above the last of them.
pub(crate) const MAX_ROWS: usize = FIRST_PAGE + 40 * NEXT_PAGE;
/// The rows kept loaded below the view's top when the tail is dropped.
pub(crate) const KEEP_BELOW: usize = 10 * NEXT_PAGE;

/// A page's end: the rows and commits loaded with it, and the layout after its last row.
pub(super) struct Mark {
    rows: usize,
    commits: usize,
    layout: Layout,
}

/// The page ends of the rows loaded, in order.
#[derive(Default)]
pub(super) struct Marks(Vec<Mark>);

impl Marks {
    /// The marks of rows read afresh: one at their end.
    pub(crate) fn reset(&mut self, rows: usize, commits: usize, layout: &Layout) {
        self.0.clear();
        self.push(rows, commits, layout);
    }

    pub(crate) fn push(&mut self, rows: usize, commits: usize, layout: &Layout) {
        self.0.push(Mark { rows, commits, layout: layout.clone() });
    }

    /// Rows inserted (`by` > 0) or taken away at row `at`: the page ends below it move with them.
    pub(crate) fn shift(&mut self, at: usize, by: isize) {
        for m in self.0.iter_mut().filter(|m| m.rows > at) {
            m.rows = m.rows.saturating_add_signed(by);
        }
    }

    /// The first page end at or below row `at`, the ends after it dropped.
    fn cut_at(&mut self, at: usize) -> Option<&Mark> {
        let i = self.0.iter().position(|m| m.rows >= at)?;
        self.0.truncate(i + 1);
        self.0.last()
    }
}

impl GitView {
    /// Drops the rows below a page's end while more than [`MAX_ROWS`] are loaded and the view's top is more
    /// than twice [`KEEP_BELOW`] above the last; the selected row stays.
    pub(super) fn trim_tail(&mut self, cx: &mut Context<Self>) {
        let pages = self.pages.read(cx);
        if pages.rows.len() <= MAX_ROWS || pages.paging || self.seek.is_some() || pages.layout.is_none() {
            return;
        }
        let top = (self.table.scroll_top() / px(graph::ROW)).floor().max(0.) as usize;
        let keep = (top + KEEP_BELOW).max(self.table.selected().map_or(0, |s| s + 1));
        let Some((rows, tail)) = self.pages.update(cx, |p, _| p.cut(keep)) else { return };
        self.agents.dropped_from(rows);
        cx.background_spawn(async move { drop(tail) }).detach();
    }
}

impl Pages {
    /// The rows below the page end at or below `keep` dropped, unless fewer than [`KEEP_BELOW`] lie below it: where
    /// they were cut and the rows dropped.
    fn cut(&mut self, keep: usize) -> Option<(usize, Vec<GraphRow>)> {
        if keep + KEEP_BELOW > self.rows.len() {
            return None;
        }
        let mark = self.marks.cut_at(keep).filter(|m| m.rows < self.rows.len())?;
        let (rows, commits, layout) = (mark.rows, mark.commits, mark.layout.clone());
        let tail = Rc::make_mut(&mut self.rows).split_off(rows);
        self.commits = commits;
        self.layout = Some(layout);
        self.more = true;
        Some((rows, tail))
    }
}
