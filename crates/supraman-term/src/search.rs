// Vendored from Zed crates/terminal/src/terminal.rs @ 03efd49220ba06703d04f9c4732fdf814f9c523c
// (GPL-3.0-or-later).

use crate::alacritty::{AlacrittySearch, search_matches};
use crate::events::InternalEvent;
use crate::mouse::Selection;
use crate::{Range, Terminal};
use gpui::AppContext as _;
use gpui::{Context, Task};

#[derive(Clone, Debug)]
pub struct Search {
    pub(crate) search: AlacrittySearch,
}

impl Terminal {
    pub fn activate_match(&mut self, index: usize) {
        if let Some(search_match) = self.matches.get(index).cloned() {
            self.set_selection(Some(Selection::simple_range(search_match)));
            self.events
                .push_back(InternalEvent::ScrollToPoint(search_match.start()));
        }
    }

    pub fn find_matches(&self, searcher: Search, cx: &Context<Self>) -> Task<Vec<Range>> {
        let term = self.term.clone();
        cx.background_spawn(async move {
            let term = term.lock();
            search_matches(&term, searcher)
        })
    }
}
