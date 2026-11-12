//! The tab's ref writes (docs/research/gitkraken-12.6.1.md §4 part A, §B), each through the tab's [`WriteGate`].

mod dialog;
pub(crate) mod menu;
mod view;
pub(crate) mod worktree;

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{AppContext as _, ClipboardItem, Context, Entity, EventEmitter, Pixels, Point, Render, Window, div, prelude::*};
use supraman_gitman::write::branch::{self, CheckedOut, Merged, Plan, Target, relation};
use supraman_gitman::write::rebase::{self, Autostash, Rebased};
use supraman_gitman::write::{LockSet, Session, WriteError};
use supraman_gitman::{Refs, short_sha};
use supraman_ui::confirm::{Answer, Confirm};
use supraman_ui::hit::HitTargets;
use supraman_ui::menu::{ContextMenu, MenuItem};

pub(crate) use dialog::{NameAsk, NameDialog};
pub(crate) use view::DraggedPill;
pub use view::OpenGit;
pub(crate) use view::{group_subject, pill_graph_items};
use menu::{Dragged, Facts, RefAct, Subject};

use super::ref_panel::RefAction;
use super::remote::RemoteFacts;
use super::remote::menu as remote_menu;
use super::write_gate::{STILL_OPENING, WriteGate, Writes, Written};
use supraman_core::paths::folder_name;
use supraman_core::text::capitalised;

/// A ref write ran, or did not, or wants the tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BranchEvent {
    /// It ran: the toast it says, if any; the tab refreshes.
    Wrote(Option<String>),
    /// A worktree's remove started (true) or ended: its rows show it.
    Removing(bool),
    Why(String),
    /// Done without a write: the toast.
    Said(String),
    /// The ref is checked out already: the WIP row.
    SelectWip,
    /// Open the Git tab of this folder.
    OpenFolder(PathBuf),
    /// The ref panel's own item.
    Panel(RefAction),
    /// Name a branch or a light tag at `at`: in its row when the graph shows it, else in the dialog.
    Name(NameAsk),
    /// A commit's, the WIP row's or a file's item.
    Commit(crate::git_view::commit::CommitAct),
    Remote(crate::git_view::remote::RemoteAct),
}

/// A question before a write.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Ask {
    /// The local branch and the remote went apart: reset the local (yes) or a new branch there (alt).
    Clash(Plan),
    /// git refused the checkout over local changes.
    Stash(Plan),
    Delete { name: String, only_here: bool },
    DeleteAnyway(String),
    DeleteTag(String),
    /// A merge into another branch stopped on conflicts: stay (yes) or abandon it (no).
    Away { away: branch::Away, into: String },
    /// `working`: the live sessions whose cwd is in that worktree.
    RemoveWorktree { path: PathBuf, branch: Option<String>, working: Vec<String> },
}

impl Ask {
    fn question(&self) -> Confirm<Ask> {
        let ask = |text: String, yes: &str| Confirm::ask(text, yes, self.clone());
        match self {
            Ask::Clash(Plan::Clash { name, remote, sha }) => ask(format!("{name} and {remote} went apart. Reset {name} to {remote}, or make a new branch at {}?", short_sha(sha)), &format!("Reset {name} to here"))
                .or("New branch here", self.clone()),
            Ask::Clash(_) => Confirm::report("Nothing to decide"),
            Ask::Stash(plan) => ask(format!("Checking out {} would overwrite your local changes. Stash them, check out and put them back?", plan.target()), "Stash and checkout"),
            Ask::Delete { name, .. } => ask(format!("Delete the branch {name}? Undo can bring it back."), "Delete").danger(),
            Ask::DeleteAnyway(name) => ask(format!("{name} has commits on no other branch. Delete it anyway?"), "Delete anyway").danger(),
            Ask::DeleteTag(name) => ask(format!("Delete the tag {name}? It cannot be undone."), "Delete").danger(),
            Ask::Away { away, into } => {
                let kept = away.autostash.as_ref().map_or(String::new(), |s| format!(" Your changes wait in the stash \u{201c}{}\u{201d}.", s.name));
                ask(format!("The merge into {into} stopped on conflicts. Stay on {into} to resolve them, or cancel the merge and go back to {}?{kept}", away.back), &format!("Stay on {into}"))
            }
            Ask::RemoveWorktree { path, branch, working } => {
                let and = branch.as_ref().map_or(String::new(), |b| format!(" and delete the branch {b}"));
                let who = match working.len() {
                    0 => String::new(),
                    1 => format!(" {} is working there.", working[0]),
                    _ => format!(" {} are working there.", working.join(", ")),
                };
                ask(format!("Remove the worktree {}{and}? Changes not committed there are lost.{who}", folder_name(path)), "Remove").danger()
            }
        }
    }
}

