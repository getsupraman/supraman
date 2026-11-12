//! The Git tab of one repository: its commit graph, its panels and their writes through its one [`write_gate::WriteGate`].

mod agent_rows;
pub mod background;
mod band;
pub mod branch;
mod cap;
mod cells;
mod colours;
mod columns;
pub(crate) mod commit;
mod details;
pub mod diff;
pub(crate) mod find;
mod graph;
mod keys;
pub(crate) mod layout;
mod live;
mod load;
mod pages;
pub mod probe;
mod refresh;
mod seek;
pub(crate) mod ref_panel;
pub(crate) mod remote;
mod splice;
pub(crate) mod stash;
mod stash_bar;
pub(crate) mod undo;
mod viewer;
mod wip;
mod wip_row;
mod worktree_panel;
mod worktree_rows;
pub(crate) mod write_gate;
#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::sync::Arc;

use gpui::{AnyElement, App, AppContext as _, Context, Div, Entity, EventEmitter, FocusHandle, Pixels, Render, SharedString, Subscription, Task, Window, deferred, div, prelude::*, px};
use supraman_gitman::write::Occupancy;
use supraman_gitman::{Repo, WipCounts, WorktreeRow};
use supraman_ui::hit::{HitTargets, hit_target};
use supraman_ui::{banner, button, focus, menu};
use supraman_ui::tooltip::Tooltip;
use supraman_ui::icon::Icon;
use supraman_ui::placeholder::placeholder;
use supraman_ui::split::SplitPane;
use supraman_ui::stall;
use supraman_ui::store::save_off_thread;
use supraman_ui::table::{self, Column, Table, TableState};
use supraman_ui::toast::ToastSlot;
use theme::ActiveTheme as _;

use cells::ref_cell;
use details::{CommitPanel, FileView, PanelEvent};
use diff::DiffView;
use colours::Palette;
use load::{Heads, Problem};
use ref_panel::{RefEvent, RefPanel};
use stash::StashActions;
use branch::{BranchActions, OpenGit};
use commit::CommitActions;
use remote::RemoteActions;
use undo::{UndoBar, UndoEvent};
use wip::WipPanel;
use worktree_panel::{WorktreeEvent, WorktreePanel};
use write_gate::WriteGate;

pub use agent_rows::{AgentMenu, OpenAgent, PageAction, SessionPage, SessionPages};

/// The table's id: its hit keys are `git-col-<c>`, `git-divider-<c>`, `git-row-<n>`.
pub(crate) const TABLE: &str = "git";

pub use keys::bind_keys;
const NEAR_END: Pixels = px(1400.);
const PANEL_WIDTH: Pixels = px(380.);
/// The toolbar's height: a toolbar button, its padding and the rule under it.
pub(crate) const TOOLBAR: Pixels = px(35.);
const GRAPH_MIN: f32 = 320.;
const PANEL_MIN: f32 = 260.;
/// The ref panel shrinks this much faster than the graph and the commit panel beside it.
const REFS_SHRINK: f32 = 1000.;
/// The open form's width, laid over the graph at the ref panel's right edge.
pub(crate) const FORM_WIDTH: f32 = 320.;
/// The hit keys of the ref panel's, the graph's and the commit panel's places.
pub(crate) const REFS_PANE: &str = "git-refs-pane";
pub(crate) const GRAPH_PANE: &str = "git-graph-pane";
pub(crate) const PANEL_PANE: &str = "git-panel-pane";
/// The tab's panes in [`supraman_ui::focus`]'s order.
pub(crate) const FOCUS_REFS: &str = "git-refs";
pub(crate) const FOCUS_GRAPH: &str = "git-graph";
pub(crate) const FOCUS_PANEL: &str = "git-panel";
pub(crate) const FOCUS_DIFF: &str = "git-diff";

/// A tab's sizes in whole pixels and its ref panel's state.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct GitSizes {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub widths: Vec<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub panel: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refs: Option<u32>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub refs_shut: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hidden: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub collapsed: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub solo: Vec<String>,
    /// The ref panel's sections' heights while open; none for an even share.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<u32>,
}

fn whole(p: Pixels) -> u32 {
    f32::from(p).round().max(0.) as u32
}

/// `graph_max` is the width of the lanes loaded so far.
fn columns(graph_max: Option<Pixels>) -> Vec<Column> {
    let graph = Column::new("graph", "Graph", px(graph::MIN_WIDTH), px(150.)).icon(Icon::GitGraph, px(56.));
    vec![
        Column::new("refs", "Branch / Tag", px(32.), px(130.)).icon(Icon::Tag, px(55.)),
        match graph_max {
            Some(max) => graph.max(max),
            None => graph,
        },
        Column::new("message", "Commit Message", px(50.), px(300.)).icon(Icon::Chat, px(55.)),
        Column::new("date", "Commit Date / Time", px(50.), px(170.)).icon(Icon::Clock, px(55.)),
        Column::new("sha", "Sha", px(50.), px(130.)).icon(Icon::Hash, px(50.)),
    ]
}

/// What is selected, for the commit details beside the graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selected {
    Wip,
    Commit(SharedString),
    /// A stash's row, its index commit.
    Stash(Arc<supraman_gitman::Stash>),
    Worktree(Arc<std::path::Path>),
    Agent(SharedString),
}

/// The selection changed; [`GitView::selected`] says to what.
pub(crate) enum GitEvent {
    Selected,
}

enum Load {
    Loading,
    Problem(Problem),
    Ready,
}

