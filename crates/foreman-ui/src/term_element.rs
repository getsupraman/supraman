// Minimal terminal grid painter adapted from Zed's
// crates/terminal_view/src/terminal_element.rs @ 03efd49220ba06703d04f9c4732fdf814f9c523c
// (GPL-3.0-or-later). Kept: cell batching into text runs, background rects,
// colour conversion, cursor, block-element glyph rects (U+2580-U+259F), selection and search
// highlights, mouse handling (register_mouse_listeners). Dropped: hyperlink hover, IME, sextant
// glyph rects, minimum-contrast adjustment, the context menu.

use gpui::{
    App, Bounds, DispatchPhase, Entity, Font, FontFeatures, FontStyle, FontWeight, Hitbox,
    HitboxBehavior, Hsla, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    Pixels, Point as GpuiPoint, StrikethroughStyle, Styled, TextRun, UnderlineStyle, Window,
    canvas, fill, point, px, size,
};
use itertools::Itertools as _;
use std::mem;
use foreman_term::{
    Cell, Color, CursorShape, IndexedCell, Modes, MouseInputMode, NamedColor, Terminal,
    TerminalBounds, is_default_background_color,
};
use theme::{ActiveTheme, Theme};

pub const FONT_FAMILY: &str = "Consolas";
pub const FONT_SIZE: Pixels = px(14.);
pub const MIN_FONT_SIZE: Pixels = px(6.);
pub const MAX_FONT_SIZE: Pixels = px(72.);

/// `size` one step up (`delta` 1) or down (-1), within [`MIN_FONT_SIZE`]..=[`MAX_FONT_SIZE`].
pub fn step_font_size(size: Pixels, delta: i32) -> Pixels {
    (size + px(delta as f32)).clamp(MIN_FONT_SIZE, MAX_FONT_SIZE)
}
const LINE_HEIGHT_FACTOR: f32 = 1.3;

struct BatchedTextRun {
    line: i32,
    column: i32,
    text: String,
    cell_count: usize,
    style: TextRun,
}

impl BatchedTextRun {
    fn new_from_char(line: i32, column: i32, c: char, style: TextRun) -> Self {
        let mut text = String::with_capacity(100);
        text.push(c);
        Self {
            line,
            column,
            text,
            cell_count: 1,
            style,
        }
    }

    fn can_append(&self, other: &TextRun) -> bool {
        self.style.font == other.font
            && self.style.color == other.color
            && self.style.background_color == other.background_color
            && self.style.underline == other.underline
            && self.style.strikethrough == other.strikethrough
    }

    fn append_char(&mut self, c: char, counts_cell: bool) {
        self.text.push(c);
        if counts_cell {
            self.cell_count += 1;
        }
        self.style.len += c.len_utf8();
    }

    fn paint(
        &self,
        origin: GpuiPoint<Pixels>,
        dims: &TerminalBounds,
        font_size: Pixels,
        window: &mut Window,
        cx: &mut App,
    ) {
        let pos = point(
            origin.x + self.column as f32 * dims.cell_width,
            origin.y + self.line as f32 * dims.line_height,
        );
        let shaped = window.text_system().shape_line(
            self.text.clone().into(),
            font_size,
            std::slice::from_ref(&self.style),
            Some(dims.cell_width),
        );
        let _ = shaped.paint(pos, dims.line_height, gpui::TextAlign::Left, None, window, cx);
    }
}

struct LayoutRect {
    line: i32,
    column: i32,
    num_of_cells: usize,
    color: Hsla,
}

impl LayoutRect {
    fn paint(&self, origin: GpuiPoint<Pixels>, dims: &TerminalBounds, window: &mut Window) {
        let position = point(
            (origin.x + self.column as f32 * dims.cell_width).floor(),
            origin.y + self.line as f32 * dims.line_height,
        );
        let size = size(
            (dims.cell_width * self.num_of_cells as f32).ceil(),
            dims.line_height,
        );
        window.paint_quad(fill(Bounds::new(position, size), self.color));
    }
}

/// Block element glyphs are painted on a subcell grid: each terminal cell is
/// divided into 8 columns (for eighth blocks) and 24 lines.
const BLOCK_SUBCELL_COLUMNS: i32 = 8;
const BLOCK_SUBCELL_LINES: i32 = 24;

#[derive(Clone, Debug, PartialEq)]
struct BlockElementLayoutRect {
    line: i32,
    column: i32,
    num_of_columns: usize,
    num_of_lines: usize,
    color: Hsla,
}

