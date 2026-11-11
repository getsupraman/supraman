//! The message box edited in place: HEAD's amended, an older commit's reworded by GitMan's rebase, or a stash's saved.

use supraman_core::text::count;
use std::sync::{Arc, Mutex, PoisonError};

use gpui::{App, ClickEvent, Context, Div, Hsla, Stateful, Subscription, Window, div, prelude::*};
use supraman_gitman::Amendable;
use supraman_gitman::write::rebase::{self, Autostash, Rebased};
use supraman_gitman::write::{LockSet, Session, WriteError, wip};
use supraman_ui::confirm::Confirm;
use supraman_ui::hit::hit_target;
use supraman_ui::text_input::{InputEvent, TextInput};
use supraman_ui::tooltip::Tooltip;
use supraman_ui::button;
use theme::ActiveTheme as _;

use super::{CommitPanel, PanelEvent, Shown};
use crate::git_view::keys::{AMEND_CONTEXT, NextField, PreviousField};
use crate::git_view::stash::StashRef;
use crate::git_view::wip::compose::{failure_banner, message_fields, summary_row};
use crate::git_view::write_gate::{STILL_OPENING, Written};
use supraman_core::text::capitalised;

pub(crate) const CLICK_TO_AMEND: &str = "Click to amend your commit message";
pub(crate) const CLICK_TO_EDIT_STASH: &str = "Click to edit the stash message";
pub(crate) const CHECK_OUT_A_BRANCH: &str = "Check out a branch to amend";
pub(crate) const CLICK_TO_REWORD: &str = "Click to reword your commit message";
pub(crate) const CHECK_OUT_TO_REWORD: &str = "Check out a branch to reword: HEAD is detached";
pub(crate) const STOPPED: &str = "The rebase stopped on a conflict: resolve it, then Continue rebase, Skip commit or Abort rebase";
pub(crate) const TYPE_A_SUMMARY: &str = "Type a commit summary";
pub(crate) const EDIT_KEY: &str = "commit-message-edit";
pub(crate) const UPDATE_KEY: &str = "commit-amend-update";
pub(crate) const CANCEL_KEY: &str = "commit-amend-cancel";
pub(crate) const ASK_KEY: &str = "commit-amend-ask";
pub(crate) const REBASES_KEY: &str = "commit-reword-rebases";

/// What the open field edits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Target {
    Head { branch: String },
    /// An older commit of `branch`, the `above` commits over it rebased.
    Older { sha: String, branch: String, above: usize },
    Stash(StashRef),
}

pub(crate) struct Field {
    pub(crate) target: Target,
    pub(crate) summary: gpui::Entity<TextInput>,
    pub(crate) description: gpui::Entity<TextInput>,
    _events: [Subscription; 2],
}

#[derive(Default)]
pub(crate) struct Reword {
    pub(crate) field: Option<Field>,
    /// The commit whose message field opens once the panel shows it.
    pub(crate) edit_next: Option<String>,
    asking: Option<Confirm<String>>,
    failure: Option<String>,
    /// Whether a remote holds the field's commit is being read before Update Message asks or writes.
    checking: bool,
    /// The amend under way: its gate ticket.
    doing: Option<u64>,
    /// HEAD's sha after the amend, as the job left it.
    reworded: Arc<Mutex<Option<String>>>,
    /// How an older commit's reword ended, as the job left it.
    rebased: Arc<Mutex<Option<Rebased>>>,
}

