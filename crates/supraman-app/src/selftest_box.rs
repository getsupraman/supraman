//! `--selftest` step 7t: a box-drawing table in a captured frame has unbroken strokes that meet.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context as _, anyhow, bail};
use supraman_term::Terminal;
use supraman_ui::term_element;
use gpui::{AsyncWindowContext, Entity, Pixels, px};

use crate::{
    selftest::{sleep, wait_for_tab_terminal},
    selftest_manage::{capture, shot},
    selftest_zoom::laid_out_with,
    view::FleetView,
};

const WITHIN: Duration = Duration::from_secs(15);
const SIZES: [Pixels; 2] = [term_element::FONT_SIZE, px(23.)];
/// `┌───┬───┐` / `│   │   │` / `├───┼───┤` / `└───┴───┘` in rgb(255, 0, 255).
const TABLE: &str = "$e=[char]27; $m=\"$e[38;2;255;0;255m\"; $h=([string][char]0x2500)*3; \
    Write-Host ($m+[char]0x250C+$h+[char]0x252C+$h+[char]0x2510); \
    Write-Host ($m+[char]0x2502+'   '+[char]0x2502+'   '+[char]0x2502); \
    Write-Host ($m+[char]0x251C+$h+[char]0x253C+$h+[char]0x2524); \
    Write-Host ($m+[char]0x2514+$h+[char]0x2534+$h+[char]0x2518+\"$e[0m\")\r";
const ROWS: usize = 4;
const COLUMNS: usize = 9;
const VERTICALS: [usize; 3] = [0, 4, 8];
const HORIZONTALS: [usize; 3] = [0, 2, 3];

pub(crate) async fn run(view: &Entity<FleetView>, home: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let saved = view.update(cx, |v, _| std::mem::replace(&mut v.shell_args, vec!["-NoLogo".into(), "-NoProfile".into()]));
    let result = check(view, home, cx).await;
    view.update(cx, |v, _| v.shell_args = saved);
    // whatever failed, the next run starts at the default
    view.update(cx, |v, cx| v.set_term_font_size(term_element::FONT_SIZE, cx));
    close_all(view, cx);
    result
}

async fn check(view: &Entity<FleetView>, home: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    close_all(view, cx);
    let tab = cx.update(|window, cx| view.update(cx, |v, cx| v.new_shell(window, cx)))?;
    let terminal = wait_for_tab_terminal(view, tab, WITHIN, cx).await?;
    terminal.update(cx, |t, _| t.input(TABLE.as_bytes().to_vec()));
    let mut shown = Vec::new();
    for size in SIZES {
        view.update(cx, |v, cx| v.set_term_font_size(size, cx));
        let font = view.read_with(cx, |v, _| v.term_font());
        laid_out_with(view, tab, &terminal, &font, cx).await?;
        let corner = printed(&terminal, cx).await?;
        view.update(cx, |_, cx| cx.notify());
        sleep(cx, Duration::from_millis(300)).await;
        let frame = Frame::capture(home.join(format!("box-{}.png", f32::from(size))), &terminal, corner, cx).await?;
        shown.push(frame.judge().with_context(|| format!("{} at {size:?}", font.family))?);
        shot(&format!("m5-58-box-drawing-{}", f32::from(size)), cx).await;
    }
    Ok(shown.join("; "))
}

/// The display line and column of the table's `┌`, once all four rows are on screen.
async fn printed(terminal: &Entity<Terminal>, cx: &mut AsyncWindowContext) -> anyhow::Result<(i32, usize)> {
    let started = Instant::now();
    loop {
        let found = terminal.read_with(cx, |t, _| {
            let content = t.last_content();
            let at = |ch| content.cells.iter().find(|c| c.cell.character() == ch).map(|c| (c.point.line + content.display_offset as i32, c.point.column));
            at('┌').zip(at('┘')).filter(|(top, bottom)| bottom.0 - top.0 == ROWS as i32 - 1).map(|(top, _)| top)
        });
        if let Some(corner) = found {
            return Ok(corner);
        }
        if started.elapsed() > WITHIN {
            bail!("the box-drawing table is not on screen within {} s", WITHIN.as_secs());
        }
        sleep(cx, Duration::from_millis(50)).await;
    }
}

/// The captured window and the table's cell edges in its pixels.
struct Frame {
    image: image::RgbaImage,
    xs: Vec<u32>,
    ys: Vec<u32>,
    scale: f32,
}

