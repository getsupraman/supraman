//! The graph's own keys and the walk through the panel's files in the viewer; the panel reads a commit once the selection settles.

use std::time::Duration;

use gpui::{AnyElement, App, Context, KeyBinding, KeyDownEvent, KeyUpEvent, Subscription, div, prelude::*};

use super::diff::{self, FileStep};
use super::{GitView, Selected};

gpui::actions!(
    git_graph,
    [FirstParent, Child, GoToHead, GoToUpstream, OpenFirstFile, ToggleRefs, FilterRefs, Find, FindHistory, Undo, Redo, StageAll, UnstageAll, FocusMessage, StageSelected, UnstageSelected, StageAllAndCommit, FetchAll]
);

pub(crate) const KEY_CONTEXT: &str = "GitGraph";
/// The whole tab's: the ref panel's keys work from the graph, the panels and the viewer.
pub(crate) const TAB_CONTEXT: &str = "GitTab";
/// The WIP panel's file lists: S and U act on the selected file there, never in a text field.
pub(crate) const WIP_LIST_CONTEXT: &str = "GitWipList";
/// The commit message's fields and the WIP row's input.
pub(crate) const MESSAGE_CONTEXT: &str = "GitMessage";
/// The graph's and its table's keys that type or move in the WIP row's field instead.
const GRAPH_KEYS: [&str; 10] = ["h", "shift-h", "j", "k", "pageup", "pagedown", "ctrl-up", "ctrl-down", "shift-up", "shift-down"];
/// How long a selection rests before the panel reads its commit without a key coming up.
pub(crate) const SETTLE: Duration = Duration::from_millis(150);

/// Right pressed before the selected row's panel showed its files: the row, and the watch on its panel.
pub(super) struct Waiting {
    row: Selected,
    _landed: Subscription,
}

pub fn bind_keys(cx: &mut App) {
    let c = Some(KEY_CONTEXT);
    cx.bind_keys([
        KeyBinding::new("shift-down", FirstParent, c),
        KeyBinding::new("shift-up", Child, c),
        KeyBinding::new("h", GoToHead, c),
        KeyBinding::new("shift-h", GoToUpstream, c),
        KeyBinding::new("right", OpenFirstFile, c),
        KeyBinding::new("ctrl-j", ToggleRefs, Some(TAB_CONTEXT)),
        KeyBinding::new("ctrl-alt-f", FilterRefs, Some(TAB_CONTEXT)),
        KeyBinding::new("ctrl-p", Find, Some(TAB_CONTEXT)),
        KeyBinding::new("ctrl-shift-p", Find, Some(TAB_CONTEXT)),
        KeyBinding::new("ctrl-shift-h", FindHistory, Some(TAB_CONTEXT)),
        KeyBinding::new("ctrl-z", Undo, Some(TAB_CONTEXT)),
        KeyBinding::new("ctrl-y", Redo, Some(TAB_CONTEXT)),
        KeyBinding::new("ctrl-shift-z", Redo, Some(TAB_CONTEXT)),
        KeyBinding::new("ctrl-shift-s", StageAll, Some(TAB_CONTEXT)),
        KeyBinding::new("ctrl-shift-u", UnstageAll, Some(TAB_CONTEXT)),
        KeyBinding::new("ctrl-shift-m", FocusMessage, Some(TAB_CONTEXT)),
        KeyBinding::new("ctrl-l", FetchAll, Some(TAB_CONTEXT)),
        KeyBinding::new("s", StageSelected, Some(WIP_LIST_CONTEXT)),
        KeyBinding::new("u", UnstageSelected, Some(WIP_LIST_CONTEXT)),
        KeyBinding::new("ctrl-shift-enter", StageAllAndCommit, Some(MESSAGE_CONTEXT)),
    ]);
    cx.bind_keys(GRAPH_KEYS.iter().map(|key| KeyBinding::new(key, gpui::NoAction, Some(MESSAGE_CONTEXT))));
    diff::bind_keys(cx);
}

