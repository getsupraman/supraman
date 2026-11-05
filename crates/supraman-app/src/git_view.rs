//! The Git tab of one repository: its commit graph, read-only (C146), loaded off the UI thread page by page.

mod cells;
mod colours;
mod details;
pub mod diff;
mod graph;
mod live;
mod load;
pub mod probe;
mod refresh;
mod wip;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod live_tests;

use std::path::PathBuf;
use std::sync::Arc;

use gpui::{AnyElement, App, AppContext as _, Context, Entity, EventEmitter, Pixels, Render, SharedString, Subscription, Window, div, prelude::*, px};
use supraman_gitman::{Layout, Repo};
use supraman_ui::hit::HitTargets;
use supraman_ui::split::SplitPane;
use supraman_ui::stall;
use supraman_ui::table::{Column, Table, TableState};
use theme::ActiveTheme as _;

use cells::ref_cell;
use details::{CommitPanel, PanelEvent};
use diff::{DiffView, Layout as DiffLayout, Mode as DiffMode};
use colours::Palette;
use load::{GraphRow, Problem, Wip};
use wip::WipPanel;

/// The table's id: its hit keys are `git-col-<c>`, `git-divider-<c>`, `git-row-<n>`.
pub const TABLE: &str = "git";
/// The near-end distance that asks for the next page.
const NEAR_END: Pixels = px(1400.);
const PANEL_WIDTH: Pixels = px(380.);

/// A tab's sizes in whole pixels: the columns', the commit panel's width and its message box's height.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitSizes {
    pub widths: Vec<u32>,
    pub panel: Option<u32>,
    pub message: Option<u32>,
}

fn whole(p: Pixels) -> u32 {
    f32::from(p).round().max(0.) as u32
}

fn columns() -> Vec<Column> {
    vec![
        Column::new("refs", "Branch / Tag", px(60.), px(170.)),
        Column::new("graph", "Graph", px(40.), px(120.)),
        Column::new("message", "Commit Message", px(120.), px(400.)).flexible(),
        Column::new("date", "Commit Date / Time", px(80.), px(140.)),
        Column::new("sha", "Sha", px(50.), px(76.)),
    ]
}

/// What is selected, for the commit details beside the graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selected {
    Wip,
    Commit(SharedString),
}

/// The selection changed; [`GitView::selected`] says to what.
pub enum GitEvent {
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
    repo: Option<Repo>,
    tips: Arc<[String]>,
    /// Away while a page is laid out with it.
    layout: Option<Layout>,
    rows: Vec<GraphRow>,
    commits: usize,
    wip: Option<Wip>,
    more: bool,
    paging: bool,
    page_error: Option<String>,
    table: TableState,
    widths: Vec<Pixels>,
    hover_ref: Option<usize>,
    over_list: Option<usize>,
    panel: Entity<CommitPanel>,
    wip_panel: Entity<WipPanel>,
    panel_width: Pixels,
    _panel_events: [Subscription; 2],
    /// The file viewer in the graph's place, while one is open.
    diff: Option<(Entity<DiffView>, Subscription)>,
    /// The last viewer's mode and layout, the next one's start.
    diff_mode: DiffMode,
    diff_layout: DiffLayout,
    /// The viewer opened since the last frame and takes the keyboard on the next.
    focus_diff: bool,
    follow: refresh::Follow,
}

impl EventEmitter<GitEvent> for GitView {}

impl GitView {
    /// `sizes` are the ones the tab was saved with.
    pub fn open(folder: PathBuf, home: Option<PathBuf>, hits: HitTargets, sizes: &GitSizes, cx: &mut App) -> Entity<GitView> {
        cx.new(|cx| {
            let defaults = columns();
            let widths = defaults.iter().enumerate().map(|(i, c)| sizes.widths.get(i).map_or(c.default, |w| px(*w as f32))).collect();
            let panel = cx.new(|cx| CommitPanel::new(hits.clone(), sizes.message.map(|h| px(h as f32)), cx));
            let wip_panel = cx.new(|cx| WipPanel::new(hits.clone(), cx));
            let events = [cx.subscribe(&panel, Self::panel_event), cx.subscribe(&wip_panel, Self::panel_event)];
            let mut view = GitView {
                folder,
                home,
                hits,
                load: Load::Loading,
                repo: None,
                tips: Arc::from([]),
                layout: None,
                rows: Vec::new(),
                commits: 0,
                wip: None,
                more: false,
                paging: false,
                page_error: None,
                table: TableState::new(cx),
                widths,
                hover_ref: None,
                over_list: None,
                panel,
                wip_panel,
                panel_width: sizes.panel.map_or(PANEL_WIDTH, |w| px(w as f32)),
                _panel_events: events,
                diff: None,
                diff_mode: DiffMode::default(),
                diff_layout: DiffLayout::default(),
                focus_diff: false,
                follow: refresh::Follow::default(),
            };
            view.reload(cx);
            view
        })
    }

