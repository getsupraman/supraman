//! A table of columns over rows that lays out only the rows in view ([`gpui::uniform_list`]): a header whose
//! dividers drag to resize, one selected row moved by Up/Down, and a call when the view nears the last row.
//! The owner keeps the column widths and the rows; the table keeps the selection and the scroll.

use std::{cell::RefCell, ops::Range, rc::Rc};

use gpui::{
    AnyElement, App, ClickEvent, Context, Div, DragMoveEvent, FocusHandle, Hsla, KeyDownEvent, Pixels, ScrollStrategy, SharedString, Stateful,
    UniformListScrollHandle, Window, canvas, div, prelude::*, px, uniform_list,
};
use theme::ActiveTheme as _;

use crate::clipped::clipped;
use crate::hit::{HitTargets, hit_target};
use crate::split::{DraggedDivider, handle};

#[derive(Clone, Debug)]
pub struct Column {
    pub id: SharedString,
    pub caption: SharedString,
    pub min: Pixels,
    /// What a double-click on the column's divider restores.
    pub default: Pixels,
    /// Takes the width the other columns leave; without one, the last column does.
    pub flexible: bool,
}

impl Column {
    pub fn new(id: impl Into<SharedString>, caption: impl Into<SharedString>, min: Pixels, default: Pixels) -> Self {
        Self { id: id.into(), caption: caption.into(), min, default, flexible: false }
    }

    pub fn flexible(mut self) -> Self {
        self.flexible = true;
        self
    }
}

/// The hover group of every row of table `id`, for a cell's `group_hover`.
pub fn row_group(id: &str) -> SharedString {
    SharedString::from(format!("{id}-row"))
}

fn flexible(columns: &[Column]) -> usize {
    columns.iter().position(|c| c.flexible).unwrap_or(columns.len().saturating_sub(1))
}

/// The column the divider right of column `divider` resizes: the one on its side away from the flexible one.
fn resized_by(columns: &[Column], divider: usize) -> usize {
    if divider < flexible(columns) { divider } else { divider + 1 }
}

/// Each column's width in a table `total` wide: the owner's `widths` held at their mins, the flexible column
/// taking the rest; when that leaves it under its min, the columns after it, then those before it, give way
/// down to their mins. Only when every column is at its min does the sum pass `total`.
pub fn fit(columns: &[Column], widths: &[Pixels], total: Pixels) -> Vec<Pixels> {
    let flex = flexible(columns);
    let mut fitted: Vec<Pixels> = columns
        .iter()
        .enumerate()
        .map(|(i, c)| if i == flex { c.min } else { widths.get(i).copied().unwrap_or(c.default).max(c.min) })
        .collect();
    let others = |w: &[Pixels]| w.iter().enumerate().filter(|(i, _)| *i != flex).fold(px(0.), |sum, (_, w)| sum + *w);
    let mut excess = others(&fitted) + columns.get(flex).map_or(px(0.), |c| c.min) - total;
    let order = (flex + 1..columns.len()).rev().chain((0..flex).rev());
    for i in order {
        if excess <= px(0.) {
            break;
        }
        let give = (fitted[i] - columns[i].min).min(excess);
        fitted[i] -= give;
        excess -= give;
    }
    if let Some(c) = columns.get(flex) {
        fitted[flex] = (total - others(&fitted)).max(c.min);
    }
    fitted
}

/// The widths after the divider right of column `divider` is dragged to `x` (from the table's left edge):
/// a divider left of the flexible column resizes the column on its left, one right of it the column on its
/// right, and the flexible column takes up the difference, neither going under its min.
pub fn drag(columns: &[Column], widths: &[Pixels], total: Pixels, divider: usize, x: Pixels) -> Vec<Pixels> {
    let flex = flexible(columns);
    let mut fitted = fit(columns, widths, total);
    if divider + 1 >= columns.len() {
        return fitted;
    }
    let left = |i: usize| fitted[..i].iter().fold(px(0.), |sum, w| sum + *w);
    let target = resized_by(columns, divider);
    let wanted = if target == divider { x - left(divider) } else { left(divider + 2) - x };
    let slack = (fitted[flex] - columns[flex].min).max(px(0.));
    let max = (fitted[target] + slack).max(columns[target].min);
    let width = wanted.clamp(columns[target].min, max);
    let freed = fitted[target] - width;
    fitted[flex] += freed;
    fitted[target] = width;
    fitted
}

