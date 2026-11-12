//! The read-only panel of another worktree's WIP row: its header, Open Worktree, and its files as the WIP panel groups them.

use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{AppContext as _, ClickEvent, Context, Div, EventEmitter, Render, SharedString, Window, div, prelude::*, px};
use supraman_gitman::{FileChange, WorktreeWip};
use supraman_ui::button;
use supraman_ui::clipped::path_label;
use supraman_ui::hit::{HitTargets, hit_target};
use supraman_ui::icon::Icon;
use supraman_ui::stall;
use supraman_ui::themes::secondary;
use supraman_ui::tree::Order;
use theme::ActiveTheme as _;

use super::cells;
use super::colours::{Palette, blend};
use super::details::{FileView, PANEL_STEP, files, placeholder, sort_changes};
use super::diff::{Of, Target};
use super::wip::{List, Section, of_status};
use super::worktree_rows::WorktreeView;

/// What the panel asks of the tab.
pub(crate) enum WorktreeEvent {
    /// Show this file of the worktree at this path.
    File(Arc<Path>, Target),
    /// Open Worktree: the selected row's worktree in its own tab.
    Open,
}

/// The worktree shown, its lists in the panel's order.
#[derive(Debug, Clone)]
pub(crate) struct Shown {
    pub(crate) path: Arc<Path>,
    pub(crate) said: WorktreeView,
    /// The read the lists were made from, to tell a new read from the same one.
    wip: WorktreeWip,
    pub(crate) conflicted: Vec<FileChange>,
    pub(crate) unstaged: Vec<FileChange>,
    pub(crate) staged: Vec<FileChange>,
}

impl Shown {
    pub(crate) fn files(&self, section: Section) -> &[FileChange] {
        match section {
            Section::Conflicted => &self.conflicted,
            Section::Unstaged => &self.unstaged,
            Section::Staged => &self.staged,
        }
    }

    fn sections(&self) -> &'static [Section] {
        if self.conflicted.is_empty() { &[Section::Unstaged, Section::Staged] } else { &[Section::Conflicted, Section::Unstaged, Section::Staged] }
    }
}

enum State {
    Empty,
    Reading,
    Failed(String),
    Shown(Rc<Shown>),
}

pub(crate) struct WorktreePanel {
    hits: HitTargets,
    /// Bumped by each read; a read that finishes under another one is dropped.
    token: u64,
    /// The read under way keeps the lists' open folders and selected files.
    in_place: bool,
    state: State,
    view: FileView,
    order: Order,
    conflicted: List,
    unstaged: List,
    staged: List,
}

impl EventEmitter<WorktreeEvent> for WorktreePanel {}

/// The list's id in its hit keys.
pub(crate) fn list_id(section: Section) -> &'static str {
    match section {
        Section::Conflicted => "worktree-conflicted",
        Section::Unstaged => "worktree-unstaged",
        Section::Staged => "worktree-staged",
    }
}

impl WorktreePanel {
    pub(crate) fn new(hits: HitTargets, cx: &mut Context<Self>) -> Self {
        WorktreePanel { hits, token: 0, in_place: false, state: State::Empty, view: FileView::Path, order: Order::Ascending, conflicted: List::new(cx), unstaged: List::new(cx), staged: List::new(cx) }
    }

    /// Shows the worktree at `path` as `wip` read it, its lists opened afresh; nothing while it is not read yet.
    pub(crate) fn show(&mut self, path: Arc<Path>, wip: Option<WorktreeWip>, said: Option<WorktreeView>, cx: &mut Context<Self>) {
        self.in_place = false;
        self.read(path, wip, said, cx);
    }

    /// A new read of the worktree shown, its lists kept in place; whether its files changed.
    pub(crate) fn refresh(&mut self, wip: Option<WorktreeWip>, said: Option<WorktreeView>, cx: &mut Context<Self>) -> bool {
        let Some(shown) = self.shown() else { return false };
        if wip.as_ref() == Some(&shown.wip) && said.as_ref() == Some(&shown.said) {
            return false;
        }
        let changed = wip.as_ref().is_none_or(|w| w.files() != shown.wip.files());
        self.in_place = true;
        self.read(shown.path.clone(), wip, said, cx);
        changed
    }

