//! The tab's stash writes through its [`WriteGate`] (docs/research/gitkraken-12.6.1.md, "Stashing"); none is undoable.

use supraman_core::text::count;

use gpui::{AppContext as _, ClickEvent, Context, Entity, EventEmitter, Pixels, Point, Render, Subscription, Window, div, prelude::*};
use supraman_gitman::write::stash::{self, Applied};
use supraman_gitman::write::{LockSet, Session, WriteError};
use supraman_gitman::FileChange;
use supraman_ui::button;
use supraman_ui::confirm::Confirm;
use supraman_ui::hit::HitTargets;
use supraman_ui::menu::{ContextMenu, MenuItem, item};
use supraman_ui::text_input::{InputEvent, TextInput};

use super::write_gate::{WriteGate, Writes, Written};
use supraman_core::text::capitalised;

/// A stash as its row names it: its commit and its label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StashRef {
    pub(crate) sha: String,
    pub(crate) label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StashAct {
    Apply,
    Pop,
    Delete,
    EditMessage,
}

/// The stash row's menu, in the ref panel and on the graph: Apply, Pop, Delete, then Edit stash message.
pub(crate) fn menu_items<A>(stash: &StashRef, wrap: impl Fn(StashAct, StashRef) -> A) -> Vec<MenuItem<A>> {
    let act = |a: StashAct| wrap(a, stash.clone());
    vec![
        item("stash-apply", "Apply stash", act(StashAct::Apply)),
        item("stash-pop", "Pop stash", act(StashAct::Pop)),
        item("stash-delete", "Delete stash", act(StashAct::Delete)),
        item("stash-edit", "Edit stash message", act(StashAct::EditMessage)).under("Message"),
    ]
}

/// A write asked about first.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Ask {
    Files { files: Vec<FileChange>, message: String },
    Apply { stash: StashRef, pop: bool },
    Delete(Vec<StashRef>),
}

impl Ask {
    fn question(&self) -> (String, &'static str) {
        match self {
            Ask::Files { files, .. } => (format!("Stash {}?", count(files.len(), "file", "files")), "Stash"),
            Ask::Apply { stash, pop: false } => (format!("Apply \u{201c}{}\u{201d} over your unstaged changes?", stash.label), "Apply"),
            Ask::Apply { stash, pop: true } => (format!("Pop \u{201c}{}\u{201d} over your unstaged changes?", stash.label), "Pop"),
            Ask::Delete(stashes) => match stashes.as_slice() {
                [one] => (format!("Delete the stash \u{201c}{}\u{201d}? It cannot be undone.", one.label), "Delete"),
                many => (format!("Delete {} stashes? It cannot be undone.", many.len()), "Delete"),
            },
        }
    }
}

/// The write under way.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Doing {
    StashAll,
    Files(usize),
    Apply { stash: StashRef, pop: bool },
    ApplyFiles(usize),
    Delete(Vec<StashRef>),
    Message,
}

/// A stash write ran, or did not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StashEvent {
    /// It ran: the toast it says, if any; the tab refreshes.
    Wrote(Option<String>),
    /// Why it did not run, or failed.
    Why(String),
    /// A stash was applied: its message, for an empty commit summary.
    Applied(String),
    /// Everything was stashed with the commit summary: the summary is spent.
    Stashed,
    /// A row's menu chose this.
    Picked(StashAct, StashRef),
}

pub(crate) struct StashActions {
    hits: HitTargets,
    writes: Writes<Ask, Doing, Applied>,
    menu: Option<ContextMenu<(StashAct, StashRef)>>,
    /// The stash whose message is being edited, and its field once a window made it.
    editing: Option<(StashRef, Option<(Entity<TextInput>, Subscription)>)>,
    /// The stash an apply or a pop left conflicted, kept.
    conflicted: Option<StashRef>,
}

impl EventEmitter<StashEvent> for StashActions {}

impl StashActions {
    pub(crate) fn band(&self, cx: &mut Context<Self>) -> Option<gpui::Div> {
        self.writes.band(&self.hits, "git-stash-ask", Self::answer, cx)
    }

    pub(crate) fn new(gate: Entity<WriteGate>, hits: HitTargets, cx: &mut Context<Self>) -> StashActions {
        StashActions { hits, writes: Writes::new(gate, Self::written, cx), menu: None, editing: None, conflicted: None }
    }