/// Whether the view has come near the end: once per approach, and again each time more rows arrive.
#[derive(Debug, Default)]
pub struct NearEnd {
    armed: bool,
    rows: usize,
}

impl NearEnd {
    /// True when the view, ending at row `end` of `rows`, comes within `within` rows of the last one for the
    /// first time since it was last further away or the row count last changed.
    pub fn check(&mut self, end: usize, rows: usize, within: usize) -> bool {
        if rows != self.rows {
            self.rows = rows;
            self.armed = true;
        }
        if rows == 0 || end + within < rows {
            self.armed = true;
            return false;
        }
        std::mem::take(&mut self.armed)
    }
}

#[derive(Default)]
struct Inner {
    selected: Option<usize>,
    rows: usize,
    width: Pixels,
    near_end: NearEnd,
}

/// What the owner keeps of a table between frames.
#[derive(Clone)]
pub struct TableState {
    inner: Rc<RefCell<Inner>>,
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
}

impl TableState {
    pub fn new(cx: &mut App) -> Self {
        Self { inner: Rc::default(), scroll: UniformListScrollHandle::new(), focus: cx.focus_handle() }
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }

    pub fn selected(&self) -> Option<usize> {
        self.inner.borrow().selected
    }

    /// Selects `row` (none with `None`) without scrolling to it.
    pub fn select(&self, row: Option<usize>) {
        self.inner.borrow_mut().selected = row;
    }

    /// Scrolls `row` into view, the least distance.
    pub fn reveal(&self, row: usize) {
        self.scroll.scroll_to_item(row, ScrollStrategy::Nearest);
    }

    /// Moves the selection `by` rows within the last frame's rows, reveals it, and returns it; the first row
    /// when none was selected.
    pub fn step(&self, by: isize) -> Option<usize> {
        let row = {
            let mut inner = self.inner.borrow_mut();
            let last = inner.rows.checked_sub(1)?;
            let row = match inner.selected {
                Some(row) => row.saturating_add_signed(by).min(last),
                None => 0,
            };
            inner.selected = Some(row);
            row
        };
        self.reveal(row);
        Some(row)
    }
}

type CellFn<V> = dyn Fn(&mut V, usize, usize, &mut Window, &mut Context<V>) -> AnyElement;
type BandFn<V> = dyn Fn(&V, usize, &App) -> Option<Hsla>;
type SelectFn<V> = dyn Fn(&mut V, usize, &mut Window, &mut Context<V>);
type ResizeFn<V> = dyn Fn(&mut V, Vec<Pixels>, bool, &mut Context<V>);
type NearEndFn<V> = dyn Fn(&mut V, &mut Window, &mut Context<V>);

/// One frame's table, built in the owner's render. Its element ids and hit keys start with `id`:
/// `<id>-col-<column>`, `<id>-divider-<column>` (right of that column) and `<id>-row-<index>`.
pub struct Table<V> {
    id: SharedString,
    state: TableState,
    hits: HitTargets,
    columns: Rc<[Column]>,
    widths: Vec<Pixels>,
    rows: usize,
    row_height: Pixels,
    cell: Rc<CellFn<V>>,
    band: Option<Rc<BandFn<V>>>,
    on_select: Option<Rc<SelectFn<V>>>,
    on_resize: Option<Rc<ResizeFn<V>>>,
    near_end: Option<(Pixels, Rc<NearEndFn<V>>)>,
}

