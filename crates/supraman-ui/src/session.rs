//! A live session as every view holds it, and its lead in a row: its state's dot, its name, then the owner's counts.

use std::path::PathBuf;

use gpui::{AnyElement, Div, SharedString, div, prelude::*};

use crate::marks::{self, Activity, Mark};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveSession {
    /// Its name as every view shows it (the app's `fleet::display_name`).
    pub name: String,
    pub claude_session_id: String,
    pub cwd: PathBuf,
    pub activity: Activity,
    /// The id `claude attach` takes, while the session can be attached (`fleet::attachable`).
    pub attach: Option<String>,
}

/// A session's lead in a row: `mark`'s dot, `name`, then `counts` (the owner's, such as `is working here`).
pub fn lead(mark: Mark, name: &str, counts: Option<AnyElement>) -> Div {
    div()
        .flex()
        .items_center()
        .gap_1p5()
        .min_w_0()
        .whitespace_nowrap()
        .child(marks::dot(mark))
        .child(SharedString::from(name.to_string()))
        .children(counts)
}
