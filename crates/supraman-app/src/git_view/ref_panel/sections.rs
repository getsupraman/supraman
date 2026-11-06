//! The panel's five trees, one per [`Section`], each under its section's folder with that folder painted as the
//! pane's header rather than as a row; addressed by the whole path, as one tree would be.

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
    pub fn new(cx: &mut App) -> Self {
        let trees = std::array::from_fn(|_| {
            let tree = TreeState::new(cx);
            tree.match_with(|entry: &Entry, filter| entry.matches(filter));
            tree.hide_roots();
            tree
        });
        Sections { trees }
    }

    pub fn get(&self, section: Section) -> &TreeState<Entry, Keys> {
        &self.trees[at(section.key())]
    }

    pub fn set(&self, trees: [Tree<Entry, Keys>; 5]) {
        for (state, tree) in self.trees.iter().zip(trees) {
            state.set_tree(tree);
        }
    }

    /// The folder at `path`'s aggregate, a section's own included.
    pub fn aggregate(&self, path: &str) -> Option<Keys> {
        self.trees[at(path)].aggregate(path)
    }

    pub fn is_open(&self, path: &str) -> bool {
        self.trees[at(path)].is_open(path)
    }

    pub fn toggle(&self, path: &str) {
        self.trees[at(path)].toggle(path);
    }

    pub fn expand_all(&self) {
        self.trees.iter().for_each(TreeState::expand_all);
    }

    pub fn closed(&self) -> Vec<SharedString> {
        self.trees.iter().flat_map(TreeState::closed).collect()
    }

    pub fn filter(&self) -> String {
        self.trees[0].filter()
    }

    pub fn set_filter(&self, filter: &str) {
        self.trees.iter().for_each(|t| t.set_filter(filter));
    }

    /// Whether `section` shows: it holds a ref the filter keeps.
    pub fn shows(&self, section: Section) -> bool {
        self.get(section).any_kept()
    }

    /// The rows shown top to bottom, each shown section's header first.
    pub fn visible(&self) -> Vec<SharedString> {
        let mut out = Vec::new();
        for section in Section::ALL.into_iter().filter(|s| self.shows(*s)) {
            out.push(SharedString::from(section.key()));
            out.extend(self.get(section).visible());
        }
        out
    }
}
