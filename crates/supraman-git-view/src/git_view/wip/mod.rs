//! The WIP panel right of the graph while the WIP row is selected: its files, their writes and the commit area.

mod actions;
pub(crate) mod compose;
mod compose_render;
mod conflicts;
mod load;
mod render;

#[cfg(test)]
pub(crate) mod tests;

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, AppContext as _, Context, Entity, EventEmitter, SharedString};
use supraman_gitman::{ChangeKind, Conflict, FileChange, OperationState, Repo, Status, UnderWay};
use supraman_ui::hit::HitTargets;
use supraman_ui::stall;
use supraman_ui::table::TableState;
use supraman_ui::tree::{Order, TreeState};

use super::details::{Counts, FileView, PanelEvent, sort_changes};
use super::diff::{Of, Target};
use super::write_gate::{WriteGate, Writes};

pub(crate) use actions::{Ask, Doing};
pub(crate) use load::{Branch, Section, file_changes, of_status, sections};
pub(crate) use render::banner;

/// The working directory shown, its lists in the panel's order.
#[derive(Debug, Clone)]
pub(crate) struct Shown {
    pub(crate) branch: Branch,
    pub(crate) total: usize,
    pub(crate) conflicted: Vec<FileChange>,
    pub(crate) unstaged: Vec<FileChange>,
    pub(crate) staged: Vec<FileChange>,
    pub(crate) state: OperationState,
    pub(crate) under_way: Option<UnderWay>,
    pub(crate) conflicts: HashMap<String, Conflict>,
}

impl Shown {
    pub(crate) fn files(&self, section: Section) -> &[FileChange] {
        match section {
            Section::Conflicted => &self.conflicted,
            Section::Unstaged => &self.unstaged,
            Section::Staged => &self.staged,
        }
    }

    /// An operation is under way, or files are conflicted: the panel shows the conflict header and lists.
    pub(crate) fn conflicting(&self) -> bool {
        self.state != OperationState::None || !self.conflicted.is_empty()
    }

    /// The lists the panel shows, top to bottom.
    pub(crate) fn sections(&self) -> &'static [Section] {
        if self.conflicting() { &[Section::Conflicted, Section::Unstaged, Section::Staged] } else { &[Section::Unstaged, Section::Staged] }
    }
}

pub(crate) enum State {
    Loading,
    Failed(String),
    Shown(Rc<Shown>),
}

/// One list's view state.
pub(crate) struct List {
    pub(crate) tree: TreeState<Option<ChangeKind>, Counts>,
    pub(crate) paths: TableState,
    pub(crate) open: bool,
}

impl List {
    pub(crate) fn new(cx: &mut App) -> List {
        List { tree: TreeState::new(cx), paths: TableState::new(cx), open: true }
    }
}

pub(crate) struct WipPanel {
    hits: HitTargets,
    repo: Option<Repo>,
    /// Bumped by each selection; a load that finishes under another one is dropped.
    token: u64,
    /// The last read keeps the lists' state.
    in_place: bool,
    state: State,
    /// The WIP row's lane colour, for the branch's pill.
    colour: usize,
    view: FileView,
    order: Order,
    conflicted: List,
    unstaged: List,
    staged: List,
    /// Each branch's lane colour in the graph, for the conflict header's pills.
    ref_colours: Rc<HashMap<String, usize>>,
    writes: Writes<Ask, Doing, actions::Kept>,
    /// The repo runs commit hooks: Skip hooks is offered.
    hooks: bool,
    /// Made once a window is at hand.
    composer: Option<compose::Composer>,
    #[cfg(test)]
    pub(crate) status_reads: usize,
}

impl EventEmitter<PanelEvent> for WipPanel {}

impl WipPanel {
    pub(crate) fn new(hits: HitTargets, gate: Entity<WriteGate>, cx: &mut Context<Self>) -> Self {
        WipPanel {
            hits,
            repo: None,
            token: 0,
            in_place: false,
            state: State::Loading,
            colour: 0,
            view: FileView::Path,
            order: Order::Ascending,
            conflicted: List::new(cx),
            unstaged: List::new(cx),
            staged: List::new(cx),
            ref_colours: Rc::default(),
            writes: Writes::new(gate, Self::written, cx),
            hooks: false,
            composer: None,
            #[cfg(test)]
            status_reads: 0,
        }
    }