impl BlockElementLayoutRect {
    fn paint(&self, origin: GpuiPoint<Pixels>, dims: &TerminalBounds, window: &mut Window) {
        let subcell_width = dims.cell_width / BLOCK_SUBCELL_COLUMNS as f32;
        let subcell_height = dims.line_height / BLOCK_SUBCELL_LINES as f32;
        let position = point(
            origin.x + self.column as f32 * subcell_width,
            origin.y + self.line as f32 * subcell_height,
        );
        let size = size(
            subcell_width * self.num_of_columns as f32,
            subcell_height * self.num_of_lines as f32,
        );
        window.paint_quad(fill(Bounds::new(position, size), self.color));
    }
}

/// An inclusive rectangle on the subcell grid.
#[derive(Debug, Clone)]
struct BlockRegion {
    start_line: i32,
    start_col: i32,
    end_line: i32,
    end_col: i32,
    color: Hsla,
}

impl BlockRegion {
    fn can_merge_with(&self, other: &BlockRegion) -> bool {
        if self.color != other.color {
            return false;
        }
        if self.start_line == other.start_line && self.end_line == other.end_line {
            return self.end_col + 1 == other.start_col || other.end_col + 1 == self.start_col;
        }
        if self.start_col == other.start_col && self.end_col == other.end_col {
            return self.end_line + 1 == other.start_line || other.end_line + 1 == self.start_line;
        }
        false
    }

    fn merge_with(&mut self, other: &BlockRegion) {
        self.start_line = self.start_line.min(other.start_line);
        self.start_col = self.start_col.min(other.start_col);
        self.end_line = self.end_line.max(other.end_line);
        self.end_col = self.end_col.max(other.end_col);
    }
}

fn merge_block_regions(mut merged: Vec<BlockRegion>) -> Vec<BlockRegion> {
    let mut changed = true;
    while changed {
        changed = false;
        let mut i = 0;
        while i < merged.len() {
            let mut j = i + 1;
            while j < merged.len() {
                if merged[i].can_merge_with(&merged[j]) {
                    let other = merged.remove(j);
                    merged[i].merge_with(&other);
                    changed = true;
                } else {
                    j += 1;
                }
            }
            i += 1;
        }
    }
    merged
}

/// Filled quadrants as a bitmap, bit `row * 2 + column` per 2x2 subcell.
fn quadrant_char_to_filled_bits(ch: char) -> Option<u8> {
    Some(match ch {
        '▘' => 0b0001,
        '▝' => 0b0010,
        '▖' => 0b0100,
        '▗' => 0b1000,
        '▚' => 0b1001,
        '▞' => 0b0110,
        '▛' => 0b0111,
        '▜' => 0b1011,
        '▙' => 0b1101,
        '▟' => 0b1110,
        _ => return None,
    })
}

/// `(column, line, num_of_columns, num_of_lines)` in subcell units for block
/// elements that are a single rectangle.
fn block_char_to_rect(ch: char) -> Option<(i32, i32, i32, i32)> {
    let codepoint = ch as u32;
    Some(match codepoint {
        // ▀
        0x2580 => (0, 0, 8, 12),
        // ▁▂▃▄▅▆▇█
        0x2581..=0x2588 => {
            let eighths = (codepoint - 0x2580) as i32;
            (0, 24 - eighths * 3, 8, eighths * 3)
        }
        // ▉▊▋▌▍▎▏
        0x2589..=0x258F => (0, 0, (0x2590 - codepoint) as i32, 24),
        // ▐
        0x2590 => (4, 0, 4, 24),
        // ▔
        0x2594 => (0, 0, 8, 3),
        // ▕
        0x2595 => (7, 0, 1, 24),
        _ => return None,
    })
}

/// `░▒▓` as the foreground at reduced opacity rather than a stipple pattern.
fn shade_char_to_opacity(ch: char) -> Option<f32> {
    match ch {
        '░' => Some(0.25),
        '▒' => Some(0.5),
        '▓' => Some(0.75),
        _ => None,
    }
}

