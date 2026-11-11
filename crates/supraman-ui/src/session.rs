//! A live session as every view holds it, and its lead in a row: its state's dot, its name, then the owner's counts.

use std::path::PathBuf;

use gpui::{AnyElement, Div, SharedString, Stateful, div, prelude::*, px};

use crate::clipped::clipped_with;
use crate::tooltip::Tooltip;
use crate::ink;
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

/// A session's lead in a row: `mark`'s dot, `name` cut under `<key>-name`, then `slot` (the owner's, such as `is working here`).
/// A narrow row cuts the name; the slot shrinks as the owner styles it (`flex_none` keeps it whole).
pub fn lead(key: &str, mark: Mark, name: &str, slot: Option<AnyElement>) -> Div {
    body(key, mark, name, slot, true)
}

/// [`lead`] with no slot and `told` (such as its name and state's word) as the whole lead's tooltip, the name's own given up.
pub fn lead_told(key: &str, mark: Mark, name: &str, told: impl Into<SharedString>) -> Stateful<Div> {
    body(key, mark, name, None, false).id(SharedString::from(format!("{key}-told"))).tooltip(Tooltip::text(told))
}

fn body(key: &str, mark: Mark, name: &str, slot: Option<AnyElement>, own_tooltip: bool) -> Div {
    div()
        .flex()
        .items_center()
        .gap_1p5()
        .min_w(px(marks::DOT))
        .flex_shrink(1.)
        .overflow_hidden()
        .whitespace_nowrap()
        .child(div().flex_none().child(ink::mark(marks::dot(mark))))
        .child(clipped_with(format!("{key}-name"), name.to_string(), false, own_tooltip).flex_shrink(1.))
        .children(slot)
}
