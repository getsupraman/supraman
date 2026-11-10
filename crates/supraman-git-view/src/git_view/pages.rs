//! The graph's rows as loaded page by page: the walk the next page reads on, the layout it continues and the page
//! ends the tail is cut at.

use std::rc::Rc;
use std::sync::Arc;

use supraman_gitman::{History, Layout};

use super::cap::Marks;
use super::load::{GraphRow, Page};

pub(crate) struct Next {
    pub(crate) tips: Arc<[String]>,
    pub(crate) skip: usize,
    pub(crate) layout: Layout,
    pub(crate) history: Option<History>,
}

#[derive(Default)]
pub(crate) struct Pages {
    /// Shared with a paint while it runs; written through [`Rc::make_mut`].
    pub(crate) rows: Rc<Vec<GraphRow>>,
    pub(crate) commits: usize,
    pub(crate) tips: Arc<[String]>,
    /// Away while a page is laid out with it.
    pub(crate) layout: Option<Layout>,
    /// The walk's `rev-list`, read on by the next page; away while a page reads it.
    pub(crate) history: Option<History>,
    pub(crate) more: bool,
    pub(crate) paging: bool,
    pub(crate) page_error: Option<String>,
    pub(crate) marks: Marks,
}

impl Pages {
    /// Rows read afresh, the first page and any kept below it.
    pub(crate) fn start(&mut self, rows: Vec<GraphRow>, more: bool, layout: Layout, history: Option<History>, tips: Vec<String>) {
        self.commits = rows.iter().filter(|r| r.commit()).count();
        self.rows = Rc::new(rows);
        self.more = more;
        self.marks.reset(self.rows.len(), self.commits, &layout);
        self.layout = Some(layout);
        self.history = history;
        self.tips = tips.into();
    }

    /// What the next page reads, taken while it reads, unless one is on its way or none is left.
    pub(crate) fn next(&mut self) -> Option<Next> {
        if self.paging || !self.more {
            return None;
        }
        let layout = self.layout.take()?;
        self.paging = true;
        Some(Next { tips: self.tips.clone(), skip: self.commits, layout, history: self.history.take() })
    }

    /// A page in: where its rows start, or `None` when it failed.
    pub(crate) fn landed(&mut self, page: Result<(Page, Option<History>), String>) -> Option<usize> {
        self.paging = false;
        match page {
            Ok((page, walker)) => {
                self.history = walker;
                self.commits += page.rows.iter().filter(|r| r.commit()).count();
                let from = self.rows.len();
                Rc::make_mut(&mut self.rows).extend(page.rows);
                self.more = page.more;
                self.marks.push(self.rows.len(), self.commits, &page.layout);
                self.layout = Some(page.layout);
                Some(from)
            }
            Err(e) => {
                self.more = false;
                self.page_error = Some(e);
                None
            }
        }
    }
}
