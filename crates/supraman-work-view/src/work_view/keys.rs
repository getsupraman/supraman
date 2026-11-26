//! The Work tab's two lists on the list core: the run's rows (a move selects and shows the row) and the navigator
//! (a move selects an entry, Enter opens it).

use supraman_ui::list::{self, Key, Rows, Selection, Step};

use super::projects::{active, closed_runs};
use super::*;

/// What a navigator entry opens.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum NavAct {
    Page(Page),
    Inbox,
    Run(String),
    Closed(String),
}

/// A navigator entry: its hit key, its type-ahead text and what Enter or a click opens.
pub(crate) struct NavEntry {
    pub key: String,
    pub text: String,
    pub act: NavAct,
}

impl WorkView {
    /// The navigator's entries in the order it paints them.
    pub(crate) fn nav_entries(&self) -> Vec<NavEntry> {
        let entry = |key: String, text: &str, act| NavEntry { key, text: text.to_string(), act };
        let mut out = vec![
            entry("work-nav-overview".into(), "Overview", NavAct::Page(Page::Overview)),
            entry("work-nav-inbox".into(), "Needs you", NavAct::Inbox),
            entry("work-nav-activity".into(), "Activity", NavAct::Page(Page::Activity)),
        ];
        for project in &self.projects {
            let qref = &project.item.qualified_ref;
            out.extend(active(project).map(|r| entry(format!("work-nav-run-{}", r.item.qualified_ref), &r.item.title, NavAct::Run(r.item.id.clone()))));
            let closed: Vec<_> = closed_runs(project).collect();
            if closed.is_empty() {
                continue;
            }
            out.push(entry(format!("work-closed-{qref}"), "closed", NavAct::Closed(project.item.id.clone())));
            if self.closed_open.contains(&project.item.id) {
                out.extend(closed.into_iter().map(|r| entry(format!("work-nav-run-{}", r.item.qualified_ref), &r.item.title, NavAct::Run(r.item.id.clone()))));
            }
        }
        out
    }

    /// The navigator entry shown now: the page's, or the run's.
    fn nav_on(&self) -> Option<String> {
        match &self.page {
            Page::Overview => Some("work-nav-overview".into()),
            Page::Activity => Some("work-nav-activity".into()),
            Page::Run(_) => self.nav_entries().into_iter().find(|e| matches!(&e.act, NavAct::Run(id) if Some(id.as_str()) == self.shown_run())).map(|e| e.key),
        }
    }

    /// The entry the navigator's selection is on: the one the keys moved to, else the one shown.
    pub(crate) fn nav_selected(&self) -> Option<String> {
        self.nav_cursor.clone().or_else(|| self.nav_on())
    }

    pub(crate) fn nav_act(&mut self, act: NavAct, cx: &mut Context<Self>) {
        match act {
            NavAct::Page(page) => self.show_page(page, cx),
            NavAct::Inbox => cx.emit(WorkEvent::ShowInbox),
            NavAct::Run(id) => self.open_run(id, cx),
            NavAct::Closed(project) => self.toggle_closed(&project, cx),
        }
    }

    /// A list key in the navigator.
    pub(super) fn nav_key(&mut self, key: &Key, cx: &mut Context<Self>) -> bool {
        let entries = self.nav_entries();
        let at = self.nav_selected().and_then(|k| entries.iter().position(|e| e.key == k));
        if *key == Key::Activate {
            if let Some(e) = at.and_then(|i| entries.into_iter().nth(i)) {
                self.nav_cursor = None;
                self.nav_act(e.act, cx);
            }
            return true;
        }
        let mut selection = Selection::default();
        selection.select(at);
        let rows = Rows::new(entries.len()).page(list::page_of(self.nav_scroll.bounds().size.height, ROW));
        if let Some(to) = selection.key(key, &rows, || Some(entries.iter().map(|e| (e.text.clone(), true)).collect())) {
            self.nav_cursor = Some(entries[to].key.clone());
            cx.notify();
        }
        true
    }

    /// A list key in the run's rows: the row it moves to is selected and shown; Up from none the last.
    pub(super) fn list_key(&mut self, key: &Key, cx: &mut Context<Self>) -> bool {
        let (ids, texts): (Vec<String>, Vec<(String, bool)>) = self.listed().iter().map(|r| (r.item.id.clone(), (r.item.title.clone(), true))).unzip();
        if ids.is_empty() || *key == Key::Activate {
            return false;
        }
        let at = self.selected.as_ref().and_then(|s| ids.iter().position(|id| id == s));
        let key = match key {
            Key::Step(Step::By(by)) if at.is_none() && *by < 0 => Key::Step(Step::Last),
            key => key.clone(),
        };
        let mut selection = Selection::default();
        selection.select(at);
        let rows = Rows::new(ids.len()).page(list::page_of(self.list_scroll.bounds().size.height, ROW));
        if let Some(to) = selection.key(&key, &rows, || Some(texts)) {
            self.select(ids[to].clone(), cx);
        }
        true
    }
}

/// About a row's height in either list: what a page counts by.
const ROW: gpui::Pixels = supraman_ui::tokens::control::LARGE.height;
