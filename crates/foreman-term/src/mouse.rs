// Vendored from Zed crates/terminal/src/terminal.rs @ 03efd49220ba06703d04f9c4732fdf814f9c523c
// (GPL-3.0-or-later).

#[cfg(test)]
use crate::TerminalBounds;
use crate::alacritty::selection_text;
use crate::events::InternalEvent;
use crate::mappings::mouse::{
    grid_point, grid_point_and_side, mouse_button_report, mouse_moved_report,
};
use crate::scroll::Scroll;
use crate::terminal_settings::TerminalConfig;
use crate::{Modes, Point, Range, Terminal};
#[cfg(test)]
use gpui::Point as GpuiPoint;
use gpui::{Bounds, Context, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels};
#[cfg(test)]
use std::cmp;

#[derive(Clone, Debug)]
pub(crate) struct Selection {
    pub(crate) ty: SelectionType,
    pub(crate) start: SelectionAnchor,
    pub(crate) end: SelectionAnchor,
    pub(crate) head: Point,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct SelectionAnchor {
    pub(crate) point: Point,
    pub(crate) side: SelectionSide,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SelectionSide {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SelectionType {
    Simple,
    Semantic,
    Lines,
    Block,
}

impl Selection {
    pub(crate) fn new(selection_type: SelectionType, point: Point, side: SelectionSide) -> Self {
        let anchor = SelectionAnchor { point, side };
        Self {
            ty: selection_type,
            start: anchor,
            end: anchor,
            head: point,
        }
    }

    pub(crate) fn simple_range(range: Range) -> Self {
        let mut selection = Self::new(SelectionType::Simple, range.start(), SelectionSide::Left);
        selection.update(range.end(), SelectionSide::Right);
        selection
    }

    pub(crate) fn update(&mut self, point: Point, side: SelectionSide) {
        self.end = SelectionAnchor { point, side };
        self.head = point;
    }
}

#[derive(PartialEq, Eq)]
pub(crate) enum SelectionPhase {
    Selecting,
    Ended,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MouseInputMode {
    ReportToTerminal,
    LocalSelection,
}

/// Minimum pointer movement before a left click begins a selection. This keeps
/// a click that jitters by a pixel or two (such as the window-focusing click)
/// from starting a selection and, with `copy_on_select` enabled, clobbering the
/// clipboard.
pub(crate) const SELECTION_DRAG_THRESHOLD: f64 = 2.0;

#[cfg(test)]
pub(crate) fn content_index_for_mouse(
    pos: GpuiPoint<Pixels>,
    terminal_bounds: &TerminalBounds,
) -> usize {
    let col = (pos.x / terminal_bounds.cell_width()).round() as usize;
    let clamped_col = cmp::min(col, terminal_bounds.num_columns().saturating_sub(1));
    let row = (pos.y / terminal_bounds.line_height()).round() as usize;
    let clamped_row = cmp::min(row, terminal_bounds.num_lines().saturating_sub(1));
    clamped_row * terminal_bounds.num_columns() + clamped_col
}

impl Terminal {
    pub fn selection_started(&self) -> bool {
        self.selection_phase == SelectionPhase::Selecting
    }

    pub(crate) fn set_selection(&mut self, selection: Option<Selection>) {
        self.events
            .push_back(InternalEvent::SetSelection(selection));
    }

    pub fn selection_text(&self) -> Option<String> {
        selection_text(&self.term.lock_unfair())
    }

    pub(crate) fn mouse_changed(&mut self, point: Point, side: SelectionSide) -> bool {
        match self.last_mouse {
            Some((old_point, old_side)) => {
                if old_point == point && old_side == side {
                    false
                } else {
                    self.last_mouse = Some((point, side));
                    true
                }
            }
            None => {
                self.last_mouse = Some((point, side));
                true
            }
        }
    }

    pub fn mouse_mode(&self, shift: bool, mode: MouseInputMode) -> bool {
        mode == MouseInputMode::ReportToTerminal
            && self.last_content.mode.intersects(Modes::MOUSE_MODE)
            && !shift
    }

    pub fn mouse_move(&mut self, e: &MouseMoveEvent, mode: MouseInputMode, cx: &mut Context<Self>) {
        let position = e.position - self.last_content.terminal_bounds.bounds.origin;
        if self.mouse_mode(e.modifiers.shift, mode) {
            // The press on a link sent no report, so the whole gesture is swallowed: forwarding its motion
            // and release would be a press-less sequence. `mouse_up` opens the link or drops the gesture.
            if self.mouse_down_hyperlink.is_none() {
                let (point, side) = grid_point_and_side(
                    position,
                    self.last_content.terminal_bounds,
                    self.last_content.display_offset,
                );

                if self.mouse_changed(point, side) {
                    let bytes = mouse_moved_report(
                        point,
                        e.pressed_button,
                        e.modifiers,
                        self.last_content.mode,
                    );

                    if let Some(bytes) = bytes {
                        self.write_to_pty(bytes);
                    }
                }
            }
        } else {
            self.schedule_find_hyperlink(e.modifiers, e.position, cx);
        }
        cx.notify();
    }

    pub fn mouse_drag(
        &mut self,
        e: &MouseMoveEvent,
        region: Bounds<Pixels>,
        mode: MouseInputMode,
        cx: &mut Context<Self>,
    ) {
        let position = e.position - self.last_content.terminal_bounds.bounds.origin;
        if !self.mouse_mode(e.modifiers.shift, mode) {
            if let Some(hyperlink) = &self.mouse_down_hyperlink {
                let point = grid_point(
                    position,
                    self.last_content.terminal_bounds,
                    self.last_content.display_offset,
                );

                if !hyperlink.range.contains(point) {
                    self.mouse_down_hyperlink = None;
                } else {
                    return;
                }
            }

            if self.selection_phase != SelectionPhase::Selecting
                && let Some(mouse_down_position) = self.mouse_down_position
                && (e.position - mouse_down_position).magnitude() <= SELECTION_DRAG_THRESHOLD
            {
                return;
            }

            self.selection_phase = SelectionPhase::Selecting;
            // Alacritty has the same ordering, of first updating the selection
            // then scrolling 15ms later
            self.events
                .push_back(InternalEvent::UpdateSelection(position));

            // Doesn't make sense to scroll the alt screen
            if !self.last_content.mode.contains(Modes::ALT_SCREEN) {
                let scroll_lines = match self.drag_line_delta(e, region) {
                    Some(value) => value,
                    None => return,
                };

                self.events
                    .push_back(InternalEvent::Scroll(Scroll::Delta(scroll_lines)));
            }

            cx.notify();
        }
    }

    pub(crate) fn drag_line_delta(
        &self,
        e: &MouseMoveEvent,
        region: Bounds<Pixels>,
    ) -> Option<i32> {
        let top = region.origin.y;
        let bottom = region.bottom_left().y;

        let scroll_lines = if e.position.y < top {
            let scroll_delta = (top - e.position.y).pow(1.1);
            (scroll_delta / self.last_content.terminal_bounds.line_height).ceil() as i32
        } else if e.position.y > bottom {
            let scroll_delta = -((e.position.y - bottom).pow(1.1));
            (scroll_delta / self.last_content.terminal_bounds.line_height).floor() as i32
        } else {
            return None;
        };

        Some(scroll_lines.clamp(-3, 3))
    }

    pub fn mouse_down(&mut self, e: &MouseDownEvent, mode: MouseInputMode, cx: &mut Context<Self>) {
        let position = e.position - self.last_content.terminal_bounds.bounds.origin;
        let point = grid_point(
            position,
            self.last_content.terminal_bounds,
            self.last_content.display_offset,
        );

        if e.button == MouseButton::Left
            && e.modifiers.secondary()
            && (TerminalConfig::get(cx).open_links_in_mouse_mode
                || !self.mouse_mode(e.modifiers.shift, mode))
        {
            self.mouse_down_hyperlink = self.find_hyperlink_at_point(point);

            if self.mouse_down_hyperlink.is_some() {
                return;
            }
        }

        if self.mouse_mode(e.modifiers.shift, mode) {
            let bytes =
                mouse_button_report(point, e.button, e.modifiers, true, self.last_content.mode);

            if let Some(bytes) = bytes {
                self.write_to_pty(bytes);
            }
        } else {
            match e.button {
                MouseButton::Left => {
                    self.mouse_down_position = Some(e.position);
                    let selection_type = match e.click_count {
                        0 => return, //This is a release
                        1 if e.modifiers.alt => Some(SelectionType::Block),
                        1 => Some(SelectionType::Simple),
                        2 => Some(SelectionType::Semantic),
                        3 => Some(SelectionType::Lines),
                        _ => None,
                    };

                    if selection_type == Some(SelectionType::Simple) && e.modifiers.shift {
                        if self.last_content.selection.is_some() {
                            self.events
                                .push_back(InternalEvent::UpdateSelection(position));
                        } else {
                            // With no selection yet, Shift is the escape hatch for
                            // selecting text while an app has mouse tracking enabled,
                            // so anchor a selection here for the drag to extend.
                            self.events.push_back(InternalEvent::BeginSelection(
                                SelectionType::Simple,
                                position,
                            ));
                        }
                        return;
                    }

                    if let Some(selection_type) = selection_type {
                        self.events
                            .push_back(InternalEvent::BeginSelection(selection_type, position));
                    }
                }
                #[cfg(any(target_os = "linux", target_os = "freebsd"))]
                MouseButton::Middle => {
                    if mode == MouseInputMode::ReportToTerminal
                        && let Some(item) = cx.read_from_primary()
                    {
                        let text = item.text().unwrap_or_default();
                        self.paste(&text);
                    }
                }
                _ => {}
            }
        }
    }

    pub fn mouse_up(&mut self, e: &MouseUpEvent, mode: MouseInputMode, cx: &Context<Self>) {
        let setting = TerminalConfig::get(cx);

        let position = e.position - self.last_content.terminal_bounds.bounds.origin;
        if let Some(mouse_down_hyperlink) = self.mouse_down_hyperlink.take() {
            let point = grid_point(
                position,
                self.last_content.terminal_bounds,
                self.last_content.display_offset,
            );

            if self
                .find_hyperlink_at_point(point)
                .is_some_and(|mouse_up_hyperlink| mouse_up_hyperlink == mouse_down_hyperlink)
            {
                self.events
                    .push_back(InternalEvent::ProcessHyperlink(mouse_down_hyperlink, true));
                self.selection_phase = SelectionPhase::Ended;
                self.last_mouse = None;
                self.mouse_down_position = None;
                return;
            }

            if self.mouse_mode(e.modifiers.shift, mode) {
                self.selection_phase = SelectionPhase::Ended;
                self.last_mouse = None;
                self.mouse_down_position = None;
                return;
            }
        }

        if self.mouse_mode(e.modifiers.shift, mode) {
            let point = grid_point(
                position,
                self.last_content.terminal_bounds,
                self.last_content.display_offset,
            );

            let bytes =
                mouse_button_report(point, e.button, e.modifiers, false, self.last_content.mode);

            if let Some(bytes) = bytes {
                self.write_to_pty(bytes);
            }
        } else {
            if e.button == MouseButton::Left && setting.copy_on_select {
                self.copy(Some(true));
            }

            // Links open only on Ctrl+click, for any scheme (m5/44 decision 5).
            if self.selection_phase == SelectionPhase::Ended && e.modifiers.secondary() {
                self.events
                    .push_back(InternalEvent::FindHyperlink(position, true));
            }
        }

        self.selection_phase = SelectionPhase::Ended;
        self.last_mouse = None;
        self.mouse_down_position = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alacritty::{display_offset, selection_text, set_selection as set_term_selection};
    use crate::events::InternalEvent;
    use crate::terminal_settings::{
        AlternateScroll, CursorShape as SettingsCursorShape, WORD_DELIMITERS,
    };
    use crate::test_support::*;
    use crate::{
        Cell, Content, IndexedCell, Modes, Point, SelectionRange, TerminalBounds, TerminalBuilder,
    };
    use gpui::{
        Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Point as GpuiPoint,
        ScrollDelta, ScrollEvent, TestAppContext, bounds, point, px, size,
    };
    use rand::distr;
    use rand::rngs::StdRng;

    use gpui::AppContext as _;
    use gpui::VisualContext as _;
    use rand::Rng as _;
    use util::paths::PathStyle;

    /// A left click that jitters by a pixel or two (e.g. the window-focusing
    /// click) must not begin a selection, otherwise `copy_on_select` would
    /// overwrite the clipboard. Regression test for #58970.
    #[gpui::test]
    async fn test_terminal_click_jitter_does_not_start_selection(cx: &mut TestAppContext) {
        let terminal = init_terminal_test(cx, b"hello world\r\n");

        terminal.update(cx, |terminal, cx| {
            left_mouse_down_at(terminal, point(px(50.0), px(10.0)), cx);
            terminal.events.clear();

            // One pixel of movement is below the drag threshold.
            left_mouse_drag_to(terminal, point(px(51.0), px(10.0)), cx);

            assert!(
                !terminal
                    .events
                    .iter()
                    .any(|event| matches!(event, InternalEvent::UpdateSelection(_))),
                "a sub-threshold click jitter should not start a selection"
            );
            assert!(terminal.selection_phase == SelectionPhase::Ended);
        });
    }

    /// A deliberate drag past the threshold must still start a selection.
    #[gpui::test]
    async fn test_terminal_deliberate_drag_starts_selection(cx: &mut TestAppContext) {
        let terminal = init_terminal_test(cx, b"hello world\r\n");

        terminal.update(cx, |terminal, cx| {
            left_mouse_down_at(terminal, point(px(50.0), px(10.0)), cx);
            terminal.events.clear();

            left_mouse_drag_to(terminal, point(px(90.0), px(10.0)), cx);

            assert!(
                terminal
                    .events
                    .iter()
                    .any(|event| matches!(event, InternalEvent::UpdateSelection(_))),
                "a deliberate drag should start a selection"
            );
            assert!(terminal.selection_phase == SelectionPhase::Selecting);
        });
    }

    #[gpui::test]
    async fn test_local_selection_scrolls_without_terminal_reports(cx: &mut TestAppContext) {
        let mut output = Vec::new();
        output.extend_from_slice(b"\x1b[?1002h\x1b[?1006h");
        for line in 0..80 {
            output.extend_from_slice(format!("scrollback line {line}\r\n").as_bytes());
        }
        let (terminal, cx) = init_terminal_test_with_window(cx, &output);
        cx.run_until_parked();

        cx.update_window_entity(&terminal, |terminal, window, cx| {
            terminal.sync(window, cx);
            assert!(terminal.last_content.mode.intersects(Modes::MOUSE_MODE));
            assert_eq!(display_offset(&terminal.term.lock()), 0);

            let scroll_event = ScrollEvent {
                position: point(px(50.0), px(10.0)),
                delta: ScrollDelta::Lines(point(0.0, 1.0)),
                ..Default::default()
            };

            terminal.scroll_wheel(&scroll_event, 1.0, MouseInputMode::LocalSelection);
            terminal.sync(window, cx);
            assert!(display_offset(&terminal.term.lock()) > 0);
            assert!(terminal.take_input_log().is_empty());
            assert!(terminal.take_pty_write_log().is_empty());

            terminal.scroll_wheel(&scroll_event, 1.0, MouseInputMode::ReportToTerminal);
            assert!(!terminal.take_pty_write_log().is_empty());

            terminal.write_output(b"\x1b[?1049h\x1b[?1007h", cx);
        });
        cx.run_until_parked();

        cx.update_window_entity(&terminal, |terminal, window, cx| {
            terminal.sync(window, cx);
            assert!(
                terminal
                    .last_content
                    .mode
                    .contains(Modes::ALT_SCREEN | Modes::ALTERNATE_SCROLL)
            );
            assert_eq!(display_offset(&terminal.term.lock()), 0);

            let scroll_event = ScrollEvent {
                position: point(px(50.0), px(10.0)),
                delta: ScrollDelta::Lines(point(0.0, 1.0)),
                ..Default::default()
            };
            terminal.scroll_wheel(&scroll_event, 1.0, MouseInputMode::LocalSelection);
            terminal.sync(window, cx);
            assert_eq!(display_offset(&terminal.term.lock()), 0);
            assert!(terminal.take_input_log().is_empty());
            assert!(terminal.take_pty_write_log().is_empty());
        });
    }

    /// Scrolled back, output that arrives after the last frame bumps `display_offset` so the view
    /// stays put. A drag dispatched before the next frame must anchor on the row the
    /// pointer is over, as its head does, not one line below it.
    #[gpui::test]
    async fn test_drag_anchors_on_the_shown_row_when_output_arrives_scrolled_back(
        cx: &mut TestAppContext,
    ) {
        let mut output = Vec::new();
        for line in 0..80 {
            output.extend_from_slice(format!("scrollback line {line}\r\n").as_bytes());
        }
        let (terminal, cx) = init_terminal_test_with_window(cx, &output);
        cx.run_until_parked();

        cx.update_window_entity(&terminal, |terminal, window, cx| {
            terminal.sync(window, cx);
            terminal.scroll_up_by(3);
            terminal.sync(window, cx);
            assert_eq!(terminal.last_content.display_offset, 3);
            let row = 2;
            let shown: String = terminal
                .last_content
                .cells
                .iter()
                .filter(|c| c.point.line + 3 == row)
                .map(|c| c.cell.character())
                .collect::<String>()
                .trim_end()
                .to_string();
            assert!(
                shown.starts_with("scrollback line "),
                "row {row} shows {shown:?}"
            );

            terminal.write_output(b"more output\r\n", cx);
            assert_eq!(
                display_offset(&terminal.term.lock()),
                4,
                "output must not move the view"
            );

            let bounds = terminal.last_content.terminal_bounds;
            let y = bounds.line_height * (row as f32 + 0.5);
            let from = point(bounds.cell_width * 0.25, y);
            let to = point(bounds.cell_width * (shown.len() as f32 - 0.25), y);
            left_mouse_down_at(terminal, from, cx);
            left_mouse_drag_to(terminal, to, cx);
            left_mouse_up_at(terminal, to, cx);
            terminal.sync(window, cx);
            assert_eq!(terminal.selection_text().as_deref(), Some(shown.as_str()));
        });
    }

    /// The modes `claude` enables at startup (measured: `?1000h ?1002h ?1003h ?1006h`) make a
    /// click at a cell, a drag and the release reach the PTY as SGR reports for that cell.
    #[gpui::test]
    async fn test_claude_mouse_modes_report_sgr_click_drag_release(cx: &mut TestAppContext) {
        let terminal = init_terminal_test(
            cx,
            b"\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1006h> prompt\r\n",
        );
        terminal.update(cx, |terminal, cx| {
            // 10 px cells, 20 px lines: (55, 45) is column 5, line 2, so 1-based 6;3
            left_mouse_down_at(terminal, point(px(55.0), px(45.0)), cx);
            assert_eq!(
                terminal.take_pty_write_log(),
                vec![b"\x1b[<0;6;3M".to_vec()]
            );

            terminal.mouse_move(
                &MouseMoveEvent {
                    position: point(px(75.0), px(45.0)),
                    pressed_button: Some(MouseButton::Left),
                    modifiers: Modifiers::none(),
                },
                MouseInputMode::ReportToTerminal,
                cx,
            );
            assert_eq!(
                terminal.take_pty_write_log(),
                vec![b"\x1b[<32;8;3M".to_vec()]
            );

            left_mouse_up_at(terminal, point(px(75.0), px(45.0)), cx);
            assert_eq!(
                terminal.take_pty_write_log(),
                vec![b"\x1b[<0;8;3m".to_vec()]
            );
            assert!(
                terminal.events.is_empty(),
                "a reported click must not select"
            );
        });
    }

    /// With mouse tracking active (e.g. htop), Shift is the escape hatch to
    /// select terminal text. Shift+drag must start a selection rather than being
    /// swallowed as a "extend existing selection" no-op. Regression test for #60254.
    #[gpui::test]
    async fn test_terminal_shift_drag_selects_while_mouse_tracking(cx: &mut TestAppContext) {
        // `?1002h` enables button-event mouse tracking, `?1006h` selects SGR encoding.
        let terminal = init_terminal_test(cx, b"\x1b[?1002h\x1b[?1006hhello world\r\n");

        terminal.update(cx, |terminal, cx| {
            assert!(
                terminal.last_content.mode.intersects(Modes::MOUSE_MODE),
                "mouse tracking should be active"
            );

            let shift = Modifiers {
                shift: true,
                ..Modifiers::none()
            };
            terminal.mouse_down(
                &MouseDownEvent {
                    button: MouseButton::Left,
                    position: point(px(50.0), px(10.0)),
                    modifiers: shift,
                    click_count: 1,
                    first_mouse: true,
                },
                MouseInputMode::ReportToTerminal,
                cx,
            );

            // With no selection yet, the shift press must anchor a new selection
            // so the following drag has something to extend.
            assert!(
                terminal.events.iter().any(|event| matches!(
                    event,
                    InternalEvent::BeginSelection(SelectionType::Simple, _)
                )),
                "shift+click with no existing selection should anchor a selection"
            );
            terminal.events.clear();

            let region = terminal.last_content.terminal_bounds.bounds;
            terminal.mouse_drag(
                &MouseMoveEvent {
                    position: point(px(90.0), px(10.0)),
                    pressed_button: Some(MouseButton::Left),
                    modifiers: shift,
                },
                region,
                MouseInputMode::ReportToTerminal,
                cx,
            );

            assert!(
                terminal
                    .events
                    .iter()
                    .any(|event| matches!(event, InternalEvent::UpdateSelection(_))),
                "shift+drag should extend the selection while mouse tracking is active"
            );
            assert!(terminal.selection_phase == SelectionPhase::Selecting);
        });
    }

    /// Shift+click with a selection already on screen must keep extending it
    /// (the behavior added in #25143), not re-anchor a fresh one.
    #[gpui::test]
    async fn test_terminal_shift_click_extends_existing_selection(cx: &mut TestAppContext) {
        let terminal = init_terminal_test(cx, b"hello world\r\n");

        terminal.update(cx, |terminal, cx| {
            // A visible selection, as a sync would have populated in production.
            terminal.last_content.selection = Some(SelectionRange {
                start: Point::new(0, 0),
                end: Point::new(0, 5),
                is_block: false,
            });
            terminal.events.clear();

            terminal.mouse_down(
                &MouseDownEvent {
                    button: MouseButton::Left,
                    position: point(px(90.0), px(10.0)),
                    modifiers: Modifiers {
                        shift: true,
                        ..Modifiers::none()
                    },
                    click_count: 1,
                    first_mouse: true,
                },
                MouseInputMode::ReportToTerminal,
                cx,
            );

            assert!(
                terminal
                    .events
                    .iter()
                    .any(|event| matches!(event, InternalEvent::UpdateSelection(_))),
                "shift+click with an existing selection should extend it"
            );
            assert!(
                !terminal.events.iter().any(|event| matches!(
                    event,
                    InternalEvent::SetSelection(Some(_)) | InternalEvent::BeginSelection(..)
                )),
                "shift+click should extend, not re-anchor, an existing selection"
            );
        });
    }

    #[gpui::test]
    fn test_mouse_to_cell_test(mut rng: StdRng) {
        const ITERATIONS: usize = 10;
        const PRECISION: usize = 1000;

        for _ in 0..ITERATIONS {
            let viewport_cells = rng.random_range(15..20);
            let cell_size =
                rng.random_range(5 * PRECISION..20 * PRECISION) as f32 / PRECISION as f32;

            let size = crate::TerminalBounds {
                cell_width: Pixels::from(cell_size),
                line_height: Pixels::from(cell_size),
                bounds: bounds(
                    GpuiPoint::default(),
                    size(
                        Pixels::from(cell_size * (viewport_cells as f32)),
                        Pixels::from(cell_size * (viewport_cells as f32)),
                    ),
                ),
            };

            let cells = get_cells(size, &mut rng);
            let content = convert_cells_to_content(size, &cells);

            for row in 0..(viewport_cells - 1) {
                let row = row as usize;
                for col in 0..(viewport_cells - 1) {
                    let col = col as usize;

                    let row_offset = rng.random_range(0..PRECISION) as f32 / PRECISION as f32;
                    let col_offset = rng.random_range(0..PRECISION) as f32 / PRECISION as f32;

                    let mouse_pos = point(
                        Pixels::from(col as f32 * cell_size + col_offset),
                        Pixels::from(row as f32 * cell_size + row_offset),
                    );

                    let content_index =
                        content_index_for_mouse(mouse_pos, &content.terminal_bounds);
                    let mouse_cell = content.cells[content_index].character();
                    let real_cell = cells[row][col];

                    assert_eq!(mouse_cell, real_cell);
                }
            }
        }
    }

    #[gpui::test]
    fn test_mouse_to_cell_clamp(mut rng: StdRng) {
        let size = crate::TerminalBounds {
            cell_width: Pixels::from(10.),
            line_height: Pixels::from(10.),
            bounds: bounds(
                GpuiPoint::default(),
                size(Pixels::from(100.), Pixels::from(100.)),
            ),
        };

        let cells = get_cells(size, &mut rng);
        let content = convert_cells_to_content(size, &cells);

        assert_eq!(
            content.cells[content_index_for_mouse(
                point(Pixels::from(-10.), Pixels::from(-10.)),
                &content.terminal_bounds,
            )]
            .character(),
            cells[0][0]
        );
        assert_eq!(
            content.cells[content_index_for_mouse(
                point(Pixels::from(1000.), Pixels::from(1000.)),
                &content.terminal_bounds,
            )]
            .character(),
            cells[9][9]
        );
    }

    fn get_cells(size: TerminalBounds, rng: &mut StdRng) -> Vec<Vec<char>> {
        let mut cells = Vec::new();

        for _ in 0..size.num_lines() {
            let mut row_vec = Vec::new();
            for _ in 0..size.num_columns() {
                let cell_char = rng.sample(distr::Alphanumeric) as char;
                row_vec.push(cell_char)
            }
            cells.push(row_vec)
        }

        cells
    }

    fn convert_cells_to_content(terminal_bounds: TerminalBounds, cells: &[Vec<char>]) -> Content {
        let mut ic = Vec::new();

        for (index, row) in cells.iter().enumerate() {
            for (cell_index, cell_char) in row.iter().enumerate() {
                let mut cell = Cell::default();
                cell.set_character(*cell_char);
                ic.push(IndexedCell {
                    point: Point::new(index as i32, cell_index),
                    cell,
                });
            }
        }

        Content {
            cells: ic,
            terminal_bounds,
            ..Default::default()
        }
    }

    #[test]
    fn word_delimiters_are_windows_terminals() {
        for c in [
            ' ', '/', '\\', '(', '"', '\'', '-', '.', ',', ':', ';', '=', '?', '│',
        ] {
            assert!(WORD_DELIMITERS.contains(c), "{c:?}");
        }
        assert!(!WORD_DELIMITERS.contains('_'));
    }

    #[gpui::test]
    async fn double_click_selects_a_word_up_to_windows_terminal_delimiters(
        cx: &mut TestAppContext,
    ) {
        let terminal = cx.new(|cx| {
            TerminalBuilder::new_display_only(
                SettingsCursorShape::Bar,
                AlternateScroll::On,
                None,
                cx.background_executor(),
                PathStyle::local(),
            )
            .subscribe(cx)
        });
        let text = cx.update(|cx| {
            terminal.update(cx, |terminal, cx| {
                terminal.write_output(
                    b"Get-Item -LiteralPath C:\\temp\\fm_x.txt\nabcdef\nghijkl",
                    cx,
                );
                let term = terminal.term.clone();
                let mut term = term.lock();
                // "LiteralPath": columns 10..=20
                let point = Point::new(0, 14);
                set_term_selection(
                    &mut term,
                    Some(&Selection::new(
                        SelectionType::Semantic,
                        point,
                        SelectionSide::Left,
                    )),
                );
                selection_text(&term)
            })
        });
        assert_eq!(text.as_deref(), Some("LiteralPath"));
        let text = cx.update(|cx| {
            terminal.update(cx, |terminal, _| {
                let term = terminal.term.clone();
                let mut term = term.lock();
                set_term_selection(
                    &mut term,
                    Some(&Selection::new(
                        SelectionType::Semantic,
                        Point::new(0, 33),
                        SelectionSide::Left,
                    )),
                );
                selection_text(&term)
            })
        });
        assert_eq!(text.as_deref(), Some("fm_x"));
        // Alt+drag: a block from column 1 to 3 on both lines
        let text = cx.update(|cx| {
            terminal.update(cx, |terminal, _| {
                let term = terminal.term.clone();
                let mut term = term.lock();
                let mut block =
                    Selection::new(SelectionType::Block, Point::new(1, 1), SelectionSide::Left);
                block.update(Point::new(2, 3), SelectionSide::Right);
                set_term_selection(&mut term, Some(&block));
                selection_text(&term)
            })
        });
        assert_eq!(text.as_deref(), Some("bcd\nhij"));
    }
}
