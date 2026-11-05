//! The commit panel right of the graph: the selected commit's message, author, parents and files, read
//! off the UI thread on each selection, read-only (C146).

pub mod files;
mod load;
mod render;

#[cfg(test)]
mod tests;

use std::collections::HashSet;
use std::rc::Rc;

use gpui::{App, AppContext as _, Context, Entity, EventEmitter, Pixels, ScrollHandle, SharedString, Subscription, Window, px};
use supraman_gitman::{FileChange, Person, Repo};
use supraman_ui::hit::HitTargets;
use supraman_ui::stall;
use supraman_ui::table::TableState;
use supraman_ui::text_input::{InputEvent, TextInput};
use supraman_ui::tree::{Order, TreeState};

pub use files::{kind_word, mark_icon};
pub use load::{Counts, Mark};
pub(crate) use render::{PANEL_STEP, placeholder};

use super::Selected;

pub const MESSAGE_HEIGHT: Pixels = px(140.);

/// What the panel asks of its owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanelEvent {
    /// Select this parent in the graph.
    Parent(SharedString),
    /// A file was clicked: its change in the shown commit, against the first parent.
    File(super::diff::Target),
}

/// The commit shown, its texts ready to paint.
#[derive(Debug, Clone)]
pub struct Shown {
    pub sha: SharedString,
    pub summary: SharedString,
    pub body: SharedString,
    pub author: SharedString,
    pub author_email: SharedString,
    /// Epoch milliseconds.
    pub authored_ms: i64,
    pub parents: Vec<SharedString>,
    pub co_authors: Vec<Person>,
    pub counts: Counts,
    /// The changed files in path order, Z–A when the order is.
    pub changes: Vec<FileChange>,
    /// Every file at the commit and every changed one.
    files: HashSet<String>,
}

