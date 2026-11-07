//! A Ctrl+P palette over its owner's entries: a query box, the entries its characters match in order
//! ([`matcher::subsequence`]) ranked best first, then the owner's remote hits ([`Palette::set_remote`]);
//! Up/Down move, Enter or a click opens ([`PaletteEvent::Open`]), Esc or a click outside closes.
//! The owner places it, paints each row and acts on its events; rows paint under `<id>-row-<key>`.

use std::ops::Range;
use std::rc::Rc;

use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, MouseButton, Render, SharedString, Subscription,
    Window, div, prelude::*, px,
};
use theme::ActiveTheme as _;

use crate::hit::{HitTargets, hit_target};
use crate::keyed_list::KeyedList;
use crate::matcher::subsequence;
use crate::text_input::{self, InputEvent, TextInput};
use crate::button;

/// The most rows listed at once; a longer query narrows them.
pub const MAX_SHOWN: usize = 200;
const LIST_HEIGHT: f32 = 380.;

pub trait Entry: Clone + 'static {
    /// Unique among the entries.
    fn key(&self) -> SharedString;
    /// What the query is matched against; the painter's marks are byte ranges of it.
    fn text(&self) -> String;
}

pub enum PaletteEvent<T> {
    /// The query after an edit: the owner may look it up elsewhere and hand back [`Palette::set_remote`].
    Query(String),
    Open(T),
    Close,
}

pub type Paint<T> = Rc<dyn Fn(&T, &[Range<usize>], bool, &mut App) -> AnyElement>;

pub struct Palette<T: Entry> {
    id: SharedString,
    hits: HitTargets,
    input: Entity<TextInput>,
    _input: Subscription,
    local: Vec<T>,
    remote: (String, Vec<T>),
    shown: Vec<(T, Vec<Range<usize>>)>,
    selected: usize,
    list: KeyedList,
    paint: Paint<T>,
    caption: SharedString,
}

impl<T: Entry> EventEmitter<PaletteEvent<T>> for Palette<T> {}

/// `marks` that fall in `range`, cut to it and counted from its start: one field's share of a row's marks.
pub fn marks_in(marks: &[Range<usize>], range: Range<usize>) -> Vec<Range<usize>> {
    marks
        .iter()
        .filter(|m| m.start < range.end && m.end > range.start)
        .map(|m| m.start.max(range.start) - range.start..m.end.min(range.end) - range.start)
        .collect()
}

/// `entries` the query matches, best first: fewer runs of matched characters, then an earlier first match; all of them for an empty query.
pub fn ranked<T: Entry>(entries: &[T], query: &str) -> Vec<(T, Vec<Range<usize>>)> {
    let mut found: Vec<(usize, usize, usize, T, Vec<Range<usize>>)> = entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| subsequence(&e.text(), query).map(|marks| (marks.len(), marks.first().map_or(0, |m| m.start), i, e.clone(), marks)))
        .collect();
    found.sort_by_key(|(runs, first, i, ..)| (*runs, *first, *i));
    found.into_iter().take(MAX_SHOWN).map(|(.., e, marks)| (e, marks)).collect()
}

impl<T: Entry> Palette<T> {
    pub fn new(id: impl Into<SharedString>, hits: HitTargets, entries: Vec<T>, paint: Paint<T>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| TextInput::new("Search refs, titles…", None, window, cx));
        let sub = cx.subscribe(&input, |this: &mut Self, input, e: &InputEvent, cx| match e {
            InputEvent::Edited => {
                let query = input.read(cx).text().to_string();
                this.refresh(false, cx);
                cx.emit(PaletteEvent::Query(query));
            }
            InputEvent::Confirm => this.open_selected(cx),
            InputEvent::Cancel => cx.emit(PaletteEvent::Close),
            _ => {}
        });
        let mut palette = Palette {
            id: id.into(),
            hits,
            input,
            _input: sub,
            local: entries,
            remote: (String::new(), Vec::new()),
            shown: Vec::new(),
            selected: 0,
            list: KeyedList::default(),
            paint,
            caption: "ITEMS".into(),
        };
        palette.refresh(false, cx);
        palette
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.input.update(cx, |input, cx| input.focus(window, cx));
    }

    pub fn query(&self, cx: &App) -> String {
        self.input.read(cx).text().to_string()
    }

    pub fn shown(&self) -> &[(T, Vec<Range<usize>>)] {
        &self.shown
    }

    pub fn selected(&self) -> Option<&T> {
        self.shown.get(self.selected).map(|(e, _)| e)
    }

    pub fn set_entries(&mut self, entries: Vec<T>, cx: &mut Context<Self>) {
        self.local = entries;
        self.refresh(true, cx);
    }

    /// Hits found elsewhere for `query`, listed after the local ones; dropped when the query has moved on.
    pub fn set_remote(&mut self, query: &str, hits: Vec<T>, cx: &mut Context<Self>) {
        if query != self.query(cx) {
            return;
        }
        self.remote = (query.to_string(), hits);
        self.refresh(true, cx);
    }

    /// `keep`: the selected entry stays selected while listed; else the first is.
    fn refresh(&mut self, keep: bool, cx: &mut Context<Self>) {
        let query = self.query(cx);
        let key = self.selected().map(|e| e.key()).filter(|_| keep);
        let mut shown = ranked(&self.local, &query);
        if self.remote.0 == query {
            for hit in &self.remote.1 {
                if shown.len() < MAX_SHOWN && !shown.iter().any(|(e, _)| e.key() == hit.key()) {
                    let marks = subsequence(&hit.text(), &query).unwrap_or_default();
                    shown.push((hit.clone(), marks));
                }
            }
        }
        self.selected = key.and_then(|k| shown.iter().position(|(e, _)| e.key() == k)).unwrap_or(0);
        self.shown = shown;
        self.list.set_rows(self.shown.iter().map(|(e, _)| (e.key(), vec![self.row_key(&e.key())])).collect());
        if !keep {
            self.list.to_top();
        }
        cx.notify();
    }

    fn row_key(&self, key: &str) -> String {
        format!("{}-row-{key}", self.id)
    }

    pub fn step(&mut self, by: isize, cx: &mut Context<Self>) {
        if self.shown.is_empty() {
            return;
        }
        self.selected = (self.selected as isize + by).clamp(0, self.shown.len() as isize - 1) as usize;
        let key = self.shown[self.selected].0.key();
        self.list.reveal_where(|k| k == key.as_ref());
        cx.notify();
    }

    fn open_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(entry) = self.selected().cloned() {
            cx.emit(PaletteEvent::Open(entry));
        }
    }

    fn render_row(&mut self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some((entry, marks)) = self.shown.get(ix).cloned() else { return div().into_any_element() };
        let on = ix == self.selected;
        let colors = cx.theme().colors().clone();
        let body = (self.paint)(&entry, &marks, on, cx);
        button::ghost(&self.hits, self.row_key(&entry.key()), cx)
            .w_full()
            .px_3()
            .py(px(5.))
            .when(on, |d| d.bg(colors.element_selected).child(hit_target(&self.hits, format!("{}-selected", self.id))))
            .child(body)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.selected = ix;
                this.open_selected(cx);
            }))
            .into_any_element()
    }
}

