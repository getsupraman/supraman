//! A Ctrl+P palette over its owner's entries: a query box, the entries its characters match in order
//! ([`matcher::subsequence`]) ranked best first, then the owner's remote hits ([`Palette::set_remote`]);
//! Up/Down move, Enter or a click opens ([`PaletteEvent::Open`]), Esc or a click outside closes.
//! [`Mode`]s search other sources: entered by an entry's [`Entry::mode`], a typed prefix or [`Palette::enter_mode`],
//! shown as a chip left of the query, left by Backspace on an empty query.
//! The owner places it, paints each row and acts on its events; rows paint under `<id>-row-<key>`.

use std::cmp::Reverse;
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, AnyElement, App, ClickEvent, Context, Div, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, MouseButton, Render,
    SharedString, Subscription, Transformation, Window, div, percentage, prelude::*, px,
};
use theme::ActiveTheme as _;

use crate::hit::{HitTargets, hit_target};
use crate::icon::Icon;
use crate::keyed_list::KeyedList;
use crate::matcher::subsequence;
use crate::text_input::{self, InputEvent, TextInput};
use crate::tooltip::Tooltip;
use crate::button;

/// The most rows listed at once; a longer query narrows them.
pub const MAX_SHOWN: usize = 200;
const LIST_HEIGHT: f32 = 380.;
const PLACEHOLDER: &str = "Search refs, titles…";

pub trait Entry: Clone + 'static {
    /// Unique among the entries.
    fn key(&self) -> SharedString;
    /// What the query is matched against; the painter's marks are byte ranges of it.
    fn text(&self) -> String;
    /// The [`Mode`] a pick of it enters, in place of opening it.
    fn mode(&self) -> Option<SharedString> {
        None
    }
    /// Why it cannot act now: listed dimmed with this as its tooltip, a pick of it does nothing.
    fn dimmed(&self) -> Option<SharedString> {
        None
    }
}

pub enum PaletteEvent<T> {
    /// The query after an edit: the owner may look it up elsewhere and hand back [`Palette::set_remote`].
    Query(String),
    Open(T),
    Close,
}

pub type Paint<T> = Rc<dyn Fn(&T, &[Range<usize>], bool, &mut App) -> AnyElement>;
pub type Marks = Vec<Range<usize>>;
/// A match's place in the list, lower first: its class, then its runs of matched characters, then where it starts.
pub type Fit = (u8, usize, usize);
/// The entries a query lists, best first, with their marks.
pub type Rank<T> = Rc<dyn Fn(&[T], &str) -> Vec<(T, Marks)>>;

/// A source searched instead of the top list, under a chip.
pub struct Mode<T> {
    pub key: SharedString,
    pub chip: SharedString,
    pub placeholder: SharedString,
    /// Typed first in the top list, it enters this mode.
    pub prefix: Option<char>,
    /// What an empty list says.
    pub empty: SharedString,
    pub entries: Vec<T>,
    pub rank: Rank<T>,
    /// Its entries are on their way ([`Palette::set_mode_entries`]): a spinner in place of the list.
    pub loading: bool,
}

