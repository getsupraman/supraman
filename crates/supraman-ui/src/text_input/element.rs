//! The engine both modes share: the text shaped and laid out over its rows, the selection, the IME's underline and the caret.

use std::{ops::Range, sync::Arc};

use gpui::{
    App, AvailableSpace, Bounds, ElementId, ElementInputHandler, Entity, GlobalElementId, Hsla, LayoutId, LineLayout, PaintQuad, Pixels, Point,
    SharedString, Style, TextRun, UnderlineStyle, Window, WrappedLine, fill, point, prelude::*, px, relative, size,
};
use theme::ActiveTheme as _;

use super::line::{LineRows, Rows, row_of, wraps};
use super::{CARET_ROOM, MAX_ROWS, MIN_ROWS, TextInput};
use crate::tokens;

/// What was painted last: each text line's shaped layout and where it starts, and the visual rows.
#[derive(Clone, Default)]
pub(super) struct Layout {
    /// The text shaped; empty while the placeholder shows.
    pub(super) text: SharedString,
    lines: Vec<(usize, Arc<LineLayout>)>,
    pub(super) rows: Vec<Range<usize>>,
    /// Per row: its text line, and the x its start has in that line's unwrapped layout.
    row_line: Vec<(usize, Pixels)>,
}

impl Layout {
    fn new(text: &str, lines: &[WrappedLine]) -> Self {
        let mut layout = Self { text: SharedString::from(text.to_string()), ..Default::default() };
        let mut start = 0;
        for (i, line) in lines.iter().enumerate() {
            let unwrapped = line.unwrapped_layout.clone();
            let ends = line.wrap_boundaries().iter().map(|b| unwrapped.runs[b.run_ix].glyphs[b.glyph_ix].index).chain([line.len()]);
            let mut from = 0;
            for end in ends {
                layout.rows.push(start + from..start + end);
                layout.row_line.push((i, unwrapped.x_for_index(from)));
                from = end;
            }
            layout.lines.push((start, unwrapped));
            start += line.len() + 1;
        }
        if layout.rows.is_empty() {
            layout.rows.push(0..0);
            layout.row_line.push((0, Pixels::ZERO));
        }
        layout
    }

    /// `offset`'s x on `row` (a wrapped row's end is its own, not the next row's start).
    pub(super) fn x_in_row(&self, row: usize, offset: usize) -> Pixels {
        let (line, start_x) = self.row_line[row];
        match self.lines.get(line) {
            Some((start, l)) => l.x_for_index(offset.saturating_sub(*start).min(l.len)) - start_x,
            None => Pixels::ZERO,
        }
    }

    fn x(&self, offset: usize) -> Pixels {
        self.x_in_row(row_of(&self.rows, offset), offset)
    }
}

impl Rows for Layout {
    fn rows(&self) -> &[Range<usize>] {
        &self.rows
    }

    fn x_for(&self, offset: usize) -> f32 {
        f32::from(self.x(offset))
    }

    fn offset_at(&self, row: usize, x: f32) -> usize {
        let Some(r) = self.rows.get(row) else {
            return self.text.len();
        };
        let (line, start_x) = self.row_line[row];
        let Some((start, l)) = self.lines.get(line) else {
            return r.start;
        };
        let at = (start + l.closest_index_for_x(px(x) + start_x)).clamp(r.start, r.end);
        // a wrapped row's end is painted as the next row's start: stay on this row
        match self.text[r.clone()].chars().next_back() {
            Some(c) if at == r.end && wraps(&self.rows, row) => r.end - c.len_utf8(),
            _ => at,
        }
    }
}

