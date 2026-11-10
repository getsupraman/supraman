//! The viewer's line picks on a working-tree diff of a modified file (docs/research/gitkraken-12.6.1.md,
//! "Staging"): each hunk's buttons, a changed row's + or − on hover, and the right-click menu of the rows under the
//! pointer or selected. Every write goes through the tab's [`WriteGate`]; the tab reads the file again after it.

use supraman_core::text::count;
use gpui::{AnyElement, App, AppContext as _, ClickEvent, Context, Entity, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Point, Subscription, Window, anchored, deferred, div, point, prelude::*, px};
use supraman_editor::{ZedEditor, rows};
use supraman_gitman::write::lines::{self, Change, Pick, Picked};
use supraman_gitman::write::{LockSet, Resource, Writer};
use supraman_gitman::{ChangeKind, OperationState, Repo};
use supraman_ui::button;
use supraman_ui::icon::Icon;
use supraman_ui::menu::{ContextMenu, MenuItem, item};
use supraman_ui::tooltip::Tooltip;
use theme::ActiveTheme as _;

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

/// The changed row under the pointer: its editor, its row there and the line it shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Hover {
    pane: Pane,
    row: u32,
    line: Line,
}

pub(crate) struct Picks {
    gate: Entity<WriteGate>,
    writer: Option<Writer>,
    state: OperationState,
    hover: Option<Hover>,
    menu: Option<ContextMenu<Act>>,
    doing: Option<(u64, Act)>,
    /// The inline editor's line of each row, read once per editor.
    inline_lines: Option<(gpui::EntityId, Vec<Option<(u32, bool)>>)>,
    _gate: [Subscription; 2],
}

impl Picks {
    /// The pointer's row and the open menu belong to the text a reload replaced.
    pub(super) fn forget_rows(&mut self) {
        (self.hover, self.menu, self.inline_lines) = (None, None, None);
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
        self.picks = Some(Picks { gate, writer: None, state: OperationState::None, hover: None, menu: None, doing: None, inline_lines: None, _gate: [written, observed] });
        self.read_pick_tools(repo, cx);
    }