pub struct Palette<T: Entry> {
    id: SharedString,
    hits: HitTargets,
    input: Entity<TextInput>,
    _input: Subscription,
    local: Vec<T>,
    rank: Rank<T>,
    remote: (String, Vec<T>),
    modes: Vec<Mode<T>>,
    mode: Option<usize>,
    placeholder: SharedString,
    shown: Vec<(T, Marks)>,
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

/// [`subsequence`] as a [`Fit`] of class 0.
pub fn fuzzy(text: &str, query: &str) -> Option<(Fit, Marks)> {
    subsequence(text, query).map(|marks| ((0, marks.len(), marks.first().map_or(0, |m| m.start)), marks))
}

/// A `/` path's match: the whole query within its last segment (class 0) above one spread over its folders (class 1).
pub fn path_fuzzy(path: &str, query: &str) -> Option<(Fit, Marks)> {
    let name = path.rfind('/').map_or(0, |i| i + 1);
    if let Some(marks) = subsequence(&path[name..], query) {
        let first = marks.first().map_or(0, |m| m.start);
        return Some(((0, marks.len(), first), marks.into_iter().map(|m| m.start + name..m.end + name).collect()));
    }
    fuzzy(path, query).map(|((_, runs, first), marks)| ((1, runs, first), marks))
}

/// `entries` that `fit` the query, best fit first, a higher `boost` first among equal fits, then in their order.
pub fn ranked_by<T: Entry>(entries: &[T], query: &str, fit: impl Fn(&T, &str) -> Option<(Fit, Marks)>, boost: impl Fn(&T) -> u32) -> Vec<(T, Marks)> {
    let mut found: Vec<(Fit, Reverse<u32>, usize, T, Marks)> =
        entries.iter().enumerate().filter_map(|(i, e)| fit(e, query).map(|(f, marks)| (f, Reverse(boost(e)), i, e.clone(), marks))).collect();
    found.sort_by_key(|(f, boost, i, ..)| (*f, *boost, *i));
    found.into_iter().take(MAX_SHOWN).map(|(.., e, marks)| (e, marks)).collect()
}

/// `entries` the query matches, best first: fewer runs of matched characters, then an earlier first match; all of them for an empty query.
pub fn ranked<T: Entry>(entries: &[T], query: &str) -> Vec<(T, Marks)> {
    ranked_by(entries, query, |e, q| fuzzy(&e.text(), q), |_| 0)
}

impl<T: Entry> Palette<T> {
    pub fn new(id: impl Into<SharedString>, hits: HitTargets, entries: Vec<T>, paint: Paint<T>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| TextInput::new(PLACEHOLDER, None, window, cx));
        let sub = cx.subscribe(&input, |this: &mut Self, _, e: &InputEvent, cx| match e {
            InputEvent::Edited => this.edited(cx),
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
            rank: Rc::new(ranked),
            remote: (String::new(), Vec::new()),
            modes: Vec::new(),
            mode: None,
            placeholder: PLACEHOLDER.into(),
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

    /// How the top list ranks its entries; [`ranked`] until set.
    pub fn set_rank(&mut self, rank: Rank<T>, cx: &mut Context<Self>) {
        self.rank = rank;
        self.refresh(true, cx);
    }

    /// The top list's placeholder and caption.
    pub fn set_top(&mut self, placeholder: impl Into<SharedString>, caption: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.placeholder = placeholder.into();
        self.caption = caption.into();
        if self.mode.is_none() {
            self.show_placeholder(cx);
        }
        cx.notify();
    }

    pub fn add_mode(&mut self, mode: Mode<T>) {
        self.modes.push(mode);
    }

    /// The entries of mode `key` arrived, or why none: the reason then is its empty list's line.
    pub fn set_mode_entries(&mut self, key: &str, entries: Result<Vec<T>, String>, cx: &mut Context<Self>) {
        let Some(mode) = self.modes.iter_mut().find(|m| m.key == key) else { return };
        mode.loading = false;
        match entries {
            Ok(entries) => mode.entries = entries,
            Err(why) => {
                mode.entries = Vec::new();
                mode.empty = why.into();
            }
        }
        self.refresh(true, cx);
    }

    /// The mode shown, by its key; `None` for the top list.
    pub fn mode(&self) -> Option<SharedString> {
        self.mode.map(|ix| self.modes[ix].key.clone())
    }

    /// Shows mode `key` with an empty query.
    pub fn enter_mode(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(ix) = self.modes.iter().position(|m| m.key == key) else { return };
        self.mode = Some(ix);
        self.input.update(cx, |input, cx| input.set_text("", cx));
        self.show_placeholder(cx);
        self.refresh(false, cx);
    }

    pub fn leave_mode(&mut self, cx: &mut Context<Self>) {
        if self.mode.take().is_some() {
            self.show_placeholder(cx);
            self.refresh(false, cx);
        }
    }

    fn show_placeholder(&mut self, cx: &mut Context<Self>) {
        let placeholder = self.mode.map_or_else(|| self.placeholder.clone(), |ix| self.modes[ix].placeholder.clone());
        self.input.update(cx, |input, cx| input.set_placeholder(placeholder, cx));
    }

    /// An edit: a mode's prefix typed first in the top list enters it, the rest of the query kept.
    fn edited(&mut self, cx: &mut Context<Self>) {
        let query = self.query(cx);
        let prefixed = self.mode.is_none().then(|| query.chars().next()).flatten().and_then(|c| self.modes.iter().position(|m| m.prefix == Some(c)).map(|ix| (ix, c)));
        let query = match prefixed {
            Some((ix, c)) => {
                let rest = query[c.len_utf8()..].to_string();
                self.mode = Some(ix);
                self.input.update(cx, |input, cx| input.set_text(&rest, cx));
                self.show_placeholder(cx);
                rest
            }
            None => query,
        };
        self.refresh(false, cx);
        cx.emit(PaletteEvent::Query(query));
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
        let mut shown = match self.mode {
            Some(ix) => (self.modes[ix].rank)(&self.modes[ix].entries, &query),
            None => (self.rank)(&self.local, &query),
        };
        if self.mode.is_none() && self.remote.0 == query {
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
        let Some(entry) = self.selected().cloned() else { return };
        if entry.dimmed().is_some() {
            return;
        }
        match entry.mode() {
            Some(mode) => self.enter_mode(&mode, cx),
            None => cx.emit(PaletteEvent::Open(entry)),
        }
    }

    fn render_row(&mut self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some((entry, marks)) = self.shown.get(ix).cloned() else { return div().into_any_element() };
        let on = ix == self.selected;
        let colors = cx.theme().colors().clone();
        let body = (self.paint)(&entry, &marks, on, cx);
        let row = button::ghost(&self.hits, self.row_key(&entry.key()), cx)
            .w_full()
            .px_3()
            .py(px(5.))
            .when(on, |d| d.bg(colors.element_selected).child(hit_target(&self.hits, format!("{}-selected", self.id))))
            .child(body)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.selected = ix;
                this.open_selected(cx);
            }));
        match entry.dimmed() {
            Some(why) => button::dimmed(row).tooltip(Tooltip::text(why)).into_any_element(),
            None => row.into_any_element(),
        }
    }