impl GitView {
    fn row_of(&self, sha: &str, cx: &App) -> Option<usize> {
        self.pages.read(cx).rows.iter().position(|r| r.sha.as_ref() == sha)
    }

    /// The row of `ix`'s first parent: HEAD's commit for the WIP row.
    pub(super) fn first_parent(&self, ix: usize, cx: &App) -> Option<usize> {
        let row = self.pages.read(cx).rows.get(ix)?;
        let parent = if row.wip() { self.heads.head.as_ref() } else { row.parents.first() };
        self.row_of(parent?, cx)
    }

    /// The row of `ix`'s child: the WIP row for HEAD, else the nearest child, merge or stash made on it.
    pub(super) fn child(&self, ix: usize, cx: &App) -> Option<usize> {
        let rows = &self.pages.read(cx).rows;
        let row = rows.get(ix).filter(|r| !r.wip())?;
        if let Some(wip) = self.wip_row(cx).filter(|_| self.heads.head.as_ref() == Some(&row.sha)) {
            return Some(wip);
        }
        let above = |stash: bool| (0..ix).rev().filter(move |i| rows[*i].stash().is_some() == stash);
        above(false)
            .find(|i| rows[*i].parents.first() == Some(&row.sha))
            .or_else(|| above(false).find(|i| rows[*i].parents.contains(&row.sha)))
            .or_else(|| above(true).find(|i| rows[*i].parents.contains(&row.sha)))
    }

    /// Selects the row of `sha` as a key does, or seeks it when its page is not loaded.
    pub(super) fn go_to(&mut self, sha: Option<gpui::SharedString>, cx: &mut Context<Self>) {
        let Some(sha) = sha else { return };
        match self.row_of(&sha, cx) {
            Some(ix) => self.select_by_key(Some(ix), cx),
            None => self.seek_sha(sha, cx),
        }
    }

    /// Selects row `ix` as a key does: at once, the panel's read deferred.
    fn select_by_key(&mut self, ix: Option<usize>, cx: &mut Context<Self>) {
        let Some(ix) = ix.filter(|ix| self.table.selected() != Some(*ix)) else { return };
        self.table.select(Some(ix));
        self.table.reveal(ix);
        self.stepped(cx);
        cx.notify();
    }