pub struct GitView {
    pub folder: PathBuf,
    home: Option<PathBuf>,
    hits: HitTargets,
    load: Load,
    /// Why some of `[gitman]` is not taken, until dismissed.
    settings_notice: Option<String>,
    repo: Option<Repo>,
    heads: Heads,
    pages: Entity<pages::Pages>,
    /// The other worktrees whose WIP rows the last graph read laid out.
    worktree_rows: Vec<WorktreeRow>,
    agents: agent_rows::Agents,
    wip: Option<WipCounts>,
    table: TableState,
    /// The columns' preferred widths.
    widths: Vec<Pixels>,
    graph_max: Option<Pixels>,
    hover_ref: Option<usize>,
    over_list: Option<usize>,
    panel: Entity<CommitPanel>,
    wip_panel: Entity<WipPanel>,
    worktree_panel: Entity<WorktreePanel>,
    panel_width: Pixels,
    ref_panel: Entity<RefPanel>,
    seek: Option<seek::Seek>,
    /// Why a pick or a key selected nothing, over the graph.
    toast: ToastSlot,
    /// A ref's commit to reveal once the refresh under way walks it.
    reveal_after: Option<SharedString>,
    _panel_events: [Subscription; 4],
    viewer: Entity<viewer::Viewer>,
    /// The viewer closed since the last frame and the graph takes the keyboard on the next.
    focus_graph: bool,
    band: band::Band,
    /// The panels' read of a selection a key moved, waiting for the key to come up.
    settle: Option<Task<()>>,
    follow: refresh::Follow,
    /// The repo's key in the layout file, once read.
    key: Option<String>,
    own_graph: bool,
    find: Option<find::Find>,
    undo: Entity<UndoBar>,
    _undo_events: Subscription,
    gate: Entity<WriteGate>,
    stash: Entity<StashActions>,
    _stash_events: Subscription,
    branch: Entity<BranchActions>,
    _branch_events: Subscription,
    commit: Entity<CommitActions>,
    _commit_events: Subscription,
    remote: Entity<RemoteActions>,
    _remote_events: Subscription,
    hidden_columns: Vec<String>,
    column_menu: Option<supraman_ui::menu::ContextMenu<columns::ColumnAct>>,
    /// The commit summary takes the keyboard on the next frame.
    focus_summary: bool,
    /// The tab's own: a click on nothing focusable in it keeps the keyboard in the tab's key context.
    focus: FocusHandle,
    /// The right panel's pane, whichever panel it shows.
    panel_focus: FocusHandle,
}

impl EventEmitter<GitEvent> for GitView {}
impl EventEmitter<OpenGit> for GitView {}

impl GitView {
    #[cfg(any(test, feature = "test-support"))]
    pub fn open(folder: PathBuf, home: Option<PathBuf>, hits: HitTargets, cx: &mut App) -> Entity<GitView> {
        GitView::open_with(folder, home, hits, Arc::new(|_: &std::path::Path| Vec::new()), cx)
    }

    /// `occupancy`: the live sessions working in it.
    pub fn open_with(folder: PathBuf, home: Option<PathBuf>, hits: HitTargets, occupancy: Arc<dyn Occupancy>, cx: &mut App) -> Entity<GitView> {
        cx.new(|cx| {
            let gate = cx.new(|_| WriteGate::new(occupancy));
            let panel = cx.new(|cx| CommitPanel::new(hits.clone(), gate.clone(), None, cx));
            let wip_panel = cx.new(|cx| WipPanel::new(hits.clone(), gate.clone(), cx));
            let worktree_panel = cx.new(|cx| WorktreePanel::new(hits.clone(), cx));
            let focus = cx.focus_handle();
            let ref_panel = cx.new(|cx| RefPanel::new(hits.clone(), cx));
            let events = [cx.subscribe(&panel, Self::panel_event), cx.subscribe(&wip_panel, Self::panel_event), cx.subscribe(&ref_panel, Self::ref_event), cx.subscribe(&worktree_panel, Self::worktree_event)];
            let undo = cx.new(|cx| UndoBar::new(gate.clone(), hits.clone(), cx));
            let undo_events = cx.subscribe(&undo, Self::undo_event);
            let stash = cx.new(|cx| StashActions::new(gate.clone(), hits.clone(), cx));
            let stash_events = cx.subscribe(&stash, Self::stash_event);
            let branch = cx.new(|cx| BranchActions::new(gate.clone(), hits.clone(), cx));
            let branch_events = cx.subscribe(&branch, Self::branch_event);
            let commit = cx.new(|cx| CommitActions::new(gate.clone(), hits.clone(), cx));
            let commit_events = cx.subscribe(&commit, Self::commit_event);
            let remote = cx.new(|cx| RemoteActions::new(gate.clone(), hits.clone(), home.clone(), cx));
            let remote_events = cx.subscribe(&remote, Self::remote_event);
            let mut view = GitView {
                folder,
                home,
                hits,
                load: Load::Loading,
                settings_notice: None,
                repo: None,
                heads: Heads::default(),
                pages: cx.new(|_| pages::Pages::default()),
                worktree_rows: Vec::new(),
                agents: agent_rows::Agents::default(),
                wip: None,
                table: TableState::new(cx),
                widths: columns(None).iter().map(|c| c.default).collect(),
                graph_max: None,
                hover_ref: None,
                over_list: None,
                panel,
                wip_panel,
                worktree_panel,
                panel_width: PANEL_WIDTH,
                ref_panel,
                seek: None,
                toast: ToastSlot::default(),
                reveal_after: None,
                _panel_events: events,
                viewer: cx.new(|_| viewer::Viewer::default()),
                focus_graph: false,
                band: band::Band::new(cx),
                settle: None,
                follow: refresh::Follow::default(),
                key: None,
                own_graph: false,
                find: None,
                undo,
                _undo_events: undo_events,
                gate,
                stash,
                _stash_events: stash_events,
                branch,
                _branch_events: branch_events,
                commit,
                _commit_events: commit_events,
                remote,
                _remote_events: remote_events,
                hidden_columns: Vec::new(),
                column_menu: None,
                focus_summary: false,
                focus,
                panel_focus: cx.focus_handle(),
            };
            view.start(cx);
            view
        })
    }

