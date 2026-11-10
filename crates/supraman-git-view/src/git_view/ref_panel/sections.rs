//! The panel's five trees, one per [`Section`], its folder painted as the pane's header; addressed by the whole path.

use gpui::{App, SharedString};
use supraman_ui::tree::{Tree, TreeState};

use super::{Entry, Keys, Section};

pub(super) struct Sections {
    trees: [TreeState<Entry, Keys>; 5],
}

/// The section a whole path lies in.
fn at(path: &str) -> usize {
    Section::of(path).and_then(|s| Section::ALL.iter().position(|a| *a == s)).unwrap_or(0)
}

impl Sections {
    pub(crate) fn new(cx: &mut App) -> Self {
        let trees = std::array::from_fn(|_| {
            let tree = TreeState::new(cx);
            tree.match_with(|entry: &Entry, filter| entry.matches(filter));
            tree.hide_roots();
            tree
        });
        Sections { trees }
    }

    pub(crate) fn get(&self, section: Section) -> &TreeState<Entry, Keys> {
        &self.trees[at(section.key())]
    }

    pub(crate) fn set(&self, trees: [Tree<Entry, Keys>; 5]) {
        for (state, tree) in self.trees.iter().zip(trees) {
            state.set_tree(tree);
        }
    }

    /// The folder at `path`'s aggregate, a section's own included.
    pub(crate) fn aggregate(&self, path: &str) -> Option<Keys> {
        self.trees[at(path)].aggregate(path)
    }

    pub(crate) fn is_open(&self, path: &str) -> bool {
        self.trees[at(path)].is_open(path)
    }

    pub(crate) fn toggle(&self, path: &str) {
        self.trees[at(path)].toggle(path);
    }

    pub(crate) fn expand_all(&self) {
        self.trees.iter().for_each(TreeState::expand_all);
    }

    pub(crate) fn closed(&self) -> Vec<SharedString> {
        self.trees.iter().flat_map(TreeState::closed).collect()
    }

    pub(crate) fn filter(&self) -> String {
        self.trees[0].filter()
    }

    pub(crate) fn set_filter(&self, filter: &str) {
        self.trees.iter().for_each(|t| t.set_filter(filter));
    }

    /// Whether `section` shows: it holds a ref the filter keeps.
    pub(crate) fn shows(&self, section: Section) -> bool {
        self.get(section).any_kept()
    }

    /// The rows shown top to bottom, each shown section's header first.
    #[cfg(test)]
    pub(crate) fn visible(&self) -> Vec<SharedString> {
        let mut out = Vec::new();
        for section in Section::ALL.into_iter().filter(|s| self.shows(*s)) {
            out.push(SharedString::from(section.key()));
            out.extend(self.get(section).visible());
        }
        out
    }
}
