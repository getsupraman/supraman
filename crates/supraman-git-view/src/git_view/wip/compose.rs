//! The commit area's state: summary, description and the WIP row's input as one summary, Amend's draft, the button's words.

use supraman_core::text::count;
use gpui::{AppContext as _, Context, Div, Entity, SharedString, Subscription, Window, div, prelude::*, px};
use supraman_gitman::OperationState;
use supraman_gitman::write::conflicts::Sequence;
use supraman_ui::hit::{HitTargets, hit_target};
use supraman_ui::icon::Icon;
use supraman_ui::text_input::{InputEvent, TextInput};
use supraman_ui::tooltip::Tooltip;
use supraman_ui::{banner, button};
use theme::ActiveTheme as _;

use super::WipPanel;

/// The summary's length past which most tools cut it.
pub(crate) const SUMMARY_FITS: i64 = 72;
pub(crate) const SUMMARY_TOOLTIP: &str = "Up to 72 characters show in full in most tools";

/// What the commit button runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Act {
    Commit,
    ContinueRebase,
    SkipCommit,
    Continue(Sequence),
    Skip(Sequence),
}

/// The commit button's words and what it runs; `None` while it cannot act.
pub(crate) fn button(state: OperationState, staged: usize, amend: bool, has_message: bool) -> (String, Option<Act>) {
    let commit = |words: &str| (words.to_string(), has_message.then_some(Act::Commit));
    match state {
        OperationState::Merge if staged > 0 => commit("Commit and merge"),
        OperationState::Merge => ("No changes to merge".into(), None),
        OperationState::Rebase { .. } if staged > 0 => ("Continue rebase".into(), Some(Act::ContinueRebase)),
        OperationState::Rebase { .. } => ("Skip commit".into(), Some(Act::SkipCommit)),
        OperationState::CherryPick { .. } | OperationState::Revert { .. } | OperationState::Am => {
            let seq = Sequence::of(state).unwrap_or(Sequence::CherryPick);
            if staged > 0 { (format!("Continue {}", super::actions::noun(seq)), Some(Act::Continue(seq))) } else { ("Skip commit".into(), Some(Act::Skip(seq))) }
        }
        _ if amend => commit("Amend previous commit"),
        _ if staged == 0 => ("Stage changes to commit".into(), None),
        _ if !has_message => ("Type a commit message".into(), None),
        _ => (format!("Commit changes to {}", count(staged, "file", "files")), Some(Act::Commit)),
    }
}

/// 72 less the summary's length in characters; below 0 past [`SUMMARY_FITS`].
pub(crate) fn left(summary: &str) -> i64 {
    SUMMARY_FITS - summary.chars().count() as i64
}

/// The counter's colour: the warning colour past 72.
pub(crate) fn counter_colour(left: i64, cx: &gpui::App) -> gpui::Hsla {
    let theme = theme::ActiveTheme::theme(cx);
    if left < 0 { theme.status().warning } else { theme.colors().text_muted }
}

/// A whole message split as the two fields show it: the first line, and the rest after its blank line.
pub(crate) fn split(message: &str) -> (String, String) {
    let (summary, rest) = message.split_once('\n').unwrap_or((message, ""));
    (summary.trim_end().to_string(), rest.trim_start_matches(['\r', '\n']).trim_end().to_string())
}

/// A commit message's summary and description fields: Enter in the summary says Confirm, Ctrl+Enter Submit in either.
pub(crate) fn message_fields<T: 'static>(window: &mut Window, cx: &mut Context<T>) -> (Entity<TextInput>, Entity<TextInput>) {
    let summary = cx.new(|cx| TextInput::new("Commit summary", None, window, cx).submit_apart());
    let description = cx.new(|cx| TextInput::new("Description", None, window, cx).multi_line().submit_apart());
    (summary, description)
}

/// The summary field and its count of characters left (negative past 72, in the warning colour), painted as `counter`.
pub(crate) fn summary_row(hits: &HitTargets, summary: &Entity<TextInput>, counter: &'static str, cx: &gpui::App) -> Div {
    let left = left(summary.read(cx).text());
    let count = div().id(counter).relative().flex_none().w(px(32.)).text_right().text_color(counter_colour(left, cx)).child(SharedString::from(left.to_string())).child(hit_target(hits, counter.into())).tooltip(Tooltip::text(SUMMARY_TOOLTIP));
    div().flex().items_center().gap_1().child(div().flex_1().min_w_0().child(summary.clone())).child(count)
}

