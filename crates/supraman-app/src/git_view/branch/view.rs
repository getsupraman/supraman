//! The tab's side of the ref writes: the ref panel's right-click and double-click and the graph's pills (their
//! menu, double-click and drag onto another pill) turned into [`BranchActions`]' subjects and targets, and its
//! events back in the tab.

use std::path::PathBuf;
use std::sync::Arc;

use gpui::{App, Context, Entity, Pixels, Point, SharedString, Window};
use supraman_gitman::write::branch::Target;
use supraman_gitman::{RefGroup, RefKind, Refs, Status};
use supraman_ui::menu::item;

use super::menu::{Dragged, RefAct, Subject};
use super::{BranchActions, BranchEvent, NameAsk};
use crate::git_view::GitView;
use crate::git_view::ref_panel::{Entry, RefPanel};

/// Asks the workspace to open the Git tab of this folder.
pub struct OpenGit(pub PathBuf);

/// A pill on its way to another: its group and its commit.
#[derive(Debug, Clone)]
pub struct DraggedPill {
    pub group: RefGroup,
    pub sha: SharedString,
}

impl gpui::Render for DraggedPill {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        supraman_ui::tabs::chip(SharedString::from(self.group.name.clone()), cx)
    }
}

/// The local branch of a pill's group, else its remote branches.
fn split(group: &RefGroup) -> (Vec<String>, Vec<String>, Vec<String>) {
    let of = |kinds: &[RefKind]| group.refs.iter().filter(|r| kinds.contains(&r.kind)).map(|r| r.name.clone()).collect::<Vec<_>>();
    (of(&[RefKind::Local, RefKind::Worktree]), of(&[RefKind::Remote]), of(&[RefKind::Tag]))
}

pub fn dragged(group: &RefGroup, sha: &str) -> Option<Dragged> {
    let (locals, remotes, _) = split(group);
    let local = locals.first().is_some();
    let name = locals.into_iter().chain(remotes).next()?;
    Some(Dragged { name, local, sha: sha.to_string() })
}

/// The menu subject of a pill's group at `sha`: its branches, else its tag.
pub fn group_subject(refs: &Refs, group: &RefGroup, sha: &str) -> Option<Subject> {
    let (locals, remotes, tags) = split(group);
    if !locals.is_empty() || !remotes.is_empty() {
        return Some(Subject::Refs { locals, remotes, sha: sha.to_string() });
    }
    let name = tags.into_iter().next()?;
    let annotated = refs.tags.iter().any(|t| t.name == name && t.annotated);
    Some(Subject::Tag { name, sha: sha.to_string(), annotated })
}

/// The menu subject of the ref panel row at `path`.
pub fn entry_subject(refs: &Refs, path: &str, entry: &Entry, top: Option<&std::path::Path>) -> Option<Subject> {
    Some(match entry {
        Entry::Branch { name, sha, .. } => Subject::Refs { locals: vec![name.clone()], remotes: Vec::new(), sha: sha.clone() },
        Entry::Remote { name, sha } => Subject::Refs { locals: Vec::new(), remotes: vec![name.clone()], sha: sha.clone() },
        Entry::Tag { name, commit } => Subject::Tag { name: name.clone(), sha: commit.clone()?, annotated: refs.tags.iter().any(|t| &t.name == name && t.annotated) },
        Entry::Detached { sha } => Subject::Detached { sha: sha.clone() },
        Entry::Worktree { label, main, .. } => {
            let w = refs.worktrees.get(path.strip_prefix("worktrees/")?.parse::<usize>().ok()?)?;
            let current = top.is_some_and(|t| supraman_core::paths::path_key(t) == supraman_core::paths::path_key(&w.path));
            Subject::Worktree { path: w.path.clone(), label: label.clone(), branch: w.branch.clone(), main: *main, current, locked: w.locked, prunable: w.prunable }
        }
        Entry::Stash { .. } => return None,
    })
}

impl GitView {
    /// The refs a read found handed to the ref writes.
    pub(crate) fn hand_refs(&mut self, refs: Arc<Refs>, status: &Status, cx: &mut Context<Self>) {
        let under_way = status.operation.is_some();
        self.branch.update(cx, |b, _| b.set_refs(refs, under_way));
    }

    pub(crate) fn branch_event(&mut self, _: Entity<BranchActions>, e: &BranchEvent, cx: &mut Context<Self>) {
        match e {
            BranchEvent::Wrote(toast) => {
                if let Some(toast) = toast {
                    self.toast.show(toast.clone(), |v: &mut GitView| &mut v.toast, cx);
                }
                self.refresh_after_write(cx);
            }
            BranchEvent::Why(why) => self.toast.show_why(why.clone(), |v: &mut GitView| &mut v.toast, cx),
            BranchEvent::Said(words) => self.toast.show(words.clone(), |v: &mut GitView| &mut v.toast, cx),
            BranchEvent::SelectWip => self.select_wip(cx),
            BranchEvent::OpenFolder(path) => cx.emit(OpenGit(path.clone())),
            BranchEvent::Panel(action) => {
                let action = action.clone();
                self.ref_panel.update(cx, |p, cx| p.act(action, cx));
            }
            BranchEvent::Name(ask) => self.ask_name(ask.clone(), cx),
            BranchEvent::Commit(act) => self.commit_act(act.clone(), cx),
        }
    }

