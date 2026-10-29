// Terminal grid painter adapted from Zed's
// crates/terminal_view/src/terminal_element.rs @ 03efd49220ba06703d04f9c4732fdf814f9c523c
// (GPL-3.0-or-later).

use gpui::{
    App, Bounds, ContentMask, DispatchPhase, Entity, Font, FontFeatures, FontId, FontStyle,
    FontWeight, Hitbox, HitboxBehavior, Hsla, IntoElement, Keystroke, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, PathBuilder, Pixels, Point as GpuiPoint, SharedString, ShapedLine,
    StrikethroughStyle, Styled, TextRun, UnderlineStyle, Window, WindowTextSystem, canvas, fill,
    point, px, size,
};
use itertools::Itertools as _;
use std::{cell::RefCell, collections::HashMap, mem, sync::Arc};
use foreman_term::{
    Cell, Color, CursorShape, IndexedCell, Modes, MouseInputMode, NamedColor, Terminal,
    TerminalBounds, is_default_background_color,
};
use theme::{ActiveTheme, Theme};

use crate::builtin_glyphs::{self, Mark};

pub const FONT_SIZE: Pixels = px(14.);
pub const MIN_FONT_SIZE: Pixels = px(6.);
pub const MAX_FONT_SIZE: Pixels = px(72.);

pub fn step_font_size(size: Pixels, delta: i32) -> Pixels {
    (size + px(delta as f32)).clamp(MIN_FONT_SIZE, MAX_FONT_SIZE)
}
const LINE_HEIGHT_FACTOR: f32 = 1.3;

/// The font every terminal grid paints with: `settings.toml` `terminal_font_family`,
/// `terminal_font_size` and `terminal_ligatures`.
#[derive(Clone, Debug, PartialEq)]
pub struct TermFont {
    pub family: SharedString,
    pub size: Pixels,
    pub ligatures: bool,
}

impl TermFont {
    pub fn font(&self) -> Font {
        Font {
            features: if self.ligatures { FontFeatures::default() } else { no_ligatures() },
            ..gpui::font(self.family.clone())
        }
    }
}

pub fn no_ligatures() -> FontFeatures {
    FontFeatures(Arc::new(["calt", "liga", "clig"].map(|tag| (tag.to_string(), 0)).to_vec()))
}

struct BatchedTextRun {
    line: i32,
    column: i32,
    text: String,
    cell_count: usize,
    cell_starts: Vec<usize>,
    /// One character fitted into its cells ([`BatchedTextRun::paint_fitted`]).
    fit: bool,
    /// The blank cells before and after a fitted character that its ink may overhang ([`give_room`]).
    room: (usize, usize),
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
            cell_starts: vec![0],
            fit: false,
            room: (0, 0),
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
        if counts_cell {
            self.cell_starts.push(self.text.len());
            self.cell_count += 1;
        }
        self.text.push(c);
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
        if self.fit {
            return self.paint_fitted(origin, pos, dims, font_size, window, cx);
        }
        let shape = |style: &TextRun, window: &mut Window| {
            window.text_system().shape_line(self.text.clone().into(), font_size, std::slice::from_ref(style), Some(dims.cell_width))
        };
        let mut shaped = shape(&self.style, window);
        if !self.on_cells(&shaped, dims.cell_width) && self.style.font.features != no_ligatures() {
            let mut style = self.style.clone();
            style.font.features = no_ligatures();
            shaped = shape(&style, window);
        }
        record(self, &shaped, pos - origin, false, window);
        let _ = shaped.paint(pos, dims.line_height, gpui::TextAlign::Left, None, window, cx);
    }

    /// Every glyph that starts a cell is at that cell's x: a ligature glyph standing for several
    /// cells' characters would move the glyphs after it off their cells.
    fn on_cells(&self, shaped: &ShapedLine, cell_width: Pixels) -> bool {
        shaped.runs.iter().flat_map(|run| &run.glyphs).all(|glyph| match self.cell_starts.binary_search(&glyph.index) {
            Ok(cell) => (glyph.position.x - cell_width * cell as f32).abs() <= px(1.),
            Err(_) => true,
        })
    }

    /// As Windows Terminal: full size on the terminal font's baseline; moved, and scaled about its
    /// centre only when its ink is wider than its cells and `room`; nothing is painted outside them.
    fn paint_fitted(&self, origin: GpuiPoint<Pixels>, pos: GpuiPoint<Pixels>, dims: &TerminalBounds, font_size: Pixels, window: &mut Window, cx: &mut App) {
        let cell = dims.cell_width;
        let (before, after) = (cell * self.room.0 as f32, cell * self.room.1 as f32);
        let cells = cell * self.cell_count as f32;
        let text_system = window.text_system().clone();
        let runs = std::slice::from_ref(&self.style);
        let natural = text_system.shape_line(self.text.clone().into(), font_size, runs, None);
        let base_style = TextRun { len: 1, ..self.style.clone() };
        let base = text_system.shape_line("m".into(), font_size, &[base_style], None);
        let ink_at = |shaped: &ShapedLine| {
            ink(&self.text, shaped, &text_system).unwrap_or_else(|| Bounds::new(point(px(0.), -shaped.ascent), size(shaped.width, shaped.ascent)))
        };
        let full = ink_at(&natural);
        let scale = ((before + cells + after) / full.size.width.max(px(0.01))).min(1.);
        let shaped = if scale < 0.99 {
            text_system.shape_line(self.text.clone().into(), font_size * scale, runs, None)
        } else {
            natural.clone()
        };
        let scaled = ink_at(&shaped);
        let ink_left = (full.left() + (full.size.width - scaled.size.width) / 2.).min(cells + after - scaled.size.width).max(-before);
        let centre_y = baseline(&base, dims.line_height) + full.center().y;
        let at = point(
            pos.x + ink_left - scaled.left(),
            pos.y + centre_y - scaled.center().y - baseline(&shaped, dims.line_height),
        );
        record(self, &shaped, at - origin, true, window);
        let mask = ContentMask { bounds: Bounds::new(point(pos.x - before, pos.y), size(before + cells + after, dims.line_height)) };
        window.with_content_mask(Some(mask), |window| {
            let _ = shaped.paint(at, dims.line_height, gpui::TextAlign::Left, None, window, cx);
        });
    }
}

