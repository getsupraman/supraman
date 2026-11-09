//! The graph's own keys (the shared table moves the selection) and the walk through the panel's files in the
//! viewer: a key moves the selection at once and the panel loads its commit when the key comes up or the
//! selection settles.

use std::time::Duration;

use gpui::{AnyElement, App, Context, KeyBinding, KeyUpEvent, div, prelude::*};

use super::diff::{self, FileStep};
use super::{GitView, Selected};

gpui::actions!(
    git_graph,
    [FirstParent, Child, GoToHead, GoToUpstream, OpenFirstFile, ToggleRefs, FilterRefs, Find, FindHistory, Undo, Redo, StageAll, UnstageAll, FocusMessage, StageSelected, UnstageSelected, StageAllAndCommit, NextField, PreviousField, FetchAll]
);

pub(crate) const KEY_CONTEXT: &str = "GitGraph";
/// The whole tab's: the ref panel's keys work from the graph, the panels and the viewer.
pub(crate) const TAB_CONTEXT: &str = "GitTab";
/// The WIP panel's file lists: S and U act on the selected file there, never in a text field.
pub(crate) const WIP_LIST_CONTEXT: &str = "GitWipList";
/// The commit message's fields and the WIP row's input.
pub(crate) const MESSAGE_CONTEXT: &str = "GitMessage";
/// The commit panel's message field, open to amend: Tab and Shift+Tab move between the summary and the description.
pub(crate) const AMEND_CONTEXT: &str = "GitAmend";
/// The graph's and its table's keys that type or move in the WIP row's field instead.
const GRAPH_KEYS: [&str; 10] = ["h", "shift-h", "j", "k", "pageup", "pagedown", "ctrl-up", "ctrl-down", "shift-up", "shift-down"];
/// How long a selection rests before the panel reads its commit without a key coming up.
pub(crate) const SETTLE: Duration = Duration::from_millis(150);

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
        KeyBinding::new("tab", NextField, Some(AMEND_CONTEXT)),
        KeyBinding::new("shift-tab", PreviousField, Some(AMEND_CONTEXT)),
    ]);
    cx.bind_keys(GRAPH_KEYS.iter().map(|key| KeyBinding::new(key, gpui::NoAction, Some(MESSAGE_CONTEXT))));
    diff::bind_keys(cx);
}

impl GitView {
    fn row_of(&self, sha: &str) -> Option<usize> {
        self.rows.iter().position(|r| r.sha.as_ref() == sha)
    }

    /// The row of `ix`'s first parent: HEAD's commit for the WIP row.
    pub(super) fn first_parent(&self, ix: usize) -> Option<usize> {
        let row = self.rows.get(ix)?;
        let parent = if row.wip() { self.heads.head.as_ref() } else { row.parents.first() };
        self.row_of(parent?)
    }

    /// The row of `ix`'s child: the WIP row for HEAD's commit, else the nearest row above whose first parent
    /// it is, else the nearest that merged it, else the nearest stash made on it.
    pub(super) fn child(&self, ix: usize) -> Option<usize> {
        let row = self.rows.get(ix).filter(|r| !r.wip())?;
        if let Some(wip) = self.wip_row().filter(|_| self.heads.head.as_ref() == Some(&row.sha)) {
            return Some(wip);
        }
        let above = |stash: bool| (0..ix).rev().filter(move |i| self.rows[*i].stash().is_some() == stash);
        above(false)
            .find(|i| self.rows[*i].parents.first() == Some(&row.sha))
            .or_else(|| above(false).find(|i| self.rows[*i].parents.contains(&row.sha)))
            .or_else(|| above(true).find(|i| self.rows[*i].parents.contains(&row.sha)))
    }

    /// Selects the row of `sha` as a key does, or seeks it when its page is not loaded.
    pub(super) fn go_to(&mut self, sha: Option<gpui::SharedString>, cx: &mut Context<Self>) {
        let Some(sha) = sha else { return };
        match self.row_of(&sha) {
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
        match self.selected() {
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
        let worktree = match self.selected() {
            Some(Selected::Worktree(path)) => Some(path),
            _ => None,
        };
        let at = self.diff().and_then(|d| files.iter().position(|f| *f == d.read(cx).target && d.read(cx).worktree == worktree));
        let Some(target) = step.index(files.len(), at).map(|ix| files[ix].clone()) else { return };
        match self.selected() {
            Some(Selected::Wip) => self.wip_panel.update(cx, |p, cx| p.mark_file(&target, cx)),
            Some(Selected::Worktree(_)) => self.worktree_panel.update(cx, |p, cx| p.mark_file(&target, cx)),
            _ => self.panel.update(cx, |p, cx| p.mark_file(&target, cx)),
        }
        self.open_diff_in(worktree, target, cx);
    }

    /// Right on a row: its first file's diff, once the panel shows the row.
    fn open_first_file(&mut self, cx: &mut Context<Self>) {
        self.settled(cx);
        if self.diff.is_none() {
            self.go_file(FileStep::First, cx);
        }
    }

    /// The table under the graph's own keys.
    pub(super) fn render_graph(&mut self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .size_full()
            .key_context(KEY_CONTEXT)
            .on_action(cx.listener(|this, _: &FirstParent, _, cx| {
                let to = this.table.selected().and_then(|ix| this.first_parent(ix));
                this.select_by_key(to, cx)
            }))
            .on_action(cx.listener(|this, _: &Child, _, cx| {
                let to = this.table.selected().and_then(|ix| this.child(ix));
                this.select_by_key(to, cx)
            }))
            .on_action(cx.listener(|this, _: &GoToHead, _, cx| this.go_to(this.heads.head.clone(), cx)))
            .on_action(cx.listener(|this, _: &GoToUpstream, _, cx| this.go_to(this.heads.upstream.clone(), cx)))
            .on_action(cx.listener(|this, _: &OpenFirstFile, _, cx| this.open_first_file(cx)))
            .on_key_up(cx.listener(|this, _: &KeyUpEvent, _, cx| this.settled(cx)))
            .child(self.render_table(cx))
            .into_any_element()
    }
}
