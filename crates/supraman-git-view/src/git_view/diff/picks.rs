//! The viewer's line picks on a working-tree diff of a modified file (docs/research/gitkraken-12.6.1.md, "Staging").

use supraman_core::text::count;
use gpui::{AnyElement, App, AppContext as _, Context, Entity, MouseButton, MouseDownEvent, Pixels, Point, Subscription, Window, div, prelude::*};
use supraman_editor::{ZedEditor, rows};
use supraman_gitman::write::lines::{self, Change, Pick, Picked};
use supraman_gitman::write::{LockSet, Resource};
use supraman_gitman::{ChangeKind, OperationState, Repo};
use supraman_ui::menu::{ContextMenu, MenuItem, item};

use super::{DiffView, Layout, Mode, Of};
use crate::git_view::write_gate::{WriteGate, Written};

/// A row of the diff's old text (`new` false) or of its new one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Line {
    pub(crate) new: bool,
    pub(crate) row: u32,
}

/// A pick of a whole hunk or of some changed lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Act {
    Hunk(Pick, usize),
    Lines(Pick, Vec<Line>),
}

/// How a pick went, for the tab's toast; it then reads the file again.
pub(crate) enum PickEvent {
    Wrote(Option<String>),
    Why(String),
}

/// One editor of the layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pane {
    Old,
    New,
    Inline,
}

pub(crate) struct Picks {
    gate: Entity<WriteGate>,
    /// The operation under way, once read with the tab's writer.
    state: Option<OperationState>,
    menu: Option<ContextMenu<Act>>,
    doing: Option<(u64, Act)>,
    /// The inline editor's line of each row, read once per editor.
    inline_lines: Option<(gpui::EntityId, Vec<Option<(u32, bool)>>)>,
    _gate: [Subscription; 2],
}

impl Picks {
    /// The open menu and the rows' lines belong to the text a reload replaced.
    pub(super) fn forget_rows(&mut self) {
        (self.menu, self.inline_lines) = (None, None);
    }
}

impl Act {
    fn pick(&self) -> Pick {
        match self {
            Act::Hunk(pick, _) | Act::Lines(pick, _) => *pick,
        }
    }

    fn toast(&self) -> Option<String> {
        match self {
            Act::Hunk(Pick::Discard, _) => Some("Discarded a hunk; you can undo it".into()),
            Act::Lines(Pick::Discard, lines) => Some(format!("Discarded {}; you can undo it", count(lines.len(), "line", "lines"))),
            _ => None,
        }
    }
}

impl DiffView {
    /// Offers the picks on this working-tree diff, each written through `gate`.
    pub(crate) fn enable_picks(&mut self, gate: Entity<WriteGate>, repo: Repo, cx: &mut Context<Self>) {
        let written = cx.subscribe(&gate, |this: &mut DiffView, _, w: &Written, cx| this.pick_written(w, cx));
        let observed = cx.observe(&gate, |_, _, cx| cx.notify());
        self.picks = Some(Picks { gate, state: None, menu: None, doing: None, inline_lines: None, _gate: [written, observed] });
        self.read_pick_tools(repo, cx);
    }

