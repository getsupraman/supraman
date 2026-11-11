//! The graph's WIP rows of the repository's other worktrees: which the layout places, their cells, and Open Worktree.

use std::path::Path;

use gpui::{AnyElement, ClickEvent, Context, MouseButton, MouseDownEvent, SharedString, div, prelude::*, px};
use supraman_core::paths::path_key;
use supraman_gitman::{WorktreeRow, WorktreeWip};
use supraman_ui::button;
use supraman_ui::hit::hit_target;
use supraman_ui::icon::Icon;
use supraman_ui::tooltip::Tooltip;

use super::branch::OpenGit;
use super::colours::Palette;
use super::live::Refresh;
use super::load::{GraphRow, Wip};
use super::ref_panel::RefWips;
use super::{GitView, Load, Selected, TABLE, agent_rows, background, cells, graph};

/// What a worktree's WIP row says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorktreeView {
    /// `on <branch>` or `Detached HEAD`.
    pub(crate) label: String,
    /// The tab's WIP row's counts: `+1 ✎2 −1`.
    pub(crate) counts: String,
    /// Ahead of and behind its upstream ([`super::ref_panel::track`]).
    pub(crate) upstream: String,
    /// The live session working there, whose pill stands in place of `WIP`.
    pub(crate) session: Option<String>,
    pub(crate) detached: bool,
}

impl WorktreeView {
    pub(crate) fn of(wip: &WorktreeWip, session: Option<String>) -> WorktreeView {
        let label = match wip.branch() {
            Some(branch) => format!("on {branch}"),
            None => "Detached HEAD".into(),
        };
        let c = wip.counts();
        let counts = Wip { added: c.added as usize, modified: (c.modified + c.renamed) as usize, deleted: c.deleted as usize, conflicts: 0 }.counts();
        let (ahead, behind) = wip.upstream().map_or((0, 0), |(_, ahead, behind)| (ahead, behind));
        let upstream = super::ref_panel::track(ahead, behind);
        WorktreeView { label, counts, upstream, session, detached: wip.detached() }
    }
}

impl GitView {
    /// The other worktrees given a WIP row: read, with a HEAD, changed or worked in; none while `[gitman] worktree_wip` is off.
    pub(super) fn wanted_worktrees(&self, cx: &Context<Self>) -> Vec<WorktreeRow> {
        if !background::worktree_wips(cx) {
            return Vec::new();
        }
        let occupancy = self.gate.read(cx).occupancy().clone();
        let mut rows: Vec<WorktreeRow> = self
            .other_wips(cx)
            .iter()
            .filter(|w| w.status().is_some() && (w.counts().total() > 0 || !occupancy.who_works_in(w.path()).is_empty()))
            .filter_map(|w| Some(WorktreeRow { head: w.head()?.to_string(), path: w.path().to_path_buf() }))
            .collect();
        rows.sort();
        rows
    }

    /// A read of the other worktrees landed or the setting changed: the graph relaid when its worktree rows change.
    pub(crate) fn worktrees_read(&mut self, cx: &mut Context<Self>) {
        if matches!(self.load, Load::Ready) && self.wanted_worktrees(cx) != self.worktree_rows {
            self.live_refresh(Refresh::Graph, cx);
        }
        self.worktree_panel_read(cx);
        self.hand_wips(cx);
        cx.notify();
    }

    /// The ref panel's worktree rows given the other worktrees' words as last read, and whether a read runs.
    pub(crate) fn hand_wips(&mut self, cx: &mut Context<Self>) {
        let top = self.gate.read(cx).top();
        let wips = background::wips(cx.entity_id(), cx).map(|w| w.read(cx));
        let shown = RefWips {
            reading: wips.is_some_and(|w| w.reading()),
            landed: wips.is_some_and(|w| w.landed()),
            own: top.as_deref().map(path_key),
            said: self.other_wips(cx).iter().map(|w| (path_key(w.path()), WorktreeView::of(w, None))).collect(),
        };
        self.ref_panel.update(cx, |p, cx| p.set_wips(shown, cx));
    }

    /// The selected worktree row's read and words, as last read.
    pub(super) fn worktree_read(&self, cx: &Context<Self>) -> (Option<WorktreeWip>, Option<WorktreeView>) {
        let Some(ix) = self.table.selected() else { return (None, None) };
        let Some(path) = self.pages.read(cx).rows.get(ix).and_then(GraphRow::worktree) else { return (None, None) };
        let wip = background::wips(cx.entity_id(), cx).and_then(|w| w.read(cx).get(path).cloned());
        (wip, self.worktree_view(ix, cx))
    }

