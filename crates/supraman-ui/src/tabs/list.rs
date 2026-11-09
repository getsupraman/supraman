//! The strip's ⌄ list of every tab: the shared [`Palette`], its query filtering the titles, the active tab marked.

use std::ops::Range;
use std::rc::Rc;

use gpui::{AnyElement, App, SharedString, div, prelude::*, px};
use theme::ActiveTheme as _;

use crate::hit::{HitTargets, hit_target};
use crate::matcher::highlighted;
use crate::palette::{Entry, Paint};

/// The palette's id: its rows paint under `tab-list-row-<tab id>`.
pub const LIST: &str = "tab-list";

#[derive(Clone, Debug, PartialEq)]
pub struct TabEntry {
    pub id: u64,
    pub title: SharedString,
    pub active: bool,
}

impl Entry for TabEntry {
    fn key(&self) -> SharedString {
        self.id.to_string().into()
    }

    fn text(&self) -> String {
        self.title.to_string()
    }
}

/// A row: a dot before the active tab's title, painted under `tab-list-active-<id>`, then the title with the query's marks.
pub fn paint(hits: HitTargets) -> Paint<TabEntry> {
    Rc::new(move |entry: &TabEntry, marks: &[Range<usize>], _selected: bool, cx: &mut App| -> AnyElement {
        let colors = cx.theme().colors();
        let mark = div()
            .relative()
            .flex_none()
            .size(px(6.))
            .rounded_full()
            .when(entry.active, |d| d.bg(colors.text_accent).child(hit_target(&hits, format!("{LIST}-active-{}", entry.id))));
        div()
            .flex()
            .items_center()
            .gap_2()
            .text_sm()
            .child(mark)
            .child(
                highlighted(SharedString::from(format!("{LIST}-title-{}", entry.id)), entry.title.clone(), marks, cx)
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .when(entry.active, |d| d.text_color(colors.text_accent)),
            )
            .into_any_element()
    })
}