/// The muted line the read-only box shows, and what a click on it edits.
pub(crate) fn offer(shown: &Shown) -> Option<(&'static str, Option<Target>)> {
    if let Some(stash) = &shown.stash {
        return Some((CLICK_TO_EDIT_STASH, Some(Target::Stash(StashRef { sha: stash.sha.clone(), label: shown.summary.to_string() }))));
    }
    match shown.amendable.as_ref()? {
        Amendable::No => None,
        Amendable::Detached => Some((CHECK_OUT_A_BRANCH, None)),
        Amendable::Branch { name } => Some((CLICK_TO_AMEND, Some(Target::Head { branch: name.clone() }))),
        Amendable::OlderDetached => Some((CHECK_OUT_TO_REWORD, None)),
        Amendable::Older { name, above } => Some((CLICK_TO_REWORD, Some(Target::Older { sha: shown.sha.to_string(), branch: name.clone(), above: *above }))),
    }
}

/// What a reword of a commit with `above` commits over it does.
pub(crate) fn rebases_words(above: usize) -> String {
    format!("Rewords this commit and rebases the {} above it", count(above, "commit", "commits"))
}

/// The question before an older commit's reword: once pushed, or with work in progress to stash; none else.
pub(crate) fn reword_question(branch: &str, above: usize, pushed: bool, dirty: bool) -> Option<String> {
    let stashed = "Your changes are stashed and put back afterwards.";
    match (pushed, dirty) {
        (true, _) => {
            let pushed = format!("{branch} was already pushed. Rewording rewrites {} and the next push must force.", count(above + 1, "commit", "commits"));
            Some(if dirty { format!("{pushed} {stashed} Reword anyway?") } else { format!("{pushed} Reword anyway?") })
        }
        (false, true) => Some(format!("Reword it? {stashed}")),
        (false, false) => None,
    }
}

/// A toast's words when GitMan's autostash could not be put back.
pub(crate) fn kept_words(autostash: &Autostash) -> Option<String> {
    match autostash {
        Autostash::Kept { name, why } => Some(format!("Your changes stay in the stash \u{201c}{name}\u{201d}: {why}")),
        Autostash::None | Autostash::Popped => None,
    }
}

pub(crate) fn pushed_question(branch: &str) -> String {
    format!("{branch} was already pushed. Amending rewrites it, and the next push must force. Amend anyway?")
}

impl CommitPanel {
    /// The field of `sha`'s message, opened once the panel shows it.
    pub(crate) fn edit_when_shown(&mut self, sha: String, cx: &mut Context<Self>) {
        self.reword.edit_next = Some(sha);
        cx.notify();
    }