/// Where [`ShapedLine::paint`] puts `shaped`'s baseline below the top of a line `line_height` high.
fn baseline(shaped: &ShapedLine, line_height: Pixels) -> Pixels {
    (line_height - shaped.ascent - shaped.descent) / 2. + shaped.ascent
}

/// The inked box of `shaped`, relative to its origin and baseline (y grows downwards).
fn ink(text: &str, shaped: &ShapedLine, text_system: &WindowTextSystem) -> Option<Bounds<Pixels>> {
    shaped
        .runs
        .iter()
        .flat_map(|run| run.glyphs.iter().map(move |g| (run.font_id, g)))
        .filter_map(|(font_id, glyph)| {
            let ch = text[glyph.index..].chars().next()?;
            let b = text_system.typographic_bounds(font_id, shaped.font_size, ch).ok()?;
            let left = glyph.position.x + b.origin.x;
            Some(Bounds::from_corners(point(left, -(b.origin.y + b.size.height)), point(left + b.size.width, -b.origin.y)))
        })
        .reduce(|a, b| a.union(&b))
}

/// A glyph as the grid last painted it, for the selftest ([`record_painted_glyphs`]).
#[derive(Clone, Debug, PartialEq)]
pub struct PaintedGlyph {
    pub line: i32,
    pub column: i32,
    pub ch: char,
    pub glyph: u32,
    /// Relative to the grid's origin, as is `ink`.
    pub x: Pixels,
    pub ink: Option<(Pixels, Pixels)>,
    pub ink_height: Option<Pixels>,
    pub fitted: bool,
    pub font_size: Pixels,
}

thread_local! {
    static PAINTED: RefCell<Option<Vec<PaintedGlyph>>> = const { RefCell::new(None) };
}

/// While on, each grid paint replaces [`painted_glyphs`].
pub fn record_painted_glyphs(on: bool) {
    PAINTED.with_borrow_mut(|p| *p = on.then(Vec::new));
}

pub fn painted_glyphs() -> Vec<PaintedGlyph> {
    PAINTED.with_borrow(|p| p.clone().unwrap_or_default())
}

