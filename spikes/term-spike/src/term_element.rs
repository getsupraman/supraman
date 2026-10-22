// Minimal terminal grid painter adapted from Zed's
// crates/terminal_view/src/terminal_element.rs @ 03efd49220ba06703d04f9c4732fdf814f9c523c
// (GPL-3.0-or-later). Kept: cell batching into text runs, background rects,
// colour conversion, block cursor. Dropped: selection/search highlights, hyperlinks,
// IME, block-element glyph rects, minimum-contrast adjustment, mouse handling.

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

pub struct LayoutState {
    dims: TerminalBounds,
    background: Hsla,
    rects: Vec<LayoutRect>,
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
    let (rects, runs) = layout_grid(content.cells.iter(), &theme);

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
    if let Some((bounds, color)) = state.cursor {
        window.paint_quad(fill(bounds + origin, color));
    }
}

fn layout_grid<'a>(
    grid: impl Iterator<Item = &'a IndexedCell>,
    theme: &Theme,
) -> (Vec<LayoutRect>, Vec<BatchedTextRun>) {
    let mut runs = Vec::new();
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
    (rects, runs)
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