    pub(crate) fn set_repo(&mut self, repo: Repo, cx: &mut Context<Self>) {
        self.repo = Some(repo);
        self.read_tools(cx);
    }

    /// Whether the repo runs commit hooks, off the UI thread.
    fn read_tools(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else { return };
        let read = cx.background_spawn(async move { supraman_gitman::commit_hooks(&repo).unwrap_or(false) });
        cx.spawn(async move |this, cx| {
            let hooks = read.await;
            this.update(cx, |this, cx| {
                this.hooks = hooks;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn list(&self, section: Section) -> &List {
        match section {
            Section::Conflicted => &self.conflicted,
            Section::Unstaged => &self.unstaged,
            Section::Staged => &self.staged,
        }
    }

    fn list_mut(&mut self, section: Section) -> &mut List {
        match section {
            Section::Conflicted => &mut self.conflicted,
            Section::Unstaged => &mut self.unstaged,
            Section::Staged => &mut self.staged,
        }
    }

    pub(crate) fn set_ref_colours(&mut self, colours: HashMap<String, usize>) {
        self.ref_colours = Rc::new(colours);
    }

    /// Reads the working directory again off the UI thread, its lists opened afresh; the token of this read.
    pub(crate) fn show(&mut self, colour: usize, cx: &mut Context<Self>) -> u64 {
        self.in_place = false;
        self.read_tools(cx);
        self.read(colour, None, cx)
    }

    /// [`WipPanel::show`] of `status`, read already, keeping the lists' open folders and selected files.
    pub(crate) fn refresh(&mut self, status: Arc<Status>, cx: &mut Context<Self>) -> u64 {
        let token = self.read(self.colour, Some(status), cx);
        self.in_place = true;
        token
    }

    fn read(&mut self, colour: usize, status: Option<Arc<Status>>, cx: &mut Context<Self>) -> u64 {
        self.token += 1;
        let token = self.token;
        self.colour = colour;
        let Some(repo) = self.repo.clone() else {
            self.state = State::Loading;
            cx.notify();
            return token;
        };
        if !matches!(self.state, State::Shown(_)) {
            self.state = State::Loading;
        }
        let order = self.order;
        #[cfg(test)]
        {
            self.status_reads += usize::from(status.is_none());
        }
        let loaded = cx.background_spawn(async move {
            match status {
                Some(status) => Ok(load::of_status(&status, order)),
                None => load::load(&repo, order),
            }
        });
        cx.spawn(async move |this, cx| {
            let loaded = loaded.await;
            this.update(cx, |this, cx| this.apply(token, loaded, cx)).ok();
        })
        .detach();
        cx.notify();
        token
    }

    /// `loaded` for the read of `token`; dropped when another read started since.
    pub(crate) fn apply(&mut self, token: u64, loaded: Result<load::Loaded, String>, cx: &mut Context<Self>) {
        if token != self.token {
            return;
        }
        let _stall = stall::section("wip panel: apply");
        match loaded {
            Ok(l) => {
                let before = self.shown().map(|s| s.state);
                for (list, tree) in [(&self.conflicted, l.conflicted_tree), (&self.unstaged, l.unstaged_tree), (&self.staged, l.staged_tree)] {
                    list.tree.set_tree(tree);
                    if !self.in_place {
                        list.tree.expand_all();
                        list.tree.select(None);
                        list.paths.select(None);
                    }
                }
                let shown = Shown { branch: l.branch, total: l.total, conflicted: l.conflicted, unstaged: l.unstaged, staged: l.staged, state: l.state, under_way: l.under_way, conflicts: l.conflicts };
                if before != Some(shown.state) {
                    self.fill_merge_message(&shown, cx);
                }
                self.state = State::Shown(Rc::new(shown));
            }
            Err(why) => self.state = State::Failed(why),
        }
        cx.notify();
    }

    pub(crate) fn shown(&self) -> Option<Rc<Shown>> {
        match &self.state {
            State::Shown(s) => Some(s.clone()),
            _ => None,
        }
    }

    pub(crate) fn set_order(&mut self, order: Order, cx: &mut Context<Self>) {
        self.order = order;
        for list in [&self.conflicted, &self.unstaged, &self.staged] {
            list.tree.set_order_off_thread(order, cx);
        }
        if let State::Shown(shown) = &mut self.state {
            let shown = Rc::make_mut(shown);
            sort_changes(&mut shown.conflicted, order);
            sort_changes(&mut shown.unstaged, order);
            sort_changes(&mut shown.staged, order);
        }
        cx.notify();
    }

    pub(crate) fn set_view(&mut self, view: FileView, cx: &mut Context<Self>) {
        self.view = view;
        cx.notify();
    }

    pub(crate) fn toggle(&mut self, section: Section, cx: &mut Context<Self>) {
        let list = self.list_mut(section);
        list.open = !list.open;
        cx.notify();
    }

    #[cfg(test)]
    pub(crate) fn selected_row(&self, section: Section) -> Option<usize> {
        self.list(section).paths.selected()
    }

    pub(crate) fn deselect_files(&mut self, cx: &mut Context<Self>) {
        for list in [&self.conflicted, &self.unstaged, &self.staged] {
            list.tree.select(None);
            list.paths.select(None);
        }
        cx.notify();
    }

    pub(crate) fn file_clicked(&mut self, section: Section, path: &str, cx: &mut Context<Self>) {
        let Some(shown) = self.shown() else { return };
        if let Some(file) = shown.files(section).iter().find(|f| f.path == path) {
            cx.emit(PanelEvent::File(section.target(file)));
        }
    }

    /// A section's files top to bottom as its list shows them, every folder of a tree open.
    fn paths(&self, section: Section, shown: &Shown) -> Vec<String> {
        match self.view {
            FileView::Path => shown.files(section).iter().map(|f| f.path.clone()).collect(),
            FileView::Tree => self.list(section).tree.files().iter().map(|p| p.to_string()).collect(),
        }
    }

    /// The conflicted files, the unstaged, then the staged, as the lists show them.
    pub(crate) fn file_order(&self) -> Vec<Target> {
        let Some(shown) = self.shown() else { return Vec::new() };
        [Section::Conflicted, Section::Unstaged, Section::Staged]
            .into_iter()
            .flat_map(|section| {
                let files: HashMap<&str, &FileChange> = shown.files(section).iter().map(|f| (f.path.as_str(), f)).collect();
                self.paths(section, &shown).into_iter().filter_map(|p| files.get(p.as_str()).map(|f| section.target(f))).collect::<Vec<_>>()
            })
            .collect()
    }

    /// Selects `target`'s row alone, opening its section and the folders above it.
    pub(crate) fn mark_file(&mut self, target: &Target, cx: &mut Context<Self>) {
        let Some(shown) = self.shown() else { return };
        self.deselect_files(cx);
        let section = match target.of {
            Of::Staged => Section::Staged,
            Of::Conflicted => Section::Conflicted,
            _ => Section::Unstaged,
        };
        let view = self.view;
        let list = self.list_mut(section);
        list.open = true;
        match view {
            FileView::Path => {
                let row = shown.files(section).iter().position(|f| f.path == target.path);
                list.paths.select(row);
                if let Some(row) = row {
                    list.paths.reveal(row);
                }
            }
            FileView::Tree => list.tree.select_opening(&target.path),
        }
        cx.notify();
    }

    /// The header's words before the branch: `3 file changes on`.
    pub(crate) fn header_words(shown: &Shown) -> SharedString {
        let on = match shown.branch {
            Branch::Named(_) => "on",
            Branch::Detached(_) => "on detached HEAD",
        };
        format!("{} {on}", file_changes(shown.total)).into()
    }
}
