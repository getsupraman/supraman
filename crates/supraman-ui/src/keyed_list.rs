//! A long list that lays out only the rows in view ([`gpui::list`]), each row known by a key and the tokens its hit
//! keys contain: the row on top stays on top when rows come and go, and [`KeyedList::reveal`] scrolls to the row a
//! hit key belongs to.

use gpui::{ListAlignment, ListOffset, ListState, Pixels, SharedString, px};

const OVERDRAW: Pixels = px(200.);

pub struct KeyedList {
    state: ListState,
    keys: Vec<SharedString>,
    tokens: Vec<Vec<String>>,
}

impl Default for KeyedList {
    fn default() -> Self {
        Self { state: ListState::new(0, ListAlignment::Top, OVERDRAW), keys: Vec::new(), tokens: Vec::new() }
    }
}

impl KeyedList {
    pub fn state(&self) -> ListState {
        self.state.clone()
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// This frame's rows: a key each and the tokens of its hit keys.
    pub fn set_rows(&mut self, rows: Vec<(SharedString, Vec<String>)>) {
        let (keys, tokens): (Vec<_>, Vec<_>) = rows.into_iter().unzip();
        self.tokens = tokens;
        if keys == self.keys {
            return;
        }
        let top = self.state.logical_scroll_top();
        let anchor = self.keys.get(top.item_ix).cloned();
        self.state.reset(keys.len());
        if let Some(item_ix) = anchor.and_then(|k| keys.iter().position(|x| *x == k)) {
            self.state.scroll_to(ListOffset { item_ix, offset_in_item: top.offset_in_item });
        }
        self.keys = keys;
    }

    /// The row whose longest token `hit` contains.
    pub fn row_of(&self, hit: &str) -> Option<usize> {
        self.tokens
            .iter()
            .enumerate()
            .flat_map(|(i, tokens)| tokens.iter().filter(|t| !t.is_empty() && hit.contains(t.as_str())).map(move |t| (t.len(), i)))
            .max_by_key(|(len, i)| (*len, std::cmp::Reverse(*i)))
            .map(|(_, i)| i)
    }

    /// Scrolls the row `hit` belongs to into view; false when no row claims it.
    pub fn reveal(&self, hit: &str) -> bool {
        match self.row_of(hit) {
            Some(ix) => {
                self.state.scroll_to_reveal_item(ix);
                true
            }
            None => false,
        }
    }

    /// Scrolls to the first row whose key `wanted` accepts; false while there is none.
    pub fn reveal_where(&self, wanted: impl Fn(&str) -> bool) -> bool {
        match self.keys.iter().position(|k| wanted(k)) {
            Some(ix) => {
                self.state.scroll_to_reveal_item(ix);
                true
            }
            None => false,
        }
    }

    /// Scrolls to the first row.
    pub fn to_top(&self) {
        self.state.scroll_to(ListOffset::default());
    }

    /// Scrolls one viewport down; false at the end.
    pub fn page_down(&self) -> bool {
        let before = self.state.logical_scroll_top();
        self.state.scroll_by(self.state.viewport_bounds().size.height.max(px(40.)));
        self.state.logical_scroll_top() != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(keys: &[(&str, &[&str])]) -> Vec<(SharedString, Vec<String>)> {
        keys.iter().map(|(k, t)| (SharedString::from(k.to_string()), t.iter().map(|s| s.to_string()).collect())).collect()
    }

    #[test]
    fn a_hit_key_belongs_to_the_row_with_its_longest_token() {
        let mut list = KeyedList::default();
        list.set_rows(rows(&[("ws:a", &["a"]), ("repo:a/x", &["a/x"]), ("s:1", &["5f3e"]), ("more:a/x", &["more-a/x"])]));
        assert_eq!(list.row_of("ws-row-a"), Some(0));
        assert_eq!(list.row_of("repo-row-a/x"), Some(1));
        assert_eq!(list.row_of("star-5f3e@x"), Some(2));
        assert_eq!(list.row_of("more-a/x"), Some(3));
        assert_eq!(list.row_of("notices-zone"), None);
        assert_eq!(list.len(), 4);
    }
}