fn collect_block_element_regions(
    line: i32,
    column: i32,
    ch: char,
    color: Hsla,
    regions: &mut Vec<BlockRegion>,
) -> bool {
    if let Some((sub_col, sub_line, cols, lines)) = block_char_to_rect(ch) {
        push_block_element_region(line, column, sub_col, sub_line, cols, lines, color, regions);
        return true;
    }
    if let Some(filled) = quadrant_char_to_filled_bits(ch) {
        for row in 0..2 {
            for col in 0..2 {
                if filled & (1 << (row * 2 + col)) != 0 {
                    push_block_element_region(
                        line,
                        column,
                        col * 4,
                        row * 12,
                        4,
                        12,
                        color,
                        regions,
                    );
                }
            }
        }
        return true;
    }
    if let Some(opacity) = shade_char_to_opacity(ch) {
        push_block_element_region(line, column, 0, 0, 8, 24, color.opacity(opacity), regions);
        return true;
    }
    false
}

#[allow(clippy::too_many_arguments)]
fn push_block_element_region(
    line: i32,
    column: i32,
    sub_col: i32,
    sub_line: i32,
    num_of_columns: i32,
    num_of_lines: i32,
    color: Hsla,
    regions: &mut Vec<BlockRegion>,
) {
    let start_line = line * BLOCK_SUBCELL_LINES + sub_line;
    let start_col = column * BLOCK_SUBCELL_COLUMNS + sub_col;
    let end_line = start_line + num_of_lines - 1;
    let end_col = start_col + num_of_columns - 1;

    // Extend the previous region eagerly so runs of `█` keep the quadratic merge small.
    if let Some(last) = regions.last_mut()
        && last.color == color
        && last.start_line == start_line
        && last.end_line == end_line
        && last.end_col + 1 == start_col
    {
        last.end_col = end_col;
        return;
    }
    regions.push(BlockRegion {
        start_line,
        start_col,
        end_line,
        end_col,
        color,
    });
}

fn block_element_regions_to_rects(regions: Vec<BlockRegion>) -> Vec<BlockElementLayoutRect> {
    merge_block_regions(regions)
        .into_iter()
        .map(|r| BlockElementLayoutRect {
            line: r.start_line,
            column: r.start_col,
            num_of_columns: (r.end_col - r.start_col + 1) as usize,
            num_of_lines: (r.end_line - r.start_line + 1) as usize,
            color: r.color,
        })
        .collect()
}

pub struct LayoutState {
    dims: TerminalBounds,
    font_size: Pixels,
    background: Hsla,
    rects: Vec<LayoutRect>,
    block_rects: Vec<BlockElementLayoutRect>,
    runs: Vec<BatchedTextRun>,
    cursor: Option<(Bounds<Pixels>, Hsla)>,
    selection: Vec<LayoutRect>,
    matches: Vec<LayoutRect>,
    hitbox: Hitbox,
}

fn base_font() -> Font {
    Font {
        features: FontFeatures::disable_ligatures(),
        ..gpui::font(FONT_FAMILY)
    }
}

/// Cell metrics for the terminal font at `font_size`; used for the grid and for the PTY size.
pub fn cell_metrics(font_size: Pixels, cx: &App) -> (Pixels, Pixels) {
    let text_system = cx.text_system();
    let font_id = text_system.resolve_font(&base_font());
    let cell_width = text_system
        .advance(font_id, font_size, 'm')
        .map(|s| s.width)
        .unwrap_or(font_size * 0.55);
    let line_height = px((f32::from(font_size) * LINE_HEIGHT_FACTOR).round());
    (line_height, cell_width)
}

pub fn terminal_grid(terminal: Entity<Terminal>, font_size: Pixels) -> impl IntoElement {
    let terminal_for_paint = terminal.clone();
    canvas(
        move |bounds, window, cx| prepaint(&terminal, font_size, bounds, window, cx),
        move |_bounds, state, window, cx| {
            register_mouse_listeners(&terminal_for_paint, &state.hitbox, window);
            paint(state, window, cx)
        },
    )
    .size_full()
}