    /// Reads the repo's key and the layout file off the UI thread, then opens at the repo's layout.
    fn start(&mut self, cx: &mut Context<Self>) {
        find::frecency::start(self.home.clone(), cx);
        let ready = layout::ready(self.home.clone(), cx);
        let folder = self.folder.clone();
        let key = cx.background_spawn(async move { layout::key(&folder) });
        cx.spawn(async move |this, cx| {
            ready.await;
            let key = key.await;
            this.update(cx, |this, cx| this.open_at(key, cx)).ok();
        })
        .detach();
    }

    fn open_at(&mut self, key: String, cx: &mut Context<Self>) {
        let opening = layout::opening(&key, cx);
        self.hidden_columns = layout::hidden_columns(cx);
        let view = layout::file_view(cx);
        self.show_file_view(view, cx);
        self.apply(&opening.sizes, cx);
        self.own_graph = opening.own_graph;
        self.key = Some(key);
        self.reload(cx);
    }

    fn apply(&mut self, sizes: &GitSizes, cx: &mut Context<Self>) {
        let defaults = columns(None);
        let wanted: Vec<Pixels> = defaults.iter().enumerate().map(|(i, c)| sizes.widths.get(i).filter(|w| **w > 0).map_or(c.default, |w| px(*w as f32))).collect();
        self.widths = table::clamp(&defaults, &wanted);
        self.panel_width = sizes.panel.map_or(PANEL_WIDTH, |w| px(w as f32));
        self.panel.update(cx, |p, cx| p.set_message_height(sizes.message.map(|h| px(h as f32)), cx));
        self.ref_panel.update(cx, |p, cx| p.restore(sizes, cx));
    }

    /// Keeps the tab's sizes as its repo's layout.
    fn keep_layout(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.key.clone() else { return };
        self.own_graph = true;
        let save = layout::remember(&key, &self.sizes(cx), cx);
        save_off_thread(save, cx, Self::kept);
    }

    fn kept(&mut self, saved: Result<(), String>, cx: &mut Context<Self>) {
        if let Err(why) = saved {
            layout::set_notice(Some(why), cx);
            cx.notify();
        }
    }

    fn panel_event<P>(&mut self, _: Entity<P>, e: &PanelEvent, cx: &mut Context<Self>) {
        match e {
            PanelEvent::Parent(sha) => self.seek_sha(sha.clone(), cx),
            PanelEvent::File(target) => self.toggle_diff(target.clone(), cx),
            PanelEvent::Resized => self.keep_layout(cx),
            PanelEvent::Wrote(toast) => {
                if let Some(toast) = toast {
                    self.toast.show(toast.clone(), |v: &mut GitView| &mut v.toast, cx);
                }
                self.refresh_after_write(cx);
            }
            PanelEvent::Why(why) => self.toast.show_why(why.clone(), |v: &mut GitView| &mut v.toast, cx),
            PanelEvent::StashFiles(files) => self.stash_files(files.clone(), cx),
            PanelEvent::ApplyFiles(sha, files) => {
                let (sha, files) = (sha.clone(), files.clone());
                self.stash.update(cx, |s, cx| s.apply_files(sha, files, cx));
            }
            PanelEvent::Amend(message) => {
                let (message, no_verify) = (message.clone(), self.wip_panel.read(cx).skips_hooks());
                self.panel.update(cx, |p, cx| p.amend(message, no_verify, cx));
            }
            PanelEvent::Reword(message) => {
                let (message, dirty) = (message.clone(), self.wip.is_some_and(|w| w.total() > 0));
                self.panel.update(cx, |p, cx| p.reword_older(message, dirty, cx));
            }
            PanelEvent::HeadMoved(sha, toast) => {
                self.reveal_after = Some(sha.clone());
                if let Some(toast) = toast {
                    self.toast.show(toast.clone(), |v: &mut GitView| &mut v.toast, cx);
                }
                self.refresh_after_write(cx);
            }
            PanelEvent::StashMessage(stash, message) => {
                let (stash, message) = (stash.clone(), message.clone());
                self.stash.update(cx, |s, cx| s.set_message(stash, message, cx));
            }
            PanelEvent::FileMenu(click) => self.file_menu(click.clone(), cx),
            PanelEvent::View(view) => self.set_file_view(*view, cx),
        }
    }

    fn worktree_event(&mut self, _: Entity<WorktreePanel>, e: &WorktreeEvent, cx: &mut Context<Self>) {
        match e {
            WorktreeEvent::File(path, target) => self.toggle_diff_in(Some(path.clone()), target.clone(), cx),
            WorktreeEvent::Open => {
                if let Some(ix) = self.table.selected() {
                    self.open_worktree(ix, true, cx);
                }
            }
            WorktreeEvent::View(view) => self.set_file_view(*view, cx),
        }
    }

    /// Path | Tree for every file list of the tab, kept for every Git tab.
    pub(crate) fn set_file_view(&mut self, view: FileView, cx: &mut Context<Self>) {
        self.show_file_view(view, cx);
        let save = layout::set_file_view(view, cx);
        save_off_thread(save, cx, Self::kept);
    }

    fn show_file_view(&mut self, view: FileView, cx: &mut Context<Self>) {
        self.panel.update(cx, |p, cx| p.set_view(view, cx));
        self.wip_panel.update(cx, |p, cx| p.set_view(view, cx));
        self.worktree_panel.update(cx, |p, cx| p.set_view(view, cx));
    }

