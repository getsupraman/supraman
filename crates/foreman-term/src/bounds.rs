// Vendored from Zed crates/terminal/src/terminal.rs @ 03efd49220ba06703d04f9c4732fdf814f9c523c
// (GPL-3.0-or-later).

use crate::Terminal;
use crate::events::InternalEvent;
use gpui::{Bounds, Pixels, Point as GpuiPoint, Size, px};
use serde::{Deserialize, Serialize};
use std::cmp;

pub(crate) const DEBUG_TERMINAL_WIDTH: Pixels = px(500.);

pub(crate) const DEBUG_TERMINAL_HEIGHT: Pixels = px(30.);

pub(crate) const DEBUG_CELL_WIDTH: Pixels = px(5.);

pub(crate) const DEBUG_LINE_HEIGHT: Pixels = px(5.);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalBounds {
    pub cell_width: Pixels,
    pub line_height: Pixels,
    pub bounds: Bounds<Pixels>,
}

impl TerminalBounds {
    pub fn new(line_height: Pixels, cell_width: Pixels, bounds: Bounds<Pixels>) -> Self {
        TerminalBounds {
            cell_width,
            line_height,
            bounds,
        }
    }

    pub fn num_lines(&self) -> usize {
        // Tolerance to prevent f32 precision from losing a row:
        // `N * line_height / line_height` can be N-epsilon, which floor()
        // would round down, pushing the first line into invisible scrollback.
        let raw = self.bounds.size.height / self.line_height;
        raw.next_up().floor() as usize
    }

    pub fn num_columns(&self) -> usize {
        let raw = self.bounds.size.width / self.cell_width;
        raw.next_up().floor() as usize
    }

    pub(crate) fn height(&self) -> Pixels {
        self.bounds.size.height
    }

    pub(crate) fn width(&self) -> Pixels {
        self.bounds.size.width
    }

    pub(crate) fn cell_width(&self) -> Pixels {
        self.cell_width
    }

    pub(crate) fn line_height(&self) -> Pixels {
        self.line_height
    }
}

impl Default for TerminalBounds {
    fn default() -> Self {
        TerminalBounds::new(
            DEBUG_LINE_HEIGHT,
            DEBUG_CELL_WIDTH,
            Bounds {
                origin: GpuiPoint::default(),
                size: Size {
                    width: DEBUG_TERMINAL_WIDTH,
                    height: DEBUG_TERMINAL_HEIGHT,
                },
            },
        )
    }
}

pub(crate) fn normalize_terminal_bounds(mut bounds: TerminalBounds) -> TerminalBounds {
    bounds.bounds.size.height = cmp::max(bounds.line_height, bounds.height());
    bounds.bounds.size.width = cmp::max(bounds.cell_width, bounds.width());
    bounds
}

impl Terminal {
    ///Resize the terminal and the PTY.
    pub fn set_size(&mut self, new_bounds: TerminalBounds) {
        let new_bounds = normalize_terminal_bounds(new_bounds);

        let old_bounds = self.last_content.terminal_bounds;
        self.last_content.terminal_bounds = new_bounds;

        // Avoid spamming PTY resizes on pixel-level size changes (e.g. while dragging edges),
        // since those can generate excessive SIGWINCH/reflows and cause visible flicker.
        let requires_resize = old_bounds.num_lines() != new_bounds.num_lines()
            || old_bounds.num_columns() != new_bounds.num_columns()
            || old_bounds.cell_width != new_bounds.cell_width
            || old_bounds.line_height != new_bounds.line_height;

        if !requires_resize {
            return;
        }

        match self.events.back_mut() {
            Some(InternalEvent::Resize(pending_bounds)) => *pending_bounds = new_bounds,
            _ => self.events.push_back(InternalEvent::Resize(new_bounds)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::TerminalBuilder;
    use crate::events::InternalEvent;
    use crate::terminal_settings::{AlternateScroll, CursorShape as SettingsCursorShape};
    use gpui::{Bounds, Pixels, Point as GpuiPoint, Size, TestAppContext, bounds, px, size};

    use util::paths::PathStyle;

    #[gpui::test]
    async fn test_set_size_coalesces_pixel_only_changes(cx: &mut TestAppContext) {
        let builder = cx.update(|cx| {
            TerminalBuilder::new_display_only(
                SettingsCursorShape::Block,
                AlternateScroll::On,
                None,
                cx.background_executor(),
                PathStyle::local(),
            )
        });
        let mut terminal = builder.terminal;

        let base_bounds = TerminalBounds {
            cell_width: Pixels::from(10.),
            line_height: Pixels::from(10.),
            bounds: bounds(
                GpuiPoint::default(),
                size(Pixels::from(100.), Pixels::from(100.)),
            ),
        };

        terminal.set_size(base_bounds);
        terminal.events.clear();
        assert_eq!(terminal.last_content.terminal_bounds, base_bounds);

        // Pixel-only change: height grows by 1px but still the same number of rows/cols.
        let mut pixel_changed = base_bounds;
        pixel_changed.bounds.size.height = Pixels::from(101.);
        terminal.set_size(pixel_changed);
        assert!(terminal.events.is_empty());
        assert_eq!(terminal.last_content.terminal_bounds, pixel_changed);

        // Grid change: height increases enough to add a row.
        let mut grid_changed = base_bounds;
        grid_changed.bounds.size.height = Pixels::from(110.);
        terminal.set_size(grid_changed);
        assert!(matches!(
            terminal.events.back(),
            Some(InternalEvent::Resize(_))
        ));
    }

    #[test]
    fn test_num_lines_float_precision() {
        let line_heights = [
            20.1f32, 16.7, 18.3, 22.9, 14.1, 15.6, 17.8, 19.4, 21.3, 23.7,
        ];
        for &line_height in &line_heights {
            for n in 1..=100 {
                let height = n as f32 * line_height;
                let bounds = TerminalBounds::new(
                    px(line_height),
                    px(8.0),
                    Bounds {
                        origin: GpuiPoint::default(),
                        size: Size {
                            width: px(800.0),
                            height: px(height),
                        },
                    },
                );
                assert_eq!(
                    bounds.num_lines(),
                    n,
                    "num_lines() should be {n} for height={height}, line_height={line_height}"
                );
            }
        }
    }

    #[test]
    fn test_num_columns_float_precision() {
        let cell_widths = [8.1f32, 7.3, 9.7, 6.9, 10.1];
        for &cell_width in &cell_widths {
            for n in 1..=200 {
                let width = n as f32 * cell_width;
                let bounds = TerminalBounds::new(
                    px(20.0),
                    px(cell_width),
                    Bounds {
                        origin: GpuiPoint::default(),
                        size: Size {
                            width: px(width),
                            height: px(400.0),
                        },
                    },
                );
                assert_eq!(
                    bounds.num_columns(),
                    n,
                    "num_columns() should be {n} for width={width}, cell_width={cell_width}"
                );
            }
        }
    }
}
