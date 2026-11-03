// Vendored from Zed crates/terminal/src/terminal.rs @ 03efd49220ba06703d04f9c4732fdf814f9c523c
// (GPL-3.0-or-later).

use crate::events::InternalEvent;
use crate::mappings::mouse::{alt_scroll, grid_point, scroll_report};
use crate::{Modes, MouseInputMode, Terminal};
use gpui::{ScrollEvent, TouchPhase, px};

#[derive(Clone, Copy, Debug)]
pub(crate) enum Scroll {
    Delta(i32),
    PageUp,
    PageDown,
    Top,
    Bottom,
}

impl Terminal {
    pub fn scroll_up_by(&mut self, lines: usize) {
        self.events
            .push_back(InternalEvent::Scroll(Scroll::Delta(lines as i32)));
    }

    pub fn scroll_down_by(&mut self, lines: usize) {
        self.events
            .push_back(InternalEvent::Scroll(Scroll::Delta(-(lines as i32))));
    }

    pub fn scroll_page_up(&mut self) {
        self.events.push_back(InternalEvent::Scroll(Scroll::PageUp));
    }

    pub fn scroll_page_down(&mut self) {
        self.events
            .push_back(InternalEvent::Scroll(Scroll::PageDown));
    }

    pub fn scroll_to_top(&mut self) {
        self.events.push_back(InternalEvent::Scroll(Scroll::Top));
    }

    pub fn scroll_to_bottom(&mut self) {
        self.events.push_back(InternalEvent::Scroll(Scroll::Bottom));
    }

    pub fn scroll_wheel(&mut self, e: &ScrollEvent, scroll_multiplier: f32, mode: MouseInputMode) {
        let mouse_mode = self.mouse_mode(e.shift, mode);
        let scroll_multiplier = if mouse_mode { 1. } else { scroll_multiplier };

        if let Some(scroll_lines) = self.determine_scroll_lines(e, scroll_multiplier)
            && scroll_lines != 0
        {
            if mouse_mode {
                let point = grid_point(
                    e.position - self.last_content.terminal_bounds.bounds.origin,
                    self.last_content.terminal_bounds,
                    self.last_content.display_offset,
                );

                if let Some(scrolls) = scroll_report(point, scroll_lines, e, self.last_content.mode)
                {
                    for scroll in scrolls {
                        self.write_to_pty(scroll);
                    }
                };
            } else if mode == MouseInputMode::ReportToTerminal
                && self
                    .last_content
                    .mode
                    .contains(Modes::ALT_SCREEN | Modes::ALTERNATE_SCROLL)
                && !e.shift
            {
                self.write_to_pty(alt_scroll(scroll_lines));
            } else {
                self.events
                    .push_back(InternalEvent::Scroll(Scroll::Delta(scroll_lines)));
            }
        }
    }

    pub(crate) fn determine_scroll_lines(
        &mut self,
        e: &ScrollEvent,
        scroll_multiplier: f32,
    ) -> Option<i32> {
        let line_height = self.last_content.terminal_bounds.line_height;
        match e.touch_phase {
            /* Reset scroll state on started */
            TouchPhase::Started => {
                self.scroll_px = px(0.);
                None
            }
            /* Calculate the appropriate scroll lines */
            TouchPhase::Moved => {
                let old_offset = (self.scroll_px / line_height) as i32;

                self.scroll_px += e.delta.pixel_delta(line_height).y * scroll_multiplier;

                let new_offset = (self.scroll_px / line_height) as i32;

                // Reset at the edges, so a change of direction responds at once
                self.scroll_px %= self.last_content.terminal_bounds.height();

                Some(new_offset - old_offset)
            }
            // Cancellation does not commit a scroll, same as a plain end.
            TouchPhase::Ended | TouchPhase::Cancelled => None,
        }
    }
}

#[cfg(test)]
mod tests {

    use crate::alacritty::display_offset;
    use crate::events::InternalEvent;
    use crate::links::FIND_HYPERLINK_THROTTLE_PX;
    use crate::test_support::*;
    use crate::{Modes, MouseInputMode};
    use gpui::VisualContext as _;
    use gpui::{
        Modifiers, MouseMoveEvent, Point as GpuiPoint, ScrollDelta, ScrollEvent, TestAppContext,
        point, px,
    };
    use util::default;