    fn ref_event(&mut self, _: Entity<RefPanel>, e: &RefEvent, cx: &mut Context<Self>) {
        match e {
            RefEvent::Pick(sha, walked) => self.picked(sha.clone(), *walked, cx),
            RefEvent::Walk => {
                self.keep_layout(cx);
                self.refresh_graph(None, cx);
            }
            RefEvent::Layout => self.keep_layout(cx),
            RefEvent::Shown(sha) => self.reveal_after = Some(sha.clone()),
            RefEvent::Stash(act, stash) => self.stash_act(*act, stash.clone(), cx),
            RefEvent::Menu(path, position) => self.ref_menu(path, *position, cx),
            RefEvent::Checkout(path) => self.ref_double_clicked(path, cx),
            RefEvent::RefreshWips => background::refresh_wips(cx.entity_id(), cx),
            RefEvent::AddRemote => {
                self.branch.update(cx, |b, cx| b.close_add_worktree(cx));
                self.remote.update(cx, |r, cx| r.open_add(cx));
                cx.notify();
            }
            RefEvent::AddWorktree => self.open_add_worktree(cx),
        }
    }

    fn undo_event(&mut self, _: Entity<UndoBar>, e: &UndoEvent, cx: &mut Context<Self>) {
        match e {
            UndoEvent::Done(toast) => {
                self.toast.show(toast.clone(), |v: &mut GitView| &mut v.toast, cx);
                self.refresh_after_write(cx);
            }
            UndoEvent::Why(why) => {
                self.toast.show_why(why.clone(), |v: &mut GitView| &mut v.toast, cx);
                self.refresh_undo(cx);
            }
        }
    }

    /// Undo's and Redo's preview read again.
    fn refresh_undo(&mut self, cx: &mut Context<Self>) {
        if let Some(repo) = self.repo.clone() {
            self.undo.update(cx, |u, cx| u.refresh(&repo, cx));
        }
    }

    #[cfg(test)]
    pub(crate) fn undo_bar(&self) -> &Entity<UndoBar> {
        &self.undo
    }

    #[cfg(test)]
    pub(crate) fn gate(&self) -> &Entity<WriteGate> {
        &self.gate
    }

    pub(crate) fn ref_panel(&self) -> &Entity<RefPanel> {
        &self.ref_panel
    }

    /// Closes the viewer and selects the row of `sha` ([`GitView::seek_sha`]).
    pub(crate) fn reveal_sha(&mut self, sha: SharedString, cx: &mut Context<Self>) {
        self.close_diff(cx);
        self.seek_sha(sha, cx);
    }

    /// Opens `target`, or goes back to the graph when it is the one open.
    pub(crate) fn toggle_diff(&mut self, target: diff::Target, cx: &mut Context<Self>) {
        self.toggle_diff_in(None, target, cx);
    }

    /// [`GitView::toggle_diff`] of a file of the worktree at `worktree`, else of the tab's.
    pub(crate) fn toggle_diff_in(&mut self, worktree: Option<Arc<std::path::Path>>, target: diff::Target, cx: &mut Context<Self>) {
        if self.diff(cx).is_none_or(|d| d.read(cx).target != target || d.read(cx).worktree != worktree) {
            return self.open_diff_in(worktree, target, cx);
        }
        self.back_to_graph(cx);
    }

    /// Closes the viewer, deselects the panels' files and gives the graph's row the keyboard.
    pub(crate) fn back_to_graph(&mut self, cx: &mut Context<Self>) {
        self.close_diff(cx);
        self.panel.update(cx, |p, cx| p.deselect_files(cx));
        self.wip_panel.update(cx, |p, cx| p.deselect_files(cx));
        self.worktree_panel.update(cx, |p, cx| p.deselect_files(cx));
        self.focus_graph = true;
        cx.notify();
    }

    pub fn sizes(&self, cx: &App) -> GitSizes {
        let refs = self.ref_panel.read(cx);
        GitSizes {
            widths: self.widths.iter().map(|w| whole(*w)).collect(),
            panel: Some(whole(self.panel_width)),
            message: Some(whole(self.panel.read(cx).message_height())),
            refs: Some(whole(refs.width())),
            refs_shut: refs.shut(),
            hidden: refs.walk().hidden.iter().cloned().collect(),
            collapsed: refs.collapsed(),
            solo: refs.walk().solo.iter().cloned().collect(),
            sections: refs.sections(),
        }
    }

    #[cfg(test)]
    pub(crate) fn panel(&self) -> &Entity<CommitPanel> {
        &self.panel
    }

    #[cfg(test)]
    pub(crate) fn wip_panel(&self) -> &Entity<WipPanel> {
        &self.wip_panel
    }