fn prepaint(
    terminal: &Entity<Terminal>,
    font_size: Pixels,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) -> LayoutState {
    let (line_height, cell_width) = cell_metrics(font_size, cx);
    let gutter = cell_width;
    let mut grid_bounds = bounds;
    grid_bounds.origin.x += gutter;
    grid_bounds.size.width = (grid_bounds.size.width - gutter).max(cell_width * 2.);
    let scale_factor = window.scale_factor();
    let snap = |v: Pixels| Pixels::from((f32::from(v) * scale_factor).floor() / scale_factor);
    grid_bounds.origin = point(snap(grid_bounds.origin.x), snap(grid_bounds.origin.y));
    let dims = TerminalBounds::new(line_height, cell_width, grid_bounds);

    terminal.update(cx, |terminal, cx| {
        terminal.set_size(dims);
        terminal.sync(window, cx);
    });

    let theme = cx.theme().clone();
    let (lit, _) = terminal.read(cx).cursor_blink(std::time::Instant::now());
    terminal.update(cx, |terminal, _| terminal.cursor_painted_lit = lit);
    let terminal_ref = terminal.read(cx);
    let content = terminal_ref.last_content();
    // Windows Terminal: a white selection, its text in the background colour, so it reads inverted (m5/26)
    let selected = content.selection.as_ref().map(|sel| {
        let offset = content.display_offset as i32;
        Selected {
            start: (sel.start.line + offset, sel.start.column),
            end: (sel.end.line + offset, sel.end.column),
            is_block: sel.is_block,
            fg: theme.colors().terminal_background,
        }
    });
    let (rects, block_rects, runs) = layout_grid(content.cells.iter(), &theme, selected.as_ref());
    let selected_text = selected.as_ref().filter(|s| runs.iter().any(|r| r.style.color == s.fg && s.contains(r.line, r.column as usize)));

    let cursor = if content.cursor.shape == CursorShape::Hidden || !lit {
        None
    } else {
        let line = content.cursor.point.line + content.display_offset as i32;
        (line >= 0 && (line as usize) < dims.num_lines()).then(|| {
            let origin = point(
                (content.cursor.point.column as f32 * cell_width).floor(),
                (line as f32 * line_height).floor(),
            );
            let width = match content.cursor.shape {
                CursorShape::Bar => px(2.),
                _ => cell_width,
            };
            let height = match content.cursor.shape {
                CursorShape::Underline => px(2.),
                _ => line_height,
            };
            let y_off = if content.cursor.shape == CursorShape::Underline {
                line_height - px(2.)
            } else {
                px(0.)
            };
            let mut color = theme.players().local().cursor;
            if matches!(content.cursor.shape, CursorShape::Block | CursorShape::HollowBlock) {
                color.a *= 0.6;
            }
            (
                Bounds::new(point(origin.x, origin.y + y_off), size(width, height)),
                color,
            )
        })
    };

    let selection = selected
        .as_ref()
        .map(|sel| selection_rects(sel.start, sel.end, sel.is_block, dims.num_lines(), dims.num_columns(), SELECTION_BACKGROUND))
        .unwrap_or_default();
    let painted = selected_text.map(|s| (SELECTION_BACKGROUND, s.fg));
    let offset = content.display_offset as i32;
    let matches = terminal_ref
        .matches
        .iter()
        .flat_map(|m| {
            selection_rects(
                (m.start().line + offset, m.start().column),
                (m.end().line + offset, m.end().column),
                false,
                dims.num_lines(),
                dims.num_columns(),
                theme.colors().search_match_background,
            )
        })
        .collect();
    terminal.update(cx, |terminal, _| terminal.selection_painted = painted);

    LayoutState {
        dims,
        font_size,
        background: theme.colors().terminal_background,
        rects,
        block_rects,
        runs,
        cursor,
        selection,
        matches,
        hitbox: window.insert_hitbox(bounds, HitboxBehavior::Normal),
    }
}

/// One rect per visible line of an inclusive selection from `start` to `end` (display line,
/// column), clipped to the viewport.
fn selection_rects(
    start: (i32, usize),
    end: (i32, usize),
    is_block: bool,
    num_lines: usize,
    num_columns: usize,
    color: Hsla,
) -> Vec<LayoutRect> {
    let last_column = num_columns.saturating_sub(1);
    (start.0.max(0)..=end.0.min(num_lines as i32 - 1))
        .map(|line| {
            let (first, last) = if is_block {
                (start.1.min(end.1), start.1.max(end.1))
            } else {
                (
                    if line == start.0 { start.1 } else { 0 },
                    if line == end.0 { end.1.min(last_column) } else { last_column },
                )
            };
            LayoutRect {
                line,
                column: first as i32,
                num_of_cells: last.saturating_sub(first) + 1,
                color,
            }
        })
        .collect()
}

