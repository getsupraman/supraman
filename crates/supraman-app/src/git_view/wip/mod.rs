//! The WIP panel right of the graph while the WIP row is selected: the working directory's unstaged and
//! staged files, read off the UI thread on each selection, read-only (C146).

mod load;
mod render;

#[cfg(test)]
pub(crate) mod tests;

use std::rc::Rc;

use gpui::{App, AppContext as _, Context, EventEmitter, SharedString};
use supraman_gitman::{ChangeKind, FileChange, Repo};
use supraman_ui::hit::HitTargets;
use supraman_ui::stall;
use supraman_ui::table::TableState;
use supraman_ui::tree::{Order, TreeState};

use super::details::{Counts, FileView, PanelEvent, sort_changes};

pub use load::{Branch, Section, file_changes};
pub use render::banner;

/// The working directory shown, its lists in the panel's order.
#[derive(Debug, Clone)]
pub struct Shown {
    pub branch: Branch,
    pub total: usize,
    pub unstaged: Vec<FileChange>,
    pub staged: Vec<FileChange>,
}

impl Shown {
    pub fn files(&self, section: Section) -> &[FileChange] {
        match section {
            Section::Unstaged => &self.unstaged,
            Section::Staged => &self.staged,
        }
    }
}

pub enum State {
    Loading,
    Failed(String),
    Shown(Rc<Shown>),
}

/// One list's view state.
struct List {
    tree: TreeState<Option<ChangeKind>, Counts>,
    paths: TableState,
    open: bool,
}

impl List {
    fn new(cx: &mut App) -> List {
        List { tree: TreeState::new(cx), paths: TableState::new(cx), open: true }
    }
}

pub struct WipPanel {
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
    unstaged: List,
    staged: List,
}

impl EventEmitter<PanelEvent> for WipPanel {}

impl WipPanel {
    pub fn new(hits: HitTargets, cx: &mut App) -> Self {
        WipPanel {
            hits,
            repo: None,
            token: 0,
            in_place: false,
            state: State::Loading,
            colour: 0,
            view: FileView::Path,
            order: Order::Ascending,
            unstaged: List::new(cx),
            staged: List::new(cx),
        }
    }

    pub fn set_repo(&mut self, repo: Repo) {
        self.repo = Some(repo);
    }

    fn list(&self, section: Section) -> &List {
        match section {
            Section::Unstaged => &self.unstaged,
            Section::Staged => &self.staged,
        }
    }

    fn list_mut(&mut self, section: Section) -> &mut List {
        match section {
            Section::Unstaged => &mut self.unstaged,
            Section::Staged => &mut self.staged,
        }
    }

    /// Reads the working directory again off the UI thread, its lists opened afresh; the token of this read.
    pub fn show(&mut self, colour: usize, cx: &mut Context<Self>) -> u64 {
        self.in_place = false;
        self.read(colour, cx)
    }

    /// [`WipPanel::show`] keeping the lists' open folders and selected files.
    pub fn refresh(&mut self, cx: &mut Context<Self>) -> u64 {
        let token = self.read(self.colour, cx);
        self.in_place = true;
        token
    }

    fn read(&mut self, colour: usize, cx: &mut Context<Self>) -> u64 {
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
        let loaded = cx.background_spawn(async move { load::load(&repo, order) });
        cx.spawn(async move |this, cx| {
            let loaded = loaded.await;
            this.update(cx, |this, cx| this.apply(token, loaded, cx)).ok();
        })
        .detach();
        cx.notify();
        token
    }

    /// `loaded` for the read of `token`; dropped when another read started since.
    pub fn apply(&mut self, token: u64, loaded: Result<load::Loaded, String>, cx: &mut Context<Self>) {
        if token != self.token {
            return;
        }
        let _stall = stall::section("wip panel: apply");
        match loaded {
            Ok(l) => {
                for (list, tree) in [(&self.unstaged, l.unstaged_tree), (&self.staged, l.staged_tree)] {
                    list.tree.set_tree(tree);
                    if !self.in_place {
                        list.tree.expand_all();
                        list.tree.select(None);
                        list.paths.select(None);
                    }
                }
                self.state = State::Shown(Rc::new(Shown { branch: l.branch, total: l.total, unstaged: l.unstaged, staged: l.staged }));
            }
            Err(why) => self.state = State::Failed(why),
        }
        cx.notify();
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    fn shown(&self) -> Option<Rc<Shown>> {
        match &self.state {
            State::Shown(s) => Some(s.clone()),
            _ => None,
        }
    }

    pub fn set_order(&mut self, order: Order, cx: &mut Context<Self>) {
        self.order = order;
        self.unstaged.tree.set_order(order);
        self.staged.tree.set_order(order);
        if let Some(shown) = self.shown() {
            let mut shown = (*shown).clone();
            sort_changes(&mut shown.unstaged, order);
            sort_changes(&mut shown.staged, order);
            self.state = State::Shown(Rc::new(shown));
        }
        cx.notify();
    }

    pub fn set_view(&mut self, view: FileView, cx: &mut Context<Self>) {
        self.view = view;
        cx.notify();
    }

    pub fn toggle(&mut self, section: Section, cx: &mut Context<Self>) {
        let list = self.list_mut(section);
        list.open = !list.open;
        cx.notify();
    }

    /// Opens `path`'s change in `section`; a folder of the tree opens nothing.
    pub fn deselect_files(&mut self, cx: &mut Context<Self>) {
        for list in [&self.unstaged, &self.staged] {
            list.tree.select(None);
            list.paths.select(None);
        }
        cx.notify();
    }

    pub fn file_clicked(&mut self, section: Section, path: &str, cx: &mut Context<Self>) {
        let Some(shown) = self.shown() else { return };
        if let Some(file) = shown.files(section).iter().find(|f| f.path == path) {
            cx.emit(PanelEvent::File(section.target(file)));
        }
    }

    /// The header's words before the branch: `3 file changes on`.
    pub fn header_words(shown: &Shown) -> SharedString {
        let on = match shown.branch {
            Branch::Named(_) => "on",
            Branch::Detached(_) => "on detached HEAD",
        };
        format!("{} {on}", file_changes(shown.total)).into()
    }
}