    /// A branch's or a light tag's name in its row's field when the graph shows the commit, else in the dialog.
    fn ask_name(&mut self, ask: NameAsk, cx: &mut Context<Self>) {
        let at = match &ask {
            NameAsk::Branch { at } | NameAsk::Tag { at } => Some(at.clone()),
            _ => None,
        };
        let row = at.as_ref().and_then(|sha| self.rows.iter().position(|r| r.sha.as_ref() == sha.as_str()));
        if let Some(ix) = row {
            self.table.reveal(ix);
        }
        self.branch.update(cx, |b, cx| b.name(ask, row.and(at), cx));
    }

    /// A ref panel row's menu: its writes, with the panel's own Solo and Hide in their group.
    pub(crate) fn ref_menu(&mut self, path: &str, position: Point<Pixels>, cx: &mut Context<Self>) {
        let panel = self.ref_panel.read(cx);
        let Some(entry) = panel.entry(path).cloned() else { return };
        let graph = panel.menu_items(path).into_iter().map(|i| item(i.key, i.label, RefAct::Panel(i.action))).collect();
        let refs = self.branch.read(cx).refs().clone();
        let top = self.branch.read(cx).top();
        let Some(subject) = entry_subject(&refs, path, &entry, top.as_deref()) else { return };
        self.branch.update(cx, |b, cx| b.open_menu(subject, graph, position, cx));
    }

    /// A ref panel row double-clicked: a branch or a remote branch checked out, a worktree's tab opened.
    pub(crate) fn ref_double_clicked(&mut self, path: &str, cx: &mut Context<Self>) {
        let target = match self.ref_panel.read(cx).entry(path).cloned() {
            Some(Entry::Branch { name, .. }) => Target::Branch(name),
            Some(Entry::Remote { name, .. }) => Target::Remote(name),
            Some(entry @ Entry::Worktree { .. }) => {
                let (refs, top) = (self.branch.read(cx).refs().clone(), self.branch.read(cx).top());
                if let Some(Subject::Worktree { path: folder, current: false, .. }) = entry_subject(&refs, path, &entry, top.as_deref()) {
                    cx.emit(OpenGit(folder));
                }
                return;
            }
            _ => return,
        };
        self.branch.update(cx, |b, cx| b.checkout(target, cx));
    }

    /// The graph pill of row `ix`'s group `group` right-clicked.
    pub(crate) fn pill_menu(&mut self, ix: usize, group: usize, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(ix) else { return };
        let Some(g) = row.row.refs.get(group).cloned() else { return };
        let refs = self.branch.read(cx).refs().clone();
        let Some(subject) = group_subject(&refs, &g, &row.sha) else { return };
        let graph = pill_graph_items(&self.ref_panel, &g, cx);
        let sha = row.sha.to_string();
        self.branch.update(cx, |b, cx| b.open_commit_menu(vec![subject], sha, graph, position, cx));
    }

    /// The graph pill of row `ix`'s group `group` double-clicked: its local branch checked out, else its one remote
    /// branch, else a menu of its remote branches; a tag's or HEAD's nothing.
    pub(crate) fn pill_double_clicked(&mut self, ix: usize, group: usize, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(g) = self.rows.get(ix).and_then(|r| r.row.refs.get(group)).cloned() else { return };
        let (locals, remotes, _) = split(&g);
        let target = match (locals.first(), remotes.as_slice()) {
            (Some(local), _) => Target::Branch(local.clone()),
            (None, [one]) => Target::Remote(one.clone()),
            (None, []) => return,
            (None, many) => {
                let items = many.iter().map(|r| item(format!("ref-checkout-{r}"), format!("Checkout {r}"), RefAct::Checkout(Target::Remote(r.clone())))).collect();
                return self.branch.update(cx, |b, cx| b.show_menu(g.name.clone(), items, position, cx));
            }
        };
        self.branch.update(cx, |b, cx| b.checkout(target, cx));
    }

    /// `pill` dropped on row `ix`'s group `group`.
    pub(crate) fn pill_dropped(&mut self, pill: &DraggedPill, ix: usize, group: usize, window: &Window, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(ix) else { return };
        let Some(target) = row.row.refs.get(group).and_then(|g| dragged(g, &row.sha)) else { return };
        let Some(source) = dragged(&pill.group, &pill.sha) else { return };
        let position = window.mouse_position();
        self.branch.update(cx, |b, cx| b.dropped(source, target, position, cx));
    }
}

/// A pill's Solo and Hide: those of its branches' rows in the ref panel.
pub(crate) fn pill_graph_items(panel: &Entity<RefPanel>, group: &RefGroup, cx: &App) -> Vec<supraman_ui::menu::MenuItem<RefAct>> {
    let (locals, remotes, _) = split(group);
    let path = locals.first().map(|l| format!("local/{l}")).or_else(|| remotes.first().map(|r| format!("remote/{r}")));
    path.map(|p| panel.read(cx).menu_items(&p).into_iter().map(|i| item(i.key, i.label, RefAct::Panel(i.action))).collect()).unwrap_or_default()
}
