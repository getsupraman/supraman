//! The message box edited in place: HEAD's message as a summary and a description, amended through the tab's
//! write gate (asked first once a remote holds HEAD), or a stash's message, saved by the tab's stash writes.

use std::sync::{Arc, Mutex, PoisonError};

use gpui::{App, ClickEvent, Context, Div, Hsla, Stateful, Subscription, Window, div, prelude::*};
use supraman_gitman::Amendable;
use supraman_gitman::write::{LockSet, Session, WriteError, Writer, wip};
use supraman_gitman::Repo;
use supraman_ui::confirm::Confirm;
use supraman_ui::hit::hit_target;
use supraman_ui::text_input::{InputEvent, TextInput};
use supraman_ui::tooltip::Tooltip;
use supraman_ui::button;
use theme::ActiveTheme as _;

use super::{CommitPanel, PanelEvent, Shown};
use crate::git_view::keys::{AMEND_CONTEXT, NextField, PreviousField};
use crate::git_view::stash::{STILL_OPENING, StashRef};
use crate::git_view::wip::compose::{failure_banner, message_fields, summary_row};
use crate::git_view::write_gate::{Written, capitalised};

pub const CLICK_TO_AMEND: &str = "Click to amend your commit message";
pub const CLICK_TO_EDIT_STASH: &str = "Click to edit the stash message";
pub const CHECK_OUT_A_BRANCH: &str = "Check out a branch to amend";
pub const TYPE_A_SUMMARY: &str = "Type a commit summary";
pub const EDIT_KEY: &str = "commit-message-edit";
pub const UPDATE_KEY: &str = "commit-amend-update";
pub const CANCEL_KEY: &str = "commit-amend-cancel";
pub const ASK_KEY: &str = "commit-amend-ask";

/// What the open field edits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Head { branch: String, pushed: bool },
    Stash(StashRef),
}

pub struct Field {
    pub target: Target,
    pub summary: gpui::Entity<TextInput>,
    pub description: gpui::Entity<TextInput>,
    _events: [Subscription; 2],
}

#[derive(Default)]
pub struct Reword {
    pub field: Option<Field>,
    asking: Option<Confirm<String>>,
    failure: Option<String>,
    /// The amend under way: its gate ticket.
    doing: Option<u64>,
    /// HEAD's sha after the amend, as the job left it.
    reworded: Arc<Mutex<Option<String>>>,
    /// Opened off the UI thread with the repo.
    writer: Option<Writer>,
}

/// The muted line the read-only box shows, and what a click on it edits.
pub fn offer(shown: &Shown) -> Option<(&'static str, Option<Target>)> {
    if let Some(stash) = &shown.stash {
        return Some((CLICK_TO_EDIT_STASH, Some(Target::Stash(StashRef { sha: stash.sha.clone(), label: shown.summary.to_string() }))));
    }
    match &shown.amendable {
        Amendable::No => None,
        Amendable::Detached => Some((CHECK_OUT_A_BRANCH, None)),
        Amendable::Branch { name, pushed } => Some((CLICK_TO_AMEND, Some(Target::Head { branch: name.clone(), pushed: *pushed }))),
    }
}

pub fn pushed_question(branch: &str) -> String {
    format!("{branch} was already pushed. Amending rewrites it, and the next push must force. Amend anyway?")
}

impl CommitPanel {
    /// The repo's writer, once, off the UI thread.
    pub(super) fn open_writer(&mut self, repo: Repo, cx: &mut Context<Self>) {
        if self.reword.writer.is_some() {
            return;
        }
        let open = cx.background_spawn(async move { Writer::open(&repo).map_err(|e| e.to_string()) });
        cx.spawn(async move |this, cx| {
            let writer = open.await;
            this.update(cx, |this, _| match writer {
                Ok(w) => this.reword.writer = Some(w),
                Err(why) => log::warn!("git tab: cannot open the repository to amend: {why}"),
            })
            .ok();
        })
        .detach();
    }

