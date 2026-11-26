//! The text and its selection as the user edits them, with undo; what Up, Down, Home and End move by.

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation as _;

/// Undo steps kept; the oldest go first.
pub(super) const UNDO_STEPS: usize = 100;

/// What one edit was, for undo: typed characters run together into one step until the caret moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditKind {
    Typing,
    Other,
}

#[derive(Debug, Clone)]
struct Snapshot {
    text: String,
    selected: Range<usize>,
    reversed: bool,
}

/// One line of text, its selection (byte offsets on grapheme boundaries; the caret is the end that moved)
/// and the IME's marked range, with undo and redo.
#[derive(Debug, Clone, Default)]
pub struct LineText {
    text: String,
    selected: Range<usize>,
    reversed: bool,
    marked: Option<Range<usize>>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// The last edit was typing that ended here: the next character typed at it joins its undo step.
    typing_at: Option<usize>,
    /// Where Up/Down aim on the next row: the x the caret had when the run of vertical moves began.
    goal_x: Option<f32>,
}

/// The text's visual rows as they were painted: what Up/Down/Home/End move by in a multi-line input.
pub trait Rows {
    /// Each visual row's byte range, in order, without the newline that ends a text line. A row that wraps
    /// ends where the next begins.
    fn rows(&self) -> &[Range<usize>];
    /// `offset`'s x within its row.
    fn x_for(&self, offset: usize) -> f32;
    /// The offset in `row` closest to `x`.
    fn offset_at(&self, row: usize, x: f32) -> usize;
}

/// The row `offset` is painted on: at a wrap, the next row's start.
pub fn row_of(rows: &[Range<usize>], offset: usize) -> usize {
    rows.iter()
        .enumerate()
        .position(|(i, r)| offset >= r.start && (offset < r.end || (offset == r.end && rows.get(i + 1).is_none_or(|n| n.start != offset))))
        .unwrap_or(rows.len().saturating_sub(1))
}

