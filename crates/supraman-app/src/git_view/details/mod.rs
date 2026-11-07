//! The commit panel right of the graph: the selected commit's or stash's message, author, parents and files, read
//! off the UI thread on each selection; a commit's or a stash's message is edited in place ([`reword`]).

pub mod files;
mod load;
mod render;
pub mod reword;

#[cfg(test)]
pub(crate) mod tests;
#[cfg(test)]
mod reword_tests;

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{AppContext as _, Context, Entity, EventEmitter, Pixels, ScrollHandle, SharedString, Subscription, Window, px};
use supraman_gitman::{Amendable, FileChange, Person, Repo, Stash};
use supraman_ui::hit::HitTargets;
use supraman_ui::stall;
use supraman_ui::table::TableState;
use supraman_ui::text_input::{self, TextInput};
use supraman_ui::tree::{Order, TreeState};

pub use files::{kind_word, mark_icon};
pub use load::{Counts, Mark};
pub(crate) use render::{PANEL_STEP, placeholder};

use super::Selected;
use super::stash::StashRef;
use super::write_gate::{WriteGate, Written};

pub const MESSAGE_HEIGHT: Pixels = px(140.);

/// What the panel asks of its owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanelEvent {
    /// Select this parent in the graph.
    Parent(SharedString),
    File(super::diff::Target),
    /// The message box's height was dragged or reset.
    Resized,
    /// A write of the panel's ran: the toast it says, if any.
    Wrote(Option<String>),
    /// Why a write did not run or failed.
    Why(String),
    /// Stash these WIP files, asked first.
    StashFiles(Vec<FileChange>),
    /// Put these files of the stash (its commit) in the work tree.
    ApplyFiles(String, Vec<FileChange>),
    /// Amend HEAD's message to this, with the commit area's Skip hooks.
    Amend(String),
    /// Reword the open field's older commit to this message, asked first when the tab has work in progress.
    Reword(String),
    /// A write rewrote HEAD: select the new HEAD, with the toast.
    HeadMoved(SharedString, Option<String>),
    /// Give this stash this message.
    StashMessage(StashRef, String),
    /// A file or folder of a list right-clicked.
    FileMenu(super::commit::files::FileClick),
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
    pub amendable: Amendable,
    /// The stash shown, its parents only its base.
    pub stash: Option<Arc<Stash>>,
    pub counts: Counts,
    /// The changed files in path order, Z–A when the order is.
    pub changes: Vec<FileChange>,
    /// Each change's place in `changes` by its path.
    index: HashMap<String, usize>,
    /// Every changed file, and once the whole tree is read every file at the commit.
    files: Rc<HashSet<String>>,
}

/// The whole tree's read for the shown commit, made only with View all files on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Whole {
    Unread,
    /// Being read for the selection of this token.
    Reading(u64),
    Read,
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
    /// The shown commit's counts and files belong to an earlier selection while the next one loads: they keep their place, unpainted.
    stale: bool,
    /// The selected row's lane colour, for the author's avatar.
    colour: usize,
    view: FileView,
    all_files: bool,
    whole: Whole,
    /// Expand All (`true`) or Collapse All pressed while the whole tree is read, for when it is in.
    whole_expand: Option<bool>,
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
    gate: Entity<WriteGate>,
    reword: reword::Reword,
    _written: Subscription,
}

impl EventEmitter<PanelEvent> for CommitPanel {}

impl CommitPanel {
    pub fn new(hits: HitTargets, gate: Entity<WriteGate>, message_height: Option<Pixels>, cx: &mut Context<Self>) -> Self {
        let written = cx.subscribe(&gate, |this: &mut CommitPanel, _, w: &Written, cx| this.written(w, cx));
        cx.observe(&gate, |_, _, cx| cx.notify()).detach();
        CommitPanel {
            hits,
            repo: None,
            token: 0,
            state: State::Empty,
            stale: false,
            colour: 0,
            view: FileView::Tree,
            all_files: false,
            whole: Whole::Unread,
            whole_expand: None,
            order: Order::Ascending,
            changed: TreeState::new(cx),
            all: TreeState::new(cx),
            changed_expanded: true,
            all_expanded: false,
            paths: TableState::new(cx),
            filter: None,
            message_height: message_height.unwrap_or(MESSAGE_HEIGHT),
            message_scroll: ScrollHandle::new(),
            gate,
            reword: reword::Reword::default(),
            _written: written,
        }
    }

    pub fn set_repo(&mut self, repo: Repo, cx: &mut Context<Self>) {
        self.open_writer(repo.clone(), cx);
        self.repo = Some(repo);
    }

    pub fn message_height(&self) -> Pixels {
        self.message_height
    }

