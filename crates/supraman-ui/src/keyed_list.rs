//! A long list that lays out only the rows in view ([`gpui::list`]), each row known by a key and the tokens its hit
//! keys contain: the row on top stays on top when rows come and go, and [`KeyedList::reveal`] scrolls to the row a
//! hit key belongs to. Its selection is the list core's ([`crate::list::Selection`]), kept by row key.

use gpui::{ListAlignment, ListOffset, ListState, Pixels, SharedString, px};

const OVERDRAW: Pixels = px(200.);

pub struct KeyedList {
    state: ListState,
    keys: Vec<SharedString>,
    tokens: Vec<Vec<String>>,
    acts: Vec<bool>,
    selection: crate::list::Selection,
    selected: Option<SharedString>,
}

impl Default for KeyedList {
    fn default() -> Self {
        Self { state: ListState::new(0, ListAlignment::Top, OVERDRAW), keys: Vec::new(), tokens: Vec::new(), acts: Vec::new(), selection: Default::default(), selected: None }
    }
}

impl KeyedList {
    pub fn state(&self) -> ListState {
        self.state.clone()
    }

    /// The list's bar, laid beside its [`gpui::list`] in a `relative` parent.
    pub fn scrollbar(&self, id: impl Into<gpui::ElementId>) -> crate::scroll::Scrollbar {
        crate::scroll::Scrollbar::new(id, &self.state)
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
        self.selection.select(self.selected.as_ref().and_then(|k| keys.iter().position(|x| x == k)));
        let top = self.state.logical_scroll_top();
        let anchor = self.keys.get(top.item_ix).cloned();
        self.state.reset(keys.len());
        if let Some(item_ix) = anchor.and_then(|k| keys.iter().position(|x| *x == k)) {
            self.state.scroll_to(ListOffset { item_ix, offset_in_item: top.offset_in_item });
        }
        self.keys = keys;
    }

    /// Which rows a key can select, one per row; every row while unset.
    pub fn set_selectable(&mut self, acts: Vec<bool>) {
        self.acts = acts;
    }

    pub fn selected_key(&self) -> Option<&SharedString> {
        self.selected.as_ref()
    }

    pub fn is_selected(&self, key: &str) -> bool {
        self.selected.as_deref() == Some(key)
    }

    /// The row of `key`.
    pub fn index_of(&self, key: &str) -> Option<usize> {
        self.keys.iter().position(|k| k == key)
    }

    /// Selects the row `key` (none with `None`), kept by its key as rows come and go.
    pub fn select_key(&mut self, key: Option<SharedString>) {
        self.selection.select(key.as_ref().and_then(|k| self.keys.iter().position(|x| x == k)));
        self.selected = key;
    }

    /// The rows a page holds, as the last frame showed them.
    pub fn page(&self) -> usize {
        let top = self.state.logical_scroll_top().item_ix;
        let row = self.state.bounds_for_item(top).map_or(Pixels::ZERO, |b| b.size.height);
        crate::list::page_of(self.state.viewport_bounds().size.height, row)
    }

    /// A list key: the selection moved and revealed, the key of the row it moved to; `text` a row's type-ahead text.
    pub fn key(&mut self, key: &crate::list::Key, text: impl Fn(usize) -> String) -> Option<SharedString> {
        let acts: Vec<bool> = (0..self.keys.len()).map(|i| self.acts.get(i).copied().unwrap_or(self.acts.is_empty())).collect();
        let rows = crate::list::Rows::new(self.keys.len()).page(self.page()).acting(&acts);
        let labels = || Some((0..self.keys.len()).map(|i| (text(i), acts[i])).collect());
        let ix = self.selection.key(key, &rows, labels)?;
        self.state.scroll_to_reveal_item(ix);
        self.selected = self.keys.get(ix).cloned();
        self.selected.clone()
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