    pub(crate) fn diff<'a>(&self, cx: &'a App) -> Option<&'a Entity<DiffView>> {
        self.viewer.read(cx).view()
    }

    /// Shows `target` in the graph's place; the graph keeps its selection and scroll for when it closes.
    pub fn open_diff(&mut self, target: diff::Target, cx: &mut Context<Self>) {
        self.open_diff_in(None, target, cx);
    }

    /// [`GitView::open_diff`] of a file of the worktree at `worktree`, read-only, else of the tab's.
    pub(crate) fn open_diff_in(&mut self, worktree: Option<Arc<std::path::Path>>, target: diff::Target, cx: &mut Context<Self>) {
        let Some(tab) = self.repo.clone() else { return };
        self.close_diff(cx);
        let working = worktree.is_none() && matches!(target.of, diff::Of::Unstaged | diff::Of::Staged);
        let repo = worktree.as_ref().map_or_else(|| tab.clone(), |w| Repo::new(tab.git().clone(), w.to_path_buf()));
        let (mode, layout) = self.viewer.read_with(cx, |v, _| (v.mode, v.layout));
        let view = DiffView::open(repo.clone(), target, mode, layout, self.hits.clone(), cx);
        view.update(cx, |d, _| d.worktree = worktree);
        if working {
            view.update(cx, |d, cx| d.enable_picks(self.gate.clone(), repo, cx));
        }
        let closed = cx.subscribe(&view, |this: &mut GitView, _, _: &diff::Close, cx| this.back_to_graph(cx));
        let go = cx.subscribe(&view, |this: &mut GitView, _, go: &diff::Go, cx| this.go_file(go.0, cx));
        let picked = cx.subscribe(&view, |this: &mut GitView, _, e: &diff::PickEvent, cx| this.picked_lines(e, cx));
        self.viewer.update(cx, |v, _| v.show(view, [closed, go, picked]));
        cx.notify();
    }

    fn picked_lines(&mut self, e: &diff::PickEvent, cx: &mut Context<Self>) {
        match e {
            diff::PickEvent::Wrote(toast) => {
                if let Some(toast) = toast {
                    self.toast.show(toast.clone(), |v: &mut GitView| &mut v.toast, cx);
                }
                self.refresh_after_write(cx);
            }
            diff::PickEvent::Why(why) => self.toast.show_why(why.clone(), |v: &mut GitView| &mut v.toast, cx),
        }
    }

    pub(crate) fn close_diff(&mut self, cx: &mut Context<Self>) {
        if self.viewer.update(cx, |v, cx| v.close(cx)) {
            cx.notify();
        }
    }

    /// The row whose ref list is open.
    pub(crate) fn listed(&self) -> Option<usize> {
        self.hover_ref.or(self.over_list)
    }

    pub(crate) fn selected(&self, cx: &App) -> Option<Selected> {
        let row = self.pages.read(cx).rows.get(self.table.selected()?)?;
        if let Some(id) = &row.agent {
            return Some(Selected::Agent(id.clone()));
        }
        if let Some(path) = &row.row.worktree {
            return Some(Selected::Worktree(path.clone()));
        }
        Some(match (row.wip(), row.stash()) {
            (true, _) => Selected::Wip,
            (false, Some(stash)) => Selected::Stash(stash.clone()),
            (false, None) => Selected::Commit(row.sha.clone()),
        })
    }

    fn selection_moved(&mut self, cx: &mut Context<Self>) {
        self.settle = None;
        self.load_details(cx);
        cx.emit(GitEvent::Selected);
    }

    /// The panels read the selected row.
    fn load_details(&mut self, cx: &mut Context<Self>) {
        let colour = self.table.selected().and_then(|ix| self.pages.read(cx).rows.get(ix)).map_or(0, |r| r.row.colour);
        let selected = self.selected(cx);
        if selected == Some(Selected::Wip) {
            let colours = self.ref_colours(cx);
            self.wip_panel.update(cx, |p, cx| {
                p.set_ref_colours(colours);
                p.show(colour, cx)
            });
        }
        if let Some(Selected::Worktree(path)) = &selected {
            let (wip, said) = self.worktree_read(cx);
            let path = path.clone();
            self.worktree_panel.update(cx, |p, cx| p.show(path, wip, said, cx));
        }
        self.panel.update(cx, |p, cx| p.show(selected, colour, cx));
    }

    /// Selects the row of `sha` when a loaded page holds it.
    pub(crate) fn select_sha(&mut self, sha: &str, cx: &mut Context<Self>) {
        let Some(ix) = self.pages.read(cx).rows.iter().position(|r| r.sha.as_ref() == sha) else { return };
        self.select_ix(ix, cx);
    }

    /// Gives the graph the keys when asked or when the tab's own element holds them, on HEAD's commit while no row is selected.
    fn graph_takes_keys(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let held = self.focus.is_focused(window) && matches!(self.load, Load::Ready);
        if !std::mem::take(&mut self.focus_graph) && !held {
            return;
        }
        window.focus(self.table.focus_handle(), cx);
        if self.table.selected().is_none() {
            self.select_head(cx);
        }
    }

    fn select_ix(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.table.select(Some(ix));
        self.table.reveal(ix);
        self.selection_moved(cx);
        cx.notify();
    }

    /// Selects HEAD's commit row, when a loaded page holds it.
    fn select_head(&mut self, cx: &mut Context<Self>) {
        let Some(head) = self.heads.head.clone() else { return };
        let Some(ix) = self.pages.read(cx).rows.iter().position(|r| r.sha == head && r.agent.is_none() && r.row.worktree.is_none() && !r.wip() && r.stash().is_none()) else { return };
        self.select_ix(ix, cx);
    }

    /// The WIP row's index: the first row under the agent rows, when it is the WIP row.
    pub(crate) fn wip_row(&self, cx: &App) -> Option<usize> {
        let rows = &self.pages.read(cx).rows;
        rows.iter().position(|r| r.agent.is_none()).filter(|ix| rows[*ix].wip())
    }

    /// Selects the WIP row, when the graph has one.
    pub(crate) fn select_wip(&mut self, cx: &mut Context<Self>) {
        let Some(ix) = self.wip_row(cx) else { return };
        self.select_ix(ix, cx);
    }

    pub(crate) fn reload(&mut self, cx: &mut Context<Self>) {
        self.load = Load::Loading;
        self.pages.update(cx, |p, _| p.history = None);
        let (home, folder, walk) = (self.home.clone(), self.folder.clone(), self.ref_panel.read(cx).walk().clone());
        let first = cx.background_spawn(async move { load::first(home.as_deref(), &folder, &walk) });
        cx.spawn(async move |this, cx| {
            let first = first.await;
            this.update(cx, |this, cx| this.apply_first(first, cx)).ok();
        })
        .detach();
        cx.notify();
    }

    fn apply_first(&mut self, first: Result<load::First, Problem>, cx: &mut Context<Self>) {
        let _stall = stall::section("git tab: first page");
        match first {
            Ok(first) => {
                if let Some((settings, notice)) = first.settings {
                    self.take_settings(settings, notice, cx);
                }
                let (page, history, tips) = (first.page, first.history, first.tips);
                self.pages.update(cx, |p, _| p.start(page.rows, page.more, page.layout, history, tips));
                self.heads = first.heads;
                self.follow.walked = first.walk;
                self.ref_panel.update(cx, |p, cx| p.set_built(first.refs, cx));
                self.fit_graph(cx);
                self.gate.update(cx, |g, cx| drop(g.open(&first.repo, cx)));
                self.panel.update(cx, |p, _| p.set_repo(first.repo.clone()));
                self.wip_panel.update(cx, |p, cx| p.set_repo(first.repo.clone(), cx));
                self.hand_refs(&first.repo, first.all.clone(), &first.status, cx);
                if let Some(debounces) = refresh::follows(cx) {
                    self.follow(first.repo.clone(), debounces, cx);
                }
                self.repo = Some(first.repo);
                let before = std::mem::replace(&mut self.wip, first.wip);
                self.load = Load::Ready;
                self.stack_fresh(cx);
                self.select_conflict(before, cx);
                self.refresh_undo(cx);
                if *self.ref_panel.read(cx).walk() != self.follow.walked || self.wanted_worktrees(cx) != self.worktree_rows {
                    self.refresh_graph(None, cx);
                }
            }
            Err(problem) => self.load = Load::Problem(problem),
        }
        cx.notify();
    }

    /// The tab's `[gitman]`, read by its first load, handed to its remote calls and the background schedule.
    fn take_settings(&mut self, settings: supraman_gitman::TabSettings, notice: Option<String>, cx: &mut Context<Self>) {
        if let Some(remote) = settings.remote {
            self.remote.update(cx, |r, cx| r.set_settings(remote, cx));
        }
        if self.home.is_some() {
            background::offer(self.home.clone(), settings.background, cx);
        }
        self.settings_notice = notice;
    }

    /// Widens the graph column to the first page's lanes, up to [`graph::MAX_FIT`], unless the repo keeps its own width.
    fn fit_graph(&mut self, cx: &App) {
        self.measure_lanes(cx);
        if self.own_graph {
            return;
        }
        let lanes = self.graph_max.unwrap_or(px(graph::MAX_FIT));
        if let Some(width) = self.widths.get_mut(1) {
            *width = (*width).max(lanes.min(px(graph::MAX_FIT))).min(lanes).max(px(graph::MIN_WIDTH));
        }
    }

    fn measure_lanes(&mut self, cx: &App) {
        self.graph_max = Some(px(graph::width_for(graph::lanes(&self.pages.read(cx).rows))));
    }

    /// [`GitView::measure_lanes`] of the rows from `from` on, kept when wider.
    fn measure_lanes_from(&mut self, from: usize, cx: &App) {
        let rows = &self.pages.read(cx).rows;
        let added = px(graph::width_for(graph::lanes(&rows[from.min(rows.len())..])));
        self.graph_max = Some(self.graph_max.map_or(added, |max| max.max(added)));
    }

    /// The next page, unless one is on its way or none is left.
    pub(crate) fn load_more(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else { return };
        let Some(next) = self.pages.update(cx, |p, _| p.next()) else { return };
        let generation = self.follow.generation;
        let page = cx.background_spawn(async move { load::next(&repo, &next.tips, next.skip, next.layout, next.history) });
        cx.spawn(async move |this, cx| {
            let page = page.await;
            this.update(cx, |this, cx| if this.follow.generation == generation { this.apply_page(page, cx) }).ok();
        })
        .detach();
        cx.notify();
    }

    fn apply_page(&mut self, page: Result<(load::Page, Option<supraman_gitman::History>), String>, cx: &mut Context<Self>) {
        let _stall = stall::section("git tab: next page");
        if let Some(from) = self.pages.update(cx, |p, _| p.landed(page)) {
            self.measure_lanes_from(from, cx);
            self.relay_agents(cx);
        }
        self.seek_on(cx);
        cx.notify();
    }

    fn cell(&mut self, ix: usize, col: usize, palette: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let rows = self.pages.read(cx).rows.clone();
        let Some(row) = rows.get(ix) else {
            let footer = if col == 2 { self.pages.read(cx).page_error.clone().unwrap_or_else(|| "Loading more commits…".into()) } else { String::new() };
            return div().text_color(palette.muted).child(SharedString::from(footer)).into_any_element();
        };
        if row.worktree().is_some() {
            return self.worktree_cell(ix, col, palette, cx);
        }
        if row.agent.is_some() {
            return self.agent_cell(ix, col, palette, cx);
        }
        let cell = match col {
            0 => match self.branch.read(cx).inline_field(&row.sha) {
                Some(field) => cells::naming(ix, field, &self.hits),
                None => {
                    let view = ref_cell(&row.row, self.listed() == Some(ix));
                    cells::refs(ix, row, view, self.table.selected() == Some(ix), palette, &self.hits, cx)
                }
            },
            1 => graph::cell(ix, row, !row.row.refs.is_empty(), palette, &self.hits),
            2 if row.wip() => return self.wip_message(ix, palette, cx),
            2 => cells::message(ix, row, None, palette, &self.hits),
            3 => cells::date(ix, row, palette, &self.hits),
            _ => cells::sha(ix, row, palette, &self.hits),
        };
        let cell = self.commit_row(ix, cell, cx);
        self.stash_row(ix, cell, cx)
    }

    fn render_table(&mut self, cx: &mut Context<Self>) -> AnyElement {
        self.trim_tail(cx);
        let palette = Palette::of(cx.theme());
        let pages = self.pages.read(cx);
        let (footer, rows) = (usize::from(pages.paging || pages.page_error.is_some()), pages.rows.len());
        let shown = self.shown_columns();
        let all = columns(self.graph_max);
        let (cols, widths): (Vec<Column>, Vec<Pixels>) = shown.iter().map(|i| (all[*i].clone(), self.widths[*i])).unzip();
        let (painted, resized) = (shown.clone(), shown);
        Table::new(TABLE, &self.table, &self.hits, cols, &widths, rows + footer, move |this: &mut GitView, ix, col, _, cx| this.cell(ix, painted[col], &palette, cx))
            .row_height(px(graph::ROW))
            .inset(px(graph::GAP / 2.))
            .footer(footer)
            .on_select(|this: &mut GitView, _, _, cx| this.stepped(cx))
            .on_click(|this: &mut GitView, _, _, cx| this.settled(cx))
            .on_resize(move |this: &mut GitView, widths, cx| {
                for (width, i) in widths.into_iter().zip(&resized) {
                    this.widths[*i] = width;
                }
                this.keep_layout(cx);
                cx.notify();
            })
            .on_header_menu(|this: &mut GitView, position, _, cx| this.header_menu(position, cx))
            .on_near_end(NEAR_END, |this: &mut GitView, _, cx| this.load_more(cx))
            .render(cx)
            .into_any_element()
    }
}