/// As Zed's `TerminalElement::register_mouse_listeners`: while the program has enabled mouse
/// reporting (and Shift is not held) the Terminal encodes presses, motion and releases for the
/// PTY; otherwise a left drag selects, double-click a word, triple-click a line, and a right
/// click copies the selection or, without one, pastes the clipboard.
fn register_mouse_listeners(terminal: &Entity<Terminal>, hitbox: &Hitbox, window: &mut Window) {
    let mode = MouseInputMode::ReportToTerminal;
    window.on_mouse_event({
        let (terminal, hitbox) = (terminal.clone(), hitbox.clone());
        move |e: &MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || !hitbox.is_hovered(window) {
                return;
            }
            terminal.update(cx, |terminal, cx| {
                if e.button == MouseButton::Left || terminal.mouse_mode(e.modifiers.shift, mode) {
                    terminal.mouse_down(e, mode, cx);
                } else if e.button == MouseButton::Right {
                    if terminal.last_content().selection.is_some() {
                        terminal.copy(Some(false));
                    } else if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                        terminal.paste(&text);
                    }
                }
                cx.notify();
            });
        }
    });
    window.on_mouse_event({
        let (terminal, hitbox) = (terminal.clone(), hitbox.clone());
        move |e: &MouseMoveEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            let hovered = hitbox.is_hovered(window);
            terminal.update(cx, |terminal, cx| {
                if e.pressed_button.is_some() && (terminal.selection_started() || hovered) {
                    terminal.mouse_drag(e, hitbox.bounds, mode, cx);
                }
                if hovered {
                    terminal.mouse_move(e, mode, cx);
                }
            });
        }
    });
    window.on_mouse_event({
        let (terminal, hitbox) = (terminal.clone(), hitbox.clone());
        move |e: &MouseUpEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            terminal.update(cx, |terminal, cx| {
                let reporting = terminal.last_content().mode.intersects(Modes::MOUSE_MODE);
                let ours = hitbox.is_hovered(window) || terminal.selection_started();
                if ours && (e.button == MouseButton::Left || reporting) {
                    terminal.mouse_up(e, mode, cx);
                    cx.notify();
                }
            });
        }
    });
}

fn paint(state: LayoutState, window: &mut Window, cx: &mut App) {
    let dims = state.dims;
    window.paint_quad(fill(dims.bounds, state.background));
    let origin = dims.bounds.origin;
    for rect in &state.rects {
        rect.paint(origin, &dims, window);
    }
    for rect in state.matches.iter().chain(&state.selection) {
        rect.paint(origin, &dims, window);
    }
    for run in &state.runs {
        run.paint(origin, &dims, state.font_size, window, cx);
    }
    for rect in &state.block_rects {
        rect.paint(origin, &dims, window);
    }
    if let Some((bounds, color)) = state.cursor {
        window.paint_quad(fill(bounds + origin, color));
    }
}

/// Windows Terminal's `selectionBackground`.
pub const SELECTION_BACKGROUND: Hsla = Hsla { h: 0., s: 0., l: 1., a: 1. };

/// The shown selection (display line, column), and the colour its text is drawn in.
struct Selected {
    start: (i32, usize),
    end: (i32, usize),
    is_block: bool,
    fg: Hsla,
}

impl Selected {
    fn contains(&self, line: i32, column: usize) -> bool {
        if line < self.start.0 || line > self.end.0 {
            return false;
        }
        if self.is_block {
            return (self.start.1.min(self.end.1)..=self.start.1.max(self.end.1)).contains(&column);
        }
        (line > self.start.0 || column >= self.start.1) && (line < self.end.0 || column <= self.end.1)
    }
}