impl TextInput {
    /// What is shaped: the text with the IME's composition underlined, or the placeholder.
    pub(super) fn shaping(&self, window: &Window, cx: &App) -> (SharedString, Vec<TextRun>, Pixels) {
        let text = self.line.text();
        let style = window.text_style();
        let (shown, color) = if text.is_empty() {
            let placeholder = if self.read_only { SharedString::default() } else { self.placeholder.clone() };
            (placeholder, cx.theme().colors().text_placeholder)
        } else {
            (self.shown(), style.color)
        };
        let run = TextRun { len: shown.len(), font: style.font(), color, background_color: None, underline: None, strikethrough: None };
        let runs = match self.line.marked().filter(|_| !text.is_empty()) {
            Some(m) => vec![
                TextRun { len: m.start, ..run.clone() },
                TextRun { len: m.end - m.start, underline: Some(UnderlineStyle { color: Some(color), thickness: tokens::stroke::HAIRLINE, wavy: false }), ..run.clone() },
                TextRun { len: shown.len() - m.end, ..run },
            ]
            .into_iter()
            .filter(|r| r.len > 0)
            .collect(),
            None => vec![run],
        };
        (shown, runs, style.font_size.to_pixels(window.rem_size()))
    }
}

/// The visual rows painted last, or the text lines while the last paint is older than the text.
pub(super) fn painted_rows<'a>(layout: &'a Option<Layout>, text: &str) -> Box<dyn Rows + 'a> {
    match layout.as_ref().filter(|l| l.text.as_ref() == text) {
        Some(layout) => Box::new(layout.clone()),
        None => Box::new(LineRows::new(text)),
    }
}

/// The input's text (or the placeholder): wrapped rows when multi-line, the selection, the IME's underline
/// and the caret.
pub(super) struct TextElement {
    pub(super) input: Entity<TextInput>,
}

pub(super) struct Prepainted {
    lines: Vec<WrappedLine>,
    /// The visual row each text line starts on.
    first_rows: Vec<usize>,
    layout: Layout,
    scroll: Point<Pixels>,
    selections: Vec<PaintQuad>,
    caret: Option<PaintQuad>,
}

impl IntoElement for TextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = Prepainted;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(&mut self, _: Option<&GlobalElementId>, _: Option<&gpui::InspectorElementId>, window: &mut Window, cx: &mut App) -> (LayoutId, ()) {
        let input = self.input.read(cx);
        let line_height = window.line_height();
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        if !input.wrap {
            style.size.height = line_height.into();
            return (window.request_layout(style, [], cx), ());
        }
        let (shown, runs, font_size) = input.shaping(window, cx);
        let plain = input.plain;
        let id = window.request_measured_layout(style, move |known, available, window, _| {
            let width = known.width.or(match available.width {
                AvailableSpace::Definite(w) => Some(w),
                _ => None,
            });
            let rows: usize = window
                .text_system()
                .shape_text(shown.clone(), font_size, &runs, width.map(|w| w - CARET_ROOM), None)
                .map_or(1, |lines| lines.iter().map(|l| l.wrap_boundaries().len() + 1).sum());
            let rows = if plain { rows.max(1) } else { rows.clamp(MIN_ROWS, MAX_ROWS) };
            size(width.unwrap_or_default(), line_height * rows as f32)
        });
        (id, ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Prepainted {
        let input = self.input.read(cx);
        let text = input.line.text();
        let line_height = window.line_height();
        let (shown, runs, font_size) = input.shaping(window, cx);
        let wrap = input.wrap.then(|| bounds.size.width - CARET_ROOM);
        let lines: Vec<WrappedLine> = window.text_system().shape_text(shown, font_size, &runs, wrap, None).map(|l| l.into_iter().collect()).unwrap_or_default();
        let layout = if text.is_empty() { Layout::new("", &[]) } else { Layout::new(&input.shown(), &lines) };
        let mut first_rows = Vec::with_capacity(lines.len());
        let mut rows = 0;
        for line in &lines {
            first_rows.push(rows);
            rows += line.wrap_boundaries().len() + 1;
        }
        let cursor = input.line.cursor();
        let caret_row = row_of(&layout.rows, cursor);
        let caret = point(layout.x_in_row(caret_row, cursor), line_height * caret_row as f32);
        let scroll = input.scrolled(bounds, &lines, rows, caret, line_height);
        let lit = input.caret_shown(window, cx);
        input.caret_painted.set(lit);
        let origin = bounds.origin - scroll;
        let roles = tokens::roles(cx);
        let fill_color = if input.focus_handle.is_focused(window) { roles.selection } else { roles.selection_inactive };
        let selections = selection_quads(&layout, input.line.selection(), text.len(), origin, line_height, fill_color);
        let caret = lit.then(|| fill(Bounds::new(origin + caret, size(px(1.5), line_height)), cx.theme().colors().text));
        self.input.update(cx, |input, _| input.scroll = scroll);
        Prepainted { lines, first_rows, layout, scroll, selections, caret }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        prepaint: &mut Prepainted,
        window: &mut Window,
        cx: &mut App,
    ) {
        let (focus_handle, read_only) = { let input = self.input.read(cx); (input.focus_handle.clone(), input.read_only) };
        if !read_only {
            window.handle_input(&focus_handle, ElementInputHandler::new(bounds, self.input.clone()), cx);
        }
        let line_height = window.line_height();
        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
            for selection in prepaint.selections.drain(..) {
                window.paint_quad(selection);
            }
            let origin = bounds.origin - prepaint.scroll;
            for (line, first) in prepaint.lines.iter().zip(&prepaint.first_rows) {
                line.paint(point(origin.x, origin.y + line_height * *first as f32), line_height, gpui::TextAlign::Left, None, window, cx).ok();
            }
            if let Some(caret) = prepaint.caret.take() {
                window.paint_quad(caret);
            }
        });
        let layout = std::mem::take(&mut prepaint.layout);
        self.input.update(cx, |input, _| {
            input.last_layout = Some(layout);
            input.last_bounds = Some(bounds);
            input.last_line_height = line_height;
            input.autoscroll = false;
        });
    }
}