    pub(crate) fn busy(&self, cx: &gpui::App) -> Option<&'static str> {
        self.writes.busy(cx)
    }

    #[cfg(test)]
    pub(crate) fn asking(&self) -> Option<&str> {
        self.writes.asked()
    }

    #[cfg(test)]
    pub(crate) fn conflicted(&self) -> Option<&StashRef> {
        self.conflicted.as_ref()
    }

    fn request(&mut self, doing: Doing, job: impl FnOnce(&Session<'_>) -> Result<(), WriteError> + Send + 'static, cx: &mut Context<Self>) {
        if let Err(why) = self.writes.request(doing, LockSet::stash(), job, cx) {
            cx.emit(StashEvent::Why(why.into()));
        }
    }

    fn ask(&mut self, ask: Ask, cx: &mut Context<Self>) {
        let (text, yes) = ask.question();
        let destroys = matches!(ask, Ask::Delete(_));
        let asked = Confirm::ask(text, yes, ask);
        self.writes.ask(if destroys { asked.danger() } else { asked }, cx);
    }

    /// The answer to the question shown; a no writes nothing.
    pub(crate) fn answer(&mut self, yes: bool, cx: &mut Context<Self>) {
        let Some(ask) = self.writes.answer(yes) else { return cx.notify() };
        match ask {
            Ask::Files { files, message } => self.request(Doing::Files(files.len()), move |s| stash::stash_files(s, &files, &message), cx),
            Ask::Apply { stash, pop } => self.run_apply(stash, pop, cx),
            Ask::Delete(stashes) => {
                let shas: Vec<String> = stashes.iter().map(|s| s.sha.clone()).collect();
                self.request(Doing::Delete(stashes), move |s| shas.iter().try_for_each(|sha| stash::delete(s, sha)), cx);
            }
        }
    }

    /// Every change, untracked files too, with `message` (the commit summary, else `WIP on <branch>`).
    pub(crate) fn stash_all(&mut self, message: String, cx: &mut Context<Self>) {
        self.request(Doing::StashAll, move |s| stash::stash_all(s, &message, false), cx);
    }

    /// `files` stashed, asked first.
    pub(crate) fn stash_files(&mut self, files: Vec<FileChange>, message: String, cx: &mut Context<Self>) {
        if !files.is_empty() {
            self.ask(Ask::Files { files, message }, cx);
        }
    }

    /// Applies or pops `stash`, asked first while there are local changes.
    pub(crate) fn apply(&mut self, stash: StashRef, pop: bool, dirty: bool, cx: &mut Context<Self>) {
        if dirty {
            self.ask(Ask::Apply { stash, pop }, cx);
        } else {
            self.run_apply(stash, pop, cx);
        }
    }

    fn run_apply(&mut self, stash: StashRef, pop: bool, cx: &mut Context<Self>) {
        let sha = stash.sha.clone();
        let job = self.writes.keeping(|applied| applied, move |s| stash::apply(s, &sha, pop));
        self.request(Doing::Apply { stash, pop }, job, cx);
    }

    /// `files` of the stash `sha` put in the work tree.
    pub(crate) fn apply_files(&mut self, sha: String, files: Vec<FileChange>, cx: &mut Context<Self>) {
        if !files.is_empty() {
            self.request(Doing::ApplyFiles(files.len()), move |s| stash::apply_files(s, &sha, &files), cx);
        }
    }

    /// Deletes `stashes`, asked first naming the one or counting them.
    pub(crate) fn delete(&mut self, stashes: Vec<StashRef>, cx: &mut Context<Self>) {
        if !stashes.is_empty() {
            self.ask(Ask::Delete(stashes), cx);
        }
    }

    /// Opens the message box of `stash`, its field made on the next frame.
    pub(crate) fn edit_message(&mut self, stash: StashRef, cx: &mut Context<Self>) {
        if self.busy(cx).is_none() {
            self.editing = Some((stash, None));
            cx.notify();
        }
    }

    /// The edited message saved: the same stash stored again on top.
    pub(crate) fn save_message(&mut self, cx: &mut Context<Self>) {
        let Some((stash, Some((input, _)))) = self.editing.take() else { return };
        let message = input.read(cx).text().trim().to_string();
        self.set_message(stash, message, cx);
    }

    /// `stash` stored again on top with `message`; a blank or unchanged one writes nothing.
    pub(crate) fn set_message(&mut self, stash: StashRef, message: String, cx: &mut Context<Self>) {
        if message.is_empty() || message == stash.label {
            return cx.notify();
        }
        self.request(Doing::Message, move |s| stash::set_message(s, &stash.sha, &message), cx);
    }

    #[cfg(test)]
    pub(crate) fn type_message(&mut self, text: &str, cx: &mut Context<Self>) {
        if let Some((_, Some((input, _)))) = &self.editing {
            input.update(cx, |i, cx| i.set_text(text, cx));
        }
    }

    pub(crate) fn cancel_message(&mut self, cx: &mut Context<Self>) {
        self.editing = None;
        cx.notify();
    }

    pub(crate) fn open_menu(&mut self, stash: StashRef, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.menu = Some(ContextMenu { title: stash.label.clone(), position, items: menu_items(&stash, |a, s| (a, s)) });
        cx.notify();
    }

    fn written(&mut self, w: &Written, cx: &mut Context<Self>) {
        let Some(doing) = self.writes.written(w) else { return };
        if let Err(why) = &w.result {
            cx.emit(StashEvent::Wrote(None));
            return cx.emit(StashEvent::Why(capitalised(why)));
        }
        let toast = match doing {
            Doing::StashAll => {
                cx.emit(StashEvent::Stashed);
                Some("Stashed all changes".to_string())
            }
            Doing::Files(n) => Some(format!("Stashed {}", count(n, "file", "files"))),
            Doing::ApplyFiles(n) => Some(format!("Applied {} from the stash", count(n, "file", "files"))),
            Doing::Apply { stash, pop } => return self.applied(stash, pop, cx),
            Doing::Delete(stashes) => Some(match stashes.as_slice() {
                [one] => format!("Deleted the stash \u{201c}{}\u{201d}", one.label),
                many => format!("Deleted {} stashes", many.len()),
            }),
            Doing::Message => Some("Saved the stash's message".into()),
        };
        cx.emit(StashEvent::Wrote(toast));
        cx.notify();
    }

    fn applied(&mut self, stash: StashRef, pop: bool, cx: &mut Context<Self>) {
        let outcome = self.writes.kept();
        let verb = if pop { "Popped" } else { "Applied" };
        if outcome == Some(Applied::Conflicted) {
            let kept = if pop { "; the stash is kept" } else { "" };
            let why = format!("Applying \u{201c}{}\u{201d} left conflicts to resolve{kept}", stash.label);
            self.conflicted = Some(stash);
            cx.emit(StashEvent::Wrote(None));
            cx.emit(StashEvent::Why(why));
        } else {
            cx.emit(StashEvent::Applied(stash.label.clone()));
            cx.emit(StashEvent::Wrote(Some(format!("{verb} \u{201c}{}\u{201d}", stash.label))));
        }
        cx.notify();
    }

    /// The message box's field, made once a window is at hand, with the label to edit selected.
    fn ensure_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((stash, field @ None)) = &mut self.editing else { return };
        let label = stash.label.clone();
        let input = cx.new(|cx| TextInput::new("Stash message", None, window, cx));
        input.update(cx, |i, cx| {
            i.set_text(&label, cx);
            i.focus_all(window, cx);
        });
        let events = cx.subscribe(&input, |this: &mut StashActions, _, e: &InputEvent, cx| {
            if *e == InputEvent::Confirm {
                this.save_message(cx);
            }
        });
        *field = Some((input, events));
    }

    fn render_editing(&self, cx: &mut Context<Self>) -> Option<gpui::Div> {
        let (_, Some((input, _))) = self.editing.as_ref()? else { return None };
        let save = button::bordered(&self.hits, "git-stash-message-save", "Save", cx).on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.save_message(cx)));
        let cancel = button::bordered(&self.hits, "git-stash-message-cancel", "Cancel", cx).on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.cancel_message(cx)));
        Some(div().flex().items_center().gap_2().m_2().child(div().flex_1().min_w_0().child(input.clone())).child(save).child(cancel))
    }
}

impl Render for StashActions {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_field(window, cx);
        let menu = self.menu.as_ref().map(|menu| {
            menu.render(&self.hits, |this: &mut StashActions, (act, stash), _, cx| {
                this.menu = None;
                cx.emit(StashEvent::Picked(act, stash));
                cx.notify();
            }, |this: &mut StashActions, cx| {
                this.menu = None;
                cx.notify();
            }, cx)
        });
        div().children(self.render_editing(cx)).children(menu)
    }
}

#[cfg(test)]
mod tests;