    /// Supraman's tab passes `ReportToTerminal` for every wheel event: a plain shell
    /// scrolls its scrollback and writes nothing, later output keeps the view where the user
    /// left it, and the next keystroke scrolls back to the bottom.
    #[gpui::test]
    async fn test_report_mode_wheel_without_mouse_mode_scrolls_scrollback(cx: &mut TestAppContext) {
        let mut output = Vec::new();
        for line in 0..80 {
            output.extend_from_slice(format!("scrollback line {line}\r\n").as_bytes());
        }
        let (terminal, cx) = init_terminal_test_with_window(cx, &output);
        cx.run_until_parked();

        cx.update_window_entity(&terminal, |terminal, window, cx| {
            terminal.sync(window, cx);
            assert!(!terminal.last_content.mode.intersects(Modes::MOUSE_MODE));
            let scroll_event = ScrollEvent {
                position: point(px(50.0), px(10.0)),
                delta: ScrollDelta::Lines(point(0.0, 3.0)),
                ..Default::default()
            };
            terminal.scroll_wheel(&scroll_event, 1.0, MouseInputMode::ReportToTerminal);
            terminal.sync(window, cx);
            assert_eq!(display_offset(&terminal.term.lock()), 3);
            assert!(terminal.take_pty_write_log().is_empty());

            terminal.write_output(b"more output\r\n", cx);
        });
        cx.run_until_parked();

        cx.update_window_entity(&terminal, |terminal, window, cx| {
            terminal.sync(window, cx);
            assert!(
                display_offset(&terminal.term.lock()) >= 3,
                "output must not yank the view down"
            );

            terminal.input(b"x".as_slice());
            terminal.sync(window, cx);
            assert_eq!(display_offset(&terminal.term.lock()), 0);
        });
    }

    /// A full-screen program on the alternate screen with alternate scroll (`?1049h ?1007h`, no
    /// mouse tracking) gets arrow keys; shift forces a local scroll.
    #[gpui::test]
    async fn test_report_mode_wheel_on_alt_screen_sends_arrow_keys(cx: &mut TestAppContext) {
        let (terminal, cx) = init_terminal_test_with_window(cx, b"\x1b[?1049h\x1b[?1007h");
        cx.run_until_parked();

        cx.update_window_entity(&terminal, |terminal, window, cx| {
            terminal.sync(window, cx);
            let up = ScrollEvent {
                position: point(px(50.0), px(10.0)),
                delta: ScrollDelta::Lines(point(0.0, 2.0)),
                ..Default::default()
            };
            terminal.scroll_wheel(&up, 1.0, MouseInputMode::ReportToTerminal);
            assert_eq!(
                terminal.take_pty_write_log(),
                vec![b"\x1bOA\x1bOA".to_vec()]
            );

            let shifted = ScrollEvent {
                modifiers: Modifiers::shift(),
                ..up
            };
            terminal.scroll_wheel(&shifted, 1.0, MouseInputMode::ReportToTerminal);
            assert!(terminal.take_pty_write_log().is_empty());
        });
    }

    #[gpui::test]
    async fn scroll_long_line_benchmark(cx: &mut TestAppContext) {
        let (terminal, cx) =
            init_terminal_test_with_window(cx, "long line ".repeat(1000).as_bytes());
        let wobble = point(FIND_HYPERLINK_THROTTLE_PX, px(0.0));
        let mut scroll_by = |lines: i32| {
            cx.update_window_entity(&terminal, |terminal, window, cx| {
                let bounds = terminal.last_content.terminal_bounds.bounds;
                let center = bounds.origin + bounds.center();
                let position = center + wobble * lines as f32;

                terminal.mouse_move(
                    &MouseMoveEvent {
                        position,
                        ..default()
                    },
                    MouseInputMode::ReportToTerminal,
                    cx,
                );

                terminal.scroll_wheel(
                    &ScrollEvent {
                        position,
                        delta: ScrollDelta::Lines(GpuiPoint::new(0.0, lines as f32)),
                        ..default()
                    },
                    1.0,
                    MouseInputMode::ReportToTerminal,
                );

                assert!(
                    terminal
                        .events
                        .iter()
                        .any(|event| matches!(event, InternalEvent::Scroll(_))),
                    "Should have Scroll event when scrolling within terminal bounds"
                );
                terminal.sync(window, cx);
            });
        };

        for _ in 0..20000 {
            scroll_by(1);
            scroll_by(-1);
        }
    }
}