    fn panel_event<P>(&mut self, _: Entity<P>, e: &PanelEvent, cx: &mut Context<Self>) {
        match e {
            PanelEvent::Parent(sha) => self.select_sha(sha, cx),
            PanelEvent::File(target) => self.open_diff(target.clone(), cx),
        }
    }

    pub fn sizes(&self, cx: &App) -> GitSizes {
        GitSizes { widths: self.widths.iter().map(|w| whole(*w)).collect(), panel: Some(whole(self.panel_width)), message: Some(whole(self.panel.read(cx).message_height())) }
    }

    #[cfg(test)]
    pub fn panel(&self) -> &Entity<CommitPanel> {
        &self.panel
    }

    #[cfg(test)]
    pub fn wip_panel(&self) -> &Entity<WipPanel> {
        &self.wip_panel
    }

    pub fn diff(&self) -> Option<&Entity<DiffView>> {
        self.diff.as_ref().map(|(d, _)| d)
    }

    /// Shows `target` in the graph's place; the graph keeps its selection and scroll for when it closes.
    pub fn open_diff(&mut self, target: diff::Target, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else { return };
        self.close_diff(cx);
        let view = DiffView::open(repo, target, self.diff_mode, self.diff_layout, self.hits.clone(), cx);
        let closed = cx.subscribe(&view, |this: &mut GitView, _, _: &diff::Close, cx| this.close_diff(cx));
        self.diff = Some((view, closed));
        self.focus_diff = true;
        cx.notify();
    }

    pub fn close_diff(&mut self, cx: &mut Context<Self>) {
        if let Some((view, _)) = self.diff.take() {
            let (mode, layout, of) = view.read_with(cx, |v, _| (v.mode(), v.layout(), v.target.of.clone()));
            if of != diff::Of::Conflicted {
                self.diff_mode = mode;
            }
            self.diff_layout = layout;
            cx.notify();
        }
    }

    /// The row whose ref list is open.
    pub fn listed(&self) -> Option<usize> {
        self.hover_ref.or(self.over_list)
    }

    pub fn selected(&self) -> Option<Selected> {
        let row = self.rows.get(self.table.selected()?)?;
        Some(if row.wip() { Selected::Wip } else { Selected::Commit(row.sha.clone()) })
    }

    fn selection_moved(&mut self, cx: &mut Context<Self>) {
        let colour = self.table.selected().and_then(|ix| self.rows.get(ix)).map_or(0, |r| r.row.colour);
        let selected = self.selected();
        if selected == Some(Selected::Wip) {
            self.wip_panel.update(cx, |p, cx| p.show(colour, cx));
        }
        self.panel.update(cx, |p, cx| p.show(selected, colour, cx));
        cx.emit(GitEvent::Selected);
    }

    /// Selects the row of `sha` when a loaded page holds it.
    pub fn select_sha(&mut self, sha: &str, cx: &mut Context<Self>) {
        let Some(ix) = self.rows.iter().position(|r| r.sha.as_ref() == sha) else { return };
        self.table.select(Some(ix));
        self.table.reveal(ix);
        self.selection_moved(cx);
        cx.notify();
    }