    /// The selection moved by a key: said at once, the panel's read after [`SETTLE`] or the key's release.
    pub(super) fn stepped(&mut self, cx: &mut Context<Self>) {
        self.right_waits = None;
        cx.emit(super::GitEvent::Selected);
        self.settle = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SETTLE).await;
            this.update(cx, |v, cx| v.settled(cx)).ok();
        }));
    }

    /// The panel reads the selection a key left, when one is waiting.
    pub(super) fn settled(&mut self, cx: &mut Context<Self>) {
        if self.settle.take().is_some() {
            self.load_details(cx);
        }
    }

    /// The panel's files when it shows the selected row, in its list's order.
    fn files(&self, cx: &App) -> Vec<diff::Target> {
        match self.selected(cx) {
            Some(Selected::Wip) => self.wip_panel.read(cx).file_order(),
            Some(Selected::Commit(sha)) => self.panel.read(cx).file_order(&sha),
            Some(Selected::Stash(stash)) => self.panel.read(cx).file_order(&stash.sha),
            Some(Selected::Worktree(path)) => self.worktree_panel.read(cx).file_order().filter(|(shown, _)| *shown == path).map(|(_, files)| files).unwrap_or_default(),
            Some(Selected::Agent(_)) | None => Vec::new(),
        }
    }

    /// Opens the file `step` leads to from the one open, selected in the panel's list.
    pub(crate) fn go_file(&mut self, step: FileStep, cx: &mut Context<Self>) {
        let files = self.files(cx);
        let worktree = match self.selected(cx) {
            Some(Selected::Worktree(path)) => Some(path),
            _ => None,
        };
        let at = self.diff(cx).and_then(|d| files.iter().position(|f| *f == d.read(cx).target && d.read(cx).worktree == worktree));
        let Some(target) = step.index(files.len(), at).map(|ix| files[ix].clone()) else { return };
        match self.selected(cx) {
            Some(Selected::Wip) => self.wip_panel.update(cx, |p, cx| p.mark_file(&target, cx)),
            Some(Selected::Worktree(_)) => self.worktree_panel.update(cx, |p, cx| p.mark_file(&target, cx)),
            _ => self.panel.update(cx, |p, cx| p.mark_file(&target, cx)),
        }
        self.open_diff_in(worktree, target, cx);
    }

    /// Whether `row`'s panel is reading it, its files not shown yet.
    fn reading(&self, row: &Selected, cx: &App) -> bool {
        match row {
            Selected::Wip => self.wip_panel.read(cx).reading(),
            Selected::Commit(_) | Selected::Stash(_) => self.panel.read(cx).reading(),
            Selected::Worktree(_) => self.worktree_panel.read(cx).reading(),
            Selected::Agent(_) => false,
        }
    }

    /// Right on a row: its first file's diff, once the panel shows the row's files.
    fn open_first_file(&mut self, cx: &mut Context<Self>) {
        self.settled(cx);
        if self.diff(cx).is_some() {
            return;
        }
        match self.selected(cx).filter(|row| self.files(cx).is_empty() && self.reading(row, cx)) {
            Some(row) => self.wait_for_files(row, cx),
            None => self.go_file(FileStep::First, cx),
        }
    }

    fn wait_for_files(&mut self, row: Selected, cx: &mut Context<Self>) {
        let landed = match &row {
            Selected::Wip => cx.observe(&self.wip_panel, |v, _, cx| v.files_landed(cx)),
            Selected::Commit(_) | Selected::Stash(_) => cx.observe(&self.panel, |v, _, cx| v.files_landed(cx)),
            Selected::Worktree(_) => cx.observe(&self.worktree_panel, |v, _, cx| v.files_landed(cx)),
            Selected::Agent(_) => return,
        };
        self.right_waits = Some(Waiting { row, _landed: landed });
    }

    /// The waiting row's panel changed: its first file opens once its files are shown, while it is still selected and no diff is open.
    fn files_landed(&mut self, cx: &mut Context<Self>) {
        let Some(row) = self.right_waits.as_ref().map(|w| w.row.clone()) else { return };
        if self.selected(cx).as_ref() == Some(&row) && self.reading(&row, cx) {
            return;
        }
        self.right_waits = None;
        if self.selected(cx).as_ref() == Some(&row) && self.diff(cx).is_none() {
            self.go_file(FileStep::First, cx);
        }
    }

    #[cfg(test)]
    pub(crate) fn right_waits(&self) -> bool {
        self.right_waits.is_some()
    }

    /// The table under the graph's own keys.
    pub(super) fn render_graph(&mut self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .size_full()
            .key_context(KEY_CONTEXT)
            .on_action(cx.listener(|this, _: &FirstParent, _, cx| {
                let to = this.table.selected().and_then(|ix| this.first_parent(ix, cx));
                this.select_by_key(to, cx)
            }))
            .on_action(cx.listener(|this, _: &Child, _, cx| {
                let to = this.table.selected().and_then(|ix| this.child(ix, cx));
                this.select_by_key(to, cx)
            }))
            .on_action(cx.listener(|this, _: &GoToHead, _, cx| this.go_to(this.heads.head.clone(), cx)))
            .on_action(cx.listener(|this, _: &GoToUpstream, _, cx| this.go_to(this.heads.upstream.clone(), cx)))
            .on_action(cx.listener(|this, _: &OpenFirstFile, _, cx| this.open_first_file(cx)))
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, _| {
                if matches!(e.keystroke.key.as_str(), "left" | "escape") {
                    this.right_waits = None;
                }
            }))
            .on_key_up(cx.listener(|this, _: &KeyUpEvent, _, cx| this.settled(cx)))
            .child(self.render_table(cx))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests;
