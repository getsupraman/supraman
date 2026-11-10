//! The commit row's, the WIP row's and the file lists' menus (docs/research/gitkraken-12.6.1.md §B B1, B2, items
//! 8, 9 and 11) and the writes they start that no other owner has: cherry-pick, revert, rebase onto and reset of a
//! commit, a commit's patch, a file restored from a commit, ignored, deleted or opened. Every write runs through the
//! tab's [`WriteGate`] off the UI thread.

pub(crate) mod files;
pub(crate) mod menu;
mod view;

#[cfg(test)]
mod tests;

use supraman_core::text::count;
use std::path::PathBuf;

use gpui::{AppContext as _, ClipboardItem, Context, Entity, EventEmitter, Render, Window, div, prelude::*};
use supraman_gitman::write::commit::{self, Picked, ResetMode};
use supraman_gitman::write::files::{self as file_writes, Ignore};
use supraman_gitman::write::rebase::{self, Rebased};
use supraman_gitman::write::{LockSet, Resource, Session, WriteError, journal};
use supraman_gitman::short_sha;
use supraman_ui::confirm::{Answer, Confirm};
use supraman_ui::hit::HitTargets;

use files::FileAct;

use super::details::reword::kept_words;
use super::write_gate::{STILL_OPENING, WriteGate, Writes, Written, capitalised};

/// What a commit's, the WIP row's or a file's menu item does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CommitAct {
    CherryPick(String),
    Revert(String),
    RebaseOnto { branch: String, sha: String },
    Reset { sha: String, mode: ResetMode },
    /// HEAD's message amended, an older one reworded: the commit panel's field.
    EditMessage(String),
    Patch(String),
    StashAll,
    DiscardAll,
    StageAll,
    File(FileAct),
}