    /// The repo's writer, once, and the operation under way, off the UI thread.
    pub(super) fn read_pick_tools(&mut self, repo: Repo, cx: &mut Context<Self>) {
        let Some(picks) = &self.picks else { return };
        let writer = picks.writer.clone();
        let read = cx.background_spawn(async move {
            let writer = match writer {
                Some(w) => w,
                None => Writer::open(&repo).map_err(|e| e.to_string())?,
            };
            let state = supraman_gitman::operation_state(&writer.dirs().git_dir);
            Ok::<_, String>((writer, state))
        });
        cx.spawn(async move |this, cx| {
            let read = read.await;
            this.update(cx, |this, cx| {
                let Some(picks) = &mut this.picks else { return };
                match read {
                    Ok((writer, state)) => (picks.writer, picks.state) = (Some(writer), state),
                    Err(why) => log::warn!("diff view: cannot open the repository to write: {why}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// What a pick does here: stage and discard on the Unstaged diff, unstage on the Staged one, of a modified
    /// text file shown as a diff; `None` where no lines can be picked.
    fn pick_side(&self) -> Option<Of> {
        let text = self.text().filter(|t| t.invisible.is_empty())?;
        let ok = self.picks.is_some() && self.mode == Mode::Diff && self.target.kind() == Some(ChangeKind::Modified) && !text.diff.hunks.is_empty();
        ok.then(|| self.target.of.clone()).filter(|of| matches!(of, Of::Unstaged | Of::Staged))
    }

    /// Why no pick can run now.
    pub(crate) fn picks_blocked(&self, cx: &App) -> Option<&'static str> {
        let picks = self.picks.as_ref()?;
        if picks.writer.is_none() {
            return Some("The repository is still opening");
        }
        if picks.state != OperationState::None {
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
        let Some(writer) = picks.writer.clone() else { return };
        let pick = act.pick();
        let locks = if pick == Pick::Discard { LockSet::reset() } else { LockSet::new().write(Resource::Index) };
        let job = move |s: &supraman_gitman::write::Session<'_>| match pick {
            Pick::Stage => lines::stage(s, &picked),
            Pick::Unstage => lines::unstage(s, &picked),
            Pick::Discard => lines::discard(s, &picked),
        };
        picks.hover = None;
        picks.menu = None;
        if let Some(ticket) = picks.gate.update(cx, |g, cx| g.request(writer, locks, job, cx)) {
            picks.doing = Some((ticket, act));
        }
        cx.notify();
    }

    fn pick_written(&mut self, w: &Written, cx: &mut Context<Self>) {
        let Some(picks) = &mut self.picks else { return };
        let Some((_, act)) = picks.doing.take_if(|(ticket, _)| *ticket == w.ticket) else { return };
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

    /// The row of `pane` that shows `line`.
    fn row_of(&mut self, pane: Pane, editor: &Entity<ZedEditor>, line: Line, cx: &App) -> Option<u32> {
        match pane {
            Pane::Old | Pane::New => Some(line.row),
            Pane::Inline => self.inline_lines(editor, cx).iter().position(|l| *l == Some((line.row, !line.new))).map(|r| r as u32),
        }
    }

    fn changed(&self, line: Line) -> bool {
        self.text().is_some_and(|t| t.diff.hunks.iter().any(|h| if line.new { h.new_rows.contains(&line.row) } else { h.old_rows.contains(&line.row) }))
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

    fn hover_at(&mut self, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let hover = self.under(position, window, cx).filter(|(.., line)| self.changed(*line)).map(|(pane, _, row, line)| Hover { pane, row, line });
        if let Some(picks) = self.picks.as_mut().filter(|p| p.hover != hover) {
            picks.hover = hover;
            cx.notify();
        }
    }

    /// The menu of the changed lines selected in the editor under `position`, else of the changed line under it.
    fn open_line_menu(&mut self, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(side) = self.pick_side() else { return };
        let Some((pane, editor, row, line)) = self.under(position, window, cx) else { return };
        let mut selected = Vec::new();
        for r in rows::selected_rows(&editor, cx).filter(|rows| rows.contains(&row)).unwrap_or_default() {
            if let Some(l) = self.line_of(pane, &editor, r, cx).filter(|l| self.changed(*l)) {
                selected.push(l);
            }
        }
        let (many, lines) = match (selected.is_empty(), self.changed(line)) {
            (false, _) => (true, selected),
            (true, true) => (false, vec![line]),
            (true, false) => return,
        };
        let pick = |p: Pick| Act::Lines(p, lines.clone());
        let items: Vec<MenuItem<Act>> = match side {
            Of::Staged => vec![item("line-unstage", if many { "Unstage selection" } else { "Unstage this line" }, pick(Pick::Unstage))],
            _ => vec![
                item("line-stage", if many { "Stage selected lines" } else { "Stage this line" }, pick(Pick::Stage)),
                item("line-discard", if many { "Discard selection" } else { "Discard this line" }, pick(Pick::Discard)),
            ],
        };
        let picks = self.picks.as_mut().expect("picks are on");
        picks.menu = Some(ContextMenu { title: count(lines.len(), "line", "lines"), position, items });
        cx.notify();
    }

    /// `body` (the diff's editors) with the picks over it, where lines can be picked.
    pub(super) fn with_picks(&mut self, body: AnyElement, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(side) = self.pick_side() else { return body };
        self.watch_scrolls(window, cx);
        let mut overlays = self.hunk_buttons(&side, window, cx);
        overlays.extend(self.line_button(&side, window, cx));
        div()
            .size_full()
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, window, cx| this.hover_at(e.position, window, cx)))
            .capture_any_mouse_down(cx.listener(|this, e: &MouseDownEvent, window, cx| {
                if e.button == MouseButton::Right {
                    cx.stop_propagation();
                    this.open_line_menu(e.position, window, cx);
                }
            }))
            .child(body)
            .children(overlays)
            .into_any_element()
    }

    /// The tab notified each time an editor of the diff scrolls, so the picks follow their rows.
    fn watch_scrolls(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let panes = self.panes(cx);
        let Some(text) = self.text_mut() else { return };
        if text.watching.len() == panes.len() {
            return;
        }
        let this = cx.entity().downgrade();
        text.watching = panes
            .iter()
            .map(|(_, editor)| {
                let this = this.clone();
                rows::on_scroll(editor, move |cx| {
                    this.update(cx, |_, cx| cx.notify()).ok();
                }, cx)
            })
            .collect();
        cx.on_next_frame(window, |_, _, cx| cx.notify());
    }

    fn hunk_buttons(&mut self, side: &Of, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let panes = self.panes(cx);
        let Some(hunks) = self.text().map(|t| t.diff.hunks.clone()) else { return Vec::new() };
        let editor = |pane: Pane| panes.iter().find(|(p, _)| *p == pane).map(|(_, e)| e.clone());
        let (inline, old, new) = (editor(Pane::Inline), editor(Pane::Old), editor(Pane::New));
        let Some(right) = inline.as_ref().or(new.as_ref()).and_then(|e| rows::right(e, cx)) else { return Vec::new() };
        let blocked = self.picks_blocked(cx);
        let mut out = Vec::new();
        for (ix, h) in hunks.iter().enumerate() {
            let first = if h.new_rows.is_empty() { Line { new: false, row: h.old_rows.start } } else { Line { new: true, row: h.new_rows.start } };
            let (pane, at) = match (&inline, first.new) {
                (Some(e), _) => (Pane::Inline, e.clone()),
                (None, true) => (Pane::New, new.clone().expect("split")),
                (None, false) => (Pane::Old, old.clone().expect("split")),
            };
            let Some(row) = self.row_of(pane, &at, first, cx) else { continue };
            let Some(y) = rows::row_top(&at, row, window, cx) else { continue };
            let buttons = match side {
                Of::Staged => vec![self.hunk_button(ix, "unstage", "Unstage hunk", Act::Hunk(Pick::Unstage, ix), blocked, cx)],
                _ => vec![self.hunk_button(ix, "discard", "Discard hunk", Act::Hunk(Pick::Discard, ix), blocked, cx), self.hunk_button(ix, "stage", "Stage hunk", Act::Hunk(Pick::Stage, ix), blocked, cx)],
            };
            let bar = div().flex().gap_1().occlude().children(buttons);
            out.push(deferred(anchored().anchor(gpui::Anchor::TopRight).position(point(right - px(18.), y)).child(bar)).into_any_element());
        }
        out
    }

    fn hunk_button(&self, ix: usize, what: &str, label: &'static str, act: Act, blocked: Option<&'static str>, cx: &mut Context<Self>) -> AnyElement {
        let key = format!("diff-hunk-{ix}-{what}");
        let button = if what == "discard" {
            button::outline(&self.hits, key, label, cx.theme().status().error, cx).text_color(cx.theme().status().error).bg(cx.theme().colors().editor_background)
        } else {
            button::bordered(&self.hits, key, label, cx)
        };
        self.acting(button, act, blocked, label, cx)
    }

    /// `button` running `act` on a click, dimmed with why while `blocked`.
    fn acting(&self, button: gpui::Stateful<gpui::Div>, act: Act, blocked: Option<&'static str>, tip: &'static str, cx: &mut Context<Self>) -> AnyElement {
        match blocked {
            Some(why) => button::dimmed(button).tooltip(Tooltip::text(why)).into_any_element(),
            None => button.tooltip(Tooltip::text(tip)).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.pick(act.clone(), cx);
            })).into_any_element(),
        }
    }

    fn line_button(&mut self, side: &Of, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let hover = self.picks.as_ref()?.hover?;
        let (_, editor) = self.panes(cx).into_iter().find(|(p, _)| *p == hover.pane)?;
        let y = rows::row_top(&editor, hover.row, window, cx)?;
        let x = rows::left(&editor, cx)?;
        let (key, icon, tip, pick) = match side {
            Of::Staged => ("diff-line-unstage", Icon::Minus, "Unstage this line", Pick::Unstage),
            _ => ("diff-line-stage", Icon::Plus, "Stage this line", Pick::Stage),
        };
        let colors = cx.theme().colors();
        let button = button::icon(&self.hits, key, icon.svg(12., colors.text), px(16.), cx).bg(colors.element_background).border_1().border_color(colors.border).occlude();
        let button = self.acting(button, Act::Lines(pick, vec![hover.line]), self.picks_blocked(cx), tip, cx);
        Some(deferred(anchored().position(point(x + px(2.), y)).child(button)).into_any_element())
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
    pub(crate) fn hovered(&self) -> Option<Line> {
        self.picks.as_ref()?.hover.map(|h| h.line)
    }
}

impl gpui::EventEmitter<PickEvent> for DiffView {}

/// The hit key a test clicks for hunk `ix`'s button `what` (`stage`, `unstage` or `discard`).
#[cfg(test)]
pub(crate) fn hunk_key(ix: usize, what: &str) -> gpui::SharedString {
    format!("diff-hunk-{ix}-{what}").into()
}

#[cfg(test)]
mod tests;