/// Whether `row` wraps into the next one (ends where the next begins) rather than at a newline.
pub(super) fn wraps(rows: &[Range<usize>], row: usize) -> bool {
    rows.get(row + 1).is_some_and(|n| n.start == rows[row].end) && !rows[row].is_empty()
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl LineText {
    pub fn new(text: &str) -> Self {
        Self { text: text.to_string(), selected: text.len()..text.len(), ..Default::default() }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn selection(&self) -> Range<usize> {
        self.selected.clone()
    }

    pub fn marked(&self) -> Option<Range<usize>> {
        self.marked.clone()
    }

    pub fn cursor(&self) -> usize {
        if self.reversed { self.selected.start } else { self.selected.end }
    }

    /// The whole text replaced, the caret at its end, the history gone.
    pub fn set(&mut self, text: &str) {
        *self = Self::new(text);
    }

    pub fn move_to(&mut self, offset: usize) {
        let offset = self.clamp(offset);
        self.selected = offset..offset;
        self.reversed = false;
        self.typing_at = None;
        self.goal_x = None;
    }

    /// The caret to `offset`, the other end of the selection staying where it is.
    pub fn select_to(&mut self, offset: usize) {
        let offset = self.clamp(offset);
        if self.reversed {
            self.selected.start = offset;
        } else {
            self.selected.end = offset;
        }
        if self.selected.end < self.selected.start {
            self.reversed = !self.reversed;
            self.selected = self.selected.end..self.selected.start;
        }
        self.typing_at = None;
        self.goal_x = None;
    }

    /// `range` selected, the caret at its end (or its start when `reversed`).
    pub fn select(&mut self, range: Range<usize>, reversed: bool) {
        let (start, end) = (self.clamp(range.start), self.clamp(range.end));
        self.selected = start.min(end)..start.max(end);
        self.reversed = reversed;
        self.typing_at = None;
        self.goal_x = None;
    }

    pub fn select_all(&mut self) {
        self.select(0..self.text.len(), false);
    }

    fn clamp(&self, offset: usize) -> usize {
        let mut offset = offset.min(self.text.len());
        while !self.text.is_char_boundary(offset) {
            offset -= 1;
        }
        offset
    }

    pub fn prev_grapheme(&self, offset: usize) -> usize {
        self.text.grapheme_indices(true).rev().find_map(|(i, _)| (i < offset).then_some(i)).unwrap_or(0)
    }

    pub fn next_grapheme(&self, offset: usize) -> usize {
        self.text.grapheme_indices(true).find_map(|(i, _)| (i > offset).then_some(i)).unwrap_or(self.text.len())
    }

    /// Ctrl+Left: the start of the word before `offset`, over any separators first.
    pub fn word_left(&self, offset: usize) -> usize {
        let before: Vec<(usize, char)> = self.text[..offset].char_indices().collect();
        let mut i = before.len();
        while i > 0 && !is_word(before[i - 1].1) {
            i -= 1;
        }
        while i > 0 && is_word(before[i - 1].1) {
            i -= 1;
        }
        before.get(i).map_or(offset, |(at, _)| *at)
    }

    /// Ctrl+Right: the end of the word after `offset`, over any separators first.
    pub fn word_right(&self, offset: usize) -> usize {
        let mut chars = self.text[offset..].char_indices().peekable();
        while chars.next_if(|(_, c)| !is_word(*c)).is_some() {}
        while chars.next_if(|(_, c)| is_word(*c)).is_some() {}
        chars.peek().map_or(self.text.len(), |(at, _)| offset + at)
    }

    /// Up (`delta` -1) or Down (+1) by visual rows, the caret aiming at the x it had when the run of vertical
    /// moves began; past the first row it goes to the start, past the last to the end. `select` extends.
    pub fn move_vertically(&mut self, rows: &(impl Rows + ?Sized), delta: isize, select: bool) {
        let caret = self.cursor();
        let all = rows.rows();
        let x = self.goal_x.unwrap_or_else(|| rows.x_for(caret));
        let target = row_of(all, caret) as isize + delta;
        let to = if target < 0 {
            0
        } else if target as usize >= all.len() {
            self.text.len()
        } else {
            rows.offset_at(target as usize, x)
        };
        if select { self.select_to(to) } else { self.move_to(to) }
        self.goal_x = Some(x);
    }

    /// Home: the start of the caret's visual row.
    pub fn row_start(&self, rows: &[Range<usize>]) -> usize {
        rows.get(row_of(rows, self.cursor())).map_or(0, |r| r.start)
    }

    /// End: the end of the caret's visual row; before its last character when the row wraps, which would
    /// put the caret on the next row.
    pub fn row_end(&self, rows: &[Range<usize>]) -> usize {
        let row = row_of(rows, self.cursor());
        match rows.get(row) {
            Some(r) if wraps(rows, row) => self.prev_grapheme(r.end).max(r.start),
            Some(r) => r.end,
            None => self.text.len(),
        }
    }

    /// What a double-click at `offset` selects: the word there, or the run of separators.
    pub fn word_at(&self, offset: usize) -> Range<usize> {
        let offset = self.clamp(offset);
        let at = self.text[offset..].chars().next().or_else(|| self.text[..offset].chars().next_back());
        let Some(at) = at else {
            return offset..offset;
        };
        let kind = |c: char| if c.is_whitespace() { 0 } else if is_word(c) { 1 } else { 2 };
        let same = |c: char| kind(c) == kind(at);
        let start = self.text[..offset].char_indices().rev().take_while(|(_, c)| same(*c)).last().map_or(offset, |(i, _)| i);
        let end = self.text[offset..].char_indices().find(|(_, c)| !same(*c)).map_or(self.text.len(), |(i, _)| offset + i);
        start..end
    }

    /// What a triple-click at `offset` selects: its text line, without the newline that ends it.
    pub fn line_at(&self, offset: usize) -> Range<usize> {
        let offset = self.clamp(offset);
        let start = self.text[..offset].rfind('\n').map_or(0, |i| i + 1);
        let end = self.text[offset..].find('\n').map_or(self.text.len(), |i| offset + i);
        start..end
    }

    /// `range` replaced by `new`, the caret after it; one undo step unless typing continues the last one.
    pub fn replace(&mut self, range: Range<usize>, new: &str, kind: EditKind) {
        let range = self.clamp(range.start)..self.clamp(range.end);
        // a composition being committed already pushed its step when it began
        let joins = self.marked.is_some()
            || (kind == EditKind::Typing && range.is_empty() && self.typing_at == Some(range.start) && !new.contains(char::is_whitespace));
        if !joins {
            self.push_undo();
        }
        self.redo.clear();
        self.text.replace_range(range.clone(), new);
        let caret = range.start + new.len();
        self.selected = caret..caret;
        self.reversed = false;
        self.marked = None;
        self.typing_at = (kind == EditKind::Typing).then_some(caret);
        self.goal_x = None;
    }

    /// The IME's composition: `new` replaces `range` and stays marked, `selected` (relative to `new`) selected
    /// in it. Undo takes back the whole composition in one step.
    pub fn replace_marked(&mut self, range: Range<usize>, new: &str, selected: Option<Range<usize>>) {
        let range = self.clamp(range.start)..self.clamp(range.end);
        if self.marked.is_none() {
            self.push_undo();
            self.redo.clear();
        }
        self.text.replace_range(range.clone(), new);
        self.marked = (!new.is_empty()).then(|| range.start..range.start + new.len());
        let inside = |o: usize| range.start + o.min(new.len());
        self.selected = selected.map_or(range.start + new.len()..range.start + new.len(), |s| inside(s.start)..inside(s.end));
        self.selected = self.clamp(self.selected.start)..self.clamp(self.selected.end);
        self.reversed = false;
        self.typing_at = None;
    }

    pub fn unmark(&mut self) {
        self.marked = None;
    }

    fn push_undo(&mut self) {
        if self.undo.len() == UNDO_STEPS {
            self.undo.remove(0);
        }
        self.undo.push(self.snapshot());
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot { text: self.text.clone(), selected: self.selected.clone(), reversed: self.reversed }
    }

    fn restore(&mut self, s: Snapshot) {
        self.text = s.text;
        self.selected = s.selected;
        self.reversed = s.reversed;
        self.marked = None;
        self.typing_at = None;
    }

    pub fn undo(&mut self) -> bool {
        let Some(s) = self.undo.pop() else {
            return false;
        };
        self.redo.push(self.snapshot());
        self.restore(s);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(s) = self.redo.pop() else {
            return false;
        };
        self.push_undo();
        self.restore(s);
        true
    }

    pub fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf16 = 0;
        for (i, c) in self.text.char_indices() {
            if utf16 >= offset {
                return i;
            }
            utf16 += c.len_utf16();
        }
        self.text.len()
    }

    pub fn offset_to_utf16(&self, offset: usize) -> usize {
        self.text[..self.clamp(offset)].encode_utf16().count()
    }

    pub fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    pub fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range.start)..self.offset_from_utf16(range.end)
    }
}

