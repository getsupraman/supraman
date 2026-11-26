//! The one list core (UI-KIT §4.6): a list's [`Selection`] and the keys that move it ([`key_of`], [`Rows`]), and its rows'
//! fills ([`Chrome`], [`row`]). `table` (so `tree`), `keyed_list`, `palette` and `menu` compose it, each keeping its own
//! layout: the table its columns, the menu its popup; a feature supplies rows and their content, never a row's fill.

use std::time::{Duration, Instant};

use gpui::Keystroke;

pub mod table;
pub mod tree;
pub mod palette;
pub mod keyed_list;
mod chrome;
pub mod nav;

pub use chrome::{Chrome, row};

/// Where a key moves the selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    By(isize),
    /// A page of the rows in view, down when positive.
    Pages(isize),
    First,
    Last,
}

/// What a key asks of a list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    Step(Step),
    /// A letter or digit typed: type-ahead on the rows' text.
    Typed(String),
    /// Enter: the selected row's own action.
    Activate,
}

/// The one key map of every list: Up/Down, PageUp/PageDown, Home/End (Ctrl+Home/End, Ctrl+Up/Down too), Enter, and a
/// typed letter or digit; none with Alt or the platform key, and only the ends with Ctrl.
pub fn key_of(k: &Keystroke) -> Option<Key> {
    let m = &k.modifiers;
    if m.alt || m.platform || m.function {
        return None;
    }
    let step = match (m.control, k.key.as_str()) {
        (true, "home" | "up") => Step::First,
        (true, "end" | "down") => Step::Last,
        (true, _) => return None,
        (false, "up") => Step::By(-1),
        (false, "down") => Step::By(1),
        (false, "pageup") => Step::Pages(-1),
        (false, "pagedown") => Step::Pages(1),
        (false, "home") => Step::First,
        (false, "end") => Step::Last,
        (false, "enter") => return Some(Key::Activate),
        (false, typed) if typed.chars().count() == 1 && typed.chars().all(char::is_alphanumeric) => return Some(Key::Typed(typed.to_string())),
        _ => return None,
    };
    Some(Key::Step(step))
}

/// How long typed letters add up to one type-ahead prefix.
pub const TYPED_FOR: Duration = Duration::from_millis(800);

/// The rows a key moves over: how many, a page of them, whether Up and Down wrap (a menu's), and which can be selected.
#[derive(Clone, Copy, Debug)]
pub struct Rows<'a> {
    pub len: usize,
    pub page: usize,
    pub wrap: bool,
    pub acts: Option<&'a [bool]>,
}

impl<'a> Rows<'a> {
    /// `len` rows, every one selectable, a page of one until [`Rows::page`].
    pub fn new(len: usize) -> Self {
        Self { len, page: 1, wrap: false, acts: None }
    }

    pub fn page(self, page: usize) -> Self {
        Self { page: page.max(1), ..self }
    }

    pub fn wrapping(self) -> Self {
        Self { wrap: true, ..self }
    }

    pub fn acting(self, acts: &'a [bool]) -> Self {
        Self { acts: Some(acts), ..self }
    }

    fn acts(&self, ix: usize) -> bool {
        self.acts.is_none_or(|a| a.get(ix).copied().unwrap_or(false))
    }

    fn all_acts(&self) -> Vec<bool> {
        (0..self.len).map(|i| self.acts(i)).collect()
    }

    /// The selectable row nearest `target`, looking on in `forward`'s direction first.
    fn nearest(&self, target: usize, forward: bool) -> Option<usize> {
        let target = target.min(self.len.checked_sub(1)?);
        let ahead = |i: usize| self.acts(i).then_some(i);
        let on: Option<usize> = if forward { (target..self.len).find_map(ahead) } else { (0..=target).rev().find_map(ahead) };
        on.or_else(|| if forward { (0..target).rev().find_map(ahead) } else { (target..self.len).find_map(ahead) })
    }
}

/// One list's selection and type-ahead; the owner keeps it with its rows.
#[derive(Clone, Debug, Default)]
pub struct Selection {
    selected: Option<usize>,
    typed: String,
    typed_at: Option<Instant>,
}

impl Selection {
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    pub fn select(&mut self, row: Option<usize>) {
        self.selected = row;
    }

    /// Moves the selection as `step` says; the row it ends on when it moved. A relative step from none selects the first
    /// row (the last going up in a wrapping list); Up and Down wrap only in a wrapping list, pages and ends never.
    pub fn go(&mut self, step: Step, rows: &Rows) -> Option<usize> {
        let last = rows.len.checked_sub(1)?;
        let to = match (step, self.selected.filter(|s| *s <= last)) {
            (Step::First, _) => rows.nearest(0, true),
            (Step::Last, _) => rows.nearest(last, false),
            (Step::By(by), at) if rows.wrap => nav::step(&rows.all_acts(), at, by < 0),
            (Step::By(_) | Step::Pages(_), None) => rows.nearest(0, true),
            (Step::By(by), Some(at)) => rows.nearest(at.saturating_add_signed(by), by > 0).filter(|to| (*to > at) == (by > 0) || *to == at),
            (Step::Pages(n), Some(at)) => rows.nearest(at.saturating_add_signed(n * rows.page as isize), n > 0),
        }?;
        let moved = self.selected != Some(to);
        self.selected = Some(to);
        moved.then_some(to)
    }

    /// Type-ahead: `typed` added to the letters typed within [`TYPED_FOR`], the next row whose text starts with them
    /// ([`nav::typed`]), wrapping; `labels` the rows' text and whether each can be selected.
    pub fn type_ahead(&mut self, typed: &str, labels: &[(&str, bool)], now: Instant) -> Option<usize> {
        if self.typed_at.is_none_or(|t| now.duration_since(t) > TYPED_FOR) {
            self.typed.clear();
        }
        self.typed.push_str(typed);
        self.typed_at = Some(now);
        let at = self.selected;
        let to = nav::typed(labels, at, &self.typed).or_else(|| {
            self.typed = typed.to_string();
            nav::typed(labels, at, typed)
        })?;
        let moved = self.selected != Some(to);
        self.selected = Some(to);
        moved.then_some(to)
    }

    /// A [`key_of`] step or letter on `rows` (`labels` read only for a letter, `None` while the list has no type-ahead); the
    /// row it moved to. [`Key::Activate`] is the owner's.
    pub fn key(&mut self, key: &Key, rows: &Rows, labels: impl FnOnce() -> Option<Vec<(String, bool)>>) -> Option<usize> {
        match key {
            Key::Step(step) => self.go(*step, rows),
            Key::Typed(typed) => {
                let labels = labels()?;
                let labels: Vec<(&str, bool)> = labels.iter().map(|(t, a)| (t.as_str(), *a)).collect();
                self.type_ahead(typed, &labels, Instant::now())
            }
            Key::Activate => None,
        }
    }
}

/// The rows a page holds in `viewport`, rows `row` high; at least one.
pub fn page_of(viewport: gpui::Pixels, row: gpui::Pixels) -> usize {
    if row > gpui::Pixels::ZERO { ((viewport / row).floor() as usize).max(1) } else { 1 }
}

#[cfg(test)]
mod tests;