    /// The selected worktree's panel, and its open file, read again when its read changed.
    fn worktree_panel_read(&mut self, cx: &mut Context<Self>) {
        let Some(Selected::Worktree(path)) = self.selected(cx) else { return };
        let (wip, said) = self.worktree_read(cx);
        if !self.worktree_panel.update(cx, |p, cx| p.refresh(wip, said, cx)) {
            return;
        }
        let (Some(view), Some(repo)) = (self.diff(cx).cloned(), self.repo.clone()) else { return };
        view.update(cx, |d, cx| {
            if d.worktree.as_deref() == Some(path.as_ref()) {
                let repo = d.repo_of(&repo);
                d.reload(repo, cx);
            }
        });
    }

    pub(crate) fn worktree_view(&self, ix: usize, cx: &Context<Self>) -> Option<WorktreeView> {
        let path = self.pages.read(cx).rows.get(ix)?.worktree()?;
        let wip = background::wips(cx.entity_id(), cx)?.read(cx).get(path).cloned()?;
        let session = self.gate.read(cx).occupancy().who_works_in(path).into_iter().next();
        Some(WorktreeView::of(&wip, session))
    }

    /// Asks the workspace for row `ix`'s worktree's Git tab; a detached worktree's only when `detached_too`.
    pub(crate) fn open_worktree(&mut self, ix: usize, detached_too: bool, cx: &mut Context<Self>) {
        let Some(path) = self.pages.read(cx).rows.get(ix).and_then(|r| r.row.worktree.clone()) else { return };
        if !detached_too && self.worktree_view(ix, cx).is_none_or(|v| v.detached) {
            return;
        }
        cx.emit(OpenGit(path.to_path_buf()));
    }

    /// Row `ix`'s cell of column `col`, row `ix` a worktree's: its cells double-click to open it; a right-click is its session's menu.
    pub(super) fn worktree_cell(&mut self, ix: usize, col: usize, palette: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let view = self.worktree_view(ix, cx);
        let rows = self.pages.read(cx).rows.clone();
        let row = &rows[ix];
        let cell = match (col, view) {
            (1, _) => graph::cell(ix, row, false, palette, &self.hits),
            (0, Some(view)) => self.worktree_ref(ix, &view, palette, cx),
            (2, Some(view)) => {
                let activity = view.session.as_deref().zip(row.worktree()).and_then(|(name, path)| self.activity_in(path, name));
                let lead = match view.session {
                    Some(name) => supraman_ui::tabs::chip(name.into(), cx)
                        .relative()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .children(activity.map(|a| agent_rows::state_dot(a, cx).relative().child(hit_target(&self.hits, format!("git-worktree-state-{ix}")))))
                        .child(hit_target(&self.hits, format!("git-worktree-session-{ix}"))),
                    None => div().relative().flex_none().text_color(palette.muted).child("WIP").child(hit_target(&self.hits, format!("git-worktree-wip-{ix}"))),
                };
                cells::wip_message(row, lead, view.counts, palette)
            }
            _ => div().into_any_element(),
        };
        let path = row.worktree().map(Path::to_path_buf);
        div()
            .id(SharedString::from(format!("git-worktree-{ix}-{col}")))
            .size_full()
            .on_click(cx.listener(move |this, e: &ClickEvent, _, cx| {
                if e.click_count() == 2 {
                    this.open_worktree(ix, false, cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                    if let Some(path) = &path {
                        this.worktree_session_menu(path, e.position, cx);
                    }
                }),
            )
            .child(cell)
            .into_any_element()
    }

    /// The `on <branch>` label, then Open Worktree shown on the row's hover or selection.
    fn worktree_ref(&self, ix: usize, view: &WorktreeView, palette: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let colour = self.pages.read(cx).rows[ix].row.colour;
        let label = cells::pill_box(colour, palette)
            .relative()
            .text_color(palette.muted)
            .child(Icon::Worktree.svg(12., palette.muted))
            .child(SharedString::from(view.label.clone()))
            .child(hit_target(&self.hits, format!("git-worktree-ref-{ix}")));
        let selected = self.table.selected() == Some(ix);
        let open = button::revealed(&self.hits, format!("git-worktree-open-{ix}"), Icon::GitGraph.svg(12., palette.muted), px(18.), supraman_ui::table::row_group(TABLE), selected, cx)
            .tooltip(Tooltip::text("Open Worktree"))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.open_worktree(ix, true, cx);
            }));
        div().size_full().flex().items_center().gap_1().overflow_hidden().child(label).child(open).into_any_element()
    }
}

#[cfg(test)]
mod tests;