fn layout_grid<'a>(
    grid: impl Iterator<Item = &'a IndexedCell>,
    theme: &Theme,
    selected: Option<&Selected>,
) -> (Vec<LayoutRect>, Vec<BlockElementLayoutRect>, Vec<BatchedTextRun>) {
    let mut runs = Vec::new();
    let mut block_regions = Vec::new();
    let mut rects: Vec<LayoutRect> = Vec::new();
    let mut current: Option<BatchedTextRun> = None;
    let font = base_font();

    for (line_index, (_, line)) in grid.chunk_by(|c| c.point.line).into_iter().enumerate() {
        let display_line = line_index as i32;
        if let Some(batch) = current.take() {
            runs.push(batch);
        }
        let mut previous_cell_had_extras = false;

        for indexed in line {
            let cell = &indexed.cell;
            let col = indexed.point.column as i32;
            let mut fg = cell.foreground();
            let mut bg = cell.background();
            if cell.is_inverse() {
                mem::swap(&mut fg, &mut bg);
            }

            if !is_default_background_color(bg) {
                let color = convert_color(&bg, theme);
                match rects.last_mut() {
                    Some(last)
                        if last.color == color
                            && last.line == display_line
                            && last.column + last.num_of_cells as i32 == col =>
                    {
                        last.num_of_cells += 1;
                    }
                    _ => rects.push(LayoutRect {
                        line: display_line,
                        column: col,
                        num_of_cells: 1,
                        color,
                    }),
                }
            }

            if cell.is_wide_char_spacer() {
                continue;
            }
            if cell.character() == ' ' && previous_cell_had_extras {
                previous_cell_had_extras = false;
                continue;
            }
            previous_cell_had_extras = matches!(cell.zerowidth(), Some(chars) if !chars.is_empty());

            if is_blank(cell) {
                continue;
            }
            let mut style = cell_style(cell, fg, theme, &font);
            if let Some(selected) = selected.filter(|s| s.contains(display_line, col as usize)) {
                style.color = selected.fg;
                for color in [style.underline.as_mut().map(|u| &mut u.color), style.strikethrough.as_mut().map(|s| &mut s.color)]
                    .into_iter()
                    .flatten()
                {
                    *color = Some(selected.fg);
                }
            }
            if collect_block_element_regions(
                display_line,
                col,
                cell.character(),
                style.color,
                &mut block_regions,
            ) {
                if let Some(batch) = current.take() {
                    runs.push(batch);
                }
                continue;
            }
            let zero_width = cell.zerowidth();
            match current.as_mut() {
                Some(batch)
                    if batch.can_append(&style)
                        && batch.line == display_line
                        && batch.column + batch.cell_count as i32 == col =>
                {
                    batch.append_char(cell.character(), true);
                    for &c in zero_width.unwrap_or_default() {
                        batch.append_char(c, false);
                    }
                }
                _ => {
                    if let Some(batch) = current.take() {
                        runs.push(batch);
                    }
                    let mut batch =
                        BatchedTextRun::new_from_char(display_line, col, cell.character(), style);
                    for &c in zero_width.unwrap_or_default() {
                        batch.append_char(c, false);
                    }
                    current = Some(batch);
                }
            }
        }
    }
    if let Some(batch) = current {
        runs.push(batch);
    }
    (rects, block_element_regions_to_rects(block_regions), runs)
}

fn cell_style(cell: &Cell, fg: Color, theme: &Theme, font: &Font) -> TextRun {
    let mut fg = convert_color(&fg, theme);
    if cell.is_dim() {
        fg.a *= 0.7;
    }
    let underline = (cell.has_underline() || cell.hyperlink().is_some()).then(|| UnderlineStyle {
        color: Some(fg),
        thickness: px(1.0),
        wavy: cell.has_undercurl(),
    });
    let strikethrough = cell.has_strikeout().then(|| StrikethroughStyle {
        color: Some(fg),
        thickness: px(1.0),
    });
    TextRun {
        len: cell.character().len_utf8(),
        color: fg,
        background_color: None,
        font: Font {
            weight: if cell.is_bold() {
                FontWeight::BOLD
            } else {
                FontWeight::NORMAL
            },
            style: if cell.is_italic() {
                FontStyle::Italic
            } else {
                FontStyle::Normal
            },
            ..font.clone()
        },
        underline,
        strikethrough,
    }
}

fn is_blank(cell: &Cell) -> bool {
    cell.character() == ' '
        && is_default_background_color(cell.background())
        && cell.hyperlink().is_none()
        && !cell.has_visible_style_modifier()
}

fn convert_color(fg: &Color, theme: &Theme) -> Hsla {
    let colors = theme.colors();
    let palette = palette(theme);
    match fg {
        Color::Named(color) => match color {
            NamedColor::Foreground => colors.terminal_foreground,
            NamedColor::Background => colors.terminal_ansi_background,
            NamedColor::Cursor => theme.players().local().cursor,
            NamedColor::BrightForeground => colors.terminal_bright_foreground,
            NamedColor::DimForeground => colors.terminal_dim_foreground,
            NamedColor::DimBlack
            | NamedColor::DimRed
            | NamedColor::DimGreen
            | NamedColor::DimYellow
            | NamedColor::DimBlue
            | NamedColor::DimMagenta
            | NamedColor::DimCyan
            | NamedColor::DimWhite => palette.dim[*color as usize - NamedColor::DimBlack as usize],
            // Black..=BrightWhite are 0..=15
            _ => palette.ansi[*color as usize],
        },
        Color::Spec(rgb) => foreman_term::rgba_color(rgb.r, rgb.g, rgb.b),
        Color::Indexed(i) => foreman_term::get_color_at_index(*i as usize, &palette),
    }
}