impl TextInput {
    /// Where the text is scrolled this frame: the caret kept in view, a move the scrollbar asked for taken, the rows read into it.
    fn scrolled(&self, bounds: Bounds<Pixels>, lines: &[WrappedLine], rows: usize, caret: Point<Pixels>, line_height: Pixels) -> Point<Pixels> {
        let mut scroll = self.scroll;
        if !self.wrap {
            let width = bounds.size.width - CARET_ROOM;
            let line_width = lines.first().map_or(Pixels::ZERO, |l| l.unwrapped_layout.width);
            scroll.y = Pixels::ZERO;
            scroll.x = scroll.x.min((line_width - width).max(Pixels::ZERO));
            if caret.x - scroll.x > width {
                scroll.x = caret.x - width;
            } else if caret.x < scroll.x {
                scroll.x = caret.x;
            }
            return scroll;
        }
        let (height, content) = (bounds.size.height, line_height * rows.max(1) as f32);
        scroll.x = Pixels::ZERO;
        match self.bar.take_wanted() {
            Some(top) => scroll.y = top,
            None if self.autoscroll && caret.y < scroll.y => scroll.y = caret.y,
            None if self.autoscroll && caret.y + line_height > scroll.y + height => scroll.y = caret.y + line_height - height,
            None => {}
        }
        scroll.y = scroll.y.clamp(Pixels::ZERO, (content - height).max(Pixels::ZERO));
        self.bar.read(bounds, content, scroll.y);
        scroll
    }
}

/// The selection's fill per row; a selected newline shows as a little room after its row.
fn selection_quads(layout: &Layout, selected: Range<usize>, len: usize, origin: Point<Pixels>, line_height: Pixels, color: Hsla) -> Vec<PaintQuad> {
    if selected.is_empty() {
        return Vec::new();
    }
    let newline = px(5.);
    let quad = |(i, r): (usize, &Range<usize>)| {
        let (start, end) = (selected.start.max(r.start), selected.end.min(r.end));
        let past = r.end < len && !wraps(&layout.rows, i) && selected.start <= r.end && selected.end > r.end;
        (start < end || past).then(|| {
            let top = origin.y + line_height * i as f32;
            let right = layout.x_in_row(i, end) + if past { newline } else { Pixels::ZERO };
            fill(Bounds::from_corners(point(origin.x + layout.x_in_row(i, start), top), point(origin.x + right, top + line_height)), color)
        })
    };
    layout.rows.iter().enumerate().filter_map(quad).collect()
}