impl<T: Entry> Focusable for Palette<T> {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl<T: Entry> Render for Palette<T> {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors().clone();
        let results = if self.shown.is_empty() {
            div().px_3().py_2().text_sm().text_color(colors.text_muted).child("Nothing matches.").into_any_element()
        } else {
            gpui::list(self.list.state(), cx.processor(|this: &mut Self, ix: usize, _, cx| this.render_row(ix, cx)))
                .w_full()
                .h(px(LIST_HEIGHT))
                .into_any_element()
        };
        let hint = |key: &'static str, what: &'static str| {
            div().flex().items_center().gap_1().child(div().px_1().rounded_sm().border_1().border_color(colors.border).child(key)).child(what)
        };
        div()
            .id(self.id.clone())
            .key_context("Palette")
            .w(px(560.))
            .max_w_full()
            .flex()
            .flex_col()
            .rounded_md()
            .border_1()
            .border_color(colors.border)
            .bg(colors.elevated_surface_background)
            .shadow_lg()
            .overflow_hidden()
            .child(hit_target(&self.hits, self.id.to_string()))
            .on_action(cx.listener(|this, _: &text_input::Up, _, cx| this.step(-1, cx)))
            .on_action(cx.listener(|this, _: &text_input::Down, _, cx| this.step(1, cx)))
            .on_mouse_down_out(cx.listener(|_, _, _, cx| cx.emit(PaletteEvent::Close)))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(div().p_2().border_b_1().border_color(colors.border).child(self.input.clone()))
            .child(div().px_3().pt_2().pb_1().text_xs().font_weight(FontWeight::BOLD).text_color(colors.text_muted).child(self.caption.clone()))
            .child(results)
            .child(
                div()
                    .flex()
                    .gap_3()
                    .px_3()
                    .py_1()
                    .border_t_1()
                    .border_color(colors.border)
                    .text_xs()
                    .text_color(colors.text_muted)
                    .child(hint("↑ ↓", "move"))
                    .child(hint("Enter", "open"))
                    .child(hint("Esc", "close")),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq)]
    struct E(&'static str);

    impl Entry for E {
        fn key(&self) -> SharedString {
            self.0.into()
        }
        fn text(&self) -> String {
            self.0.to_string()
        }
    }

    #[test]
    fn a_whole_word_ranks_above_scattered_letters_and_an_empty_query_lists_all() {
        let entries = [E("m4/1 Inbox x"), E("ws/2 Index box"), E("w/3 Inbox cards"), E("m5 other")];
        let names: Vec<&str> = ranked(&entries, "inbox").iter().map(|(e, _)| e.0).collect();
        assert_eq!(names, ["w/3 Inbox cards", "m4/1 Inbox x", "ws/2 Index box"], "one run first, earlier first among equals");
        assert_eq!(ranked(&entries, "INBOX c").iter().map(|(e, m)| (e.0, m.clone())).collect::<Vec<_>>(), [("w/3 Inbox cards", vec![4..11])]);
        assert_eq!(ranked(&entries, "").len(), 4);
        assert!(ranked(&entries, "zz").is_empty());
    }

    #[test]
    fn a_fields_marks_are_counted_from_its_start() {
        assert_eq!(marks_in(&[0..2, 4..9, 12..13], 3..10), [1..6]);
        assert_eq!(marks_in(&[0..2, 4..9], 0..5), [0..2, 4..5]);
        assert!(marks_in(&[0..2], 5..9).is_empty());
    }
}