/// What a write's job left for the answer.
#[derive(Debug, Clone)]
enum Outcome {
    Checkout(CheckedOut),
    Merged(Merged),
    Rebased(Rebased),
}

/// The write under way, for its toast.
#[derive(Debug, Clone)]
enum Doing {
    Checkout(Plan),
    Created { what: String },
    Merge { source: String, into: String },
    Rebase { branch: String, onto: String },
    Worktree(PathBuf),
    RemoveWorktree(PathBuf),
    Plain(String),
}

pub(crate) struct BranchActions {
    hits: HitTargets,
    writes: Writes<Ask, Doing, Outcome>,
    refs: Arc<Refs>,
    under_way: bool,
    menu: Option<ContextMenu<RefAct>>,
    naming: Option<NameDialog>,
    adding: Option<worktree::AddWorktree>,
    remote: Arc<RemoteFacts>,
}

impl EventEmitter<BranchEvent> for BranchActions {}

/// A ref's words with the autostash's fate.
fn with_stash(done: String, autostash: &Autostash) -> String {
    match autostash {
        Autostash::Kept { name, why } => format!("{done}; your changes stay in the stash \u{201c}{name}\u{201d}: {why}"),
        _ => done,
    }
}

impl BranchActions {
    pub(crate) fn band(&self, cx: &mut Context<Self>) -> Option<gpui::Div> {
        self.writes.band_answer(&self.hits, "git-ref-ask", Self::answer, cx)
    }

    pub(crate) fn new(gate: Entity<WriteGate>, hits: HitTargets, cx: &mut Context<Self>) -> BranchActions {
        BranchActions { hits, writes: Writes::new(gate, Self::written, cx), refs: Arc::new(Refs::empty()), under_way: false, menu: None, naming: None, adding: None, remote: Arc::default() }
    }

    /// The refs the last read found, and whether an operation is under way.
    pub(crate) fn set_refs(&mut self, refs: Arc<Refs>, under_way: bool) {
        self.refs = refs;
        self.under_way = under_way;
    }

    pub(crate) fn refs(&self) -> &Arc<Refs> {
        &self.refs
    }

    /// The remotes and the pull default the remote items read.
    pub(crate) fn set_remote_facts(&mut self, facts: Arc<RemoteFacts>) {
        self.remote = facts;
    }