    fn read(&mut self, path: Arc<Path>, wip: Option<WorktreeWip>, said: Option<WorktreeView>, cx: &mut Context<Self>) {
        self.token += 1;
        let token = self.token;
        let (Some(wip), Some(said)) = (wip, said) else {
            self.state = State::Reading;
            return cx.notify();
        };
        let Some(status) = wip.status().cloned() else {
            let why = match &wip.read {
                supraman_gitman::WipRead::Failed(why) => why.clone(),
                _ => "Its folder is gone.".into(),
            };
            self.state = State::Failed(why);
            return cx.notify();
        };
        if !matches!(self.state, State::Shown(_)) {
            self.state = State::Reading;
        }
        let order = self.order;
        let lists = cx.background_spawn(async move { of_status(&status, order) });
        cx.spawn(async move |this, cx| {
            let l = lists.await;
            this.update(cx, |this, cx| {
                if this.token != token {
                    return;
                }
                let _stall = stall::section("worktree panel: apply");
                for (list, tree) in [(&this.conflicted, l.conflicted_tree), (&this.unstaged, l.unstaged_tree), (&this.staged, l.staged_tree)] {
                    list.tree.set_tree(tree);
                    if !this.in_place {
                        list.tree.expand_all();
                        list.tree.select(None);
                        list.paths.select(None);
                    }
                }
                this.state = State::Shown(Rc::new(Shown { path, said, wip, conflicted: l.conflicted, unstaged: l.unstaged, staged: l.staged }));
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn shown(&self) -> Option<Rc<Shown>> {
        match &self.state {
            State::Shown(s) => Some(s.clone()),
            _ => None,
        }
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

    pub(crate) fn set_view(&mut self, view: FileView, cx: &mut Context<Self>) {
        self.view = view;
        cx.notify();
    }

    fn set_order(&mut self, order: Order, cx: &mut Context<Self>) {
        self.order = order;
        for list in [&self.conflicted, &self.unstaged, &self.staged] {
            list.tree.set_order_off_thread(order, cx);
        }
        if let State::Shown(shown) = &mut self.state {
            let shown = Rc::make_mut(shown);
            for files in [&mut shown.conflicted, &mut shown.unstaged, &mut shown.staged] {
                sort_changes(files, order);
            }
        }
        cx.notify();
    }

    fn toggle(&mut self, section: Section, cx: &mut Context<Self>) {
        let list = self.list_mut(section);
        list.open = !list.open;
        cx.notify();
    }

    pub(crate) fn deselect_files(&mut self, cx: &mut Context<Self>) {
        for list in [&self.conflicted, &self.unstaged, &self.staged] {
            list.tree.select(None);
            list.paths.select(None);
        }
        cx.notify();
    }

    fn file_clicked(&mut self, section: Section, path: &str, cx: &mut Context<Self>) {
        let Some(shown) = self.shown() else { return };
        if let Some(file) = shown.files(section).iter().find(|f| f.path == path) {
            cx.emit(WorktreeEvent::File(shown.path.clone(), section.target(file)));
        }
    }

    /// The worktree shown and its files top to bottom as the lists show them, every folder of a tree open.
    pub(crate) fn file_order(&self) -> Option<(Arc<Path>, Vec<Target>)> {
        let shown = self.shown()?;
        let targets = [Section::Conflicted, Section::Unstaged, Section::Staged]
            .into_iter()
            .flat_map(|section| {
                let files: HashMap<&str, &FileChange> = shown.files(section).iter().map(|f| (f.path.as_str(), f)).collect();
                let paths: Vec<String> = match self.view {
                    FileView::Path => shown.files(section).iter().map(|f| f.path.clone()).collect(),
                    FileView::Tree => self.list(section).tree.files().iter().map(|p| p.to_string()).collect(),
                };
                paths.into_iter().filter_map(|p| files.get(p.as_str()).map(|f| section.target(f))).collect::<Vec<_>>()
            })
            .collect();
        Some((shown.path.clone(), targets))
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
}

impl WorktreePanel {
    /// The folder, its [`cells::branch_header`] and the counts, and Open Worktree.
    fn header(&self, shown: &Shown, cx: &mut Context<Self>) -> Div {
        let colors = cx.theme().colors().clone();
        let folder = supraman_core::paths::normalize(&shown.path);
        let path = div().relative().flex().min_w_0().child(path_label("worktree-path-dir", &folder, secondary(&colors), colors.text)).child(hit_target(&self.hits, "worktree-path".into()));
        let said = &shown.said;
        let palette = Palette::of(cx.theme());
        let counts = div().relative().flex_none().text_color(colors.text_muted).child(SharedString::from(said.counts.clone())).child(hit_target(&self.hits, "worktree-counts".into()));
        let words = div()
            .flex()
            .items_center()
            .gap_2()
            .min_w_0()
            .child(cells::branch_header("worktree", said.branch.as_deref(), &said.upstream, said.colour, &palette, &self.hits))
            .when(!said.counts.is_empty(), |d| d.child(counts));
        let open = button::action(&self.hits, "worktree-open", "Open Worktree", button::Tone::Neutral, button::Fill::Outline, cx).on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(WorktreeEvent::Open)));
        let line = div().flex().items_center().justify_between().gap_2().child(words).child(open);
        div().flex().flex_col().flex_none().gap_1().px_2().py_1p5().text_color(colors.text).child(path).child(line)
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> Div {
        let sort = files::sort_button(&self.hits, "worktree", self.order, |this: &mut WorktreePanel, order, cx| this.set_order(order, cx), cx);
        let toggle = files::view_toggle(&self.hits, "worktree", self.view, |this: &mut WorktreePanel, view, cx| this.set_view(view, cx), cx);
        div().flex().flex_none().items_center().h(px(28.)).px_2().child(sort).child(div().flex_1().flex().justify_center().child(toggle)).child(div().w(px(24.)))
    }

    /// A section's list: no row buttons, no menu.
    fn file_list(&self, section: Section, shown: &Rc<Shown>, cx: &mut Context<Self>) -> gpui::Stateful<Div> {
        let picked = move |this: &mut WorktreePanel, path: String, cx: &mut Context<WorktreePanel>| this.file_clicked(section, &path, cx);
        let list = self.list(section);
        match self.view {
            FileView::Path => {
                let rows = shown.clone();
                let file = move |ix: usize| rows.files(section).get(ix).map(|c| (c.path.clone(), c.kind));
                files::path_list(list_id(section), &list.paths, &self.hits, shown.files(section).len(), file, picked, None, None, cx)
            }
            FileView::Tree => files::tree_list(list_id(section), &list.tree, &self.hits, move |this: &mut WorktreePanel, path, cx| picked(this, path.to_string(), cx), None, None, cx),
        }
    }

    fn section(&self, section: Section, shown: &Rc<Shown>, cx: &mut Context<Self>) -> Div {
        let colors = cx.theme().colors().clone();
        let open = self.list(section).open;
        let chevron = if open { Icon::ChevronDown } else { Icon::ChevronRight };
        let title = format!("{} ({})", section.title(false), shown.files(section).len());
        let header = button::ghost(&self.hits, format!("{}-header", list_id(section)), cx)
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .h(px(26.))
            .px_1()
            .rounded_sm()
            .text_color(colors.text)
            .child(chevron.svg(12., colors.text_muted))
            .child(SharedString::from(title))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle(section, cx)));
        let rule = div().flex_none().h(px(1.)).bg(colors.border_variant);
        let column = div().flex().flex_col().min_h_0().px_1().child(header).child(rule);
        if open { column.flex_1().child(self.file_list(section, shown, cx).flex_1().min_h_0()) } else { column.flex_none() }
    }

    fn render_shown(&self, shown: Rc<Shown>, cx: &mut Context<Self>) -> Div {
        let border = cx.theme().colors().border_variant;
        let sections: Vec<Div> = shown.sections().iter().map(|s| self.section(*s, &shown, cx)).collect();
        div().flex().flex_col().size_full().gap_1().child(self.header(&shown, cx)).child(self.toolbar(cx).border_b_1().border_color(border)).children(sections)
    }
}

impl Render for WorktreePanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _stall = stall::section("render: worktree panel");
        let colors = cx.theme().colors().clone();
        let bg = blend(colors.text.opacity(PANEL_STEP), colors.editor_background);
        let content = match &self.state {
            State::Empty | State::Reading => placeholder("Reading the worktree…", None, cx),
            State::Failed(why) => placeholder("Cannot read this worktree", Some(why), cx),
            State::Shown(shown) => {
                let shown = shown.clone();
                self.render_shown(shown, cx).into_any_element()
            }
        };
        div().size_full().flex().flex_col().bg(bg).text_sm().child(content)
    }
}

#[cfg(test)]
mod tests;