impl Frame {
    async fn capture(out: std::path::PathBuf, terminal: &Entity<Terminal>, (line, column): (i32, usize), cx: &mut AsyncWindowContext) -> anyhow::Result<Self> {
        let (scale, viewport) = cx.update(|window, _| (window.scale_factor(), window.viewport_size()))?;
        let dims = terminal.read_with(cx, |t, _| t.last_content().terminal_bounds);
        let out = capture(out, None, cx).await?;
        let image = image::open(&out).with_context(|| out.display().to_string())?.into_rgba8();
        // the window's frame: as wide below the client area as on either side
        let side = (image.width() as f32 - f32::from(viewport.width) * scale) / 2.;
        let top = image.height() as f32 - f32::from(viewport.height) * scale - side;
        let device = |v: Pixels, offset: f32| (f32::from(v) * scale + offset).round().max(0.) as u32;
        let xs = (0..=COLUMNS).map(|c| device(dims.bounds.origin.x + dims.cell_width * (column + c) as f32, side)).collect();
        let ys = (0..=ROWS).map(|r| device(dims.bounds.origin.y + dims.line_height * (line + r as i32) as f32, top)).collect();
        Ok(Self { image, xs, ys, scale })
    }

    fn inked(&self, x: u32, y: u32) -> bool {
        self.image.get_pixel_checked(x, y).is_some_and(|p| p[0] >= 160 && p[2] >= 160 && p[1] <= 100)
    }

    /// The pixel line in `range` along which the most of `along` is inked.
    fn stroke(&self, range: std::ops::Range<u32>, along: std::ops::Range<u32>, at: impl Fn(u32, u32) -> (u32, u32)) -> anyhow::Result<u32> {
        range
            .map(|i| (along.clone().filter(|&j| { let (x, y) = at(i, j); self.inked(x, y) }).count(), i))
            .max()
            .filter(|(count, _)| *count > 0)
            .map(|(_, i)| i)
            .ok_or_else(|| anyhow!("no ink"))
    }

    fn judge(&self) -> anyhow::Result<String> {
        let (left, right, top, bottom) = (self.xs[0], self.xs[COLUMNS], self.ys[0], self.ys[ROWS]);
        let mut rows = Vec::new();
        for r in HORIZONTALS {
            let y = self.stroke(self.ys[r]..self.ys[r + 1], left..right, |y, x| (x, y)).with_context(|| format!("row {r} has no horizontal stroke"))?;
            rows.push(y);
        }
        let mut columns = Vec::new();
        for c in VERTICALS {
            let x = self.stroke(self.xs[c]..self.xs[c + 1], top..bottom, |x, y| (x, y)).with_context(|| format!("cell {c} has no vertical stroke"))?;
            columns.push(x);
        }
        let (first_x, last_x) = (columns[0], columns[VERTICALS.len() - 1]);
        let (first_y, last_y) = (rows[0], rows[HORIZONTALS.len() - 1]);
        for (&r, &y) in HORIZONTALS.iter().zip(&rows) {
            if let Some(gap) = (first_x..=last_x).find(|&x| !self.inked(x, y)) {
                bail!("row {r}: the horizontal stroke at y {y} has a gap at x {gap} (cell {}), from {first_x} to {last_x}", self.cell(gap));
            }
        }
        for (&c, &x) in VERTICALS.iter().zip(&columns) {
            if let Some(gap) = (first_y..=last_y).find(|&y| !self.inked(x, y)) {
                bail!("cell {c}: the vertical stroke at x {x} does not reach the horizontal ones, a gap at y {gap} between y {first_y} and {last_y}");
            }
        }
        let cell = (self.xs[1] - self.xs[0], self.ys[1] - self.ys[0]);
        Ok(format!(
            "scale {}, cells {}×{} px: horizontals at y {rows:?} unbroken over x {first_x}..={last_x}, verticals at x {columns:?} unbroken over y {first_y}..={last_y}",
            self.scale, cell.0, cell.1
        ))
    }

    fn cell(&self, x: u32) -> usize {
        self.xs.partition_point(|&edge| edge <= x).saturating_sub(1)
    }
}

fn close_all(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) {
    let ids: Vec<u64> = view.read_with(cx, |v, _| v.tabs.iter().map(|t| t.id).collect());
    view.update(cx, |v, cx| ids.into_iter().for_each(|id| v.close_tab(id, cx)));
}