impl<V: 'static> Table<V> {
    /// `widths` are the owner's, one per column; `cell` paints row × column.
    pub fn new(
        id: impl Into<SharedString>,
        state: &TableState,
        hits: &HitTargets,
        columns: impl Into<Rc<[Column]>>,
        widths: &[Pixels],
        rows: usize,
        cell: impl Fn(&mut V, usize, usize, &mut Window, &mut Context<V>) -> AnyElement + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            state: state.clone(),
            hits: hits.clone(),
            columns: columns.into(),
            widths: widths.to_vec(),
            rows,
            row_height: px(28.),
            cell: Rc::new(cell),
            band: None,
            on_select: None,
            on_resize: None,
            near_end: None,
        }
    }

    pub fn row_height(mut self, height: Pixels) -> Self {
        self.row_height = height;
        self
    }

    /// A row's own background under the hover and the selection.
    pub fn band(mut self, band: impl Fn(&V, usize, &App) -> Option<Hsla> + 'static) -> Self {
        self.band = Some(Rc::new(band));
        self
    }

    /// Called after a click or Up/Down selects a row.
    pub fn on_select(mut self, f: impl Fn(&mut V, usize, &mut Window, &mut Context<V>) + 'static) -> Self {
        self.on_select = Some(Rc::new(f));
        self
    }

    /// Gets every column's new width while a divider is dragged (`false`), and when it is let go or
    /// double-clicked (`true`): the moment to persist them.
    pub fn on_resize(mut self, f: impl Fn(&mut V, Vec<Pixels>, bool, &mut Context<V>) + 'static) -> Self {
        self.on_resize = Some(Rc::new(f));
        self
    }

    /// Called, deferred, when the rows in view come within `within` of the last row ([`NearEnd`]).
    pub fn on_near_end(mut self, within: Pixels, f: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static) -> Self {
        self.near_end = Some((within, Rc::new(f)));
        self
    }

    pub fn render(self, cx: &mut Context<V>) -> Stateful<Div> {
        let total = {
            let mut inner = self.state.inner.borrow_mut();
            inner.rows = self.rows;
            inner.width
        };
        let total = if total > px(0.) { total } else { self.widths.iter().fold(px(0.), |sum, w| sum + *w) };
        let widths: Rc<[Pixels]> = fit(&self.columns, &self.widths, total).into();
        let (state, on_select) = (self.state.clone(), self.on_select.clone());
        let keys = cx.listener(move |this: &mut V, e: &KeyDownEvent, window, cx| {
            let by = match e.keystroke.key.as_str() {
                "up" => -1,
                "down" => 1,
                _ => return,
            };
            cx.stop_propagation();
            if let (Some(row), Some(f)) = (state.step(by), &on_select) {
                f(this, row, window, cx);
            }
            cx.notify();
        });
        let header = self.header(&widths, cx);
        div()
            .id(self.id.clone())
            .track_focus(&self.state.focus)
            .on_key_down(keys)
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .overflow_hidden()
            .child(header)
            .child(self.body(widths, cx))
    }

    fn header(&self, widths: &[Pixels], cx: &mut Context<V>) -> Stateful<Div> {
        let colors = cx.theme().colors();
        let (border, background, text) = (colors.border_variant, colors.panel_background, colors.text_muted);
        let last = self.columns.len().saturating_sub(1);
        let mut cells = Vec::with_capacity(self.columns.len());
        for (i, (column, width)) in self.columns.iter().zip(widths).enumerate() {
            let key = format!("{}-col-{}", self.id, column.id);
            let caption = clipped(format!("{key}-caption"), column.caption.to_uppercase(), false);
            let divider = (i < last).then(|| self.divider(i, column, cx));
            cells.push(
                div()
                    .relative()
                    .flex()
                    .flex_none()
                    .items_center()
                    .w(*width)
                    .h_full()
                    .px_2()
                    .when(i < last, |d| d.border_r_1().border_color(border))
                    .child(hit_target(&self.hits, key))
                    .child(caption)
                    .children(divider),
            );
        }
        div()
            .id(SharedString::from(format!("{}-header", self.id)))
            .relative()
            .flex()
            .flex_none()
            .w_full()
            .h(px(24.))
            .bg(background)
            .border_b_1()
            .border_color(border)
            .text_xs()
            .text_color(text)
            .on_drag_move(self.follow_drag(cx))
            .child(width_probe(self.state.clone()))
            .children(cells)
    }

    fn follow_drag(&self, cx: &mut Context<V>) -> impl Fn(&DragMoveEvent<DraggedDivider>, &mut Window, &mut App) + 'static {
        let prefix = format!("{}-divider-", self.id);
        let (state, columns, owner, on_resize) = (self.state.clone(), self.columns.clone(), self.widths.clone(), self.on_resize.clone());
        cx.listener(move |this: &mut V, e: &DragMoveEvent<DraggedDivider>, _, cx| {
            let dragged = e.drag(cx);
            if !dragged.0.starts_with(&prefix) {
                return;
            }
            let widths = drag(&columns, &owner, e.bounds.size.width, dragged.1, e.event.position.x - e.bounds.left());
            state.inner.borrow_mut().width = e.bounds.size.width;
            if let Some(f) = &on_resize {
                f(this, widths, false, cx);
            }
            cx.notify();
        })
    }

    fn divider(&self, i: usize, column: &Column, cx: &mut Context<V>) -> Stateful<Div> {
        let key = format!("{}-divider-{}", self.id, column.id);
        let (end, reset) = (self.on_resize.clone(), self.on_resize.clone());
        let (columns, widths, state) = (self.columns.clone(), self.widths.clone(), self.state.clone());
        let ended = widths.clone();
        handle(
            key.clone().into(),
            i,
            move |this, cx| {
                if let Some(f) = &end {
                    f(this, ended.clone(), true, cx);
                }
            },
            move |this, cx| {
                let mut restored = widths.clone();
                let target = resized_by(&columns, i);
                restored[target] = columns[target].default;
                let total = state.inner.borrow().width;
                if let Some(f) = &reset {
                    f(this, fit(&columns, &restored, total), true, cx);
                }
                cx.notify();
            },
            cx,
        )
        .right(px(-4.))
        .child(hit_target(&self.hits, key))
    }

    fn body(self, widths: Rc<[Pixels]>, cx: &mut Context<V>) -> impl IntoElement {
        let scroll = self.state.scroll.clone();
        let Self { id, state, hits, rows, row_height, cell, band, on_select, near_end, .. } = self;
        let list_id = SharedString::from(format!("{id}-rows"));
        let group = row_group(&id);
        let paint = cx.processor(move |this: &mut V, range: Range<usize>, window, cx| {
            if let (Some((within, f)), Some(end)) = (&near_end, visible_end(&state.scroll, row_height)) {
                let within = (*within / row_height).ceil() as usize;
                if state.inner.borrow_mut().near_end.check(end, rows, within) {
                    let f = f.clone();
                    cx.defer_in(window, move |this, window, cx| f(this, window, cx));
                }
            }
            let selected = state.selected();
            let colors = cx.theme().colors();
            let (selection, hover) = (colors.element_selected, colors.ghost_element_hover);
            let mut out = Vec::with_capacity(range.len());
            for ix in range {
                let key = format!("{id}-row-{ix}");
                let background = band.as_ref().and_then(|b| b(this, ix, cx));
                let cells: Vec<AnyElement> = widths
                    .iter()
                    .enumerate()
                    .map(|(col, width)| {
                        let content = cell(this, ix, col, window, cx);
                        div().flex().flex_none().items_center().w(*width).h_full().px_2().overflow_hidden().child(content).into_any_element()
                    })
                    .collect();
                let (state, on_select) = (state.clone(), on_select.clone());
                out.push(
                    div()
                        .id(SharedString::from(key.clone()))
                        .group(group.clone())
                        .relative()
                        .flex()
                        .w_full()
                        .h(row_height)
                        .when_some(background, |d, bg| d.bg(bg))
                        .map(|d| if selected == Some(ix) { d.bg(selection) } else { d.hover(|s| s.bg(hover)) })
                        .child(hit_target(&hits, key))
                        .children(cells)
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            state.select(Some(ix));
                            window.focus(&state.focus, cx);
                            if let Some(f) = &on_select {
                                f(this, ix, window, cx);
                            }
                            cx.notify();
                        })),
                );
            }
            out
        });
        uniform_list(list_id, rows, paint).track_scroll(&scroll).flex_1().w_full()
    }
}