impl GitView {
    /// The WIP panel for the WIP row, else the commit panel under the banner of a dirty tree.
    fn render_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let selected = self.selected(cx);
        if selected == Some(Selected::Wip) {
            return div().size_full().child(self.wip_panel.clone()).into_any_element();
        }
        if let Some(Selected::Worktree(_)) = selected {
            return div().size_full().child(self.worktree_panel.clone()).into_any_element();
        }
        if let Some(Selected::Agent(id)) = &selected {
            return self.render_agent_panel(id, cx);
        }
        let dirty = self.wip.map_or(0, |w| w.total() as usize);
        let banner = (dirty > 0 && selected.is_some()).then(|| wip::banner(dirty, &self.hits, cx));
        div().size_full().flex().flex_col().children(banner).child(div().flex_1().min_h_0().child(self.panel.clone())).into_any_element()
    }

    fn render_split(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let pane = match self.diff(cx) {
            Some(diff) => focus::Pane::new(FOCUS_DIFF, diff.read(cx).focus_handle()).later(),
            None => focus::Pane::new(FOCUS_GRAPH, self.table.focus_handle()),
        };
        let first = match self.diff(cx).cloned() {
            Some(view) if self.selected(cx) == Some(Selected::Wip) => div()
                .size_full()
                .key_context(keys::WIP_LIST_CONTEXT)
                .on_action(cx.listener(|this, _: &keys::StageSelected, _, cx| this.wip_panel.update(cx, |p, cx| p.stage_selected(cx))))
                .on_action(cx.listener(|this, _: &keys::UnstageSelected, _, cx| this.wip_panel.update(cx, |p, cx| p.unstage_selected(cx))))
                .child(view)
                .into_any_element(),
            Some(view) => view.into_any_element(),
            None => self.render_graph(cx),
        };
        let form = self.render_form(cx).map(|form| {
            let border = cx.theme().colors().border;
            deferred(div().absolute().top_0().bottom_0().left_0().w(px(FORM_WIDTH)).max_w_full().occlude().border_r_1().border_color(border).child(form)).with_priority(menu::FORM)
        });
        let toast = self.toast.render(&self.hits, "git-toast", cx).map(|t| deferred(t).with_priority(menu::OVER_FORM));
        let table = div().relative().flex_1().min_w_0().h_full().child(first).child(hit_target(&self.hits, GRAPH_PANE.into())).child(pane.paint(&self.hits)).children(form).children(toast);
        let panel = div().relative().flex_shrink(1.).min_w_0().flex().flex_col().w(self.panel_width).h_full().track_focus(&self.panel_focus).child(self.render_panel(cx)).child(hit_target(&self.hits, PANEL_PANE.into()));
        let panel = panel.child(focus::Pane::new(FOCUS_PANEL, &self.panel_focus).paint(&self.hits));
        let split = SplitPane { id: "git-split".into(), min_first: px(GRAPH_MIN), min_second: px(PANEL_MIN), default: PANEL_WIDTH }
            .render_second(table, panel, |this: &mut GitView, width, cx| {
                this.panel_width = width;
                cx.notify();
            }, |this: &mut GitView, cx| this.keep_layout(cx), cx)
            .size_full();
        self.with_refs(split.into_any_element(), cx)
    }

    /// Add Remote, Edit Remote or Create Worktree, the one form open.
    fn render_form(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.remote.update(cx, |r, cx| r.render_add(cx)).or_else(|| self.branch.update(cx, |b, cx| b.render_add_worktree(cx)))
    }

    /// `rest` right of the ref panel, which in a narrow tab gives its width before the graph and commit panel do.
    fn with_refs(&mut self, rest: AnyElement, cx: &mut Context<Self>) -> AnyElement {
        let refs = self.ref_panel.read(cx);
        let (shut, width) = (refs.shut(), refs.width());
        let panel = self.ref_panel.clone();
        if shut {
            let border = cx.theme().colors().border;
            let strip = div().flex_none().h_full().border_r_1().border_color(border).child(panel);
            return div().flex().size_full().child(strip).child(div().flex_1().min_w_0().h_full().child(rest)).into_any_element();
        }
        let rest_min = px(GRAPH_MIN + PANEL_MIN + 1.);
        let left = div().relative().flex_shrink(REFS_SHRINK).min_w_0().overflow_hidden().w(width).h_full().child(panel).child(hit_target(&self.hits, REFS_PANE.into()));
        let rest = div().flex_grow(1.).flex_basis(rest_min).min_w_0().h_full().child(rest);
        SplitPane { id: "refpanel-split".into(), min_first: ref_panel::MIN_WIDTH, min_second: rest_min, default: ref_panel::DEFAULT_WIDTH }
            .render(left, rest, |this: &mut GitView, width, cx| this.ref_panel.update(cx, |p, cx| p.set_width(width, cx)), |this: &mut GitView, cx| this.keep_layout(cx), cx)
            .size_full()
            .into_any_element()
    }
}

