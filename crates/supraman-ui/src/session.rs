//! A live session as every view holds it, and its lead in a row: its state's dot, its name, then the owner's counts.

use std::path::PathBuf;

use gpui::{AnyElement, App, ClickEvent, Div, SharedString, Stateful, Window, div, prelude::*, px};

use crate::button::{self, Fill, Tone};
use crate::clipped::clipped;
use crate::hit::HitTargets;
use crate::icon::Icon;
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
    div()
        .flex()
        .items_center()
        .gap_1p5()
        .min_w(px(marks::DOT))
        .flex_shrink(1.)
        .overflow_hidden()
        .whitespace_nowrap()
        .child(div().flex_none().child(ink::mark(marks::dot(mark))))
        .child(clipped(format!("{key}-name"), name.to_string(), false).flex_shrink(1.))
        .children(slot)
}

/// What a session's [`open`] does now: its icon, its words (the app's `fleet::Open::hint`) and whether a click runs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opening {
    pub icon: Icon,
    pub hint: SharedString,
    pub runs: bool,
}

/// Every session's Open: `opening`'s icon and the word Open, its hint as tooltip; dimmed with the hint as why, its click a no-op, while it cannot run.
pub fn open(hits: &HitTargets, key: impl Into<SharedString>, opening: &Opening, fill: Fill, on_open: impl Fn(&mut Window, &mut App) + 'static, cx: &App) -> Stateful<Div> {
    let button = button::action_with(hits, key, opening.icon, "Open", Tone::Neutral, fill, cx).tooltip(Tooltip::text(opening.hint.clone()));
    if !opening.runs {
        return button::dimmed(button);
    }
    button.on_click(move |_: &ClickEvent, window, cx| {
        cx.stop_propagation();
        on_open(window, cx);
    })
}

#[cfg(test)]
mod tests;