    pub(crate) fn busy(&self, cx: &gpui::App) -> Option<&'static str> {
        self.writes.busy(cx)
    }

    /// The worktree whose remove runs.
    pub(crate) fn removing(&self) -> Option<&Path> {
        match self.writes.doing()? {
            (_, Doing::RemoveWorktree(path)) => Some(path),
            _ => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn asking(&self) -> Option<&str> {
        self.writes.asked()
    }

    #[cfg(test)]
    pub(crate) fn naming(&self) -> Option<&NameDialog> {
        self.naming.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn menu_labels(&self) -> Vec<String> {
        self.menu.as_ref().map(|m| m.items.iter().map(|i| i.label.clone()).collect()).unwrap_or_default()
    }

    fn request(&mut self, doing: Doing, locks: LockSet, job: impl FnOnce(&Session<'_>) -> Result<(), WriteError> + Send + 'static, cx: &mut Context<Self>) {
        if let Err(why) = self.writes.request(doing, locks, job, cx) {
            cx.emit(BranchEvent::Why(why.into()));
        }
    }

    fn ask(&mut self, ask: Ask, cx: &mut Context<Self>) {
        self.writes.ask(ask.question(), cx);
    }

    /// The answer to the question shown.
    pub(crate) fn answer(&mut self, answer: Answer, cx: &mut Context<Self>) {
        let Some(asked) = self.writes.asking.take() else { return };
        let away = match &asked.action {
            Some(Ask::Away { away, .. }) if answer == Answer::No => Some(away.clone()),
            _ => None,
        };
        if let Some(away) = away {
            self.request(Doing::Plain(format!("Went back to {}", away.back)), LockSet::checkout(), self.writes.keeping(Outcome::Checkout, move |s| branch::abandon(s, &away).map(CheckedOut::Done)), cx);
            return;
        }
        match (asked.take(answer), answer) {
            (Some(Ask::Clash(Plan::Clash { name, remote, sha })), Answer::Yes) => self.run_checkout(Plan::Forward { name, remote, sha }, true, cx),
            (Some(Ask::Clash(Plan::Clash { sha, .. })), Answer::Alt) => cx.emit(BranchEvent::Name(NameAsk::Branch { at: sha })),
            (Some(Ask::Stash(plan)), _) => self.run_checkout(plan, true, cx),
            (Some(Ask::Delete { name, only_here: true }), _) => self.ask(Ask::DeleteAnyway(name), cx),
            (Some(Ask::Delete { name, .. } | Ask::DeleteAnyway(name)), _) => {
                let words = format!("Deleted the branch {name}");
                self.request(Doing::Plain(words), LockSet::branch_delete(), move |s| branch::delete_branch(s, &name), cx);
            }
            (Some(Ask::DeleteTag(name)), _) => {
                let words = format!("Deleted the tag {name}");
                self.request(Doing::Plain(words), LockSet::branch(), move |s| branch::delete_tag(s, &name), cx);
            }
            (Some(Ask::RemoveWorktree { path, branch, .. }), _) => {
                let removing = path.clone();
                self.request(Doing::RemoveWorktree(removing), LockSet::branch_delete(), move |s| branch::remove_worktree(s, &path, branch.as_deref()), cx);
                cx.emit(BranchEvent::Removing(true));
            }
            _ => {}
        }
        cx.notify();
    }

    /// What a menu item asks for.
    pub(crate) fn act(&mut self, act: RefAct, cx: &mut Context<Self>) {
        self.menu = None;
        match act {
            RefAct::Checkout(target) => self.checkout(target, cx),
            RefAct::OpenWorktree(path) => cx.emit(BranchEvent::OpenFolder(path)),
            RefAct::FastForward { branch: b, to } => {
                let words = format!("Fast-forwarded {b} to {to}");
                self.request(Doing::Plain(words), LockSet::checkout(), move |s| branch::fast_forward(s, &b, &to), cx);
            }
            RefAct::Merge { source, into } => self.merge(source, into, cx),
            RefAct::Rebase { branch: b, onto } => {
                let (job_branch, job_onto) = (b.clone(), onto.clone());
                let job = self.writes.keeping(Outcome::Rebased, move |s| rebase::onto(s, &job_branch, &job_onto));
                self.request(Doing::Rebase { branch: b, onto }, LockSet::rebase(), job, cx);
            }
            RefAct::CreateBranch { at } => cx.emit(BranchEvent::Name(NameAsk::Branch { at })),
            RefAct::CreateTag { at, annotated: false } => cx.emit(BranchEvent::Name(NameAsk::Tag { at })),
            RefAct::CreateTag { at, annotated: true } => self.name(NameAsk::Annotated { at }, None, cx),
            RefAct::Annotate { name, at } => self.name(NameAsk::Annotate { name, at }, None, cx),
            RefAct::Rename(from) => self.name(NameAsk::Rename { from }, None, cx),
            RefAct::DeleteBranch(name) => self.delete_branch(name, cx),
            RefAct::DeleteTag(name) => self.ask(Ask::DeleteTag(name), cx),
            RefAct::Copy { text, what } => {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                cx.emit(BranchEvent::Said(format!("Copied {what}")));
            }
            RefAct::RemoveWorktree { path, branch } => {
                let working = self.writes.gate().read(cx).occupancy().who_works_in(&path);
                self.ask(Ask::RemoveWorktree { path, branch, working }, cx)
            }
            RefAct::LockWorktree { path, lock } => {
                let words = format!("{} the worktree {}", if lock { "Locked" } else { "Unlocked" }, path.display());
                self.request(Doing::Plain(words), LockSet::branch(), move |s| branch::lock_worktree(s, &path, lock), cx);
            }
            RefAct::PruneWorktrees => self.request(Doing::Plain("Pruned the worktrees".into()), LockSet::branch(), branch::prune_worktrees, cx),
            RefAct::Panel(action) => cx.emit(BranchEvent::Panel(action)),
            RefAct::Commit(act) => cx.emit(BranchEvent::Commit(act)),
            RefAct::Remote(act) => cx.emit(BranchEvent::Remote(act)),
        }
        cx.notify();
    }

    /// Asks for a name: in the dialog, or with `inline` the field in that commit's row.
    pub(crate) fn name(&mut self, ask: NameAsk, inline: Option<String>, cx: &mut Context<Self>) {
        if self.busy(cx).is_none() {
            self.naming = Some(NameDialog::new(ask, inline));
            cx.notify();
        }
    }

    /// The dialog's name submitted, unless it has a problem.
    pub(crate) fn submit_name(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = &self.naming else { return };
        if dialog.problem(cx).is_some() {
            return cx.notify();
        }
        let named = dialog.named(cx);
        self.naming = None;
        let name = named.name.clone();
        let annotate = matches!(named.ask, NameAsk::Annotate { .. });
        match named.ask {
            NameAsk::Branch { at } => {
                let checkout = named.checkout;
                let plan = Plan::Branch(name.clone());
                let job = self.writes.keeping(Outcome::Checkout, move |s| {
                    branch::create_branch(s, &name, &at)?;
                    if checkout { branch::checkout(s, &plan, false) } else { Ok(CheckedOut::Done(Autostash::None)) }
                });
                let doing = if checkout { Doing::Checkout(Plan::Branch(named.name)) } else { Doing::Created { what: format!("the branch {}", named.name) } };
                self.request(doing, LockSet::checkout(), job, cx);
            }
            NameAsk::Tag { at } => self.request(Doing::Created { what: format!("the tag {name}") }, LockSet::branch(), move |s| branch::create_tag(s, &name, &at, None, false), cx),
            NameAsk::Annotated { at } | NameAsk::Annotate { at, .. } => {
                let (force, message) = (annotate, named.message);
                self.request(Doing::Created { what: format!("the annotated tag {name}") }, LockSet::branch(), move |s| branch::create_tag(s, &name, &at, Some(&message), force), cx);
            }
            NameAsk::Rename { from } => {
                let words = format!("Renamed {from} to {name}");
                self.request(Doing::Plain(words), LockSet::branch(), move |s| branch::rename_branch(s, &from, &name), cx);
            }
        }
    }

    /// A checkout of `target`, decided off the UI thread ([`branch::plan`]).
    pub(crate) fn checkout(&mut self, target: Target, cx: &mut Context<Self>) {
        let (Some(writer), Some(top)) = (self.writes.writer(cx), self.writes.top(cx)) else { return cx.emit(BranchEvent::Why(STILL_OPENING.into())) };
        if self.busy(cx).is_some() {
            return;
        }
        let refs = self.refs.clone();
        let planned = cx.background_spawn(async move { branch::plan(writer.repo(), &refs, &top, &target).map_err(|e| e.to_string()) });
        cx.spawn(async move |this, cx| {
            let plan = planned.await;
            this.update(cx, |this, cx| match plan {
                Ok(plan) => this.follow(plan, cx),
                Err(why) => cx.emit(BranchEvent::Why(capitalised(&why))),
            })
            .ok();
        })
        .detach();
    }

    fn follow(&mut self, plan: Plan, cx: &mut Context<Self>) {
        match plan {
            Plan::Head => cx.emit(BranchEvent::SelectWip),
            Plan::Worktree(path) => cx.emit(BranchEvent::OpenFolder(path)),
            Plan::Clash { .. } => self.ask(Ask::Clash(plan), cx),
            plan => self.run_checkout(plan, false, cx),
        }
    }

    fn run_checkout(&mut self, plan: Plan, stash: bool, cx: &mut Context<Self>) {
        let job_plan = plan.clone();
        let job = self.writes.keeping(Outcome::Checkout, move |s| branch::checkout(s, &job_plan, stash));
        self.request(Doing::Checkout(plan), LockSet::checkout(), job, cx);
    }

    fn merge(&mut self, source: String, into: String, cx: &mut Context<Self>) {
        let (job_source, job_into) = (source.clone(), into.clone());
        let job = self.writes.keeping(Outcome::Merged, move |s| branch::merge(s, &job_source, &job_into));
        self.request(Doing::Merge { source, into }, LockSet::checkout(), job, cx);
    }

    /// Asks before deleting `name`, a second time when it has commits on no other branch and no upstream.
    fn delete_branch(&mut self, name: String, cx: &mut Context<Self>) {
        let Some(writer) = self.writes.writer(cx) else { return cx.emit(BranchEvent::Why(STILL_OPENING.into())) };
        let of = name.clone();
        let read = cx.background_spawn(async move { branch::only_here(writer.repo(), &of).unwrap_or(true) });
        cx.spawn(async move |this, cx| {
            let only_here = read.await;
            this.update(cx, |this, cx| this.ask(Ask::Delete { name, only_here }, cx)).ok();
        })
        .detach();
    }

    fn written(&mut self, w: &Written, cx: &mut Context<Self>) {
        let Some(doing) = self.writes.written(w) else { return };
        if matches!(doing, Doing::RemoveWorktree(_)) {
            cx.emit(BranchEvent::Removing(false));
        }
        let outcome = self.writes.kept();
        if let Err(why) = &w.result {
            cx.emit(BranchEvent::Wrote(None));
            return cx.emit(BranchEvent::Why(capitalised(why)));
        }
        let toast = match (doing, outcome) {
            (Doing::Checkout(plan), Some(Outcome::Checkout(CheckedOut::WouldOverwrite { .. }))) => return self.ask(Ask::Stash(plan), cx),
            (Doing::Checkout(plan), Some(Outcome::Checkout(CheckedOut::Done(stash)))) => with_stash(format!("Checked out {}", plan.target()), &stash),
            (Doing::Merge { source, into }, Some(Outcome::Merged(merged))) => match merged {
                Merged::UpToDate => format!("{into} is already up to date with {source}"),
                Merged::Done(stash) => with_stash(format!("Merged {source} into {into}"), &stash),
                Merged::Conflicted { away: Some(away) } => {
                    cx.emit(BranchEvent::Wrote(None));
                    return self.ask(Ask::Away { away, into }, cx);
                }
                Merged::Conflicted { away: None } => {
                    cx.emit(BranchEvent::Wrote(None));
                    return cx.emit(BranchEvent::Why(format!("Merging {source} into {into} stopped on conflicts: resolve them in the WIP")));
                }
            },
            (Doing::Rebase { branch, onto }, Some(Outcome::Rebased(rebased))) => match rebased {
                Rebased::Done { autostash, .. } => with_stash(format!("Rebased {branch} onto {onto}"), &autostash),
                Rebased::Conflicted => {
                    cx.emit(BranchEvent::Wrote(None));
                    return cx.emit(BranchEvent::Why(format!("Rebasing {branch} onto {onto} stopped on conflicts: resolve them in the WIP")));
                }
            },
            (Doing::Created { what }, _) => format!("Created {what}"),
            (Doing::Worktree(folder), _) => self.added_worktree(&folder),
            (Doing::RemoveWorktree(path), _) => format!("Removed the worktree {}", folder_name(&path)),
            (Doing::Plain(words), Some(Outcome::Checkout(CheckedOut::Done(stash)))) => with_stash(words, &stash),
            (Doing::Plain(words), _) => words,
            (_, _) => String::new(),
        };
        cx.emit(BranchEvent::Wrote(Some(toast).filter(|t| !t.is_empty())));
        cx.notify();
    }

    /// The menu of `subject` at `position` once its relation to HEAD is read; `graph` the ref panel's own items.
    pub(crate) fn open_menu(&mut self, subject: Subject, graph: Vec<MenuItem<RefAct>>, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(writer) = self.writes.writer(cx) else { return cx.emit(BranchEvent::Why(STILL_OPENING.into())) };
        let head = self.refs.head.sha().map(str::to_string);
        let pair = subject.sha().map(str::to_string).zip(head);
        let read = cx.background_spawn(async move { pair.and_then(|(sha, head)| relation(writer.repo(), &sha, &head).ok()) });
        cx.spawn(async move |this, cx| {
            let relation = read.await;
            this.update(cx, |this, cx| {
                let Some(top) = this.writes.top(cx) else { return };
                let refs = this.refs.clone();
                let facts = Facts { refs: &refs, top: &top, relation, under_way: this.under_way, graph };
                let mut built = menu::build(&subject, &facts);
                remote_menu::add(&mut built, &subject, &this.remote);
                this.show_menu(subject.title(), built.items(), position, cx);
            })
            .ok();
        })
        .detach();
    }

    /// The graph's menu of the commit `sha` with its branch groups `subjects`; `graph` the ref panel's items.
    pub(crate) fn open_commit_menu(&mut self, subjects: Vec<Subject>, sha: String, graph: Vec<MenuItem<RefAct>>, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(writer) = self.writes.writer(cx) else { return cx.emit(BranchEvent::Why(STILL_OPENING.into())) };
        let head = self.refs.head.sha().map(str::to_string);
        let at = sha.clone();
        let read = cx.background_spawn(async move {
            let relation = head.and_then(|head| relation(writer.repo(), &at, &head).ok());
            (relation, supraman_gitman::write::commit::parent_count(writer.repo(), &at).unwrap_or(1))
        });
        cx.spawn(async move |this, cx| {
            let (relation, parents) = read.await;
            this.update(cx, |this, cx| {
                let Some(top) = this.writes.top(cx) else { return };
                let refs = this.refs.clone();
                let facts = Facts { refs: &refs, top: &top, relation, under_way: this.under_way, graph };
                let commit = crate::git_view::commit::menu::CommitFacts { sha: &sha, parents, refs: &refs, under_way: this.under_way, remote: &this.remote };
                let items = crate::git_view::commit::menu::row_menu(&subjects, &facts, &commit);
                let title = match subjects.as_slice() {
                    [one] => one.title(),
                    _ => short_sha(&sha).to_string(),
                };
                this.show_menu(title, items, position, cx);
            })
            .ok();
        })
        .detach();
    }

    /// What dropping `dragged` on `target` offers, at `position`; nothing for none.
    pub(crate) fn dropped(&mut self, dragged: Dragged, target: Dragged, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(writer) = self.writes.writer(cx) else { return };
        let (a, b) = (dragged.sha.clone(), target.sha.clone());
        let read = cx.background_spawn(async move { relation(writer.repo(), &a, &b).ok() });
        cx.spawn(async move |this, cx| {
            let Some(relation) = read.await else { return };
            this.update(cx, |this, cx| {
                let Some(top) = this.writes.top(cx) else { return };
                let refs = this.refs.clone();
                let facts = Facts { refs: &refs, top: &top, relation: None, under_way: this.under_way, graph: Vec::new() };
                let mut built = menu::drop_menu(&dragged, &target, relation, &facts);
                remote_menu::drop_items(&mut built, &dragged, &target, &this.remote);
                let items = built.items();
                this.show_menu(format!("{} onto {}", dragged.name, target.name), items, position, cx);
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn show_menu(&mut self, title: String, items: Vec<MenuItem<RefAct>>, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.menu = (!items.is_empty()).then(|| ContextMenu::new(title, position, items));
        cx.notify();
    }
}

impl Render for BranchActions {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_dialog(window, cx);
        let menu = self.menu.as_ref().map(|menu| {
            menu.render(&self.hits, |this: &mut BranchActions, act, _, cx| this.act(act, cx), |this: &mut BranchActions, cx| {
                this.menu = None;
                cx.notify();
            }, cx)
        });
        div().children(self.render_dialog(cx)).children(menu)
    }
}