/// A failed commit's words (git's or a hook's) in the error colour with its ×, under `key` and `<key>-dismiss`.
pub(crate) fn failure_banner<V: 'static>(hits: &HitTargets, key: &str, why: String, dismiss: impl Fn(&mut V, &mut Context<V>) + 'static, cx: &mut Context<V>) -> Div {
    let close = button::icon(hits, format!("{key}-dismiss"), Icon::Close.svg(12., cx.theme().colors().text_muted), px(20.), cx)
        .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _, cx| dismiss(this, cx)));
    let words = div().relative().child(SharedString::from(why)).child(hit_target(hits, key.into()));
    banner::tinted(cx.theme().status().error).child(banner::dismissable(words, close))
}

pub(crate) struct Composer {
    pub(crate) summary: Entity<TextInput>,
    pub(crate) description: Entity<TextInput>,
    /// The graph's WIP row input: the summary in another place.
    pub(crate) row: Entity<TextInput>,
    pub(crate) amend: bool,
    /// The summary and description from before Amend was ticked, back when it is unticked.
    draft: Option<(String, String)>,
    pub(crate) options_open: bool,
    pub(crate) skip_hooks: bool,
    /// The last commit's failure: git's or a hook's words and exit code.
    pub(crate) failure: Option<String>,
    _inputs: [Subscription; 3],
}

impl Composer {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<WipPanel>) -> Composer {
        let (summary, description) = message_fields(window, cx);
        let row = cx.new(|cx| TextInput::new("// WIP", None, window, cx).submit_apart().quiet());
        let inputs = [
            cx.subscribe_in(&summary, window, |this: &mut WipPanel, _, e: &InputEvent, window, cx| this.summary_event(e, false, window, cx)),
            cx.subscribe_in(&row, window, |this: &mut WipPanel, _, e: &InputEvent, window, cx| this.summary_event(e, true, window, cx)),
            cx.subscribe_in(&description, window, |this: &mut WipPanel, _, e: &InputEvent, _, cx| {
                if *e == InputEvent::Submit {
                    this.commit(false, cx);
                }
            }),
        ];
        Composer { summary, description, row, amend: false, draft: None, options_open: false, skip_hooks: false, failure: None, _inputs: inputs }
    }

    pub(crate) fn texts(&self, cx: &gpui::App) -> (String, String) {
        (self.summary.read(cx).text().to_string(), self.description.read(cx).text().to_string())
    }

    /// Both fields set from outside, the WIP row's summary with them.
    pub(crate) fn set(&self, summary: &str, description: &str, cx: &mut Context<WipPanel>) {
        for input in [&self.summary, &self.row] {
            input.update(cx, |i, cx| i.set_text(summary, cx));
        }
        self.description.update(cx, |i, cx| i.set_text(description, cx));
    }

    /// Amend ticked: the draft kept until it is unticked.
    pub(crate) fn amend_on(&mut self, cx: &mut Context<WipPanel>) {
        self.amend = true;
        self.draft = Some(self.texts(cx));
    }

    pub(crate) fn amend_off(&mut self, cx: &mut Context<WipPanel>) {
        self.amend = false;
        if let Some((summary, description)) = self.draft.take() {
            self.set(&summary, &description, cx);
        }
    }

    /// After a commit: empty fields, Amend off, no failure.
    pub(crate) fn clear(&mut self, cx: &mut Context<WipPanel>) {
        self.amend = false;
        self.draft = None;
        self.failure = None;
        self.set("", "", cx);
    }
}