    /// The field asked for by [`CommitPanel::edit_when_shown`], once its commit is shown.
    pub(super) fn open_asked_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sha) = self.reword.edit_next.clone() else { return };
        let read = |s: &Shown| s.stash.is_some() || s.amendable.is_some();
        if self.shown().is_some_and(|s| s.sha.as_ref() == sha && read(&s)) && !self.stale {
            self.reword.edit_next = None;
            self.open_field(window, cx);
        }
    }

    /// The shown message in the field, the summary focused.
    pub(crate) fn open_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(shown) = self.shown() else { return };
        let Some((_, Some(target))) = offer(&shown) else { return };
        let body = if matches!(target, Target::Stash(_)) { String::new() } else { shown.body.to_string() };
        let (summary, description) = message_fields(window, cx);
        summary.update(cx, |i, cx| {
            i.set_text(&shown.summary, cx);
            i.focus(window, cx);
        });
        description.update(cx, |i, cx| i.set_text(&body, cx));
        let events = [
            cx.subscribe_in(&summary, window, |this: &mut Self, _, e: &InputEvent, window, cx| this.field_event(e, true, window, cx)),
            cx.subscribe_in(&description, window, |this: &mut Self, _, e: &InputEvent, window, cx| this.field_event(e, false, window, cx)),
        ];
        self.reword.field = Some(Field { target, summary, description, _events: events });
        self.reword.failure = None;
        cx.notify();
    }

    fn field_event(&mut self, e: &InputEvent, in_summary: bool, window: &mut Window, cx: &mut Context<Self>) {
        match e {
            InputEvent::Edited => cx.notify(),
            InputEvent::Confirm if in_summary => self.other_field(window, cx),
            InputEvent::Submit => self.update_message(cx),
            InputEvent::Cancel => self.cancel_amend(cx),
            _ => {}
        }
    }

    /// The keyboard to the field it is not in; a stash's message has only one.
    fn other_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(field) = self.reword.field.as_ref().filter(|f| !matches!(f.target, Target::Stash(_))) else { return };
        let to = if field.summary.read(cx).is_focused(window) { field.description.clone() } else { field.summary.clone() };
        to.update(cx, |i, cx| i.focus(window, cx));
    }

    pub(crate) fn cancel_amend(&mut self, cx: &mut Context<Self>) {
        self.reword.field = None;
        self.reword.asking = None;
        self.reword.failure = None;
        self.reword.checking = false;
        cx.notify();
    }

    /// Why Update Message cannot run now: a blank summary, or the gate's write.
    pub(crate) fn update_why_not(&self, cx: &App) -> Option<&'static str> {
        let field = self.reword.field.as_ref()?;
        if field.summary.read(cx).text().trim().is_empty() {
            return Some(TYPE_A_SUMMARY);
        }
        self.gate.read(cx).busy()
    }

    /// Update Message: a stash's saved by the tab's stash writes; HEAD's amended, asked first once pushed.
    pub(crate) fn update_message(&mut self, cx: &mut Context<Self>) {
        let Some(field) = &self.reword.field else { return };
        if let Some(why) = self.update_why_not(cx) {
            return cx.emit(PanelEvent::Why(why.into()));
        }
        let (summary, description) = (field.summary.read(cx).text().to_string(), field.description.read(cx).text().to_string());
        match field.target.clone() {
            Target::Stash(stash) => {
                self.reword.field = None;
                let message = summary.trim().to_string();
                if message != stash.label {
                    cx.emit(PanelEvent::StashMessage(stash, message));
                }
            }
            Target::Head { branch } => {
                let message = wip::message(&summary, &description);
                self.when_pushed("HEAD".into(), cx, move |this, pushed, cx| match pushed {
                    true => this.reword.asking = Some(Confirm::ask(pushed_question(&branch), "Amend anyway", message)),
                    false => cx.emit(PanelEvent::Amend(message)),
                });
            }
            Target::Older { .. } => cx.emit(PanelEvent::Reword(wip::message(&summary, &description))),
        }
        cx.notify();
    }

    /// An older commit's reword to `message`, asked first when pushed or `dirty` (the tab has work in progress).
    pub(crate) fn reword_older(&mut self, message: String, dirty: bool, cx: &mut Context<Self>) {
        let Some(Target::Older { sha, branch, above }) = self.reword.field.as_ref().map(|f| f.target.clone()) else { return };
        self.when_pushed(sha, cx, move |this, pushed, cx| match reword_question(&branch, above, pushed, dirty) {
            Some(question) => this.reword.asking = Some(Confirm::ask(question, "Reword", message)),
            None => this.run_reword(message, cx),
        });
    }

    /// `then` with whether a remote-tracking branch holds `rev`, read off the UI thread; dropped once the field closed or moved on.
    fn when_pushed(&mut self, rev: String, cx: &mut Context<Self>, then: impl FnOnce(&mut Self, bool, &mut Context<Self>) + 'static) {
        let (Some(repo), Some(target)) = (self.repo.clone(), self.reword.field.as_ref().map(|f| f.target.clone())) else { return };
        if std::mem::replace(&mut self.reword.checking, true) {
            return;
        }
        let read = cx.background_spawn(async move {
            supraman_gitman::pushed(&repo, &rev).unwrap_or_else(|e| {
                log::warn!("git tab: cannot tell whether a remote holds {rev}: {e}");
                true
            })
        });
        cx.spawn(async move |this, cx| {
            let pushed = read.await;
            this.update(cx, |this, cx| {
                let same = this.reword.field.as_ref().is_some_and(|f| f.target == target);
                if std::mem::take(&mut this.reword.checking) && same {
                    then(this, pushed, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn answer_amend(&mut self, yes: bool, cx: &mut Context<Self>) {
        let older = self.reword.field.as_ref().is_some_and(|f| matches!(f.target, Target::Older { .. }));
        match self.reword.asking.take().and_then(|c| c.action).filter(|_| yes) {
            Some(message) if older => self.run_reword(message, cx),
            Some(message) => cx.emit(PanelEvent::Amend(message)),
            None => cx.notify(),
        }
    }

    /// GitMan's rebase rewording the field's older commit, through the tab's gate.
    fn run_reword(&mut self, message: String, cx: &mut Context<Self>) {
        let Some(Target::Older { sha, .. }) = self.reword.field.as_ref().map(|f| f.target.clone()) else { return };
        if self.gate.read(cx).writer().is_none() {
            return cx.emit(PanelEvent::Why(STILL_OPENING.into()));
        }
        let rebased = self.reword.rebased.clone();
        let job = move |s: &Session<'_>| -> Result<(), WriteError> {
            let outcome = rebase::reword(s, &sha, &message)?;
            *rebased.lock().unwrap_or_else(PoisonError::into_inner) = Some(outcome);
            Ok(())
        };
        if let Some(ticket) = self.gate.update(cx, |g, cx| g.request(LockSet::rebase(), job, cx)) {
            self.reword.doing = Some(ticket);
        }
        cx.notify();
    }

    #[cfg(test)]
    pub(crate) fn amend_asking(&self) -> Option<&str> {
        self.reword.asking.as_ref().map(|c| c.text.as_str())
    }

    #[cfg(test)]
    pub(crate) fn amend_failure(&self) -> Option<&str> {
        self.reword.failure.as_deref()
    }

    pub(crate) fn field(&self) -> Option<&Field> {
        self.reword.field.as_ref()
    }

    /// HEAD's message amended to `message` through the tab's gate; `no_verify` is the commit area's Skip hooks.
    pub(crate) fn amend(&mut self, message: String, no_verify: bool, cx: &mut Context<Self>) {
        if self.gate.read(cx).writer().is_none() {
            return cx.emit(PanelEvent::Why(STILL_OPENING.into()));
        }
        let reworded = self.reword.reworded.clone();
        let job = move |s: &Session<'_>| -> Result<(), WriteError> {
            let sha = wip::reword_head(s, &message, no_verify)?;
            *reworded.lock().unwrap_or_else(PoisonError::into_inner) = Some(sha);
            Ok(())
        };
        if let Some(ticket) = self.gate.update(cx, |g, cx| g.request(LockSet::commit(), job, cx)) {
            self.reword.doing = Some(ticket);
        }
        cx.notify();
    }

    pub(super) fn written(&mut self, w: &Written, cx: &mut Context<Self>) {
        if self.reword.doing.take_if(|t| *t == w.ticket).is_none() {
            return;
        }
        match &w.result {
            Err(why) => self.reword.failure = Some(capitalised(why)),
            Ok(()) => {
                let summary = self.reword.field.take().map(|f| f.summary.read(cx).text().trim().to_string()).unwrap_or_default();
                self.reword.failure = None;
                let rebased = self.reword.rebased.lock().unwrap_or_else(PoisonError::into_inner).take();
                match rebased {
                    None => {
                        let sha = self.reword.reworded.lock().unwrap_or_else(PoisonError::into_inner).take().unwrap_or_default();
                        cx.emit(PanelEvent::HeadMoved(sha.into(), Some(format!("Amended \u{201c}{summary}\u{201d}"))));
                    }
                    Some(Rebased::Done { head, autostash }) => {
                        let toast = [Some(format!("Reworded \u{201c}{summary}\u{201d}")), kept_words(&autostash)].into_iter().flatten().collect::<Vec<_>>().join(". ");
                        cx.emit(PanelEvent::HeadMoved(head.into(), Some(toast)));
                    }
                    Some(Rebased::Conflicted) => cx.emit(PanelEvent::Wrote(Some(STOPPED.into()))),
                }
            }
        }
        cx.notify();
    }

    /// The read-only box's muted line, and its click where it edits.
    pub(super) fn offer_line(&self, shown: &Shown, boxed: Stateful<Div>, cx: &mut Context<Self>) -> Stateful<Div> {
        let Some((words, target)) = offer(shown) else { return boxed };
        let line = div().absolute().bottom_1().left_2().text_xs().text_color(cx.theme().colors().text_muted).child(words);
        if target.is_none() {
            return boxed.child(line);
        }
        boxed
            .group(EDIT_KEY)
            .cursor_pointer()
            .child(line.invisible().group_hover(EDIT_KEY, |s| s.visible()))
            .child(hit_target(&self.hits, EDIT_KEY.into()))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.open_field(window, cx)))
    }

    /// The open field in the box's place: the summary with its count, and HEAD's description.
    pub(super) fn field_box(&self, field: &Field, box_bg: Hsla, cx: &mut Context<Self>) -> Stateful<Div> {
        let colors = cx.theme().colors().clone();
        div()
            .id("commit-amend")
            .key_context(AMEND_CONTEXT)
            .on_action(cx.listener(|this, _: &NextField, window, cx| this.other_field(window, cx)))
            .on_action(cx.listener(|this, _: &PreviousField, window, cx| this.other_field(window, cx)))
            .size_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .rounded_md()
            .border_1()
            .border_color(colors.border_focused)
            .bg(box_bg)
            .child(summary_row(&self.hits, &field.summary, "commit-amend-counter", cx))
            .when(!matches!(field.target, Target::Stash(_)), |d| d.child(field.description.clone()))
    }

    pub(crate) fn band(&self, cx: &mut Context<Self>) -> Option<Div> {
        self.reword.asking.as_ref().map(|asked| asked.render_band(&self.hits, ASK_KEY, |this: &mut Self, yes, _, cx| this.answer_amend(yes, cx), cx))
    }

    /// Under the open field: a failure, what a reword rebases, Cancel and Update Message (Reword).
    pub(super) fn field_actions(&self, field: &Field, cx: &mut Context<Self>) -> Div {
        let (update_words, cancel_words) = match field.target {
            Target::Head { .. } => ("Update Message", "Cancel Amend"),
            Target::Older { .. } => ("Reword", "Cancel Reword"),
            Target::Stash(_) => ("Update Stash Message", "Cancel"),
        };
        let rebases = match field.target {
            Target::Older { above, .. } => Some(rebases_words(above)),
            _ => None,
        };
        let update = button::bordered(&self.hits, UPDATE_KEY, update_words, cx);
        let update = match self.update_why_not(cx) {
            Some(why) => button::dimmed(update).tooltip(Tooltip::text(why)),
            None => update.on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.update_message(cx))),
        };
        let cancel = button::bordered(&self.hits, CANCEL_KEY, cancel_words, cx).on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.cancel_amend(cx)));
        let muted = cx.theme().colors().text_muted;
        let rebases = rebases.map(|words| div().relative().text_xs().text_color(muted).child(words).child(hit_target(&self.hits, REBASES_KEY.into())));
        div()
            .flex()
            .flex_col()
            .flex_none()
            .gap_1()
            .px_2()
            .children(self.failure(cx))
            .children(rebases)
            .child(div().flex().gap_2().justify_end().child(cancel).child(update))
    }

    fn failure(&self, cx: &mut Context<Self>) -> Option<Div> {
        let why = self.reword.failure.clone()?;
        Some(failure_banner(&self.hits, "commit-amend-failure", why, |this: &mut Self, cx| {
            this.reword.failure = None;
            cx.notify();
        }, cx))
    }
}

#[cfg(test)]
mod tests;