/// A write of this owner ran, or did not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CommitEvent {
    Wrote(Option<String>),
    Why(String),
    Said(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Ask {
    /// Commit the change now (yes) or leave it staged (alt).
    Pick { sha: String, revert: bool },
    /// A rewrite of `branch`, which a remote holds.
    Reset { sha: String, mode: ResetMode, branch: String },
    Rebase { branch: String, sha: String },
    DiscardAll(Vec<String>),
    /// Ignore (yes), or ignore and stop tracking `untrack` (alt).
    Ignore { ignore: Ignore, untrack: Vec<String> },
    Delete(String),
    /// `paths` changed in the work tree too.
    Restore { sha: String, paths: Vec<String> },
}

impl Ask {
    fn question(&self) -> Confirm<Ask> {
        let ask = |text: String, yes: &str| Confirm::ask(text, yes, self.clone());
        match self {
            Ask::Pick { sha, revert } => {
                let verb = if *revert { "Revert" } else { "Cherry-pick" };
                let alt = Ask::Pick { sha: sha.clone(), revert: *revert };
                ask(format!("{verb} {}: commit the changes now?", short_sha(sha)), "Commit").or("Don't commit", alt)
            }
            Ask::Reset { branch, sha, .. } => ask(format!("{branch} was already pushed. Resetting it to {} rewrites it, and the next push must force. Reset anyway?", short_sha(sha)), "Reset"),
            Ask::Rebase { branch, .. } => ask(format!("{branch} was already pushed. Rebasing rewrites it, and the next push must force. Rebase anyway?"), "Rebase"),
            Ask::DiscardAll(paths) => ask(format!("Discard the staged and unstaged changes to {}? You can undo it.", count(paths.len(), "file", "files")), "Discard").danger(),
            Ask::Ignore { ignore, untrack } => {
                let question = ask(format!("Add {} to .gitignore?", ignore.pattern()), "Ignore");
                if untrack.is_empty() { question } else { question.or("Ignore and stop tracking", self.clone()) }
            }
            Ask::Delete(path) => ask(format!("Delete {path}? A file git does not track cannot be brought back."), "Delete").danger(),
            Ask::Restore { paths, sha } => ask(format!("Restoring {} from {} replaces your changes to it. You can undo it.", count(paths.len(), "file", "files"), short_sha(sha)), "Restore"),
        }
    }
}

#[derive(Debug, Clone)]
enum Outcome {
    Picked(Picked),
    Rebased(Rebased),
}

#[derive(Debug, Clone)]
enum Doing {
    Pick { sha: String, revert: bool, commit: bool },
    Rebase { branch: String, sha: String },
    Plain(String),
}

pub(crate) struct CommitActions {
    hits: HitTargets,
    writes: Writes<Ask, Doing, Outcome>,
    /// A patch's commit waiting for the folder picker, which the next frame opens.
    patching: Option<String>,
}

impl EventEmitter<CommitEvent> for CommitActions {}

fn index_write() -> LockSet {
    LockSet::new().write(Resource::Index)
}

impl CommitActions {
    pub(crate) fn band(&self, cx: &mut Context<Self>) -> Option<gpui::Div> {
        self.writes.band_answer(&self.hits, "git-commit-ask", Self::answer, cx)
    }

    pub(crate) fn new(gate: Entity<WriteGate>, hits: HitTargets, cx: &mut Context<Self>) -> CommitActions {
        CommitActions { hits, writes: Writes::new(gate, Self::written, cx), patching: None }
    }

    #[cfg(test)]
    pub(crate) fn asking(&self) -> Option<&str> {
        self.writes.asked()
    }

    fn request(&mut self, doing: Doing, locks: LockSet, job: impl FnOnce(&Session<'_>) -> Result<(), WriteError> + Send + 'static, cx: &mut Context<Self>) {
        if let Err(why) = self.writes.request(doing, locks, job, cx) {
            cx.emit(CommitEvent::Why(why.into()));
        }
    }

    fn ask(&mut self, ask: Ask, cx: &mut Context<Self>) {
        self.writes.ask(ask.question(), cx);
    }

    /// The menu's item `act`; the WIP panel's and the stash's items are the tab's.
    pub(crate) fn act(&mut self, act: CommitAct, cx: &mut Context<Self>) {
        match act {
            CommitAct::CherryPick(sha) => self.ask(Ask::Pick { sha, revert: false }, cx),
            CommitAct::Revert(sha) => self.ask(Ask::Pick { sha, revert: true }, cx),
            CommitAct::RebaseOnto { branch, sha } => self.unless_pushed(Ask::Rebase { branch, sha }, cx),
            CommitAct::Reset { sha, mode } => self.unless_pushed(Ask::Reset { sha, mode, branch: String::new() }, cx),
            CommitAct::Patch(sha) => {
                self.patching = Some(sha);
                cx.notify();
            }
            CommitAct::File(act) => self.file(act, cx),
            CommitAct::EditMessage(_) | CommitAct::StashAll | CommitAct::DiscardAll | CommitAct::StageAll => {}
        }
    }

    /// `ask` first when HEAD's branch is one a remote holds, else its write at once; read off the UI thread.
    fn unless_pushed(&mut self, ask: Ask, cx: &mut Context<Self>) {
        let Some(writer) = self.writes.writer(cx) else { return cx.emit(CommitEvent::Why(STILL_OPENING.into())) };
        let read = cx.background_spawn(async move {
            let branch = supraman_gitman::refs(writer.repo()).ok().and_then(|r| match r.head {
                supraman_gitman::Head::Branch { name, .. } => Some(name),
                supraman_gitman::Head::Detached { .. } => None,
            });
            let pushed = branch.is_some() && supraman_gitman::pushed(writer.repo(), "HEAD").unwrap_or(false);
            (pushed, branch.unwrap_or_default())
        });
        cx.spawn(async move |this, cx| {
            let (pushed, branch) = read.await;
            let ask = match ask {
                Ask::Reset { sha, mode, .. } => Ask::Reset { sha, mode, branch },
                ask => ask,
            };
            this.update(cx, |this, cx| if pushed { this.ask(ask, cx) } else { this.run(ask, Answer::Yes, cx) }).ok();
        })
        .detach();
    }

    /// The answer to the question shown.
    pub(crate) fn answer(&mut self, answer: Answer, cx: &mut Context<Self>) {
        let Some(asked) = self.writes.asking.take() else { return };
        if let Some(ask) = asked.take(answer) {
            self.run(ask, answer, cx);
        }
        cx.notify();
    }

    fn run(&mut self, ask: Ask, answer: Answer, cx: &mut Context<Self>) {
        match ask {
            Ask::Pick { sha, revert } => self.pick(sha, revert, answer == Answer::Yes, cx),
            Ask::Reset { sha, mode, .. } => self.reset(sha, mode, cx),
            Ask::Rebase { branch, sha } => {
                let (b, onto) = (branch.clone(), sha.clone());
                let job = self.writes.keeping(Outcome::Rebased, move |s| rebase::onto(s, &b, &onto));
                self.request(Doing::Rebase { branch, sha }, LockSet::rebase(), job, cx);
            }
            Ask::DiscardAll(paths) => {
                let words = format!("Discarded the changes to {}", count(paths.len(), "file", "files"));
                self.request(Doing::Plain(words), LockSet::reset(), move |s| journal::discard(s, &paths), cx);
            }
            Ask::Ignore { ignore, untrack } => {
                let untrack = if answer == Answer::Alt { untrack } else { Vec::new() };
                let words = format!("Added {} to .gitignore", ignore.pattern());
                self.request(Doing::Plain(words), index_write(), move |s| file_writes::ignore(s, &ignore, &untrack), cx);
            }
            Ask::Delete(path) => self.request(Doing::Plain(format!("Deleted {path}")), index_write(), move |s| file_writes::delete(s, &path), cx),
            Ask::Restore { sha, paths } => {
                let words = format!("Restored {} from {}", count(paths.len(), "file", "files"), short_sha(&sha));
                self.request(Doing::Plain(words), LockSet::reset(), move |s| file_writes::restore(s, &sha, &paths), cx);
            }
        }
    }

    fn pick(&mut self, sha: String, revert: bool, commit: bool, cx: &mut Context<Self>) {
        let job_sha = sha.clone();
        let job = self.writes.keeping(Outcome::Picked, move |s| if revert { commit::revert(s, &job_sha, commit) } else { commit::cherry_pick(s, &job_sha, commit) });
        self.request(Doing::Pick { sha, revert, commit }, LockSet::checkout(), job, cx);
    }

    fn reset(&mut self, sha: String, mode: ResetMode, cx: &mut Context<Self>) {
        let how = match mode {
            ResetMode::Soft => "soft",
            ResetMode::Mixed => "mixed",
            ResetMode::Hard => "hard",
        };
        let words = format!("Reset to {} ({how})", short_sha(&sha));
        self.request(Doing::Plain(words), LockSet::reset(), move |s| commit::reset(s, &sha, mode), cx);
    }

    fn file(&mut self, act: FileAct, cx: &mut Context<Self>) {
        let Some(top) = self.writes.top(cx) else { return cx.emit(CommitEvent::Why(STILL_OPENING.into())) };
        match act {
            FileAct::DiscardAll(paths) => self.ask(Ask::DiscardAll(paths), cx),
            FileAct::Ignore { ignore, untrack } => self.ask(Ask::Ignore { ignore, untrack }, cx),
            FileAct::Delete(path) => self.ask(Ask::Delete(path), cx),
            FileAct::Restore { sha, paths } => self.restore(sha, paths, cx),
            FileAct::Open(path) => self.open(top.join(&path), cx),
            FileAct::ShowInFolder(path) => {
                let file = top.join(&path);
                self.open(file.parent().map_or(top, PathBuf::from), cx);
            }
            FileAct::CopyPath(path) => {
                cx.write_to_clipboard(ClipboardItem::new_string(top.join(&path).display().to_string()));
                cx.emit(CommitEvent::Said("Copied the path".into()));
            }
            _ => {}
        }
    }

    /// Asks first when the work tree changed one of `paths`, read off the UI thread.
    fn restore(&mut self, sha: String, paths: Vec<String>, cx: &mut Context<Self>) {
        let Some(writer) = self.writes.writer(cx) else { return };
        let wanted = paths.clone();
        let read = cx.background_spawn(async move {
            supraman_gitman::status(writer.repo()).map(|s| s.entries.iter().any(|e| wanted.contains(&e.path))).unwrap_or(true)
        });
        cx.spawn(async move |this, cx| {
            let changed = read.await;
            this.update(cx, |this, cx| {
                let ask = Ask::Restore { sha, paths };
                if changed { this.ask(ask, cx) } else { this.run(ask, Answer::Yes, cx) }
            })
            .ok();
        })
        .detach();
    }

    /// Opens `path` with its handler off the UI thread, unless [`supraman_ui::open::document`] refuses it.
    fn open(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let checked = cx.background_spawn(async move { supraman_ui::open::document(&path).and_then(|()| supraman_ui::open::open(path.as_os_str())) });
        cx.spawn(async move |this, cx| {
            if let Err(why) = checked.await {
                this.update(cx, |_, cx| cx.emit(CommitEvent::Why(why))).ok();
            }
        })
        .detach();
    }

    /// `sha`'s patch written into `folder`, off the UI thread.
    fn save_patch(&mut self, sha: String, folder: PathBuf, cx: &mut Context<Self>) {
        let Some(writer) = self.writes.writer(cx) else { return };
        let saved = cx.background_spawn(async move { commit::format_patch(writer.repo(), &sha, &folder).map_err(|e| e.to_string()) });
        cx.spawn(async move |this, cx| {
            let saved = saved.await;
            this.update(cx, |_, cx| match saved {
                Ok(file) => cx.emit(CommitEvent::Said(format!("Saved the patch as {}", file.display()))),
                Err(why) => cx.emit(CommitEvent::Why(capitalised(&why))),
            })
            .ok();
        })
        .detach();
    }

    fn written(&mut self, w: &Written, cx: &mut Context<Self>) {
        let Some(doing) = self.writes.written(w) else { return };
        let outcome = self.writes.kept();
        if let Err(why) = &w.result {
            cx.emit(CommitEvent::Wrote(None));
            return cx.emit(CommitEvent::Why(capitalised(why)));
        }
        let (toast, why) = said(doing, outcome);
        cx.emit(CommitEvent::Wrote(toast));
        if let Some(why) = why {
            cx.emit(CommitEvent::Why(why));
        }
        cx.notify();
    }
}

/// A finished write's toast, or why it stopped.
fn said(doing: Doing, outcome: Option<Outcome>) -> (Option<String>, Option<String>) {
    match (doing, outcome) {
        (Doing::Pick { sha, revert, commit }, Some(Outcome::Picked(picked))) => {
            let (done, noun) = if revert { ("Reverted", "revert") } else { ("Cherry-picked", "cherry-pick") };
            match picked {
                Picked::Done(stash) => {
                    let words = if commit { format!("{done} {}", short_sha(&sha)) } else { format!("{done} {} into the work in progress, not committed", short_sha(&sha)) };
                    (Some(kept_words(&stash).map_or(words.clone(), |kept| format!("{words}. {kept}"))), None)
                }
                Picked::Conflicted(kept) => {
                    let waits = kept.map_or(String::new(), |s| format!(" Your changes wait in the stash \u{201c}{}\u{201d}.", s.name));
                    (None, Some(format!("The {noun} of {} stopped on conflicts: resolve them in the WIP.{waits}", short_sha(&sha))))
                }
            }
        }
        (Doing::Rebase { branch, sha }, Some(Outcome::Rebased(rebased))) => match rebased {
            Rebased::Done { autostash, .. } => {
                let words = format!("Rebased {branch} onto {}", short_sha(&sha));
                (Some(kept_words(&autostash).map_or(words.clone(), |kept| format!("{words}. {kept}"))), None)
            }
            Rebased::Conflicted => (None, Some(format!("Rebasing {branch} stopped on conflicts: resolve them in the WIP"))),
        },
        (Doing::Plain(words), _) => (Some(words), None),
        _ => (None, None),
    }
}

impl Render for CommitActions {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(sha) = self.patching.take() {
            supraman_ui::folder_picker::pick_folder("Save the patch in", window, cx, move |this: &mut CommitActions, folder, _, cx| this.save_patch(sha, folder, cx));
        }
        div()
    }
}