    fn render_query(&self, cx: &mut Context<Self>) -> Div {
        let colors = cx.theme().colors().clone();
        let chip = self.mode.map(|ix| {
            button::ghost(&self.hits, format!("{}-chip", self.id), cx)
                .flex_none()
                .px_2()
                .py(px(1.))
                .rounded_sm()
                .bg(colors.element_selected)
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .child(self.modes[ix].chip.clone())
                .tooltip(Tooltip::text("Backspace on an empty query leaves it"))
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.leave_mode(cx)))
        });
        div().flex().items_center().gap_2().p_2().border_b_1().border_color(colors.border).children(chip).child(div().flex_1().min_w_0().child(self.input.clone()))
    }

    fn render_results(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let line = || div().flex().items_center().gap_2().px_3().py_2().text_sm().text_color(colors.text_muted);
        let mode = self.mode.map(|ix| &self.modes[ix]);
        if mode.is_some_and(|m| m.loading) {
            let turning = Icon::Refresh.svg(14., colors.text_muted).with_animation(
                SharedString::from(format!("{}-loading", self.id)),
                Animation::new(Duration::from_secs(1)).repeat(),
                |icon, turned| icon.with_transformation(Transformation::rotate(percentage(turned))),
            );
            return line().child(hit_target(&self.hits, format!("{}-loading", self.id))).child(turning).child("Reading…").into_any_element();
        }
        if self.shown.is_empty() {
            let empty = mode.map_or_else(|| SharedString::from("Nothing matches."), |m| m.empty.clone());
            return line().child(hit_target(&self.hits, format!("{}-empty", self.id))).child(empty).into_any_element();
        }
        gpui::list(self.list.state(), cx.processor(|this: &mut Self, ix: usize, _, cx| this.render_row(ix, cx))).w_full().h(px(LIST_HEIGHT)).into_any_element()
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
        let caption = self.mode.map_or_else(|| self.caption.clone(), |ix| self.modes[ix].chip.to_uppercase().into());
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
            .capture_action(cx.listener(|this, _: &text_input::Backspace, _, cx| {
                if this.mode.is_some() && this.query(cx).is_empty() {
                    this.leave_mode(cx);
                    cx.stop_propagation();
                }
            }))
            .on_mouse_down_out(cx.listener(|_, _, _, cx| cx.emit(PaletteEvent::Close)))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(self.render_query(cx))
            .child(div().px_3().pt_2().pb_1().text_xs().font_weight(FontWeight::BOLD).text_color(colors.text_muted).child(caption))
            .child(self.render_results(cx))
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
                    .when(self.mode.is_some(), |d| d.child(hint("⌫", "all")))
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

    #[test]
    fn a_match_in_the_file_name_ranks_above_one_through_the_folders() {
        let entries = [E("src/main/view.rs"), E("docs/main.md"), E("main.rs")];
        let found: Vec<(&str, Marks)> = ranked_by(&entries, "main", |e, q| path_fuzzy(e.0, q), |_| 0).into_iter().map(|(e, m)| (e.0, m)).collect();
        assert_eq!(found, [("docs/main.md", vec![5..9]), ("main.rs", vec![0..4]), ("src/main/view.rs", vec![4..8])], "the names first, then the folder's");
    }

    #[test]
    fn a_boost_orders_equal_fits_only() {
        let entries = [E("refresh"), E("refs"), E("reflog")];
        let boost = |e: &E| u32::from(e.0 == "reflog") * 50;
        let names: Vec<&str> = ranked_by(&entries, "ref", |e, q| fuzzy(e.0, q), boost).into_iter().map(|(e, _)| e.0).collect();
        assert_eq!(names, ["reflog", "refresh", "refs"]);
        let boost = |e: &E| u32::from(e.0 == "refresh") * 50;
        let names: Vec<&str> = ranked_by(&entries, "refs", |e, q| fuzzy(e.0, q), boost).into_iter().map(|(e, _)| e.0).collect();
        assert_eq!(names, ["refs", "refresh"], "a better fit beats a boost");
    }
}
