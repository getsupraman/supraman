//! The commit area's state: the summary, the description and the graph's WIP row input (one summary in two
//! fields), Amend previous commit with the draft it keeps, the commit options, and what the commit button says.

use gpui::{AppContext as _, Context, Entity, Subscription, Window};
use supraman_gitman::OperationState;
use supraman_ui::text_input::{InputEvent, TextInput};

use super::WipPanel;

/// The summary's length past which most tools cut it.
pub const SUMMARY_FITS: i64 = 72;
pub const SUMMARY_TOOLTIP: &str = "Up to 72 characters show in full in most tools";

/// What the commit button runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    Commit,
    ContinueRebase,
    SkipCommit,
}

/// The commit button's words and what it runs; `None` while it cannot act.
pub fn button(state: OperationState, staged: usize, amend: bool, has_message: bool) -> (String, Option<Act>) {
    let commit = |words: &str| (words.to_string(), has_message.then_some(Act::Commit));
    match state {
        OperationState::Merge if staged > 0 => commit("Commit and merge"),
        OperationState::Merge => ("Commit and merge".into(), None),
        OperationState::Rebase { .. } if staged > 0 => ("Continue rebase".into(), Some(Act::ContinueRebase)),
        OperationState::Rebase { .. } => ("Skip commit".into(), Some(Act::SkipCommit)),
        _ if amend => commit("Amend previous commit"),
        _ if staged == 0 => ("Stage changes to commit".into(), None),
        _ if !has_message => ("Type a commit message".into(), None),
        _ => (format!("Commit changes to {staged} file{}", if staged == 1 { "" } else { "s" }), Some(Act::Commit)),
    }
}

/// 72 less the summary's length in characters; below 0 past [`SUMMARY_FITS`].
pub fn left(summary: &str) -> i64 {
    SUMMARY_FITS - summary.chars().count() as i64
}

/// The counter's colour: the warning colour past 72.
pub fn counter_colour(left: i64, cx: &gpui::App) -> gpui::Hsla {
    let theme = theme::ActiveTheme::theme(cx);
    if left < 0 { theme.status().warning } else { theme.colors().text_muted }
}

/// A whole message split as the two fields show it: the first line, and the rest after its blank line.
pub fn split(message: &str) -> (String, String) {
    let (summary, rest) = message.split_once('\n').unwrap_or((message, ""));
    (summary.trim_end().to_string(), rest.trim_start_matches(['\r', '\n']).trim_end().to_string())
}

pub struct Composer {
    pub summary: Entity<TextInput>,
    pub description: Entity<TextInput>,
    /// The graph's WIP row input: the summary in another place.
    pub row: Entity<TextInput>,
    pub amend: bool,
    /// The summary and description from before Amend was ticked, back when it is unticked.
    draft: Option<(String, String)>,
    pub options_open: bool,
    pub skip_hooks: bool,
    /// The last commit's failure: git's or a hook's words and exit code.
    pub failure: Option<String>,
    _inputs: [Subscription; 3],
}

impl Composer {
    pub fn new(window: &mut Window, cx: &mut Context<WipPanel>) -> Composer {
        let summary = cx.new(|cx| TextInput::new("Commit summary", None, window, cx).submit_apart());
        let description = cx.new(|cx| TextInput::new("Description", None, window, cx).multi_line().submit_apart());
        let row = cx.new(|cx| TextInput::new("// WIP", None, window, cx).submit_apart());
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

    pub fn texts(&self, cx: &gpui::App) -> (String, String) {
        (self.summary.read(cx).text().to_string(), self.description.read(cx).text().to_string())
    }

    /// Both fields set from outside, the WIP row's summary with them.
    pub fn set(&self, summary: &str, description: &str, cx: &mut Context<WipPanel>) {
        for input in [&self.summary, &self.row] {
            input.update(cx, |i, cx| i.set_text(summary, cx));
        }
        self.description.update(cx, |i, cx| i.set_text(description, cx));
    }

    /// Amend ticked: the draft kept until it is unticked.
    pub fn amend_on(&mut self, cx: &mut Context<WipPanel>) {
        self.amend = true;
        self.draft = Some(self.texts(cx));
    }

    pub fn amend_off(&mut self, cx: &mut Context<WipPanel>) {
        self.amend = false;
        if let Some((summary, description)) = self.draft.take() {
            self.set(&summary, &description, cx);
        }
    }

    /// After a commit: empty fields, Amend off, no failure.
    pub fn clear(&mut self, cx: &mut Context<WipPanel>) {
        self.amend = false;
        self.draft = None;
        self.failure = None;
        self.set("", "", cx);
    }
}

impl WipPanel {
    /// The commit area's fields, made once a window is at hand.
    pub fn ensure_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.composer.is_none() {
            self.composer = Some(Composer::new(window, cx));
        }
    }

    pub fn composer(&self) -> Option<&Composer> {
        self.composer.as_ref()
    }

    /// The commit summary as typed, the message a stash takes.
    pub fn pending_summary(&self, cx: &gpui::App) -> String {
        self.composer.as_ref().map_or_else(String::new, |c| c.summary.read(cx).text().trim().to_string())
    }

    /// `summary` in an empty commit summary; a typed one stays.
    pub fn fill_summary(&mut self, summary: &str, cx: &mut Context<Self>) {
        let Some(c) = &self.composer else { return };
        if self.pending_summary(cx).is_empty() {
            let description = c.description.read(cx).text().to_string();
            c.set(summary, &description, cx);
        }
    }

    /// The summary spent: both fields emptied.
    pub fn clear_summary(&mut self, cx: &mut Context<Self>) {
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
    pub fn focus_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ensure_composer(window, cx);
        if let Some(c) = &self.composer {
            let summary = c.summary.clone();
            summary.update(cx, |i, cx| i.focus(window, cx));
        }
    }

    pub fn toggle_amend(&mut self, cx: &mut Context<Self>) {
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
        assert_eq!(button(OperationState::Merge, 0, true, true), ("Commit and merge".into(), None));
        assert_eq!(button(rebase, 2, true, false), ("Continue rebase".into(), Some(Act::ContinueRebase)));
        assert_eq!(button(rebase, 0, false, true), ("Skip commit".into(), Some(Act::SkipCommit)));
        assert_eq!(button(none, 0, true, true), ("Amend previous commit".into(), Some(Act::Commit)));
        assert_eq!(button(none, 0, false, true), ("Stage changes to commit".into(), None));
        assert_eq!(button(none, 3, false, false), ("Type a commit message".into(), None));
        assert_eq!(button(none, 1, false, true), ("Commit changes to 1 file".into(), Some(Act::Commit)));
        assert_eq!(button(none, 3, false, true), ("Commit changes to 3 files".into(), Some(Act::Commit)));
    }

    #[test]
    fn the_counter_goes_below_zero_past_72_and_a_message_splits_at_its_blank_line() {
        assert_eq!(left(&"x".repeat(72)), 0);
        assert_eq!(left(&"é".repeat(75)), -3);
        assert_eq!(split("Fix it\n\nWhy\nhow"), ("Fix it".into(), "Why\nhow".into()));
        assert_eq!(split("Only"), ("Only".into(), String::new()));
    }
}
