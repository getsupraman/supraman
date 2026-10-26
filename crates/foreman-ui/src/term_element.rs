// Minimal terminal grid painter adapted from Zed's
// crates/terminal_view/src/terminal_element.rs @ 03efd49220ba06703d04f9c4732fdf814f9c523c
// (GPL-3.0-or-later). Kept: cell batching into text runs, background rects,
// colour conversion, block cursor, block-element glyph rects (U+2580-U+259F). Dropped:
// selection/search highlights, hyperlinks, IME, sextant glyph rects, minimum-contrast
// adjustment, mouse handling.

use gpui::{
    App, Bounds, Entity, Font, FontFeatures, FontStyle, FontWeight, Hsla, IntoElement, Pixels,
    Point as GpuiPoint, StrikethroughStyle, Styled, TextRun, UnderlineStyle, Window, canvas, fill,
    point, px, size,
};
use itertools::Itertools as _;
use std::mem;
use foreman_term::{
    Cell, Color, CursorShape, IndexedCell, NamedColor, Terminal, TerminalBounds,
    is_default_background_color,
};
use theme::{ActiveTheme, Theme};

pub const FONT_FAMILY: &str = "Consolas";
pub const FONT_SIZE: Pixels = px(14.);
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

    fn paint(&self, origin: GpuiPoint<Pixels>, dims: &TerminalBounds, window: &mut Window, cx: &mut App) {
        let pos = point(
            origin.x + self.column as f32 * dims.cell_width,
            origin.y + self.line as f32 * dims.line_height,
        );
        let shaped = window.text_system().shape_line(
            self.text.clone().into(),
            FONT_SIZE,
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
    background: Hsla,
    rects: Vec<LayoutRect>,
    block_rects: Vec<BlockElementLayoutRect>,
    runs: Vec<BatchedTextRun>,
    cursor: Option<(Bounds<Pixels>, Hsla)>,
}

fn base_font() -> Font {
    Font {
        features: FontFeatures::disable_ligatures(),
        ..gpui::font(FONT_FAMILY)
    }
}

/// Cell metrics for the fixed spike font; used for the grid and for the PTY size.
pub fn cell_metrics(cx: &App) -> (Pixels, Pixels) {
    let text_system = cx.text_system();
    let font_id = text_system.resolve_font(&base_font());
    let cell_width = text_system
        .advance(font_id, FONT_SIZE, 'm')
        .map(|s| s.width)
        .unwrap_or(px(8.));
    let line_height = px((f32::from(FONT_SIZE) * LINE_HEIGHT_FACTOR).round());
    (line_height, cell_width)
}

pub fn terminal_grid(terminal: Entity<Terminal>) -> impl IntoElement {
    canvas(
        move |bounds, window, cx| prepaint(&terminal, bounds, window, cx),
        |_bounds, state, window, cx| paint(state, window, cx),
    )
    .size_full()
}

fn prepaint(
    terminal: &Entity<Terminal>,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) -> LayoutState {
    let (line_height, cell_width) = cell_metrics(cx);
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
    let content = terminal.read(cx).last_content();
    let (rects, block_rects, runs) = layout_grid(content.cells.iter(), &theme);

    let cursor = if content.cursor.shape == CursorShape::Hidden {
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
            color.a *= 0.6;
            (
                Bounds::new(point(origin.x, origin.y + y_off), size(width, height)),
                color,
            )
        })
    };

    LayoutState {
        dims,
        background: theme.colors().terminal_background,
        rects,
        block_rects,
        runs,
        cursor,
    }
}

fn paint(state: LayoutState, window: &mut Window, cx: &mut App) {
    let dims = state.dims;
    window.paint_quad(fill(dims.bounds, state.background));
    let origin = dims.bounds.origin;
    for rect in &state.rects {
        rect.paint(origin, &dims, window);
    }
    for run in &state.runs {
        run.paint(origin, &dims, window, cx);
    }
    for rect in &state.block_rects {
        rect.paint(origin, &dims, window);
    }
    if let Some((bounds, color)) = state.cursor {
        window.paint_quad(fill(bounds + origin, color));
    }
}

