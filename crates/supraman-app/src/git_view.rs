//! The Git tab of one repository: its commit graph, read-only (C146), loaded off the UI thread page by page.

mod cells;
mod colours;
mod graph;
mod load;

#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::sync::Arc;

use gpui::{AnyElement, App, AppContext as _, Context, Entity, EventEmitter, Pixels, Render, SharedString, Window, div, prelude::*, px};
use supraman_gitman::{Layout, Repo};
use supraman_ui::hit::HitTargets;
use supraman_ui::stall;
use supraman_ui::table::{Column, Table, TableState};
use theme::ActiveTheme as _;

use cells::ref_cell;
use colours::Palette;
use load::{GraphRow, Problem};

/// The table's id: its hit keys are `git-col-<c>`, `git-divider-<c>`, `git-row-<n>`.
pub const TABLE: &str = "git";
/// The near-end distance that asks for the next page.
const NEAR_END: Pixels = px(1400.);

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
#[cfg_attr(not(test), allow(dead_code))]
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
    wip: Option<String>,
    more: bool,
    paging: bool,
    page_error: Option<String>,
    table: TableState,
    widths: Vec<Pixels>,
    hover_ref: Option<usize>,
    over_list: Option<usize>,
}

impl EventEmitter<GitEvent> for GitView {}

impl GitView {
    /// `widths` are the columns' widths the tab was saved with.
    pub fn open(folder: PathBuf, home: Option<PathBuf>, hits: HitTargets, widths: &[u32], cx: &mut App) -> Entity<GitView> {
        cx.new(|cx| {
            let defaults = columns();
            let widths = defaults.iter().enumerate().map(|(i, c)| widths.get(i).map_or(c.default, |w| px(*w as f32))).collect();
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
            };
            view.reload(cx);
            view
        })
    }

    pub fn widths(&self) -> Vec<u32> {
        self.widths.iter().map(|w| f32::from(*w).round().max(0.) as u32).collect()
    }

    /// The row whose ref list is open.
    pub fn listed(&self) -> Option<usize> {
        self.hover_ref.or(self.over_list)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn selected(&self) -> Option<Selected> {
        let row = self.rows.get(self.table.selected()?)?;
        Some(if row.wip() { Selected::Wip } else { Selected::Commit(row.sha.clone()) })
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
                self.repo = Some(first.repo);
                self.wip = first.wip.map(|w| w.counts());
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
        let (tips, skip) = (self.tips.clone(), self.commits);
        let page = cx.background_spawn(async move { load::next(&repo, &tips, skip, layout) });
        cx.spawn(async move |this, cx| {
            let page = page.await;
            this.update(cx, |this, cx| this.apply_page(page, cx)).ok();
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
            2 => cells::message(row, row.wip().then(|| self.wip.as_deref().unwrap_or_default()), palette),
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
            .on_select(|_: &mut GitView, _, _, cx| cx.emit(GitEvent::Selected))
            .on_resize(|this: &mut GitView, widths, _, cx| {
                this.widths = widths;
                cx.notify();
            })
            .on_near_end(NEAR_END, |this: &mut GitView, _, cx| this.load_more(cx))
            .render(cx)
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
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _stall = stall::section("render: git tab");
        let content = match &self.load {
            Load::Loading => notice("Reading the history…", self.folder.display().to_string(), cx),
            Load::Problem(Problem::Git(why)) => notice("Git is missing or too old", why.clone(), cx),
            Load::Problem(Problem::Failed(why)) => notice("Cannot read this repository", why.clone(), cx),
            Load::Ready => self.render_table(cx),
        };
        div().size_full().bg(cx.theme().colors().editor_background).text_sm().child(content)
    }
}