/// What `accept` takes of `text`: each character mapped, the refused ones dropped, the first refused returned.
pub fn filtered(text: &str, accept: fn(char) -> Option<char>) -> (String, Option<char>) {
    let mut refused = None;
    let taken = text
        .chars()
        .filter_map(|c| {
            let taken = accept(c);
            if taken.is_none() && refused.is_none() {
                refused = Some(c);
            }
            taken
        })
        .collect();
    (taken, refused)
}
/// [`Rows`] without a painted layout: each text line one row, x counted in characters.
pub struct LineRows {
    pub(super) rows: Vec<Range<usize>>,
    pub(super) text: String,
}

impl LineRows {
    pub fn new(text: &str) -> Self {
        let mut start = 0;
        let rows = text
            .split('\n')
            .map(|line| {
                let row = start..start + line.len();
                start = row.end + 1;
                row
            })
            .collect();
        Self { rows, text: text.to_string() }
    }
}

impl Rows for LineRows {
    fn rows(&self) -> &[Range<usize>] {
        &self.rows
    }

    fn x_for(&self, offset: usize) -> f32 {
        let row = &self.rows[row_of(&self.rows, offset)];
        self.text[row.start..offset.clamp(row.start, row.end)].chars().count() as f32
    }

    fn offset_at(&self, row: usize, x: f32) -> usize {
        let r = &self.rows[row];
        self.text[r.clone()].char_indices().nth(x.round().max(0.) as usize).map_or(r.end, |(i, _)| r.start + i)
    }
}

/// Typed text without its tabs: Tab moves between a form's fields (`focus_order`), it never types.
pub(super) fn without_tabs(text: &str) -> String {
    text.replace('\t', "")
}

/// What a paste inserts: on one line its line breaks as spaces (a trailing one dropped), on several `\n`.
pub fn pasted(text: &str, multi_line: bool) -> String {
    if multi_line {
        text.replace("\r\n", "\n").replace('\r', "\n")
    } else {
        text.trim_end_matches(['\r', '\n']).replace("\r\n", " ").replace(['\r', '\n'], " ")
    }
}