fn layout_grid<'a>(
    grid: impl Iterator<Item = &'a IndexedCell>,
    theme: &Theme,
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
            let style = cell_style(cell, fg, theme, &font);
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
    match fg {
        Color::Named(color) => match color {
            NamedColor::Black => colors.terminal_ansi_black,
            NamedColor::Red => colors.terminal_ansi_red,
            NamedColor::Green => colors.terminal_ansi_green,
            NamedColor::Yellow => colors.terminal_ansi_yellow,
            NamedColor::Blue => colors.terminal_ansi_blue,
            NamedColor::Magenta => colors.terminal_ansi_magenta,
            NamedColor::Cyan => colors.terminal_ansi_cyan,
            NamedColor::White => colors.terminal_ansi_white,
            NamedColor::BrightBlack => colors.terminal_ansi_bright_black,
            NamedColor::BrightRed => colors.terminal_ansi_bright_red,
            NamedColor::BrightGreen => colors.terminal_ansi_bright_green,
            NamedColor::BrightYellow => colors.terminal_ansi_bright_yellow,
            NamedColor::BrightBlue => colors.terminal_ansi_bright_blue,
            NamedColor::BrightMagenta => colors.terminal_ansi_bright_magenta,
            NamedColor::BrightCyan => colors.terminal_ansi_bright_cyan,
            NamedColor::BrightWhite => colors.terminal_ansi_bright_white,
            NamedColor::Foreground => colors.terminal_foreground,
            NamedColor::Background => colors.terminal_ansi_background,
            NamedColor::Cursor => theme.players().local().cursor,
            NamedColor::DimBlack => colors.terminal_ansi_dim_black,
            NamedColor::DimRed => colors.terminal_ansi_dim_red,
            NamedColor::DimGreen => colors.terminal_ansi_dim_green,
            NamedColor::DimYellow => colors.terminal_ansi_dim_yellow,
            NamedColor::DimBlue => colors.terminal_ansi_dim_blue,
            NamedColor::DimMagenta => colors.terminal_ansi_dim_magenta,
            NamedColor::DimCyan => colors.terminal_ansi_dim_cyan,
            NamedColor::DimWhite => colors.terminal_ansi_dim_white,
            NamedColor::BrightForeground => colors.terminal_bright_foreground,
            NamedColor::DimForeground => colors.terminal_dim_foreground,
        },
        Color::Spec(rgb) => foreman_term::rgba_color(rgb.r, rgb.g, rgb.b),
        Color::Indexed(i) => indexed_color(*i as usize, theme),
    }
}

fn indexed_color(index: usize, theme: &Theme) -> Hsla {
    static PALETTE: std::sync::OnceLock<foreman_term::terminal_settings::TerminalPalette> =
        std::sync::OnceLock::new();
    foreman_term::get_color_at_index(index, PALETTE.get_or_init(|| crate::palette(theme)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use foreman_term::{Point, Rgb};

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
    fn block_element_full_block_row_is_one_rect_and_no_text_run() {
        let theme = theme();
        let fg = Color::Spec(Rgb { r: 215, g: 119, b: 87 });
        let cells = row(0, "████", fg);
        let (_, block_rects, runs) = layout_grid(cells.iter(), &theme);
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
        let (_, block_rects, runs) = layout_grid(cells.iter(), &theme);
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
        let (_, block_rects, runs) = layout_grid(cells.iter(), &theme);
        assert_eq!(runs.iter().map(|r| r.text.as_str()).collect_vec(), ["ab", "c"]);
        assert_eq!(runs[1].column, 3);
        assert_eq!(block_rects.iter().map(shape).collect_vec(), [rect(0, 16, 8, 24)]);
    }
}
