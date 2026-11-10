//! The tab's side of the commit, WIP row and file menus: right-clicks turned into menus, each item sent to its write's owner.

use gpui::{AnyElement, AppContext as _, Context, Entity, MouseButton, MouseDownEvent, Pixels, Point, div, prelude::*};
use supraman_ui::menu::item;

use super::files::{FileAct, FileClick, FileFacts, Listed, covered, items};
use super::{CommitAct, CommitActions, CommitEvent};
use crate::git_view::GitView;
use crate::git_view::branch::menu::{RefAct, Subject};
use crate::git_view::branch::{group_subject, pill_graph_items};
use crate::git_view::wip::Ask as WipAsk;

/// The WIP row's menu.
fn wip_items() -> Vec<supraman_ui::menu::MenuItem<RefAct>> {
    vec![
        item("wip-menu-stage-all", "Stage all changes", RefAct::Commit(CommitAct::StageAll)),
        item("wip-menu-stash-all", "Stash all changes", RefAct::Commit(CommitAct::StashAll)),
        item("wip-menu-discard-all", "Discard all changes", RefAct::Commit(CommitAct::DiscardAll)).rule(),
    ]
}

impl GitView {
    /// Row `ix`'s cell taking the row's right-click: a commit's menu, the WIP row's; a stash row has its own.
    pub(crate) fn commit_row(&self, ix: usize, cell: AnyElement, cx: &mut Context<Self>) -> AnyElement {
        if self.pages.read(cx).rows.get(ix).is_none_or(|r| r.stash().is_some()) {
            return cell;
        }
        div()
            .size_full()
            .on_mouse_down(MouseButton::Right, cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                this.row_menu(ix, e.position, cx);
            }))
            .child(cell)
            .into_any_element()
    }

    /// Row `ix`'s menu at `position`: the WIP row's, else its commit's with its branch groups.
    pub(crate) fn row_menu(&mut self, ix: usize, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(row) = self.pages.read(cx).rows.get(ix).cloned() else { return };
        if row.wip() {
            return self.branch.update(cx, |b, cx| b.show_menu("Work in progress".into(), wip_items(), position, cx));
        }
        let refs = self.branch.read(cx).refs().clone();
        let groups: Vec<_> = row.row.refs.iter().filter_map(|g| group_subject(&refs, g, &row.sha).map(|s| (g.clone(), s))).filter(|(_, s)| matches!(s, Subject::Refs { .. })).collect();
        let graph = match groups.as_slice() {
            [(g, _)] => pill_graph_items(&self.ref_panel, g, cx),
            _ => Vec::new(),
        };
        let (subjects, sha) = (groups.into_iter().map(|(_, s)| s).collect(), row.sha.to_string());
        self.branch.update(cx, |b, cx| b.open_commit_menu(subjects, sha, graph, position, cx));
    }

    /// A list's file or folder right-clicked: its menu, once whether it is in the work tree is read off the UI thread.
    pub(crate) fn file_menu(&mut self, click: FileClick, cx: &mut Context<Self>) {
        let (all, under_way) = match &click.from {
            Listed::Wip(section) => match self.wip_panel.read(cx).shown() {
                Some(shown) => (shown.files(*section).to_vec(), shown.conflicting()),
                None => return,
            },
            Listed::Commit { .. } => match self.panel.read(cx).shown() {
                Some(shown) => (shown.changes.clone(), false),
                None => return,
            },
        };
        let Some(top) = self.gate.read(cx).top() else { return };
        let files = covered(&all, &click.path, click.folder);
        let files = if files.is_empty() && matches!(click.from, Listed::Commit { stash: None, .. }) && !click.folder { vec![unchanged(&click.path)] } else { files };
        let path = top.join(&click.path);
        let exists = cx.background_spawn(async move { path.exists() });
        cx.spawn(async move |this, cx| {
            let exists = exists.await;
            this.update(cx, |this, cx| {
                let facts = FileFacts { click: &click, files, exists, under_way };
                let title = click.path.rsplit('/').next().unwrap_or(&click.path).to_string();
                let menu = items(&facts);
                this.branch.update(cx, |b, cx| b.show_menu(title, menu, click.position, cx));
            })
            .ok();
        })
        .detach();
    }

    /// A menu item sent to the owner of its write.
    pub(crate) fn commit_act(&mut self, act: CommitAct, cx: &mut Context<Self>) {
        let wip = self.wip_panel.clone();
        match act {
            CommitAct::StashAll => self.stash_all(cx),
            CommitAct::StageAll => wip.update(cx, |p, cx| p.stage_all(cx)),
            CommitAct::DiscardAll => {
                self.select_wip(cx);
                wip.update(cx, |p, cx| p.ask(WipAsk::DiscardAll, cx));
            }
            CommitAct::EditMessage(sha) => {
                self.select_sha(&sha, cx);
                self.panel.update(cx, |p, cx| p.edit_when_shown(sha, cx));
            }
            CommitAct::File(FileAct::Stage(files)) => wip.update(cx, |p, cx| p.stage(files, cx)),
            CommitAct::File(FileAct::Unstage(files)) => wip.update(cx, |p, cx| p.unstage(files, cx)),
            CommitAct::File(FileAct::Discard(files)) => wip.update(cx, |p, cx| p.ask(WipAsk::Discard(files), cx)),
            CommitAct::File(FileAct::Take(paths, side)) => wip.update(cx, |p, cx| p.take(paths, side, cx)),
            CommitAct::File(FileAct::Resolve(paths)) => wip.update(cx, |p, cx| p.resolve(Some(paths), false, cx)),
            CommitAct::File(FileAct::Unresolve(paths)) => wip.update(cx, |p, cx| p.unresolve(paths, cx)),
            CommitAct::File(FileAct::Stash(files)) => self.stash_files(files, cx),
            CommitAct::File(FileAct::ApplyStashed { sha, files }) => self.stash.update(cx, |s, cx| s.apply_files(sha, files, cx)),
            act => {
                self.commit.update(cx, |c, cx| c.act(act, cx));
            }
        }
    }

    pub(crate) fn commit_event(&mut self, _: Entity<CommitActions>, e: &CommitEvent, cx: &mut Context<Self>) {
        match e {
            CommitEvent::Wrote(toast) => {
                if let Some(toast) = toast {
                    self.toast.show(toast.clone(), |v: &mut GitView| &mut v.toast, cx);
                }
                self.refresh_after_write(cx);
            }
            CommitEvent::Why(why) => self.toast.show_why(why.clone(), |v: &mut GitView| &mut v.toast, cx),
            CommitEvent::Said(words) => self.toast.show(words.clone(), |v: &mut GitView| &mut v.toast, cx),
        }
    }
}

/// A file of View all files the commit left as it was.
fn unchanged(path: &str) -> supraman_gitman::FileChange {
    supraman_gitman::FileChange { kind: supraman_gitman::ChangeKind::Modified, path: path.to_string(), old_path: None, similarity: None }
}
