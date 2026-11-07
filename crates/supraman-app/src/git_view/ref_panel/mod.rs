//! The Git tab's left panel: the repository's refs in five sections (local and remote branches, worktrees,
//! stashes, tags) on [`supraman_ui::tree`], a fuzzy filter over them, the refs hidden from the graph or soloed in
//! it, and a section's overlay beside the collapsed strip. It reads nothing itself: the tab hands it the refs it
//! loaded and walks the graph as its [`Walk`] says.

mod entries;
mod marks;
mod overlay;
mod render;
mod sections;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use gpui::{Context, Entity, EventEmitter, FocusHandle, Pixels, SharedString, Subscription, Window, px};
use supraman_gitman::Refs;
use supraman_ui::hit::HitTargets;
use supraman_ui::menu::ContextMenu;
use supraman_ui::text_input::{self, TextInput};
use supraman_ui::split::StackState;
use supraman_ui::tree::Tree;

use super::GitSizes;
use super::stash::{StashAct, StashRef};

pub use entries::{Count, Entry, Section, Walk, counts, entries, tips, without};
pub use marks::RefAction;

/// The tree's id: its rows paint under `refpanel-row-<path>`.
pub const LIST: &str = "refpanel";
pub const DEFAULT_WIDTH: Pixels = px(240.);
pub const MIN_WIDTH: Pixels = px(160.);
/// The collapsed panel's strip.
pub const STRIP: Pixels = px(36.);

/// The hide and solo keys of the refs under a folder.
#[derive(Debug, Clone, Default)]
pub struct Keys {
    pub hide: Vec<String>,
    pub solo: Vec<String>,
}

fn fold(keys: &mut Keys, entry: &Entry) {
    keys.hide.extend(entry.hide_key());
    keys.solo.extend(entry.solo_key());
}

/// The panel's rows of a [`Refs`], built off the UI thread ([`build`]).
pub struct Built {
    entries: Vec<(String, Entry)>,
    trees: [Tree<Entry, Keys>; 5],
}

/// The remote whose row `path` is: `remote/<name>`, the folder its branches sit in.
pub fn remote_folder(path: &str) -> Option<&str> {
    path.strip_prefix("remote/").filter(|name| !name.is_empty() && !name.contains('/'))
}

pub fn build(refs: &Refs) -> Built {
    let entries = entries(refs);
    let trees = Section::ALL.map(|section| Tree::build_given(entries.iter().filter(|(path, _)| Section::of(path) == Some(section)).cloned(), fold));
    Built { entries, trees }
}

pub enum RefEvent {
    /// A ref's row was picked: its commit, and whether the graph walks from it.
    Pick(SharedString, bool),
    /// The hidden or the soloed set changed.
    Walk,
    /// The panel was collapsed or expanded, or a folder opened or closed.
    Layout,
    /// A hidden ref was shown by a double-click: its commit, to select once the graph walks it.
    Shown(SharedString),
    /// A stash row's menu chose this.
    Stash(StashAct, StashRef),
    /// A ref's or a worktree's row was right-clicked: the row's path, where.
    Menu(String, gpui::Point<Pixels>),
    /// A ref's or a worktree's row was double-clicked: the row's path.
    Checkout(String),
}

pub struct RefPanel {
    hits: HitTargets,
    entries: Arc<Vec<(String, Entry)>>,
    tree: sections::Sections,
    /// Each section's height while open, in [`Section::ALL`]'s order; zero for an even share.
    heights: [Pixels; 5],
    stack: StackState,
    walk: Walk,
    /// Each section's count, made again when the refs, the walk or the filter change.
    counts: [Count; 5],
    #[cfg(test)]
    pub recounts: usize,
    filter: Option<(Entity<TextInput>, Subscription)>,
    width: Pixels,
    shut: bool,
    focus_filter: bool,
    built: bool,
    /// The folders to close when the tree is first built.
    collapsed: Vec<String>,
    overlay: Option<overlay::Overlay>,
    menu: Option<ContextMenu<RefAction>>,
    /// The tab's own focus, where the keyboard goes when the overlay holding it closes.
    tab: FocusHandle,
}

impl EventEmitter<RefEvent> for RefPanel {}

impl RefPanel {
    pub fn new(hits: HitTargets, tab: FocusHandle, cx: &mut Context<Self>) -> Self {
        RefPanel {
            hits,
            entries: Arc::new(Vec::new()),
            tree: sections::Sections::new(cx),
            heights: [px(0.); 5],
            stack: StackState::default(),
            walk: Walk::default(),
            counts: [Count::default(); 5],
            #[cfg(test)]
            recounts: 0,
            filter: None,
            width: DEFAULT_WIDTH,
            shut: false,
            focus_filter: false,
            built: false,
            collapsed: Vec::new(),
            overlay: None,
            menu: None,
            tab,
        }
    }

    /// The ref panel's part of a repo's layout: its width, collapsed, the hidden and soloed refs and the folders closed.
    pub fn restore(&mut self, sizes: &GitSizes, cx: &mut Context<Self>) {
        self.width = sizes.refs.map_or(DEFAULT_WIDTH, |w| px(w as f32)).max(MIN_WIDTH);
        self.shut = sizes.refs_shut;
        self.walk = Walk { hidden: sizes.hidden.iter().cloned().collect(), solo: sizes.solo.iter().cloned().collect() };
        self.collapsed = sizes.collapsed.clone();
        self.heights = std::array::from_fn(|i| px(sizes.sections.get(i).copied().unwrap_or(0) as f32));
        self.recount();
        if self.built {
            self.close_collapsed();
        }
        cx.notify();
    }

