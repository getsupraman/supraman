// Vendored from Zed crates/terminal/src/terminal.rs @ 03efd49220ba06703d04f9c4732fdf814f9c523c
// (GPL-3.0-or-later).

use crate::alacritty::{
    AlacrittyCell, AlacrittyHyperlink, content_text, display_offset, make_content, total_lines,
};
use crate::terminal_settings::{CURSOR_BLINK_INTERVAL, CursorShape as SettingsCursorShape};
use crate::{Event, Terminal, TerminalBounds};
use gpui::{Context, Window};
use std::ops::{BitOr, BitOrAssign, Deref};
use std::time::{Duration, Instant};
use vte::ansi::{Color, NamedColor};

pub fn is_default_background_color(color: Color) -> bool {
    matches!(color, Color::Named(NamedColor::Background))
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Hyperlink(pub(crate) AlacrittyHyperlink);

#[derive(Default, Debug, Clone, Eq, PartialEq)]
pub struct Cell {
    pub(crate) cell: AlacrittyCell,
}

#[derive(Debug, Clone)]
pub struct IndexedCell {
    pub point: Point,
    pub cell: Cell,
}

impl Deref for IndexedCell {
    type Target = Cell;

    #[inline]
    fn deref(&self) -> &Cell {
        &self.cell
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Modes(u32);

impl Modes {
    pub(crate) const NONE: Self = Self(0);
    pub(crate) const APP_CURSOR: Self = Self(1 << 0);
    pub(crate) const APP_KEYPAD: Self = Self(1 << 1);
    pub(crate) const SHOW_CURSOR: Self = Self(1 << 2);
    pub(crate) const LINE_WRAP: Self = Self(1 << 3);
    pub(crate) const ORIGIN: Self = Self(1 << 4);
    pub(crate) const INSERT: Self = Self(1 << 5);
    pub(crate) const LINE_FEED_NEW_LINE: Self = Self(1 << 6);
    pub(crate) const FOCUS_IN_OUT: Self = Self(1 << 7);
    pub(crate) const ALTERNATE_SCROLL: Self = Self(1 << 8);
    pub(crate) const BRACKETED_PASTE: Self = Self(1 << 9);
    pub(crate) const SGR_MOUSE: Self = Self(1 << 10);
    pub(crate) const UTF8_MOUSE: Self = Self(1 << 11);
    pub(crate) const ALT_SCREEN: Self = Self(1 << 12);
    pub(crate) const MOUSE_REPORT_CLICK: Self = Self(1 << 13);
    pub(crate) const MOUSE_DRAG: Self = Self(1 << 14);
    pub(crate) const MOUSE_MOTION: Self = Self(1 << 15);
    pub(crate) const VI: Self = Self(1 << 16);
    pub const MOUSE_MODE: Self =
        Self(Self::MOUSE_REPORT_CLICK.0 | Self::MOUSE_DRAG.0 | Self::MOUSE_MOTION.0);

    pub(crate) const fn empty() -> Self {
        Self::NONE
    }

    pub(crate) const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    pub(crate) fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }
}

impl BitOr for Modes {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for Modes {
    fn bitor_assign(&mut self, rhs: Self) {
        self.insert(rhs);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Cursor {
    pub shape: CursorShape,
    pub point: Point,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CursorShape {
    Block,
    Underline,
    Bar,
    HollowBlock,
    Hidden,
}

impl From<SettingsCursorShape> for CursorShape {
    fn from(shape: SettingsCursorShape) -> Self {
        match shape {
            SettingsCursorShape::Block => Self::Block,
            SettingsCursorShape::Underline => Self::Underline,
            SettingsCursorShape::Bar => Self::Bar,
            SettingsCursorShape::Hollow => Self::HollowBlock,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Point {
    pub line: i32,
    pub column: usize,
}

impl Point {
    pub fn new(line: i32, column: usize) -> Self {
        Self { line, column }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Range {
    pub(crate) start: Point,
    pub(crate) end: Point,
}

impl Range {
    pub fn new(start: Point, end: Point) -> Self {
        Self { start, end }
    }

    pub fn start(&self) -> Point {
        self.start
    }

    pub fn end(&self) -> Point {
        self.end
    }

    pub(crate) fn contains(&self, point: Point) -> bool {
        self.start <= point && point <= self.end
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SelectionRange {
    pub start: Point,
    pub end: Point,
    pub is_block: bool,
}

// TODO: Un-pub
#[derive(Clone)]
pub struct Content {
    pub cells: Vec<IndexedCell>,
    pub mode: Modes,
    pub total_lines: usize,
    pub display_offset: usize,
    pub columns: usize,
    pub screen_lines: usize,
    pub selection: Option<SelectionRange>,
    pub cursor: Cursor,
    /// The cursor style blinks: the default one does, a program's DECSCUSR may make it steady.
    pub cursor_blinking: bool,
    pub cursor_char: char,
    pub terminal_bounds: TerminalBounds,
    pub last_hovered_word: Option<HoveredWord>,
    pub grid_lines_change: GridLinesChange,
    pub scrolled_to_top: bool,
    pub scrolled_to_bottom: bool,
    pub bottom_row_occupied: bool,
}

#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub enum GridLinesChange {
    #[default]
    Unchanged,
    Changed,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct HoveredWord {
    pub word: String,
    pub word_match: Range,
    pub id: usize,
}

impl Default for Content {
    fn default() -> Self {
        Content {
            cells: Default::default(),
            mode: Default::default(),
            total_lines: Default::default(),
            display_offset: Default::default(),
            columns: Default::default(),
            screen_lines: Default::default(),
            selection: Default::default(),
            cursor: Cursor {
                shape: CursorShape::Block,
                point: Point::new(0, 0),
            },
            cursor_blinking: false,
            cursor_char: Default::default(),
            terminal_bounds: Default::default(),
            last_hovered_word: None,
            grid_lines_change: Default::default(),
            scrolled_to_top: false,
            scrolled_to_bottom: false,
            bottom_row_occupied: false,
        }
    }
}

/// Lit for the first interval after a move, then off and on every [`CURSOR_BLINK_INTERVAL`]; with the
/// time until the next change.
pub fn blink_phase(since_move: Duration) -> (bool, Duration) {
    let interval = CURSOR_BLINK_INTERVAL.as_nanos();
    let elapsed = since_move.as_nanos();
    let lit = (elapsed / interval) % 2 == 0;
    let next = Duration::from_nanos((interval - elapsed % interval) as u64);
    (lit, next)
}

impl Terminal {
    pub fn last_content(&self) -> &Content {
        &self.last_content
    }

    pub fn total_lines(&self) -> usize {
        total_lines(&self.term.lock_unfair())
    }

    /// `(display_offset, total_lines)` of the grid now; `last_content` has them as of the last frame.
    pub fn grid_extent(&self) -> (usize, usize) {
        let term = self.term.lock_unfair();
        (display_offset(&term), total_lines(&term))
    }

    /// Whether the cursor is lit at `now`, and how long until that changes (`None`: it does not blink).
    pub fn cursor_blink(&self, now: Instant) -> (bool, Option<Duration>) {
        if !self.last_content.cursor_blinking {
            return (true, None);
        }
        let (lit, next) = blink_phase(now.saturating_duration_since(self.cursor_moved_at));
        (lit, Some(next))
    }

    pub fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let term = self.term.clone();
        let mut terminal = term.lock_unfair();
        //Note that the ordering of events matters for event processing
        while let Some(e) = self.events.pop_front() {
            self.process_terminal_event(&e, &mut terminal, window, cx)
        }

        let cursor_before = self.last_content.cursor.point;
        self.last_content = make_content(&terminal, &self.last_content);
        if self.last_content.cursor.point != cursor_before {
            self.cursor_moved_at = Instant::now();
        }
        if self.last_content.grid_lines_change == GridLinesChange::Changed {
            debug_assert!(self.last_content.last_hovered_word.is_none());
            self.refresh_hovered_word(window, cx);

            // Because refresh_hovered_word() may result
            // in new events, but will not trigger a repaint
            if !self.events.is_empty() {
                cx.emit(Event::Wakeup);
            }
        }
    }

    pub fn get_content(&self) -> String {
        let term = self.term.lock_unfair();
        content_text(&term)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn terminal_cell_clone_shares_extra_storage() {
        let mut cell = Cell::default();
        cell.push_zerowidth('a');

        let clone = cell.clone();

        match (&cell.cell.extra, &clone.cell.extra) {
            (Some(extra), Some(clone_extra)) => assert!(Arc::ptr_eq(extra, clone_extra)),
            _ => panic!("expected extra storage on both cells"),
        }
    }

    #[test]
    fn blink_is_lit_for_one_interval_after_a_move_then_alternates() {
        let ms = Duration::from_millis;
        assert_eq!(blink_phase(ms(0)), (true, ms(530)));
        assert_eq!(blink_phase(ms(529)), (true, ms(1)));
        assert_eq!(blink_phase(ms(530)), (false, ms(530)));
        assert_eq!(blink_phase(ms(1000)), (false, ms(60)));
        assert_eq!(blink_phase(ms(1060)), (true, ms(530)));
    }
}