/// The row the view ends at, from the scroll as of this frame: the row painter is also called with
/// `0..1` to measure a row, so its range is not the view.
fn visible_end(scroll: &UniformListScrollHandle, row_height: Pixels) -> Option<usize> {
    let state = scroll.0.borrow();
    let viewport = state.last_item_size?.item.height;
    let top = -state.base_handle.offset().y;
    Some(((top + viewport) / row_height).ceil() as usize)
}

/// Records the header's width for the next frame's [`fit`], and asks for that frame when it changed.
fn width_probe(state: TableState) -> impl IntoElement {
    canvas(
        move |bounds, window, _| {
            let mut inner = state.inner.borrow_mut();
            if inner.width != bounds.size.width {
                inner.width = bounds.size.width;
                window.refresh();
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

#[cfg(test)]
mod tests {
    use gpui::{Entity, TestAppContext, VisualTestContext};

    use super::*;

    fn columns() -> Vec<Column> {
        vec![
            Column::new("branch", "Branch / Tag", px(60.), px(160.)),
            Column::new("graph", "Graph", px(40.), px(120.)),
            Column::new("message", "Commit Message", px(120.), px(300.)).flexible(),
            Column::new("date", "Commit Date / Time", px(80.), px(150.)),
            Column::new("sha", "Sha", px(50.), px(70.)),
        ]
    }

    fn sum(widths: &[Pixels]) -> Pixels {
        widths.iter().fold(px(0.), |s, w| s + *w)
    }

    fn within_mins(columns: &[Column], widths: &[Pixels]) -> bool {
        columns.iter().zip(widths).all(|(c, w)| *w >= c.min)
    }

    #[test]
    fn a_divider_drag_keeps_every_column_at_its_min_and_fills_the_width() {
        let columns = columns();
        let owner = vec![px(160.), px(120.), px(0.), px(150.), px(70.)];
        let total = px(900.);
        let fitted = fit(&columns, &owner, total);
        assert_eq!(fitted, vec![px(160.), px(120.), px(400.), px(150.), px(70.)]);
        for divider in 0..columns.len() - 1 {
            for x in (-200..1200).step_by(37) {
                let widths = drag(&columns, &owner, total, divider, px(x as f32));
                assert!(within_mins(&columns, &widths), "divider {divider} to {x}: {widths:?}");
                assert_eq!(sum(&widths), total, "divider {divider} to {x}: {widths:?}");
            }
        }
        let widths = drag(&columns, &owner, total, 0, px(200.));
        assert_eq!(widths[..3], [px(200.), px(120.), px(360.)], "left of the flexible column: the column on the left");
        let widths = drag(&columns, &owner, total, 3, px(800.));
        assert_eq!(widths[2..], [px(370.), px(150.), px(100.)], "right of it: the column on the right");
        let widths = drag(&columns, &owner, total, 1, px(5000.));
        assert_eq!(widths[2], px(120.), "the flexible column stops at its min");
    }

    #[test]
    fn a_narrow_table_takes_the_width_from_the_columns_after_the_flexible_one_first() {
        let columns = columns();
        let owner = vec![px(160.), px(120.), px(0.), px(150.), px(70.)];
        let widths = fit(&columns, &owner, px(500.));
        assert_eq!(widths, vec![px(160.), px(90.), px(120.), px(80.), px(50.)]);
        assert_eq!(sum(&widths), px(500.));
        let widths = fit(&columns, &owner, px(200.));
        assert_eq!(widths, columns.iter().map(|c| c.min).collect::<Vec<_>>(), "at the mins the table clips");
        let widths = fit(&columns, &[px(1.), px(1.), px(1.), px(1.), px(1.)], px(900.));
        assert!(within_mins(&columns, &widths) && sum(&widths) == px(900.));
    }

    #[test]
    fn the_near_end_call_comes_once_per_approach_and_again_when_rows_arrive() {
        let mut near = NearEnd::default();
        assert!(!near.check(10, 1000, 50));
        assert!(near.check(950, 1000, 50));
        for top in 941..990 {
            assert!(!near.check(top + 10, 1000, 50), "scrolled on to {top}");
        }
        assert!(near.check(1000, 1020, 50), "a page arrived and the end is still near");
        assert!(!near.check(1000, 1020, 50));
        assert!(!near.check(510, 1020, 50), "scrolled away");
        assert!(near.check(1010, 1020, 50), "the next approach");
        assert!(!near.check(0, 0, 50), "an empty table loads its first page by itself");
    }

    struct Grid {
        state: TableState,
        hits: HitTargets,
        rows: usize,
        painted: Vec<usize>,
        selected: Vec<usize>,
        near: usize,
        widths: Vec<Pixels>,
        resized: Vec<bool>,
    }

    impl Render for Grid {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let table = Table::new("t", &self.state, &self.hits, columns(), &self.widths, self.rows, |this: &mut Grid, row, col, _, _| {
                if col == 0 {
                    this.painted.push(row);
                }
                div().child(format!("{row}")).into_any_element()
            })
            .on_select(|this, row, _, _| this.selected.push(row))
            .on_near_end(px(280.), |this, _, _| this.near += 1)
            .on_resize(|this, widths, done, cx| {
                this.widths = widths;
                this.resized.push(done);
                cx.notify();
            });
            div().w(px(900.)).h(px(24. + 280.)).child(table.render(cx))
        }
    }

    fn grid(rows: usize, cx: &mut TestAppContext) -> (Entity<Grid>, &mut VisualTestContext) {
        cx.update(|cx| crate::init_globals(cx, "test"));
        let (grid, cx) = cx.add_window_view(|window, cx| {
            let state = TableState::new(cx);
            window.focus(state.focus_handle(), cx);
            let widths = vec![px(160.), px(120.), px(0.), px(150.), px(70.)];
            Grid { state, hits: HitTargets::default(), rows, painted: Vec::new(), selected: Vec::new(), near: 0, widths, resized: Vec::new() }
        });
        cx.run_until_parked();
        (grid, cx)
    }

    fn redraw(grid: &Entity<Grid>, cx: &mut VisualTestContext) -> Vec<usize> {
        grid.update(cx, |g, cx| {
            g.painted.clear();
            cx.notify();
        });
        cx.run_until_parked();
        cx.update(|_, _| ());
        grid.read_with(cx, |g, _| g.painted.clone())
    }

    #[gpui::test]
    fn only_the_rows_in_view_are_laid_out(cx: &mut TestAppContext) {
        let (grid, cx) = grid(10_000, cx);
        let painted = redraw(&grid, cx);
        assert!(!painted.is_empty() && painted.len() <= 12, "{} rows painted: {painted:?}", painted.len());
        assert!(painted.iter().all(|r| *r < 12));
        grid.update(cx, |g, _| g.rows = 20_000);
        let painted = redraw(&grid, cx);
        assert!(painted.len() <= 12, "a longer table paints no more rows");
        grid.read_with(cx, |g, _| assert!(g.hits.borrow().contains_key("t-row-0") && !g.hits.borrow().contains_key("t-row-500")));
    }

    #[gpui::test]
    fn up_down_and_reveal_move_and_show_the_selection(cx: &mut TestAppContext) {
        let (grid, cx) = grid(10_000, cx);
        cx.simulate_keystrokes("down down down up");
        grid.read_with(cx, |g, _| {
            assert_eq!(g.state.selected(), Some(1), "the first Down selects the first row");
            assert_eq!(g.selected, vec![0, 1, 2, 1]);
        });
        grid.read_with(cx, |g, _| g.state.reveal(5_000));
        let painted = redraw(&grid, cx);
        assert!(painted.contains(&5_000), "{painted:?}");
        grid.read_with(cx, |g, _| g.state.select(Some(5_000)));
        cx.simulate_keystrokes("down");
        let painted = redraw(&grid, cx);
        grid.read_with(cx, |g, _| assert_eq!(g.state.selected(), Some(5_001)));
        assert!(painted.contains(&5_001));
        grid.read_with(cx, |g, _| g.state.reveal(9_999));
        redraw(&grid, cx);
        cx.simulate_keystrokes("down");
        grid.read_with(cx, |g, _| assert_eq!(g.state.selected(), Some(5_002)));
    }

    #[gpui::test]
    fn the_near_end_call_fires_once_as_the_view_reaches_the_end(cx: &mut TestAppContext) {
        let (grid, cx) = grid(1_000, cx);
        assert_eq!(grid.read_with(cx, |g, _| g.near), 0);
        grid.read_with(cx, |g, _| g.state.reveal(985));
        redraw(&grid, cx);
        assert_eq!(grid.read_with(cx, |g, _| g.near), 0, "14 rows from the end is not near");
        grid.read_with(cx, |g, _| g.state.reveal(995));
        redraw(&grid, cx);
        assert_eq!(grid.read_with(cx, |g, _| g.near), 1);
        grid.read_with(cx, |g, _| g.state.reveal(999));
        redraw(&grid, cx);
        redraw(&grid, cx);
        assert_eq!(grid.read_with(cx, |g, _| g.near), 1, "once per approach");
        grid.update(cx, |g, _| g.rows = 1_005);
        redraw(&grid, cx);
        assert_eq!(grid.read_with(cx, |g, _| g.near), 2, "rows arrived and the end is still near");
    }

    #[gpui::test]
    fn dragging_a_divider_reports_the_widths_and_its_end(cx: &mut TestAppContext) {
        use gpui::{Modifiers, MouseButton, point};
        let (grid, cx) = grid(100, cx);
        redraw(&grid, cx);
        let hit = grid.read_with(cx, |g, _| g.hits.borrow().get("t-divider-branch").copied()).expect("the divider paints under its key");
        let start = hit.bounds.center();
        cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(start + point(px(20.), px(0.)), MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(start + point(px(40.), px(0.)), MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(start + point(px(40.), px(0.)), MouseButton::Left, Modifiers::none());
        grid.read_with(cx, |g, _| {
            assert_eq!(g.resized.last(), Some(&true), "{:?}", g.resized);
            assert!(g.resized.contains(&false));
            assert!(g.widths[0] > px(196.) && g.widths[0] < px(204.), "follows the pointer 40 px on: {:?}", g.widths);
            assert_eq!(sum(&g.widths), px(900.));
        });
    }
}