impl GitView {
    /// The layout file's trouble: unreadable, or a change not written.
    fn layout_notice(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let problem = layout::notice(cx)?;
        Some(self.notice_banner("git-layout-notice-dismiss", problem, |_, cx| layout::set_notice(None, cx), cx))
    }

    fn settings_notice(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let notice = self.settings_notice.clone()?;
        Some(self.notice_banner("git-settings-notice-dismiss", notice, |this, _| this.settings_notice = None, cx))
    }

    /// Why the tab shows no graph, with a Retry that loads it again.
    fn problem(&self, problem: &Problem, cx: &mut Context<Self>) -> AnyElement {
        let (title, why) = match problem {
            Problem::Git(why) => ("Git is missing or too old", why),
            Problem::Failed(why) => ("Cannot read this repository", why),
        };
        let retry = button::action(&self.hits, "git-retry", "Retry", button::Tone::Neutral, button::Fill::Outline, cx).on_click(cx.listener(|this, _: &gpui::ClickEvent, _, cx| this.reload(cx)));
        notice(title, why.clone(), cx).child(div().pt_2().child(retry)).into_any_element()
    }

    /// A warning over the tab whose × under `key` runs `dismiss`.
    fn notice_banner(&self, key: &'static str, text: String, dismiss: impl Fn(&mut GitView, &mut Context<GitView>) + 'static, cx: &mut Context<Self>) -> AnyElement {
        let color = cx.theme().status().warning;
        let close = button::icon(&self.hits, key, Icon::Close.svg(14., cx.theme().colors().text_muted), px(20.), cx)
            .tooltip(Tooltip::text("Dismiss"))
            .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _, cx| {
                dismiss(this, cx);
                cx.notify();
            }));
        banner::tinted(color).m_2().child(banner::dismissable(SharedString::from(text), close)).into_any_element()
    }
}