    /// Selects the WIP row, when the graph has one.
    pub fn select_wip(&mut self, cx: &mut Context<Self>) {
        if !self.rows.first().is_some_and(GraphRow::wip) {
            return;
        }
        self.table.select(Some(0));
        self.table.reveal(0);
        self.selection_moved(cx);
        cx.notify();
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.load = Load::Loading;
        let (home, folder) = (self.home.clone(), self.folder.clone());
        let first = cx.background_spawn(async move { load::first(home.as_deref(), &folder) });
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
                self.commits = first.page.rows.iter().filter(|r| !r.wip()).count();
                self.rows = first.page.rows;
                self.more = first.page.more;
                self.layout = Some(first.page.layout);
                self.tips = first.tips.into();
                self.panel.update(cx, |p, _| p.set_repo(first.repo.clone()));
                self.wip_panel.update(cx, |p, _| p.set_repo(first.repo.clone()));
                if refresh::follows() {
                    self.follow(first.repo.clone(), cx);
                }
                self.repo = Some(first.repo);
                self.wip = first.wip;
                self.load = Load::Ready;
            }
            Err(problem) => self.load = Load::Problem(problem),
        }
        cx.notify();
    }

    /// The next page, unless one is on its way or none is left.
    pub fn load_more(&mut self, cx: &mut Context<Self>) {
        if self.paging || !self.more {
            return;
        }
        let (Some(repo), Some(layout)) = (self.repo.clone(), self.layout.take()) else { return };
        self.paging = true;
        let (tips, skip, generation) = (self.tips.clone(), self.commits, self.follow.generation);
        let page = cx.background_spawn(async move { load::next(&repo, &tips, skip, layout) });
        cx.spawn(async move |this, cx| {
            let page = page.await;
            this.update(cx, |this, cx| if this.follow.generation == generation { this.apply_page(page, cx) }).ok();
        })
        .detach();
        cx.notify();
    }

    fn apply_page(&mut self, page: Result<load::Page, String>, cx: &mut Context<Self>) {
        let _stall = stall::section("git tab: next page");
        self.paging = false;
        match page {
            Ok(page) => {
                self.commits += page.rows.len();
                self.rows.extend(page.rows);
                self.more = page.more;
                self.layout = Some(page.layout);
            }
            Err(e) => {
                self.more = false;
                self.page_error = Some(e);
            }
        }
        cx.notify();
    }

    fn cell(&mut self, ix: usize, col: usize, palette: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let Some(row) = self.rows.get(ix) else {
            let footer = if col == 2 { self.page_error.clone().unwrap_or_else(|| "Loading more commits…".into()) } else { String::new() };
            return div().text_color(palette.muted).child(SharedString::from(footer)).into_any_element();
        };
        match col {
            0 => {
                let view = ref_cell(&row.row, self.listed() == Some(ix));
                cells::refs(ix, row, view, self.table.selected() == Some(ix), palette, &self.hits, cx)
            }
            1 => graph::cell(row, !row.row.refs.is_empty(), palette),
            2 => {
                let counts = self.wip.map(|w| w.counts()).unwrap_or_default();
                cells::message(row, row.wip().then_some(counts.as_str()), palette)
            }
            3 => cells::date(row, supraman_core::time::now_ms(), palette),
            _ => cells::sha(row, palette),
        }
    }

    fn render_table(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let palette = Palette::of(cx.theme());
        let band = palette.clone();
        let rows = self.rows.len() + usize::from(self.paging || self.page_error.is_some());
        Table::new(TABLE, &self.table, &self.hits, columns(), &self.widths, rows, move |this: &mut GitView, ix, col, _, cx| this.cell(ix, col, &palette, cx))
            .row_height(px(graph::ROW))
            .band(move |this: &GitView, ix, _| this.rows.get(ix).map(|r| band.band(r.row.colour)))
            .on_select(|this: &mut GitView, _, _, cx| this.selection_moved(cx))
            .on_resize(|this: &mut GitView, widths, _, cx| {
                this.widths = widths;
                cx.notify();
            })
            .on_near_end(NEAR_END, |this: &mut GitView, _, cx| this.load_more(cx))
            .render(cx)
            .into_any_element()
    }
}

impl GitView {
    /// The WIP panel for the WIP row, else the commit panel under the banner of a dirty tree.
    fn render_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let selected = self.selected();
        if selected == Some(Selected::Wip) {
            return div().size_full().child(self.wip_panel.clone()).into_any_element();
        }
        let dirty = self.wip.map_or(0, |w| w.total());
        let banner = (dirty > 0 && selected.is_some()).then(|| wip::banner(dirty, &self.hits, cx));
        div().size_full().flex().flex_col().children(banner).child(div().flex_1().min_h_0().child(self.panel.clone())).into_any_element()
    }

    fn render_split(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let first = match &self.diff {
            Some((view, _)) => view.clone().into_any_element(),
            None => self.render_table(cx),
        };
        let table = div().flex_1().min_w_0().h_full().child(first);
        let panel = div().flex_shrink(1.).min_w_0().flex().flex_col().w(self.panel_width).h_full().child(self.render_panel(cx));
        SplitPane { id: "git-split".into(), min_first: px(320.), min_second: px(260.), default: PANEL_WIDTH }
            .render_second(table, panel, |this: &mut GitView, width, cx| {
                this.panel_width = width;
                cx.notify();
            }, |_: &mut GitView, _| {}, cx)
            .size_full()
            .into_any_element()
    }
}

fn notice(title: &str, detail: String, cx: &App) -> AnyElement {
    let colors = cx.theme().colors();
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_2()
        .px_6()
        .bg(colors.editor_background)
        .child(div().text_color(colors.text).child(SharedString::from(title.to_string())))
        .child(div().text_sm().text_color(colors.text_muted).child(SharedString::from(detail)))
        .into_any_element()
}

impl Render for GitView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _stall = stall::section("render: git tab");
        self.watch_activation(window, cx);
        if std::mem::take(&mut self.focus_diff)
            && let Some((view, _)) = &self.diff
        {
            let focus = view.read(cx).focus_handle().clone();
            window.focus(&focus, cx);
        }
        let content = match &self.load {
            Load::Loading => notice("Reading the history…", self.folder.display().to_string(), cx),
            Load::Problem(Problem::Git(why)) => notice("Git is missing or too old", why.clone(), cx),
            Load::Problem(Problem::Failed(why)) => notice("Cannot read this repository", why.clone(), cx),
            Load::Ready => self.render_split(cx),
        };
        div().size_full().bg(cx.theme().colors().editor_background).text_sm().child(content)
    }
}