    /// The operation under way, off the UI thread once the tab's writer is open.
    pub(super) fn read_pick_tools(&mut self, repo: Repo, cx: &mut Context<Self>) {
        let Some(picks) = &self.picks else { return };
        let opening = picks.gate.update(cx, |g, cx| g.open(&repo, cx));
        let read = cx.background_spawn(async move { opening.await.map(|writer| supraman_gitman::operation_state(&writer.dirs().git_dir)) });
        cx.spawn(async move |this, cx| {
            let read = read.await;
            this.update(cx, |this, cx| {
                let Some(picks) = &mut this.picks else { return };
                if read.is_some() {
                    picks.state = read;
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// What a pick does here: stage and discard on Unstaged, unstage on Staged, for a modified text file's diff; else `None`.
    fn pick_side(&self) -> Option<Of> {
        let text = self.text().filter(|t| t.invisible.is_empty())?;
        let ok = self.picks.is_some() && self.mode == Mode::Diff && self.target.kind() == Some(ChangeKind::Modified) && !text.diff.hunks.is_empty();
        ok.then(|| self.target.of.clone()).filter(|of| matches!(of, Of::Unstaged | Of::Staged))
    }

    /// Why no pick can run now.
    pub(crate) fn picks_blocked(&self, cx: &App) -> Option<&'static str> {
        let picks = self.picks.as_ref()?;
        let Some(state) = picks.state else { return Some("The repository is still opening") };
        if state != OperationState::None {
            return Some("Lines cannot be staged or discarded during a merge or rebase");
        }
        picks.gate.read(cx).busy()
    }

    fn picked(&self, act: &Act) -> Option<Picked> {
        let text = self.text()?;
        let changes: Vec<Change> = text.diff.hunks.iter().map(|h| Change { old: h.old_rows.clone(), new: h.new_rows.clone() }).collect();
        let (path, shown) = (self.target.path.clone(), text.shown.clone());
        Some(match act {
            Act::Hunk(_, ix) => Picked::hunk(path, shown, changes, *ix),
            Act::Lines(_, picked) => {
                let rows = |new: bool| picked.iter().filter(|l| l.new == new).map(|l| l.row).collect();
                Picked { path, shown, changes, old_rows: rows(false), new_rows: rows(true) }
            }
        })
    }

    /// Runs `act` through the tab's gate; says why when it cannot run now.
    pub(crate) fn pick(&mut self, act: Act, cx: &mut Context<Self>) {
        if self.pick_side().is_none() {
            return;
        }
        if let Some(why) = self.picks_blocked(cx) {
            return cx.emit(PickEvent::Why(why.into()));
        }
        let (Some(picked), Some(picks)) = (self.picked(&act), self.picks.as_mut()) else { return };
        let pick = act.pick();
        let locks = if pick == Pick::Discard { LockSet::reset() } else { LockSet::new().write(Resource::Index) };
        let job = move |s: &supraman_gitman::write::Session<'_>| match pick {
            Pick::Stage => lines::stage(s, &picked),
            Pick::Unstage => lines::unstage(s, &picked),
            Pick::Discard => lines::discard(s, &picked),
        };
        picks.menu = None;
        let words = match pick {
            Pick::Stage => "Staging the lines",
            Pick::Unstage => "Unstaging the lines",
            Pick::Discard => "Discarding the lines",
        };
        if let Some(ticket) = picks.gate.update(cx, |g, cx| g.request(locks, words.into(), job, cx)) {
            picks.doing = Some((ticket, act));
        }
        cx.notify();
    }

    fn pick_written(&mut self, w: &Written, cx: &mut Context<Self>) {
        let Some(picks) = &mut self.picks else { return };
        let Some((_, act)) = picks.doing.take_if(|(ticket, _)| *ticket == w.ticket) else { return };
        if w.cancelled.is_some() {
            return;
        }
        match &w.result {
            Ok(()) => cx.emit(PickEvent::Wrote(act.toast())),
            Err(why) => cx.emit(PickEvent::Why(why.clone())),
        }
    }

    /// The editors of the layout shown, each with its pane.
    fn panes(&self, cx: &App) -> Vec<(Pane, Entity<ZedEditor>)> {
        let Some(text) = self.text() else { return Vec::new() };
        match self.layout {
            Layout::Inline => text.inline.iter().map(|e| (Pane::Inline, e.clone())).collect(),
            Layout::Split => text.split.iter().flat_map(|s| {
                let s = s.read(cx);
                [(Pane::Old, s.old.clone()), (Pane::New, s.new.clone())]
            }).collect(),
        }
    }

    /// The inline editor's line of each row, read once per editor.
    fn inline_lines(&mut self, editor: &Entity<ZedEditor>, cx: &App) -> &[Option<(u32, bool)>] {
        let picks = self.picks.as_mut().expect("picks are on");
        if picks.inline_lines.as_ref().is_none_or(|(id, _)| *id != editor.entity_id()) {
            picks.inline_lines = Some((editor.entity_id(), rows::row_lines(editor, cx)));
        }
        &picks.inline_lines.as_ref().expect("just read").1
    }

    fn line_of(&mut self, pane: Pane, editor: &Entity<ZedEditor>, row: u32, cx: &App) -> Option<Line> {
        match pane {
            Pane::Old => Some(Line { new: false, row }),
            Pane::New => Some(Line { new: true, row }),
            Pane::Inline => self.inline_lines(editor, cx).get(row as usize).copied().flatten().map(|(row, removed)| Line { new: !removed, row }),
        }
    }

    /// The index of the hunk that changes `line`.
    fn hunk_of(&self, line: Line) -> Option<usize> {
        self.text()?.diff.hunks.iter().position(|h| if line.new { h.new_rows.contains(&line.row) } else { h.old_rows.contains(&line.row) })
    }

    fn changed(&self, line: Line) -> bool {
        self.hunk_of(line).is_some()
    }

    /// The pane, row and line under `position`.
    fn under(&mut self, position: Point<Pixels>, window: &mut Window, cx: &mut App) -> Option<(Pane, Entity<ZedEditor>, u32, Line)> {
        for (pane, editor) in self.panes(cx) {
            if let Some(row) = rows::row_at(&editor, position, window, cx) {
                let line = self.line_of(pane, &editor, row, cx)?;
                return Some((pane, editor, row, line));
            }
        }
        None
    }

    /// The menu of the changed lines selected in the editor under `position`, else of the changed line under it and its hunk.
    fn open_line_menu(&mut self, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(side) = self.pick_side() else { return };
        let Some((pane, editor, row, line)) = self.under(position, window, cx) else { return };
        let mut selected = Vec::new();
        for r in rows::selected_rows(&editor, cx).filter(|rows| rows.contains(&row)).unwrap_or_default() {
            if let Some(l) = self.line_of(pane, &editor, r, cx).filter(|l| self.changed(*l)) {
                selected.push(l);
            }
        }
        let (hunk, lines) = match (selected.is_empty(), self.hunk_of(line)) {
            (false, _) => (None, selected),
            (true, Some(ix)) => (Some(ix), vec![line]),
            (true, None) => return,
        };
        let title = count(lines.len(), "line", "lines");
        let mut items = line_items(&side, lines, hunk.is_none());
        if let Some(ix) = hunk {
            items.extend(hunk_items(&side, ix));
        }
        if let Some(why) = self.picks_blocked(cx) {
            items = items.into_iter().map(|i| i.dimmed(why)).collect();
        }
        let picks = self.picks.as_mut().expect("picks are on");
        picks.menu = Some(ContextMenu::new(title, position, items));
        cx.notify();
    }

    /// `body` (the diff's editors), whose right-click opens the line menu where lines can be picked.
    pub(super) fn with_picks(&self, body: AnyElement, cx: &mut Context<Self>) -> AnyElement {
        if self.pick_side().is_none() {
            return body;
        }
        div()
            .size_full()
            .capture_any_mouse_down(cx.listener(|this, e: &MouseDownEvent, window, cx| {
                if e.button == MouseButton::Right {
                    cx.stop_propagation();
                    this.open_line_menu(e.position, window, cx);
                }
            }))
            .child(body)
            .into_any_element()
    }

    /// The line menu, while open.
    pub(super) fn line_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.picks.as_ref()?.menu.as_ref()?;
        Some(menu.render(&self.hits, |this: &mut DiffView, act, _, cx| this.pick(act, cx), |this: &mut DiffView, cx| {
            if let Some(p) = &mut this.picks {
                p.menu = None;
            }
            cx.notify();
        }, cx))
    }

    #[cfg(test)]
    pub(crate) fn menu_labels(&self) -> Vec<String> {
        self.picks.as_ref().and_then(|p| p.menu.as_ref()).map(|m| m.items.iter().map(|i| i.label.clone()).collect()).unwrap_or_default()
    }

    #[cfg(test)]
    pub(crate) fn menu_dimmed(&self) -> Vec<Option<String>> {
        self.picks.as_ref().and_then(|p| p.menu.as_ref()).map(|m| m.items.iter().map(|i| i.dimmed.clone()).collect()).unwrap_or_default()
    }
}

impl gpui::EventEmitter<PickEvent> for DiffView {}

/// The items of `lines`: the selection's when `many`, else one line's.
fn line_items(side: &Of, lines: Vec<Line>, many: bool) -> Vec<MenuItem<Act>> {
    let pick = |p: Pick| Act::Lines(p, lines.clone());
    match side {
        Of::Staged => vec![item("line-unstage", if many { "Unstage selection" } else { "Unstage this line" }, pick(Pick::Unstage))],
        _ => vec![
            item("line-stage", if many { "Stage selected lines" } else { "Stage this line" }, pick(Pick::Stage)),
            item("line-discard", if many { "Discard selection" } else { "Discard this line" }, pick(Pick::Discard)),
        ],
    }
}

/// The items of hunk `ix`, a group of their own.
fn hunk_items(side: &Of, ix: usize) -> Vec<MenuItem<Act>> {
    match side {
        Of::Staged => vec![item("hunk-unstage", "Unstage hunk", Act::Hunk(Pick::Unstage, ix)).rule()],
        _ => vec![item("hunk-stage", "Stage hunk", Act::Hunk(Pick::Stage, ix)).rule(), item("hunk-discard", "Discard hunk", Act::Hunk(Pick::Discard, ix))],
    }
}

#[cfg(test)]
mod tests;