    pub fn set_message_height(&mut self, height: Option<Pixels>, cx: &mut Context<Self>) {
        self.message_height = height.unwrap_or(MESSAGE_HEIGHT);
        cx.notify();
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    /// Shows `selected`, loading a commit off the UI thread; the token of this selection.
    pub fn show(&mut self, selected: Option<Selected>, colour: usize, cx: &mut Context<Self>) -> u64 {
        self.token += 1;
        let token = self.token;
        self.colour = colour;
        self.stale = false;
        if self.reword.field.is_some() {
            self.cancel_amend(cx);
        }
        if !matches!(&selected, Some(Selected::Commit(sha)) if self.reword.edit_next.as_deref() == Some(sha.as_ref())) {
            self.reword.edit_next = None;
        }
        match (selected, self.repo.clone()) {
            (None, _) => self.state = State::Empty,
            (Some(Selected::Wip), _) => self.state = State::Empty,
            (Some(Selected::Commit(_) | Selected::Stash(_)), None) => self.state = State::Loading,
            (Some(selected @ (Selected::Commit(_) | Selected::Stash(_))), Some(repo)) => {
                if matches!(self.state, State::Shown(_)) {
                    self.stale = true;
                } else {
                    self.state = State::Loading;
                }
                let (order, all_files) = (self.order, self.all_files);
                let loaded = cx.background_spawn(async move {
                    match selected {
                        Selected::Stash(stash) => load::load_stash(&repo, stash, order, all_files),
                        Selected::Commit(sha) => load::load_with(&repo, &sha, order, all_files),
                        Selected::Wip => unreachable!(),
                    }
                });
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
        self.stale = false;
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
        let files = match loaded.all {
            Some(whole) => {
                self.all.set_tree(whole.tree);
                self.whole = Whole::Read;
                whole.files
            }
            None => {
                self.all.set_tree(load::FileTree::build(std::iter::empty(), Counts::add, self.order));
                self.whole = Whole::Unread;
                loaded.files
            }
        };
        self.whole_expand = None;
        self.changed.select(None);
        self.all.select(None);
        self.paths.select(None);
        let d = loaded.details;
        let mut changes = loaded.changes;
        sort_changes(&mut changes, self.order);
        self.state = State::Shown(Rc::new(Shown {
            sha: loaded.sha.into(),
            summary: loaded.stash.as_ref().map_or(d.summary, |s| s.label()).into(),
            body: if loaded.stash.is_some() { SharedString::default() } else { d.body.into() },
            author: d.commit.author.name.into(),
            author_email: d.commit.author.email.into(),
            authored_ms: loaded.stash.as_ref().map_or(d.commit.author.time, |s| s.time) * 1000,
            parents: d.commit.parents.into_iter().take(if loaded.stash.is_some() { 1 } else { usize::MAX }).map(SharedString::from).collect(),
            co_authors: d.co_authors,
            amendable: loaded.amendable,
            stash: loaded.stash,
            counts: loaded.counts,
            index: index(&changes),
            changes,
            files: Rc::new(files),
        }));
        self.read_whole(cx);
        cx.notify();
    }

    /// Reads the shown commit's whole tree off the UI thread when View all files is on and it is not read yet.
    fn read_whole(&mut self, cx: &mut Context<Self>) {
        let (Some(shown), Some(repo)) = (self.shown(), self.repo.clone()) else { return };
        if !self.all_files || self.stale || self.whole != Whole::Unread {
            return;
        }
        let token = self.token;
        self.whole = Whole::Reading(token);
        let order = self.order;
        let (sha, changes) = (shown.sha.to_string(), shown.changes.clone());
        let whole = cx.background_spawn(async move { load::whole(&repo, &sha, &changes, order) });
        cx.spawn(async move |this, cx| {
            let whole = whole.await;
            this.update(cx, |this, cx| this.apply_whole(token, whole, cx)).ok();
        })
        .detach();
    }

    fn apply_whole(&mut self, token: u64, whole: Result<load::Whole, String>, cx: &mut Context<Self>) {
        if token != self.token || self.whole != Whole::Reading(token) {
            return;
        }
        let _stall = stall::section("commit panel: whole tree");
        match whole {
            Ok(whole) => {
                self.all.set_tree(whole.tree);
                match self.whole_expand.take() {
                    Some(true) => self.all.expand_all(),
                    Some(false) => self.all.collapse_all(),
                    None => {}
                }
                if let State::Shown(shown) = &mut self.state {
                    Rc::make_mut(shown).files = Rc::new(whole.files);
                }
                self.whole = Whole::Read;
            }
            Err(why) => self.state = State::Failed(why),
        }
        cx.notify();
    }

    /// Whether View all files waits for the whole tree.
    fn whole_reading(&self) -> bool {
        self.all_files && self.whole != Whole::Read
    }

    pub(crate) fn shown(&self) -> Option<Rc<Shown>> {
        match &self.state {
            State::Shown(s) => Some(s.clone()),
            _ => None,
        }
    }

    pub fn set_order(&mut self, order: Order, cx: &mut Context<Self>) {
        self.order = order;
        self.changed.set_order_off_thread(order, cx);
        self.all.set_order_off_thread(order, cx);
        if let State::Shown(shown) = &mut self.state {
            let shown = Rc::make_mut(shown);
            sort_changes(&mut shown.changes, order);
            shown.index = index(&shown.changes);
        }
        cx.notify();
    }

    pub fn view(&self) -> FileView {
        self.view
    }

    pub fn set_view(&mut self, view: FileView, cx: &mut Context<Self>) {
        self.view = view;
        cx.notify();
    }

    pub fn set_all_files(&mut self, on: bool, cx: &mut Context<Self>) {
        self.all_files = on;
        self.read_whole(cx);
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
        if self.whole_reading() {
            self.whole_expand = Some(!expanded);
        }
        if self.all_files { self.all_expanded = !expanded } else { self.changed_expanded = !expanded }
        cx.notify();
    }

    fn filter_box(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<TextInput> {
        self.filter.get_or_insert_with(|| text_input::filter_box("Filter files", Self::set_filter, window, cx)).0.clone()
    }

    #[cfg(test)]
    pub fn stale(&self) -> bool {
        self.stale
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
        let change = shown.index.get(path).map(|at| shown.changes[*at].clone());
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

fn index(changes: &[FileChange]) -> HashMap<String, usize> {
    changes.iter().enumerate().map(|(at, c)| (c.path.clone(), at)).collect()
}

/// By path ignoring case, then as written; reversed for Z–A.
pub(crate) fn sort_changes(changes: &mut [FileChange], order: Order) {
    changes.sort_by_cached_key(|c| (c.path.to_lowercase(), c.path.clone()));
    if order == Order::Descending {
        changes.reverse();
    }
}