/// [`crate::palette`] of `theme`, built once per theme (each built theme has its own id, m5/31).
fn palette(theme: &Theme) -> std::rc::Rc<foreman_term::terminal_settings::TerminalPalette> {
    thread_local! {
        static PALETTE: std::cell::RefCell<Option<(String, std::rc::Rc<foreman_term::terminal_settings::TerminalPalette>)>> =
            const { std::cell::RefCell::new(None) };
    }
    PALETTE.with_borrow_mut(|cached| match cached {
        Some((id, palette)) if *id == theme.id => palette.clone(),
        _ => {
            let palette = std::rc::Rc::new(crate::palette(theme));
            *cached = Some((theme.id.clone(), palette.clone()));
            palette
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use foreman_term::{Point, Rgb};

    fn spans(rects: &[LayoutRect]) -> Vec<(i32, i32, usize)> {
        rects.iter().map(|r| (r.line, r.column, r.num_of_cells)).collect()
    }

    #[test]
    fn selection_spans_lines_from_start_column_to_end_column() {
        let rects = selection_rects((1, 4), (3, 2), false, 10, 20, Hsla::default());
        assert_eq!(spans(&rects), [(1, 4, 16), (2, 0, 20), (3, 0, 3)]);
    }

    #[test]
    fn selection_is_clipped_to_the_viewport_and_block_selection_keeps_its_columns() {
        // starts in scrollback above the viewport, ends below it
        let rects = selection_rects((-5, 7), (12, 1), false, 3, 20, Hsla::default());
        assert_eq!(spans(&rects), [(0, 0, 20), (1, 0, 20), (2, 0, 20)]);
        let rects = selection_rects((0, 6), (1, 2), true, 3, 20, Hsla::default());
        assert_eq!(spans(&rects), [(0, 2, 5), (1, 2, 5)]);
        assert!(selection_rects((-4, 0), (-1, 5), false, 3, 20, Hsla::default()).is_empty());
    }

    fn theme() -> std::sync::Arc<Theme> {
        let registry = theme::ThemeRegistry::default();
        let name = registry.list().into_iter().next().expect("fallback theme").name;
        registry.get(&name).expect("fallback theme")
    }

    fn row(line: i32, text: &str, fg: Color) -> Vec<IndexedCell> {
        text.chars()
            .enumerate()
            .map(|(column, c)| IndexedCell {
                point: Point::new(line, column),
                cell: Cell::with_char(c, fg),
            })
            .collect()
    }

    fn single(ch: char) -> Vec<BlockElementLayoutRect> {
        let mut regions = Vec::new();
        assert!(collect_block_element_regions(0, 0, ch, Hsla::default(), &mut regions));
        block_element_regions_to_rects(regions)
    }

    fn rect(line: i32, column: i32, cols: usize, lines: usize) -> (i32, i32, usize, usize) {
        (line, column, cols, lines)
    }

    fn shape(r: &BlockElementLayoutRect) -> (i32, i32, usize, usize) {
        (r.line, r.column, r.num_of_columns, r.num_of_lines)
    }

    #[test]
    fn selected_text_is_drawn_in_the_background_colour_and_the_rest_is_not() {
        let theme = theme();
        let fg = Color::Named(NamedColor::Foreground);
        let background = theme.colors().terminal_background;
        let selected = Selected { start: (0, 2), end: (1, 1), is_block: false, fg: background };
        let cells: Vec<_> = row(0, "abcdef", fg).into_iter().chain(row(1, "ghijkl", fg)).collect();
        let (_, _, runs) = layout_grid(cells.iter(), &theme, Some(&selected));
        let drawn: Vec<(i32, &str, bool)> = runs.iter().map(|r| (r.line, r.text.as_str(), r.style.color == background)).collect();
        assert_eq!(drawn, [(0, "ab", false), (0, "cdef", true), (1, "gh", true), (1, "ijkl", false)]);
        assert_eq!(SELECTION_BACKGROUND, gpui::rgb(0xFFFFFF).into());
        assert!(Selected { start: (0, 4), end: (2, 1), is_block: true, ..selected }.contains(1, 3));
        assert!(!Selected { start: (0, 4), end: (2, 1), is_block: true, ..selected }.contains(1, 0));
    }

    #[test]
    fn block_element_full_block_row_is_one_rect_and_no_text_run() {
        let theme = theme();
        let fg = Color::Spec(Rgb { r: 215, g: 119, b: 87 });
        let cells = row(0, "████", fg);
        let (_, block_rects, runs) = layout_grid(cells.iter(), &theme, None);
        assert!(runs.is_empty(), "block elements must not become text runs");
        assert_eq!(block_rects.len(), 1, "adjacent full blocks merge");
        assert_eq!(shape(&block_rects[0]), rect(0, 0, 32, 24));
        assert_eq!(block_rects[0].color, foreman_term::rgba_color(215, 119, 87));
    }

    #[test]
    fn block_element_full_blocks_on_consecutive_lines_merge_vertically() {
        let theme = theme();
        let fg = Color::Named(NamedColor::Foreground);
        let mut cells = row(0, "██", fg);
        cells.extend(row(1, "██", fg));
        let (_, block_rects, runs) = layout_grid(cells.iter(), &theme, None);
        assert!(runs.is_empty());
        assert_eq!(block_rects.len(), 1);
        assert_eq!(shape(&block_rects[0]), rect(0, 0, 16, 48));
    }

    #[test]
    fn block_element_halves_quadrants_and_shades_map_to_sub_rects() {
        assert_eq!(single('▀').iter().map(shape).collect_vec(), [rect(0, 0, 8, 12)]);
        assert_eq!(single('▄').iter().map(shape).collect_vec(), [rect(12, 0, 8, 12)]);
        assert_eq!(single('▌').iter().map(shape).collect_vec(), [rect(0, 0, 4, 24)]);
        assert_eq!(single('▐').iter().map(shape).collect_vec(), [rect(0, 4, 4, 24)]);
        assert_eq!(single('▘').iter().map(shape).collect_vec(), [rect(0, 0, 4, 12)]);
        assert_eq!(single('▝').iter().map(shape).collect_vec(), [rect(0, 4, 4, 12)]);
        let mut diagonal = single('▚').iter().map(shape).collect_vec();
        diagonal.sort();
        assert_eq!(diagonal, [rect(0, 0, 4, 12), rect(12, 4, 4, 12)]);
        // ▛ = upper half plus lower-left quadrant.
        let mut upper_left = single('▛').iter().map(shape).collect_vec();
        upper_left.sort();
        assert_eq!(upper_left, [rect(0, 0, 8, 12), rect(12, 0, 4, 12)]);
        let shade = single('▒');
        assert_eq!(shade.iter().map(shape).collect_vec(), [rect(0, 0, 8, 24)]);
        assert_eq!(shade[0].color, Hsla::default().opacity(0.5));
    }

    #[test]
    fn block_element_range_is_fully_custom_painted_inside_its_cell() {
        for codepoint in 0x2580..=0x259F {
            let ch = char::from_u32(codepoint).unwrap();
            let rects = single(ch);
            assert!(!rects.is_empty(), "U+{codepoint:04X} paints nothing");
            for r in &rects {
                assert!(
                    r.line >= 0
                        && r.column >= 0
                        && r.line + r.num_of_lines as i32 <= BLOCK_SUBCELL_LINES
                        && r.column + r.num_of_columns as i32 <= BLOCK_SUBCELL_COLUMNS,
                    "U+{codepoint:04X} paints outside its cell: {r:?}"
                );
            }
        }
        let mut regions = Vec::new();
        assert!(!collect_block_element_regions(0, 0, '─', Hsla::default(), &mut regions));
        assert!(!collect_block_element_regions(0, 0, '\u{1FB00}', Hsla::default(), &mut regions));
    }

    #[test]
    fn block_element_letters_stay_text_runs_beside_blocks() {
        let theme = theme();
        let cells = row(0, "ab█c", Color::Named(NamedColor::Foreground));
        let (_, block_rects, runs) = layout_grid(cells.iter(), &theme, None);
        assert_eq!(runs.iter().map(|r| r.text.as_str()).collect_vec(), ["ab", "c"]);
        assert_eq!(runs[1].column, 3);
        assert_eq!(block_rects.iter().map(shape).collect_vec(), [rect(0, 16, 8, 24)]);
    }
}
