//! The Git tab's left panel: the repository's refs in five sections (local and remote branches, worktrees,
//! stashes, tags) on [`supraman_ui::tree`], a fuzzy filter over them, and the set of refs hidden from the graph.
//! It reads nothing itself: the tab hands it the refs it loaded and walks the graph without the hidden ones.

mod entries;
mod render;

#[cfg(test)]
mod tests;

use std::collections::BTreeSet;
use std::rc::Rc;

use gpui::{AppContext as _, Context, Entity, EventEmitter, Pixels, SharedString, Subscription, Window, px};
use supraman_gitman::Refs;
use supraman_ui::hit::HitTargets;
use supraman_ui::text_input::TextInput;
use supraman_ui::tree::TreeState;

use super::GitSizes;

pub use entries::{Count, Entry, Section, counts, entries, without};

/// The tree's id: its rows paint under `refpanel-row-<path>`.
pub const LIST: &str = "refpanel";
pub const DEFAULT_WIDTH: Pixels = px(240.);
pub const MIN_WIDTH: Pixels = px(160.);
/// The collapsed panel's strip.
pub const STRIP: Pixels = px(36.);

/// The hide keys of the refs under a folder.
#[derive(Debug, Clone, Default)]
pub struct Keys(pub Vec<String>);

fn fold(keys: &mut Keys, entry: &Entry) {
    keys.0.extend(entry.hide_key());
}

pub enum RefEvent {
    /// A ref's row was picked: its commit.
    Pick(SharedString),
    /// The hidden set changed.
    Hidden,
    /// The panel was collapsed or expanded, or a folder opened or closed.
    Layout,
}

pub struct RefPanel {
    hits: HitTargets,
    entries: Rc<Vec<(String, Entry)>>,
    tree: TreeState<Entry, Keys>,
    hidden: BTreeSet<String>,
    filter: Option<(Entity<TextInput>, Subscription)>,
    width: Pixels,
    shut: bool,
    focus_filter: bool,
    built: bool,
    /// The folders to close when the tree is first built.
    collapsed: Vec<String>,
}

impl EventEmitter<RefEvent> for RefPanel {}

impl RefPanel {
    pub fn new(hits: HitTargets, cx: &mut Context<Self>) -> Self {
        let tree = TreeState::new(cx);
        tree.match_with(|entry: &Entry, filter| entry.matches(filter));
        RefPanel {
            hits,
            entries: Rc::new(Vec::new()),
            tree,
            hidden: BTreeSet::new(),
            filter: None,
            width: DEFAULT_WIDTH,
            shut: false,
            focus_filter: false,
            built: false,
            collapsed: Vec::new(),
        }
    }

    /// The ref panel's part of a repo's layout: its width, collapsed, the hidden refs and the folders closed.
    pub fn restore(&mut self, sizes: &GitSizes, cx: &mut Context<Self>) {
        self.width = sizes.refs.map_or(DEFAULT_WIDTH, |w| px(w as f32)).max(MIN_WIDTH);
        self.shut = sizes.refs_shut;
        self.hidden = sizes.hidden.iter().cloned().collect();
        self.collapsed = sizes.collapsed.clone();
        if self.built {
            self.close_collapsed();
        }
        cx.notify();
    }

    /// The rows of `refs`; every folder open the first time but the collapsed ones, the open ones kept after.
    pub fn set_refs(&mut self, refs: &Refs, cx: &mut Context<Self>) {
        let entries = entries(refs);
        self.tree.set_entries_given(entries.iter().cloned(), fold);
        if !std::mem::replace(&mut self.built, true) {
            self.tree.expand_all();
            self.close_collapsed();
        }
        self.entries = Rc::new(entries);
        cx.notify();
    }

    fn close_collapsed(&mut self) {
        for path in std::mem::take(&mut self.collapsed) {
            if self.tree.is_open(&path) {
                self.tree.toggle(&path);
            }
        }
    }

    /// The folders closed, sections included.
    pub fn collapsed(&self) -> Vec<String> {
        self.tree.closed().iter().map(|p| p.to_string()).collect()
    }

    pub fn hidden(&self) -> &BTreeSet<String> {
        &self.hidden
    }

    /// Whether every one of `keys` is hidden; never for none.
    pub fn all_hidden(&self, keys: &[String]) -> bool {
        !keys.is_empty() && keys.iter().all(|k| self.hidden.contains(k))
    }

    /// Shows `keys` when all are hidden, else hides them all.
    pub fn toggle_hidden(&mut self, keys: &[String], cx: &mut Context<Self>) {
        if keys.is_empty() {
            return;
        }
        if self.all_hidden(keys) {
            keys.iter().for_each(|k| {
                self.hidden.remove(k);
            });
        } else {
            self.hidden.extend(keys.iter().cloned());
        }
        cx.emit(RefEvent::Hidden);
        cx.notify();
    }

    pub fn show_all(&mut self, cx: &mut Context<Self>) {
        if !self.hidden.is_empty() {
            self.hidden.clear();
            cx.emit(RefEvent::Hidden);
            cx.notify();
        }
    }

    pub fn filter(&self) -> String {
        self.tree.filter()
    }

    pub fn set_filter(&mut self, filter: &str, cx: &mut Context<Self>) {
        self.tree.set_filter(filter);
        cx.notify();
    }

    /// Each section's count as its header shows it, in [`Section::ALL`]'s order.
    pub fn counts(&self) -> [Count; 5] {
        counts(&self.entries, &self.hidden, &self.filter())
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
        cx.emit(RefEvent::Layout);
        cx.notify();
    }

    /// The panel open on `section`: its header open, selected and in view.
    pub fn open_section(&mut self, section: Section, cx: &mut Context<Self>) {
        self.shut = false;
        if !self.tree.is_open(section.key()) {
            self.tree.toggle(section.key());
        }
        self.tree.select(Some(section.key()));
        cx.emit(RefEvent::Layout);
        cx.notify();
    }

    /// The panel open with its filter box taking the keyboard on the next frame.
    pub fn focus_filter(&mut self, cx: &mut Context<Self>) {
        self.shut = false;
        self.focus_filter = true;
        cx.notify();
    }

    fn entry(&self, path: &str) -> Option<&Entry> {
        self.entries.iter().find(|(p, _)| p == path).map(|(_, e)| e)
    }

    /// A row picked: its ref's commit to the graph.
    fn picked(&mut self, path: &SharedString, cx: &mut Context<Self>) {
        if let Some(sha) = self.entry(path).and_then(Entry::commit) {
            cx.emit(RefEvent::Pick(SharedString::from(sha.to_string())));
        }
    }

    /// The paths of the rows shown, top to bottom.
    pub fn visible(&self) -> Vec<SharedString> {
        self.tree.visible()
    }

    fn filter_box(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<TextInput> {
        if let Some((input, _)) = &self.filter {
            return input.clone();
        }
        let input = cx.new(|cx| TextInput::new("Filter refs", None, window, cx));
        let sub = cx.subscribe(&input, |this, input, e: &supraman_ui::text_input::InputEvent, cx| {
            if *e == supraman_ui::text_input::InputEvent::Edited {
                let text = input.read(cx).text().to_string();
                this.set_filter(&text, cx);
            }
        });
        self.filter = Some((input.clone(), sub));
        input
    }
}