impl WipPanel {
    /// The commit area's fields, made once a window is at hand.
    pub(crate) fn ensure_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.composer.is_none() {
            self.composer = Some(Composer::new(window, cx));
        }
    }

    pub(crate) fn composer(&self) -> Option<&Composer> {
        self.composer.as_ref()
    }

    /// Skip hooks is on where the repo has commit hooks: a commit or an amend runs `--no-verify`.
    pub(crate) fn skips_hooks(&self) -> bool {
        self.hooks && self.composer.as_ref().is_some_and(|c| c.skip_hooks)
    }

    /// The commit summary as typed, the message a stash takes.
    pub(crate) fn pending_summary(&self, cx: &gpui::App) -> String {
        self.composer.as_ref().map_or_else(String::new, |c| c.summary.read(cx).text().trim().to_string())
    }

    /// `summary` in an empty commit summary; a typed one stays.
    pub(crate) fn fill_summary(&mut self, summary: &str, cx: &mut Context<Self>) {
        let Some(c) = &self.composer else { return };
        if self.pending_summary(cx).is_empty() {
            let description = c.description.read(cx).text().to_string();
            c.set(summary, &description, cx);
        }
    }

    /// The summary spent: both fields emptied.
    pub(crate) fn clear_summary(&mut self, cx: &mut Context<Self>) {
        if let Some(c) = &self.composer {
            c.set("", "", cx);
        }
    }

    /// An edit of the summary or the WIP row's copy of it; Enter in the summary moves on to the description, Ctrl+Enter commits.
    fn summary_event(&mut self, e: &InputEvent, from_row: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(c) = &self.composer else { return };
        match e {
            InputEvent::Edited => {
                let (from, to) = if from_row { (&c.row, &c.summary) } else { (&c.summary, &c.row) };
                let text = from.read(cx).text().to_string();
                to.update(cx, |i, cx| i.set_text(&text, cx));
                cx.notify();
            }
            InputEvent::Confirm if !from_row => {
                let description = c.description.clone();
                description.update(cx, |i, cx| i.focus(window, cx));
            }
            InputEvent::Submit => self.commit(false, cx),
            _ => {}
        }
    }

    /// The summary takes the keyboard.
    pub(crate) fn focus_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ensure_composer(window, cx);
        if let Some(c) = &self.composer {
            let summary = c.summary.clone();
            summary.update(cx, |i, cx| i.focus(window, cx));
        }
    }

    pub(crate) fn toggle_amend(&mut self, cx: &mut Context<Self>) {
        let Some(c) = &mut self.composer else { return };
        if c.amend {
            c.amend_off(cx);
            return cx.notify();
        }
        c.amend_on(cx);
        let Some(repo) = self.repo.clone() else { return cx.notify() };
        let read = cx.background_spawn(async move { supraman_gitman::head_message(&repo) });
        cx.spawn(async move |this, cx| {
            let message = read.await;
            this.update(cx, |this, cx| {
                let Some(c) = this.composer.as_ref().filter(|c| c.amend) else { return };
                match message {
                    Ok(Some(message)) => {
                        let (summary, description) = split(&message);
                        c.set(&summary, &description, cx);
                    }
                    Ok(None) => {}
                    Err(e) => log::warn!("git tab: cannot read HEAD's message: {e}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_button_says_what_it_does_by_priority() {
        let none = OperationState::None;
        let rebase = OperationState::Rebase { external: true };
        assert_eq!(button(OperationState::Merge, 1, false, true), ("Commit and merge".into(), Some(Act::Commit)));
        assert_eq!(button(OperationState::Merge, 0, true, true), ("No changes to merge".into(), None));
        assert_eq!(button(rebase, 2, true, false), ("Continue rebase".into(), Some(Act::ContinueRebase)));
        assert_eq!(button(rebase, 0, false, true), ("Skip commit".into(), Some(Act::SkipCommit)));
        assert_eq!(button(none, 0, true, true), ("Amend previous commit".into(), Some(Act::Commit)));
        assert_eq!(button(none, 0, false, true), ("Stage changes to commit".into(), None));
        assert_eq!(button(none, 3, false, false), ("Type a commit message".into(), None));
        assert_eq!(button(none, 1, false, true), ("Commit changes to 1 file".into(), Some(Act::Commit)));
        assert_eq!(button(none, 3, false, true), ("Commit changes to 3 files".into(), Some(Act::Commit)));
        let pick = OperationState::CherryPick { sequence: false };
        assert_eq!(button(pick, 1, false, false), ("Continue cherry-pick".into(), Some(Act::Continue(Sequence::CherryPick))));
        assert_eq!(button(OperationState::Revert { sequence: true }, 0, false, true), ("Skip commit".into(), Some(Act::Skip(Sequence::Revert))));
    }

    #[test]
    fn the_counter_goes_below_zero_past_72_and_a_message_splits_at_its_blank_line() {
        assert_eq!(left(&"x".repeat(72)), 0);
        assert_eq!(left(&"é".repeat(75)), -3);
        assert_eq!(split("Fix it\n\nWhy\nhow"), ("Fix it".into(), "Why\nhow".into()));
        assert_eq!(split("Only"), ("Only".into(), String::new()));
    }
}