    /// The shown message in the field, the summary focused.
    pub fn open_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        let Some(field) = self.reword.field.as_ref().filter(|f| matches!(f.target, Target::Head { .. })) else { return };
        let to = if field.summary.read(cx).is_focused(window) { field.description.clone() } else { field.summary.clone() };
        to.update(cx, |i, cx| i.focus(window, cx));
    }

    pub fn cancel_amend(&mut self, cx: &mut Context<Self>) {
        self.reword.field = None;
        self.reword.asking = None;
        self.reword.failure = None;
        cx.notify();
    }

    /// Why Update Message cannot run now: a blank summary, or the gate's write.
    pub fn update_why_not(&self, cx: &App) -> Option<&'static str> {
        let field = self.reword.field.as_ref()?;
        if field.summary.read(cx).text().trim().is_empty() {
            return Some(TYPE_A_SUMMARY);
        }
        self.gate.read(cx).busy()
    }

    /// Update Message: a stash's saved by the tab's stash writes; HEAD's amended, asked first once pushed.
    pub fn update_message(&mut self, cx: &mut Context<Self>) {
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
            Target::Head { branch, pushed: true } => self.reword.asking = Some(Confirm::ask(pushed_question(&branch), "Amend anyway", wip::message(&summary, &description))),
            Target::Head { pushed: false, .. } => cx.emit(PanelEvent::Amend(wip::message(&summary, &description))),
        }
        cx.notify();
    }

    pub fn answer_amend(&mut self, yes: bool, cx: &mut Context<Self>) {
        match self.reword.asking.take().and_then(|c| c.action).filter(|_| yes) {
            Some(message) => cx.emit(PanelEvent::Amend(message)),
            None => cx.notify(),
        }
    }

    #[cfg(test)]
    pub fn amend_asking(&self) -> Option<&str> {
        self.reword.asking.as_ref().map(|c| c.text.as_str())
    }

    #[cfg(test)]
    pub fn amend_failure(&self) -> Option<&str> {
        self.reword.failure.as_deref()
    }

    pub fn field(&self) -> Option<&Field> {
        self.reword.field.as_ref()
    }

    /// HEAD's message amended to `message` through the tab's gate; `no_verify` is the commit area's Skip hooks.
    pub fn amend(&mut self, message: String, no_verify: bool, cx: &mut Context<Self>) {
        let Some(writer) = self.reword.writer.clone() else { return cx.emit(PanelEvent::Why(STILL_OPENING.into())) };
        let reworded = self.reword.reworded.clone();
        let job = move |s: &Session<'_>| -> Result<(), WriteError> {
            let sha = wip::reword_head(s, &message, no_verify)?;
            *reworded.lock().unwrap_or_else(PoisonError::into_inner) = Some(sha);
            Ok(())
        };
        if let Some(ticket) = self.gate.update(cx, |g, cx| g.request(writer, LockSet::commit(), job, cx)) {
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
                let sha = self.reword.reworded.lock().unwrap_or_else(PoisonError::into_inner).take().unwrap_or_default();
                self.reword.failure = None;
                cx.emit(PanelEvent::Amended(sha.into(), format!("Amended \u{201c}{summary}\u{201d}")));
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
            .when(matches!(field.target, Target::Head { .. }), |d| d.child(field.description.clone()))
    }

    /// Under the open field: a failure, the question before a pushed HEAD's amend, Cancel Amend and Update Message.
    pub(super) fn field_actions(&self, field: &Field, cx: &mut Context<Self>) -> Div {
        let head = matches!(field.target, Target::Head { .. });
        let update = button::bordered(&self.hits, UPDATE_KEY, if head { "Update Message" } else { "Update Stash Message" }, cx);
        let update = match self.update_why_not(cx) {
            Some(why) => button::dimmed(update).tooltip(Tooltip::text(why)),
            None => update.on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.update_message(cx))),
        };
        let cancel = button::bordered(&self.hits, CANCEL_KEY, if head { "Cancel Amend" } else { "Cancel" }, cx).on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.cancel_amend(cx)));
        let question = self.reword.asking.as_ref().map(|asked| asked.render(&self.hits, ASK_KEY, |this: &mut Self, yes, _, cx| this.answer_amend(yes, cx), cx));
        div()
            .flex()
            .flex_col()
            .flex_none()
            .gap_1()
            .px_2()
            .children(self.failure(cx))
            .children(question)
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