    /// The rows of `refs`, built here: [`RefPanel::set_built`] of a few refs.
    #[cfg(test)]
    pub fn set_refs(&mut self, refs: &Refs, cx: &mut Context<Self>) {
        self.set_built(build(refs), cx);
    }

    /// The rows built; every folder open the first time but the collapsed ones, the open ones kept after.
    pub fn set_built(&mut self, built: Built, cx: &mut Context<Self>) {
        self.tree.set(built.trees);
        if !std::mem::replace(&mut self.built, true) {
            self.tree.expand_all();
            self.close_collapsed();
        }
        self.entries = Arc::new(built.entries);
        self.recount();
        self.refill_overlay(cx);
        cx.notify();
    }

    fn close_collapsed(&mut self) {
        for path in std::mem::take(&mut self.collapsed) {
            if self.tree.is_open(&path) {
                self.tree.toggle(&path);
            }
        }
    }

    /// Each section's height while open, in [`Section::ALL`]'s order; zero for an even share.
    pub fn sections(&self) -> Vec<u32> {
        let heights: Vec<u32> = self.heights.iter().map(|h| f32::from(*h).round().max(0.) as u32).collect();
        if heights.iter().all(|h| *h == 0) { Vec::new() } else { heights }
    }

    /// The folders closed, sections included.
    pub fn collapsed(&self) -> Vec<String> {
        self.tree.closed().iter().map(|p| p.to_string()).collect()
    }

    /// Every ref at its tree path, in the panel's order.
    pub fn entries(&self) -> Arc<Vec<(String, Entry)>> {
        self.entries.clone()
    }

    pub fn walk(&self) -> &Walk {
        &self.walk
    }

    /// Whether every one of `keys` is hidden; never for none.
    pub fn all_hidden(&self, keys: &[String]) -> bool {
        !keys.is_empty() && keys.iter().all(|k| self.walk.hidden.contains(k))
    }

    /// Whether every one of `keys` is soloed; never for none.
    pub fn all_soloed(&self, keys: &[String]) -> bool {
        !keys.is_empty() && keys.iter().all(|k| self.walk.solo.contains(k))
    }

    /// Shows `keys` when all are hidden, else hides them all.
    pub fn toggle_hidden(&mut self, keys: &[String], cx: &mut Context<Self>) {
        let all = self.all_hidden(keys);
        Self::toggle(&mut self.walk.hidden, keys, all);
        self.walk_changed(cx);
    }

    /// Unsolos `keys` when all are soloed, else solos them all.
    pub fn toggle_solo(&mut self, keys: &[String], cx: &mut Context<Self>) {
        let all = self.all_soloed(keys);
        Self::toggle(&mut self.walk.solo, keys, all);
        self.walk_changed(cx);
    }

    fn toggle(set: &mut std::collections::BTreeSet<String>, keys: &[String], all: bool) {
        for key in keys {
            if all {
                set.remove(key);
            } else {
                set.insert(key.clone());
            }
        }
    }

    fn walk_changed(&mut self, cx: &mut Context<Self>) {
        self.recount();
        cx.emit(RefEvent::Walk);
        cx.notify();
    }

    /// Clears the hidden and the soloed refs.
    pub fn show_all(&mut self, cx: &mut Context<Self>) {
        if self.walk != Walk::default() {
            self.walk = Walk::default();
            self.walk_changed(cx);
        }
    }

    pub fn filter(&self) -> String {
        self.tree.filter()
    }

    pub fn set_filter(&mut self, filter: &str, cx: &mut Context<Self>) {
        self.tree.set_filter(filter);
        self.recount();
        cx.notify();
    }

    fn recount(&mut self) {
        self.counts = counts(&self.entries, &self.walk, &self.filter());
        #[cfg(test)]
        {
            self.recounts += 1;
        }
    }

    /// Each section's count as its header shows it, in [`Section::ALL`]'s order.
    pub fn counts(&self) -> [Count; 5] {
        self.counts
    }

    pub fn width(&self) -> Pixels {
        self.width
    }

    pub fn set_width(&mut self, width: Pixels, cx: &mut Context<Self>) {
        self.width = width;
        cx.notify();
    }

    pub fn shut(&self) -> bool {
        self.shut
    }

    pub fn toggle_shut(&mut self, cx: &mut Context<Self>) {
        self.shut = !self.shut;
        self.overlay = None;
        cx.emit(RefEvent::Layout);
        cx.notify();
    }

    /// The panel open with its filter box taking the keyboard on the next frame.
    pub fn focus_filter(&mut self, cx: &mut Context<Self>) {
        self.shut = false;
        self.overlay = None;
        self.focus_filter = true;
        cx.notify();
    }

    pub fn entry(&self, path: &str) -> Option<&Entry> {
        self.entries.iter().find(|(p, _)| p == path).map(|(_, e)| e)
    }

    /// A row picked: its ref's commit to the graph, and whether the graph walks from it; whether it has one.
    pub(super) fn picked(&mut self, path: &str, cx: &mut Context<Self>) -> bool {
        let Some(entry) = self.entry(path) else { return false };
        let Some(sha) = entry.commit() else { return false };
        let walked = entry.tip_of(&self.walk);
        cx.emit(RefEvent::Pick(SharedString::from(sha.to_string()), walked));
        true
    }

    /// The paths of the rows shown, top to bottom.
    #[cfg(test)]
    pub fn visible(&self) -> Vec<SharedString> {
        self.tree.visible()
    }

    fn filter_box(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<TextInput> {
        self.filter.get_or_insert_with(|| text_input::filter_box("Filter refs", Self::set_filter, window, cx)).0.clone()
    }
}