impl GitView {
    fn toolbar(&self, cx: &mut Context<Self>) -> Div {
        let bar = div().h(TOOLBAR).flex().items_start().border_b_1().border_color(cx.theme().colors().border);
        let bar = bar.child(self.undo.clone()).child(self.stash_buttons(cx)).child(self.remote_buttons(cx));
        div().relative().flex_none().child(bar).child(hit_target(&self.hits, "git-toolbar".into()))
    }
}

fn notice(title: &str, detail: String, cx: &App) -> Div {
    placeholder(title.to_string(), [detail], cx).size_full().gap_2().px_6().bg(cx.theme().colors().editor_background)
}

impl gpui::Focusable for GitView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for GitView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _stall = stall::section("render: git tab");
        self.watch_activation(window, cx);
        let focus = self.viewer.update(cx, |v, cx| std::mem::take(&mut v.focus).then(|| v.view().map(|d| d.read(cx).focus_handle().clone())).flatten());
        if let Some(focus) = focus {
            window.focus(&focus, cx);
        }
        self.graph_takes_keys(window, cx);
        self.wip_panel.update(cx, |p, cx| p.ensure_composer(window, cx));
        self.branch.update(cx, |b, cx| b.ensure_dialog(window, cx));
        self.take_focus_summary(window, cx);
        let band = self.band(window, cx).map(|b| supraman_ui::confirm::cover(b, TOOLBAR * 2.));
        let content = match &self.load {
            Load::Loading => notice("Reading the history…", self.folder.display().to_string(), cx).into_any_element(),
            Load::Problem(problem) => self.problem(problem, cx),
            Load::Ready => self.render_split(cx),
        };
        div()
            .size_full()
            .track_focus(&self.focus)
            .key_context(keys::TAB_CONTEXT)
            .on_action(cx.listener(|this, _: &keys::ToggleRefs, _, cx| this.ref_panel.update(cx, |p, cx| p.toggle_shut(cx))))
            .on_action(cx.listener(|this, _: &keys::FilterRefs, _, cx| this.ref_panel.update(cx, |p, cx| p.focus_filter(cx))))
            .on_action(cx.listener(|this, _: &keys::Find, window, cx| this.open_find(None, window, cx)))
            .on_action(cx.listener(|this, _: &keys::FindHistory, window, cx| this.open_find(Some(find::entries::HISTORY), window, cx)))
            .on_action(cx.listener(|this, _: &keys::Undo, _, cx| this.undo.update(cx, |u, cx| u.undo(cx))))
            .on_action(cx.listener(|this, _: &keys::Redo, _, cx| this.undo.update(cx, |u, cx| u.redo(cx))))
            .on_action(cx.listener(|this, _: &keys::StageAll, _, cx| this.stage_all_keyed(cx)))
            .on_action(cx.listener(|this, _: &keys::UnstageAll, _, cx| this.wip_panel.update(cx, |p, cx| p.unstage_all(cx))))
            .on_action(cx.listener(|this, _: &keys::FocusMessage, _, cx| this.focus_message(cx)))
            .on_action(cx.listener(|this, _: &keys::FetchAll, _, cx| this.remote.update(cx, |r, cx| r.act(remote::RemoteAct::FetchAll, cx))))
            .relative()
            .bg(cx.theme().colors().editor_background)
            .text_sm()
            .flex()
            .flex_col()
            .child(self.toolbar(cx))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(div().mx_2().child(self.stash.clone()).child(self.branch.clone()).child(self.commit.clone()).child(self.remote.clone()))
                    .children(self.layout_notice(cx))
                    .children(self.settings_notice(cx))
                    .children(self.follow_notice(cx))
                    .child(div().flex_1().min_h_0().child(content)),
            )
            .children(band.map(|b| deferred(b).with_priority(menu::OVER_FORM)))
            .children(self.render_find().map(|f| deferred(f).with_priority(menu::OVER_FORM)))
            .children(self.render_column_menu(cx))
    }
}
