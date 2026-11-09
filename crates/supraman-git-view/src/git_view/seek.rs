//! Selecting a commit whose row may not be loaded yet: pages load until one holds it, at most [`SEEK_PAGES`],
//! and a toast over the graph says when it is not there.

use gpui::{Context, SharedString};
use supraman_gitman::short_sha;

use super::GitView;

/// The most pages a seek loads before it stops: 50 × `NEXT_PAGE` commits below the loaded rows.
pub(crate) const SEEK_PAGES: usize = 50;

/// A commit picked before its page is loaded.
pub(super) struct Seek {
    pub(crate) sha: SharedString,
    /// The pages it may still load.
    pub(crate) pages: usize,
}

impl GitView {
    /// Selects the row of `sha`, loading up to [`SEEK_PAGES`] pages until one holds it.
    pub(crate) fn seek_sha(&mut self, sha: SharedString, cx: &mut Context<Self>) {
        self.seek_within(sha, SEEK_PAGES, cx);
    }

    pub(super) fn seek_within(&mut self, sha: SharedString, pages: usize, cx: &mut Context<Self>) {
        self.seek = None;
        if self.rows.iter().any(|r| r.sha == sha) {
            return self.select_sha(&sha, cx);
        }
        if !self.more || pages == 0 {
            let beyond = self.more;
            return self.not_found(&sha, beyond, cx);
        }
        self.seek = Some(Seek { sha, pages });
        self.load_more(cx);
    }

    /// The page in has been applied: the seek goes on with one page less.
    pub(super) fn seek_on(&mut self, cx: &mut Context<Self>) {
        if let Some(seek) = self.seek.take() {
            if self.page_error.is_some() {
                return;
            }
            self.seek_within(seek.sha, seek.pages.saturating_sub(1), cx);
        }
    }

    /// A ref picked in the ref panel: a ref out of the walk only selects a row already loaded.
    pub(super) fn picked(&mut self, sha: SharedString, walked: bool, cx: &mut Context<Self>) {
        self.close_diff(cx);
        if walked || self.rows.iter().any(|r| r.sha == sha) {
            return self.seek_sha(sha, cx);
        }
        self.seek = None;
        self.not_found(&sha, false, cx);
    }

    /// Says `sha` has no row: `beyond` the pages a seek loads, else not in the graph at all.
    fn not_found(&mut self, sha: &str, beyond: bool, cx: &mut Context<Self>) {
        let text = if beyond {
            format!("{} is more than {SEEK_PAGES} pages down: scroll down to load it.", short_sha(sha))
        } else {
            format!("{} is not in the graph.", short_sha(sha))
        };
        self.toast.show_why(text, |v: &mut GitView| &mut v.toast, cx);
    }
}
