//! The task box: what the human sends a workspace session (a new task, or the answer to an
//! `APPROVE?`/`DECIDE:`). The text is a multi-line [`TextInput`] (m5/42): Enter a new line, Ctrl+Enter
//! sends, Escape cancels ([`crate::view::FleetView::on_compose_event`]). This is the rest: whom it is for,
//! what it answers, and how the last send went.

use gpui::{App, Entity, Subscription};

use foreman_ui::text_input::TextInput;

pub struct Compose {
    pub workspace: String,
    /// The outbox file this answers; `None`: a new task.
    pub reply_to: Option<String>,
    pub input: Entity<TextInput>,
    /// Why the last send was refused or failed.
    pub error: Option<String>,
    /// The first line of the last task sent from this box.
    pub sent: Option<String>,
    /// How it went: `sent direct` or `sent via relay (<why>)`.
    pub route: Option<String>,
    _events: Subscription,
}

impl Compose {
    pub fn new(workspace: &str, reply_to: Option<String>, input: Entity<TextInput>, events: Subscription) -> Self {
        Self { workspace: workspace.to_string(), reply_to, input, error: None, sent: None, route: None, _events: events }
    }

    pub fn text(&self, cx: &App) -> String {
        self.input.read(cx).text().to_string()
    }
}

/// What a row shows of an answer, and what `answered.json` keeps: its first non-blank line, at most
/// 80 characters.
pub fn first_line(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default();
    match line.char_indices().nth(80) {
        Some((i, _)) => format!("{}…", &line[..i]),
        None => line.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_line_is_the_first_non_blank_one_cut_at_80() {
        assert_eq!(first_line("\n  use postgres  \nbecause"), "use postgres");
        assert_eq!(first_line(""), "");
        let long = "x".repeat(100);
        assert_eq!(first_line(&long), format!("{}…", "x".repeat(80)));
    }
}