pub enum State {
    Empty,
    Loading,
    Failed(String),
    Shown(Rc<Shown>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileView {
    Path,
    Tree,
}

pub struct CommitPanel {
    hits: HitTargets,
    repo: Option<Repo>,
    /// Bumped by each selection; a load that finishes under another one is dropped.
    token: u64,
    state: State,
    /// The selected row's lane colour, for the author's avatar.
    colour: usize,
    view: FileView,
    all_files: bool,
    order: Order,
    changed: TreeState<Mark, Counts>,
    all: TreeState<Mark, Counts>,
    /// Whether Expand All, rather than Collapse All, last ran on the changed tree and on the whole tree.
    changed_expanded: bool,
    all_expanded: bool,
    paths: TableState,
    filter: Option<(Entity<TextInput>, Subscription)>,
    message_height: Pixels,
    message_scroll: ScrollHandle,
}

impl EventEmitter<PanelEvent> for CommitPanel {}

impl CommitPanel {
    pub fn new(hits: HitTargets, message_height: Option<Pixels>, cx: &mut App) -> Self {
        CommitPanel {
            hits,
            repo: None,
            token: 0,
            state: State::Empty,
            colour: 0,
            view: FileView::Tree,
            all_files: false,
            order: Order::Ascending,
            changed: TreeState::new(cx),
            all: TreeState::new(cx),
            changed_expanded: true,
            all_expanded: false,
            paths: TableState::new(cx),
            filter: None,
            message_height: message_height.unwrap_or(MESSAGE_HEIGHT),
            message_scroll: ScrollHandle::new(),
        }
    }

    pub fn set_repo(&mut self, repo: Repo) {
        self.repo = Some(repo);
    }

    pub fn message_height(&self) -> Pixels {
        self.message_height
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    /// Shows `selected`, loading a commit off the UI thread; the token of this selection.
    pub fn show(&mut self, selected: Option<Selected>, colour: usize, cx: &mut Context<Self>) -> u64 {
        self.token += 1;
        let token = self.token;
        self.colour = colour;
        match (selected, self.repo.clone()) {
            (None, _) => self.state = State::Empty,
            (Some(Selected::Wip), _) => self.state = State::Empty,
            (Some(Selected::Commit(_)), None) => self.state = State::Loading,
            (Some(Selected::Commit(sha)), Some(repo)) => {
                if !matches!(self.state, State::Shown(_)) {
                    self.state = State::Loading;
                }
                let order = self.order;
                let loaded = cx.background_spawn(async move { load::load(&repo, &sha, order) });
                cx.spawn(async move |this, cx| {
                    let loaded = loaded.await;
                    this.update(cx, |this, cx| this.apply(token, loaded, cx)).ok();
                })
                .detach();
            }
        }
        cx.notify();
        token
    }

    /// `loaded` for the selection of `token`; dropped when the selection moved on since.
    pub fn apply(&mut self, token: u64, loaded: Result<load::Loaded, String>, cx: &mut Context<Self>) {
        if token != self.token {
            return;
        }
        let _stall = stall::section("commit panel: apply");
        let loaded = match loaded {
            Ok(loaded) => loaded,
            Err(why) => {
                self.state = State::Failed(why);
                cx.notify();
                return;
            }
        };
        self.changed.set_tree(loaded.changed);
        if self.changed_expanded {
            self.changed.expand_all();
        }
        self.all.set_tree(loaded.all);
        self.changed.select(None);
        self.all.select(None);
        self.paths.select(None);
        let d = loaded.details;
        let mut changes = loaded.changes;
        sort_changes(&mut changes, self.order);
        self.state = State::Shown(Rc::new(Shown {
            sha: loaded.sha.into(),
            summary: d.summary.into(),
            body: d.body.into(),
            author: d.commit.author.name.into(),
            author_email: d.commit.author.email.into(),
            authored_ms: d.commit.author.time * 1000,
            parents: d.commit.parents.into_iter().map(SharedString::from).collect(),
            co_authors: d.co_authors,
            counts: loaded.counts,
            changes,
            files: loaded.files,
        }));
        cx.notify();
    }

    fn shown(&self) -> Option<Rc<Shown>> {
        match &self.state {
            State::Shown(s) => Some(s.clone()),
            _ => None,
        }
    }

    pub fn set_order(&mut self, order: Order, cx: &mut Context<Self>) {
        self.order = order;
        self.changed.set_order(order);
        self.all.set_order(order);
        if let Some(shown) = self.shown() {
            let mut shown = (*shown).clone();
            sort_changes(&mut shown.changes, order);
            self.state = State::Shown(Rc::new(shown));
        }
        cx.notify();
    }

    pub fn set_view(&mut self, view: FileView, cx: &mut Context<Self>) {
        self.view = view;
        cx.notify();
    }

    pub fn set_all_files(&mut self, on: bool, cx: &mut Context<Self>) {
        self.all_files = on;
        cx.notify();
    }

    /// The whole tree's filter, as the Filter files box sets it.
    pub fn set_filter(&mut self, filter: &str, cx: &mut Context<Self>) {
        self.all.set_filter(filter);
        cx.notify();
    }

    /// The tree on show: the whole tree with View all files, else the changed files'.
    fn tree(&self) -> (&TreeState<Mark, Counts>, bool) {
        if self.all_files { (&self.all, self.all_expanded) } else { (&self.changed, self.changed_expanded) }
    }

    fn expand_or_collapse(&mut self, cx: &mut Context<Self>) {
        let (tree, expanded) = self.tree();
        let tree = tree.clone();
        if expanded { tree.collapse_all() } else { tree.expand_all() }
        if self.all_files { self.all_expanded = !expanded } else { self.changed_expanded = !expanded }
        cx.notify();
    }

    fn filter_box(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<TextInput> {
        if let Some((input, _)) = &self.filter {
            return input.clone();
        }
        let input = cx.new(|cx| TextInput::new("Filter files", None, window, cx));
        let sub = cx.subscribe(&input, |this, input, e: &InputEvent, cx| {
            if *e == InputEvent::Edited {
                let text = input.read(cx).text().to_string();
                this.set_filter(&text, cx);
            }
        });
        self.filter = Some((input.clone(), sub));
        input
    }

    #[cfg(test)]
    pub fn token(&self) -> u64 {
        self.token
    }

    /// The selected path of the tree on show.
    #[cfg(test)]
    pub fn selected_file(&self) -> Option<SharedString> {
        self.tree().0.selected()
    }

    pub fn deselect_files(&mut self, cx: &mut Context<Self>) {
        self.changed.select(None);
        self.all.select(None);
        self.paths.select(None);
        cx.notify();
    }

    /// The diff of `path` in the shown commit: a changed file against the first parent, else the file as it is.
    fn target(shown: &Shown, path: &str) -> Option<super::diff::Target> {
        if !shown.files.contains(path) {
            return None;
        }
        let change = shown.changes.iter().find(|c| c.path == path).cloned();
        let parent = shown.parents.first().map(|p| p.to_string());
        Some(super::diff::Target::commit(shown.sha.to_string(), parent, path.to_string(), change))
    }

    fn file_clicked(&mut self, path: &SharedString, cx: &mut Context<Self>) {
        if let Some(target) = self.shown().and_then(|shown| Self::target(&shown, path)) {
            cx.emit(PanelEvent::File(target));
        }
    }

    /// The files of the list on show, top to bottom, every folder of a tree open; none until it shows `sha`.
    pub fn file_order(&self, sha: &str) -> Vec<super::diff::Target> {
        let Some(shown) = self.shown().filter(|s| s.sha == sha) else { return Vec::new() };
        let paths: Vec<String> = match (self.all_files, self.view) {
            (false, FileView::Path) => shown.changes.iter().map(|c| c.path.clone()).collect(),
            _ => self.tree().0.files().iter().map(|p| p.to_string()).collect(),
        };
        paths.iter().filter_map(|p| Self::target(&shown, p)).collect()
    }

    /// Selects `target`'s row in the list on show, opening the folders above it.
    pub fn mark_file(&mut self, target: &super::diff::Target, cx: &mut Context<Self>) {
        let Some(shown) = self.shown() else { return };
        match (self.all_files, self.view) {
            (false, FileView::Path) => {
                let row = shown.changes.iter().position(|c| c.path == target.path);
                self.paths.select(row);
                if let Some(row) = row {
                    self.paths.reveal(row);
                }
            }
            _ => self.tree().0.select_opening(&target.path),
        }
        cx.notify();
    }
}

/// By path ignoring case, then as written; reversed for Z–A.
pub(crate) fn sort_changes(changes: &mut [FileChange], order: Order) {
    changes.sort_by(|a, b| {
        let by = a.path.to_lowercase().cmp(&b.path.to_lowercase()).then_with(|| a.path.cmp(&b.path));
        if order == Order::Descending { by.reverse() } else { by }
    });
}