fn record(run: &BatchedTextRun, shaped: &ShapedLine, at: GpuiPoint<Pixels>, fitted: bool, window: &Window) {
    if PAINTED.with_borrow(Option::is_none) {
        return;
    }
    let text_system = window.text_system();
    let glyphs: Vec<PaintedGlyph> = shaped
        .runs
        .iter()
        .flat_map(|r| r.glyphs.iter().map(move |g| (r.font_id, g)))
        .filter_map(|(font_id, glyph)| {
            let ch = run.text[glyph.index..].chars().next()?;
            let cell = run.cell_starts.partition_point(|&start| start <= glyph.index).saturating_sub(1);
            let x = at.x + glyph.position.x;
            let bounds = text_system.typographic_bounds(font_id, shaped.font_size, ch).ok();
            let ink = bounds.map(|b| (x + b.origin.x, x + b.origin.x + b.size.width));
            Some(PaintedGlyph {
                line: run.line,
                column: run.column + cell as i32,
                ch,
                glyph: glyph.id.0,
                x,
                ink,
                ink_height: bounds.map(|b| b.size.height),
                fitted,
                font_size: shaped.font_size,
            })
        })
        .collect();
    PAINTED.with_borrow_mut(|p| p.iter_mut().for_each(|p| p.extend(glyphs.iter().cloned())));
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

/// A cell painted from geometry ([`builtin_glyphs`]) on the device pixels its edges round to, so
/// neighbours share their edges.
#[derive(Clone, Debug, PartialEq)]
struct BuiltinCell {
    line: i32,
    column: i32,
    ch: char,
    color: Hsla,
}

impl BuiltinCell {
    fn paint(&self, origin: GpuiPoint<Pixels>, dims: &TerminalBounds, window: &mut Window) {
        let scale = window.scale_factor();
        let device = |v: Pixels| (f32::from(v) * scale).round();
        let left = device(origin.x + dims.cell_width * self.column as f32);
        let right = device(origin.x + dims.cell_width * (self.column + 1) as f32);
        let top = device(origin.y + dims.line_height * self.line as f32);
        let bottom = device(origin.y + dims.line_height * (self.line + 1) as f32);
        let light = builtin_glyphs::light_line(f32::from(dims.cell_width) * scale);
        let at = |[x, y]: [f32; 2]| point(px((left + x) / scale), px((top + y) / scale));
        let (w, h) = (right - left, bottom - top);
        let mask = ContentMask { bounds: Bounds::from_corners(at([0., 0.]), at([w, h])) };
        for mark in builtin_glyphs::marks(self.ch, w, h, light) {
            let path = match mark {
                Mark::Quads(quads, opacity) => {
                    for [l, t, r, b] in quads {
                        window.paint_quad(fill(Bounds::from_corners(at([l, t]), at([r, b])), self.color.opacity(opacity)));
                    }
                    continue;
                }
                Mark::Stroke(points, width) => {
                    let mut path = PathBuilder::stroke(px(width / scale));
                    path.add_polygon(&points.into_iter().map(at).collect_vec(), false);
                    path
                }
                Mark::RoundRect([l, t, r, b], radius, width) => {
                    let mut path = PathBuilder::stroke(px(width / scale));
                    let radii = point(px(radius / scale), px(radius / scale));
                    let arc = |path: &mut PathBuilder, to| path.arc_to(radii, px(0.), false, true, at(to));
                    path.move_to(at([l + radius, t]));
                    path.line_to(at([r - radius, t]));
                    arc(&mut path, [r, t + radius]);
                    path.line_to(at([r, b - radius]));
                    arc(&mut path, [r - radius, b]);
                    path.line_to(at([l + radius, b]));
                    arc(&mut path, [l, b - radius]);
                    path.line_to(at([l, t + radius]));
                    arc(&mut path, [l + radius, t]);
                    path.close();
                    path
                }
            };
            if let Ok(path) = path.build() {
                window.with_content_mask(Some(mask.clone()), |window| window.paint_path(path, self.color));
            }
        }
    }
}

pub struct LayoutState {
    dims: TerminalBounds,
    font_size: Pixels,
    background: Hsla,
    rects: Vec<LayoutRect>,
    builtins: Vec<BuiltinCell>,
    runs: Vec<BatchedTextRun>,
    cursor: Option<(Bounds<Pixels>, Hsla)>,
    selection: Vec<LayoutRect>,
    matches: Vec<LayoutRect>,
    hitbox: Hitbox,
}

/// Cell metrics for `font`; used for the grid and for the PTY size.
pub fn cell_metrics(font: &TermFont, cx: &App) -> (Pixels, Pixels) {
    let text_system = cx.text_system();
    let font_id = text_system.resolve_font(&font.font());
    let cell_width = text_system
        .advance(font_id, font.size, 'm')
        .map(|s| s.width)
        .unwrap_or(font.size * 0.55);
    let line_height = px((f32::from(font.size) * LINE_HEIGHT_FACTOR).round());
    (line_height, cell_width)
}

/// Whether `family` loads and its `i`, `M`, `W` and `.` advance alike.
pub fn is_monospace(text_system: &gpui::TextSystem, family: &str) -> bool {
    if family.starts_with('@') {
        return false;
    }
    let font_id = text_system.resolve_font(&gpui::font(family.to_string()));
    let advance = |ch| text_system.advance(font_id, px(100.), ch).ok().map(|a| a.width);
    match ['i', 'M', 'W', '.'].map(advance) {
        [Some(i), rest @ ..] => rest.iter().all(|w| w.is_some_and(|w| (w - i).abs() < px(0.5))),
        _ => false,
    }
}

const WIDTH_CACHE_CAP: usize = 4096;

/// Whether `ch` in `cells` cells is painted alone and fitted: the font has no glyph for it (a
/// fallback font draws it), or its glyph is wider than the cells. ASCII and box drawing never are.
fn needs_fit(ch: char, cells: usize, font_id: FontId, font_size: Pixels, cell_width: Pixels, cx: &App) -> bool {
    if ch.is_ascii() || ('\u{2500}'..='\u{257F}').contains(&ch) {
        return false;
    }
    thread_local! {
        static WIDTH_PER_PX: RefCell<HashMap<(FontId, char), Option<f32>>> = RefCell::new(HashMap::new());
    }
    let per_px = WIDTH_PER_PX.with_borrow_mut(|cache| {
        // font changes leave the old fonts' entries behind
        if cache.len() >= WIDTH_CACHE_CAP && !cache.contains_key(&(font_id, ch)) {
            cache.clear();
        }
        *cache.entry((font_id, ch)).or_insert_with(|| {
            (!drawn_by_fallback(cx.text_system(), font_id, ch)).then(|| f32::from(cx.text_system().layout_width(font_id, px(100.), ch)) / 100.)
        })
    });
    per_px.is_none_or(|per_px| font_size * per_px > cell_width * cells as f32 + px(0.5))
}

/// A missing glyph is the font's .notdef, so the font's own advance differs from the shaped width,
/// which is the fallback font's.
pub fn drawn_by_fallback(text_system: &gpui::TextSystem, font_id: FontId, ch: char) -> bool {
    let shaped = text_system.layout_width(font_id, px(100.), ch);
    text_system.advance(font_id, px(100.), ch).ok().is_none_or(|own| (own.width - shaped).abs() > px(0.01))
}

pub fn terminal_grid(terminal: Entity<Terminal>, font: TermFont) -> impl IntoElement {
    let terminal_for_paint = terminal.clone();
    canvas(
        move |bounds, window, cx| prepaint(&terminal, &font, bounds, window, cx),
        move |_bounds, state, window, cx| {
            register_mouse_listeners(&terminal_for_paint, &state.hitbox, window);
            paint(state, window, cx)
        },
    )
    .size_full()
}

fn prepaint(
    terminal: &Entity<Terminal>,
    font: &TermFont,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) -> LayoutState {
    let (line_height, cell_width) = cell_metrics(font, cx);
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
    // Windows Terminal: a white selection, its text in the background colour, so it reads inverted
    let selected = content.selection.as_ref().map(|sel| {
        let offset = content.display_offset as i32;
        Selected {
            start: (sel.start.line + offset, sel.start.column),
            end: (sel.end.line + offset, sel.end.column),
            is_block: sel.is_block,
            fg: theme.colors().terminal_background,
        }
    });
    let base = font.font();
    let font_id = cx.text_system().resolve_font(&base);
    let fit = |ch: char, cells: usize| needs_fit(ch, cells, font_id, font.size, cell_width, cx);
    let (rects, builtins, runs) = layout_grid(content.cells.iter(), &theme, selected.as_ref(), &base, &fit);
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
        font_size: font.size,
        background: theme.colors().terminal_background,
        rects,
        builtins,
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
                    } else if !terminal.paste_clipboard(cx) {
                        terminal.try_keystroke(&Keystroke::parse("ctrl-v").unwrap(), false);
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
    PAINTED.with_borrow_mut(|p| p.iter_mut().for_each(Vec::clear));
    for run in &state.runs {
        run.paint(origin, &dims, state.font_size, window, cx);
    }
    for cell in &state.builtins {
        cell.paint(origin, &dims, window);
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
    font: &Font,
    fit: &dyn Fn(char, usize) -> bool,
) -> (Vec<LayoutRect>, Vec<BuiltinCell>, Vec<BatchedTextRun>) {
    let mut runs = Vec::new();
    let mut builtins = Vec::new();
    let mut rects: Vec<LayoutRect> = Vec::new();
    let mut current: Option<BatchedTextRun> = None;
    let mut line_runs = 0;
    let mut inked: Vec<Inked> = Vec::new();

    for (line_index, (_, line)) in grid.chunk_by(|c| c.point.line).into_iter().enumerate() {
        let display_line = line_index as i32;
        if let Some(batch) = current.take() {
            runs.push(batch);
        }
        give_room(&mut runs[line_runs..], &inked);
        line_runs = runs.len();
        inked.clear();
        let mut previous_cell_had_extras = false;

        for indexed in line {
            let cell = &indexed.cell;
            let col = indexed.point.column as i32;
            let at = col as usize;
            if inked.len() <= at {
                inked.resize(at + 1, Inked::Blank);
            }
            inked[at] = Inked::Text;
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
                inked[at] = at.checked_sub(1).map_or(Inked::Text, |before| inked[before]);
                continue;
            }
            if cell.character() == ' ' && previous_cell_had_extras {
                previous_cell_had_extras = false;
                continue;
            }
            previous_cell_had_extras = matches!(cell.zerowidth(), Some(chars) if !chars.is_empty());

            if is_blank(cell) {
                inked[at] = Inked::Blank;
                continue;
            }
            let mut style = cell_style(cell, fg, theme, font);
            if let Some(selected) = selected.filter(|s| s.contains(display_line, col as usize)) {
                style.color = selected.fg;
                for color in [style.underline.as_mut().map(|u| &mut u.color), style.strikethrough.as_mut().map(|s| &mut s.color)]
                    .into_iter()
                    .flatten()
                {
                    *color = Some(selected.fg);
                }
            }
            if builtin_glyphs::is_builtin(cell.character()) {
                runs.extend(current.take());
                builtins.push(BuiltinCell { line: display_line, column: col, ch: cell.character(), color: style.color });
                continue;
            }
            let zero_width = cell.zerowidth();
            let cells = if cell.is_wide_char() { 2 } else { 1 };
            if fit(cell.character(), cells) {
                runs.extend(current.take());
                let mut batch = BatchedTextRun::new_from_char(display_line, col, cell.character(), style);
                batch.cell_count = cells;
                batch.fit = true;
                inked[at] = Inked::Fitted;
                for &c in zero_width.unwrap_or_default() {
                    batch.append_char(c, false);
                }
                runs.push(batch);
                continue;
            }
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
    give_room(&mut runs[line_runs..], &inked);
    (rects, builtins, runs)
}

#[derive(Clone, Copy, PartialEq)]
enum Inked {
    Blank,
    Text,
    Fitted,
}

/// Each fitted run of one line may overhang the blank cell after it, and the blank cell before it
/// unless a fitted character before that may overhang it already.
fn give_room(runs: &mut [BatchedTextRun], inked: &[Inked]) {
    let at = |column: i32| usize::try_from(column).ok().and_then(|c| inked.get(c).copied());
    for run in runs.iter_mut().filter(|r| r.fit) {
        let after = at(run.column + run.cell_count as i32) == Some(Inked::Blank);
        let before = at(run.column - 1) == Some(Inked::Blank) && at(run.column - 2) != Some(Inked::Fitted);
        run.room = (before as usize, after as usize);
    }
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

/// [`crate::palette`] of `theme`, built once per theme id ([`crate::themes`]).
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

    fn grid(cells: &[IndexedCell], theme: &Theme, selected: Option<&Selected>) -> (Vec<LayoutRect>, Vec<BuiltinCell>, Vec<BatchedTextRun>) {
        layout_grid(cells.iter(), theme, selected, &gpui::font("Consolas"), &|_, _| false)
    }

    #[test]
    fn a_fitted_character_is_a_run_of_its_own_and_the_rest_keep_one_run_per_style() {
        let theme = theme();
        let cells = row(0, "a->◯x->y", Color::Named(NamedColor::Foreground));
        let (_, _, runs) = layout_grid(cells.iter(), &theme, None, &gpui::font("Consolas"), &|c, _| c == '◯');
        let shown: Vec<(&str, i32, usize, bool)> = runs.iter().map(|r| (r.text.as_str(), r.column, r.cell_count, r.fit)).collect();
        assert_eq!(shown, [("a->", 0, 3, false), ("◯", 3, 1, true), ("x->y", 4, 4, false)]);
        assert_eq!(runs[2].cell_starts, [0, 1, 2, 3]);
    }

    #[test]
    fn a_fitted_character_may_overhang_a_blank_neighbour_that_no_other_fitted_one_claims() {
        let theme = theme();
        let cells = row(0, "⏵⏵ a ⏵ ⏵", Color::Named(NamedColor::Foreground));
        let (_, _, runs) = layout_grid(cells.iter(), &theme, None, &gpui::font("Consolas"), &|c, _| c == '⏵');
        let room: Vec<(i32, (usize, usize))> = runs.iter().filter(|r| r.fit).map(|r| (r.column, r.room)).collect();
        assert_eq!(room, [(0, (0, 0)), (1, (0, 1)), (5, (1, 1)), (7, (0, 0))]);
    }

    #[test]
    fn cell_starts_skip_zero_width_characters() {
        let mut run = BatchedTextRun::new_from_char(0, 0, 'e', TextRun { len: 1, ..Default::default() });
        run.append_char('\u{301}', false);
        run.append_char('é', true);
        run.append_char('x', true);
        assert_eq!(run.cell_starts, [0, 3, 5]);
        assert_eq!(run.cell_count, 3);
        assert_eq!(run.style.len, run.text.len());
    }

    #[test]
    fn ligatures_off_disables_every_ligature_feature() {
        let off = TermFont { family: "Cascadia Code".into(), size: FONT_SIZE, ligatures: false }.font();
        let on = TermFont { ligatures: true, ..TermFont { family: "Cascadia Code".into(), size: FONT_SIZE, ligatures: false } }.font();
        assert_eq!(off.features.tag_value_list(), [("calt".to_string(), 0), ("liga".to_string(), 0), ("clig".to_string(), 0)]);
        assert!(on.features.tag_value_list().is_empty(), "the shaper's defaults: calt and liga on");
        assert_eq!(on.family, "Cascadia Code");
    }

    #[test]
    fn selected_text_is_drawn_in_the_background_colour_and_the_rest_is_not() {
        let theme = theme();
        let fg = Color::Named(NamedColor::Foreground);
        let background = theme.colors().terminal_background;
        let selected = Selected { start: (0, 2), end: (1, 1), is_block: false, fg: background };
        let cells: Vec<_> = row(0, "abcdef", fg).into_iter().chain(row(1, "ghijkl", fg)).collect();
        let (_, _, runs) = grid(&cells, &theme, Some(&selected));
        let drawn: Vec<(i32, &str, bool)> = runs.iter().map(|r| (r.line, r.text.as_str(), r.style.color == background)).collect();
        assert_eq!(drawn, [(0, "ab", false), (0, "cdef", true), (1, "gh", true), (1, "ijkl", false)]);
        assert_eq!(SELECTION_BACKGROUND, gpui::rgb(0xFFFFFF).into());
        assert!(Selected { start: (0, 4), end: (2, 1), is_block: true, ..selected }.contains(1, 3));
        assert!(!Selected { start: (0, 4), end: (2, 1), is_block: true, ..selected }.contains(1, 0));
    }

    #[test]
    fn box_drawing_and_block_elements_are_builtin_cells_between_text_runs() {
        let theme = theme();
        let fg = Color::Spec(Rgb { r: 215, g: 119, b: 87 });
        let cells = row(0, "ab┌─┐█c", fg);
        let (_, builtins, runs) = grid(&cells, &theme, None);
        assert_eq!(runs.iter().map(|r| (r.text.as_str(), r.column)).collect_vec(), [("ab", 0), ("c", 6)]);
        let shown = builtins.iter().map(|b| (b.line, b.column, b.ch)).collect_vec();
        assert_eq!(shown, [(0, 2, '┌'), (0, 3, '─'), (0, 4, '┐'), (0, 5, '█')]);
        assert!(builtins.iter().all(|b| b.color == foreman_term::rgba_color(215, 119, 87)));
    }

    #[test]
    fn a_selected_builtin_cell_is_drawn_in_the_selection_colour() {
        let theme = theme();
        let background = theme.colors().terminal_background;
        let selected = Selected { start: (0, 1), end: (0, 1), is_block: false, fg: background };
        let cells = row(0, "─│─", Color::Named(NamedColor::Foreground));
        let (_, builtins, _) = grid(&cells, &theme, Some(&selected));
        assert_eq!(builtins.iter().map(|b| b.color == background).collect_vec(), [false, true, false]);
    }
}
