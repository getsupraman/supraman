//! A table of columns over rows that lays out only the rows in view; the owner keeps the widths and rows, the table the selection and scroll.

use std::{cell::RefCell, ops::Range, rc::Rc};

use gpui::{
    AnyElement, App, ClickEvent, Context, Div, DragMoveEvent, FocusHandle, Hsla, KeyBinding, KeyDownEvent, Pixels, ScrollStrategy, SharedString,
    Stateful, UniformListScrollHandle, Window, canvas, div, prelude::*, px, rems, uniform_list,
};
use theme::ActiveTheme as _;

use crate::clipped::clipped;
use crate::hit::{HitTargets, hit_target};
use crate::icon::Icon;
use crate::split::{DraggedDivider, handle};
use crate::tooltip::Tooltip;

mod widths;

pub use widths::{clamp, drag, fit};

gpui::actions!(table, [SelectPrevious, SelectNext, SelectPageUp, SelectPageDown, SelectFirst, SelectLast]);

pub const KEY_CONTEXT: &str = "Table";

/// A cell's padding left and right, in rems.
pub const CELL_PAD: f32 = 0.5;

pub fn bind_keys(cx: &mut App) {
    let c = Some(KEY_CONTEXT);
    cx.bind_keys([
        KeyBinding::new("up", SelectPrevious, c),
        KeyBinding::new("k", SelectPrevious, c),
        KeyBinding::new("down", SelectNext, c),
        KeyBinding::new("j", SelectNext, c),
        KeyBinding::new("pageup", SelectPageUp, c),
        KeyBinding::new("pagedown", SelectPageDown, c),
        KeyBinding::new("ctrl-up", SelectFirst, c),
        KeyBinding::new("ctrl-home", SelectFirst, c),
        KeyBinding::new("ctrl-down", SelectLast, c),
        KeyBinding::new("ctrl-end", SelectLast, c),
    ]);
}

/// Where a key moves the selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    By(isize),
    /// A page of the rows in view, down when positive.
    Pages(isize),
    First,
    Last,
}

#[derive(Clone, Debug)]
pub struct Column {
    pub id: SharedString,
    pub caption: SharedString,
    pub min: Pixels,
    /// The preferred width without the owner's, and what a double-click on the column's divider restores.
    pub default: Pixels,
    /// Ignored on the last column, which takes what the others leave.
    pub max: Option<Pixels>,
    /// The icon the header shows in place of the caption at this width and narrower.
    pub icon: Option<(Icon, Pixels)>,
}

impl Column {
    pub fn new(id: impl Into<SharedString>, caption: impl Into<SharedString>, min: Pixels, default: Pixels) -> Self {
        Self { id: id.into(), caption: caption.into(), min, default, max: None, icon: None }
    }

    pub fn max(mut self, max: Pixels) -> Self {
        self.max = Some(max);
        self
    }

    pub fn icon(mut self, icon: Icon, at: Pixels) -> Self {
        self.icon = Some((icon, at));
        self
    }

    /// The icon the header shows in place of the caption when the column is `width` wide.
    pub fn header_icon(&self, width: Pixels) -> Option<Icon> {
        self.icon.filter(|(_, at)| width <= *at).map(|(icon, _)| icon)
    }
}

/// The hover group of every row of table `id`, for a cell's `group_hover`.
pub fn row_group(id: &str) -> SharedString {
    SharedString::from(format!("{id}-row"))
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
    /// The rows a key can select: the last frame's, without its footer.
    rows: usize,
    row_height: Pixels,
    width: Pixels,
    /// The columns' widths as last laid out; the owner keeps the preferred ones.
    current: Vec<Pixels>,
    near_end: NearEnd,
    shown_selection: Option<Hsla>,
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

    /// The fill the last frame painted the selected row with; `None` while it was not in view.
    pub fn selection_shown(&self) -> Option<Hsla> {
        self.inner.borrow().shown_selection
    }

    /// Selects `row` (none with `None`) without scrolling to it.
    pub fn select(&self, row: Option<usize>) {
        self.inner.borrow_mut().selected = row;
    }

    /// Scrolls `row` into view, the least distance.
    pub fn reveal(&self, row: usize) {
        self.scroll.scroll_to_item(row, ScrollStrategy::Nearest);
    }

    /// How far the rows are scrolled down.
    pub fn scroll_top(&self) -> Pixels {
        -self.scroll.0.borrow().base_handle.offset().y
    }

    /// Scrolls the rows `top` down, as a frame lays them out: past the last row is cut back then.
    pub fn set_scroll_top(&self, top: Pixels) {
        let mut state = self.scroll.0.borrow_mut();
        state.deferred_scroll_to_item = None;
        let x = state.base_handle.offset().x;
        state.base_handle.set_offset(gpui::point(x, -top.max(px(0.))));
    }

    /// The height the rows are shown in, as the last frame laid them out.
    pub fn viewport(&self) -> Option<Pixels> {
        self.scroll.0.borrow().last_item_size.map(|s| s.item.height)
    }

    /// The rows a page holds, as the last frame showed them; at least one.
    pub fn page(&self) -> usize {
        let height = self.inner.borrow().row_height;
        match self.viewport() {
            Some(viewport) if height > px(0.) => ((viewport / height).floor() as usize).max(1),
            _ => 1,
        }
    }

    /// Moves the selection to where `step` says within the last frame's rows, reveals it, and returns it
    /// when it moved; a relative step selects the first row when none was selected.
    pub fn go(&self, step: Step) -> Option<usize> {
        let page = self.page() as isize;
        let (row, moved) = {
            let mut inner = self.inner.borrow_mut();
            let last = inner.rows.checked_sub(1)?;
            let row = match (step, inner.selected) {
                (Step::First, _) => 0,
                (Step::Last, _) => last,
                (_, None) => 0,
                (Step::By(by), Some(row)) => row.saturating_add_signed(by).min(last),
                (Step::Pages(n), Some(row)) => row.saturating_add_signed(n * page).min(last),
            };
            let moved = inner.selected != Some(row);
            inner.selected = Some(row);
            (row, moved)
        };
        self.reveal(row);
        moved.then_some(row)
    }
}

type CellFn<V> = dyn Fn(&mut V, usize, usize, &mut Window, &mut Context<V>) -> AnyElement;
type BandFn<V> = dyn Fn(&V, usize, &App) -> Option<Hsla>;
type SelectFn<V> = dyn Fn(&mut V, usize, &mut Window, &mut Context<V>);
type ResizeFn<V> = dyn Fn(&mut V, Vec<Pixels>, &mut Context<V>);
type NearEndFn<V> = dyn Fn(&mut V, &mut Window, &mut Context<V>);
type KeyFn<V> = dyn Fn(&mut V, &str, &mut Window, &mut Context<V>) -> bool;
type RowKeyFn = dyn Fn(usize) -> String;
type HeaderMenuFn<V> = dyn Fn(&mut V, gpui::Point<Pixels>, &mut Window, &mut Context<V>);

/// One frame's table, built in the owner's render. Its element ids and hit keys start with `id`:
/// `<id>-col-<column>`, `<id>-divider-<column>` (right of that column), `<id>-row-<index>` and its fill `<id>-fill-<index>`.
pub struct Table<V> {
    id: SharedString,
    state: TableState,
    hits: HitTargets,
    columns: Rc<[Column]>,
    widths: Vec<Pixels>,
    rows: usize,
    row_height: Pixels,
    inset: Pixels,
    cell: Rc<CellFn<V>>,
    band: Option<Rc<BandFn<V>>>,
    on_select: Option<Rc<SelectFn<V>>>,
    on_resize: Option<Rc<ResizeFn<V>>>,
    near_end: Option<(Pixels, Rc<NearEndFn<V>>)>,
    on_click: Option<Rc<SelectFn<V>>>,
    on_key: Option<Rc<KeyFn<V>>>,
    row_key: Option<Rc<RowKeyFn>>,
    header_menu: Option<Rc<HeaderMenuFn<V>>>,
    headless: bool,
    footer: usize,
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
            inset: px(0.),
            cell: Rc::new(cell),
            band: None,
            on_select: None,
            on_resize: None,
            near_end: None,
            on_click: None,
            on_key: None,
            row_key: None,
            header_menu: None,
            headless: false,
            footer: 0,
        }
    }

    /// No header row.
    pub fn headless(mut self) -> Self {
        self.headless = true;
        self
    }

    /// The last `rows` rows are a footer no key selects.
    pub fn footer(mut self, rows: usize) -> Self {
        self.footer = rows;
        self
    }

    /// The part of a row's element id and hit key after `<id>-row-`; the row's index without one.
    pub fn row_key(mut self, key: impl Fn(usize) -> String + 'static) -> Self {
        self.row_key = Some(Rc::new(key));
        self
    }

    /// Called after a click selects a row, following [`Table::on_select`].
    pub fn on_click(mut self, f: impl Fn(&mut V, usize, &mut Window, &mut Context<V>) + 'static) -> Self {
        self.on_click = Some(Rc::new(f));
        self
    }

    /// Gets every key pressed while the table has focus that no binding of [`KEY_CONTEXT`] took; `true` when it took the key.
    pub fn on_key(mut self, f: impl Fn(&mut V, &str, &mut Window, &mut Context<V>) -> bool + 'static) -> Self {
        self.on_key = Some(Rc::new(f));
        self
    }

    /// Called on a right-click in the header, where it was.
    pub fn on_header_menu(mut self, f: impl Fn(&mut V, gpui::Point<Pixels>, &mut Window, &mut Context<V>) + 'static) -> Self {
        self.header_menu = Some(Rc::new(f));
        self
    }

    pub fn row_height(mut self, height: Pixels) -> Self {
        self.row_height = height;
        self
    }

    /// How far a row's band, hover and selection fills stop short of its top and bottom.
    pub fn inset(mut self, inset: Pixels) -> Self {
        self.inset = inset;
        self
    }

    /// A row's own background under the hover and the selection.
    pub fn band(mut self, band: impl Fn(&V, usize, &App) -> Option<Hsla> + 'static) -> Self {
        self.band = Some(Rc::new(band));
        self
    }

    /// Called after a click or a key selects a row.
    pub fn on_select(mut self, f: impl Fn(&mut V, usize, &mut Window, &mut Context<V>) + 'static) -> Self {
        self.on_select = Some(Rc::new(f));
        self
    }

    /// Every column's preferred width when a divider is let go or double-clicked: the moment to keep and persist them.
    pub fn on_resize(mut self, f: impl Fn(&mut V, Vec<Pixels>, &mut Context<V>) + 'static) -> Self {
        self.on_resize = Some(Rc::new(f));
        self
    }

    /// Called, deferred, when the rows in view come within `within` of the last row ([`NearEnd`]).
    pub fn on_near_end(mut self, within: Pixels, f: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static) -> Self {
        self.near_end = Some((within, Rc::new(f)));
        self
    }

    pub fn render(self, cx: &mut Context<V>) -> Stateful<Div> {
        let (widths, measured): (Rc<[Pixels]>, bool) = {
            let mut inner = self.state.inner.borrow_mut();
            inner.rows = self.rows.saturating_sub(self.footer);
            inner.row_height = self.row_height;
            let total = if inner.width > px(0.) { inner.width } else { clamp(&self.columns, &self.widths).iter().fold(px(0.), |s, w| s + *w) };
            inner.current = fit(&self.columns, &inner.current, &self.widths, total);
            (inner.current.as_slice().into(), inner.width > px(0.))
        };
        // Columns fitted to a width not yet measured would jump on the next frame; the last column fills the row either way.
        let unfitted = !measured && self.columns.len() > 1;
        let on_key = self.on_key.clone();
        let keys = cx.listener(move |this: &mut V, e: &KeyDownEvent, window, cx| {
            if on_key.as_ref().is_some_and(|f| f(this, e.keystroke.key.as_str(), window, cx)) {
                cx.stop_propagation();
                cx.notify();
            }
        });
        let header = (!self.headless).then(|| self.header(&widths, cx));
        div()
            .id(self.id.clone())
            .relative()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.state.focus)
            .on_key_down(keys)
            .on_action(self.stepper(|_: &SelectPrevious| Step::By(-1), cx))
            .on_action(self.stepper(|_: &SelectNext| Step::By(1), cx))
            .on_action(self.stepper(|_: &SelectPageUp| Step::Pages(-1), cx))
            .on_action(self.stepper(|_: &SelectPageDown| Step::Pages(1), cx))
            .on_action(self.stepper(|_: &SelectFirst| Step::First, cx))
            .on_action(self.stepper(|_: &SelectLast| Step::Last, cx))
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .overflow_hidden()
            .child(width_probe(self.state.clone()))
            .children(header.map(|h| h.when(unfitted, |h| h.invisible())))
            .child(self.body(widths, cx).when(unfitted, |b| b.invisible()))
    }

    /// A key's action moving the selection as `step` says, then [`Table::on_select`] when it moved.
    fn stepper<A: gpui::Action>(&self, step: impl Fn(&A) -> Step + 'static, cx: &mut Context<V>) -> impl Fn(&A, &mut Window, &mut App) + 'static {
        let (state, on_select) = (self.state.clone(), self.on_select.clone());
        cx.listener(move |this: &mut V, action: &A, window, cx| {
            if let (Some(row), Some(f)) = (state.go(step(action)), &on_select) {
                f(this, row, window, cx);
            }
            cx.notify();
        })
    }

    fn header(&self, widths: &[Pixels], cx: &mut Context<V>) -> Stateful<Div> {
        let colors = cx.theme().colors();
        let (border, background) = (colors.border_variant, colors.panel_background);
        let (text, size) = caption(colors);
        let last = self.columns.len().saturating_sub(1);
        let mut cells = Vec::with_capacity(self.columns.len());
        for (i, (column, width)) in self.columns.iter().zip(widths).enumerate() {
            let key = format!("{}-col-{}", self.id, column.id);
            let caption = match column.header_icon(*width) {
                Some(icon) => div()
                    .id(SharedString::from(format!("{key}-icon")))
                    .flex_none()
                    .child(hit_target(&self.hits, format!("{key}-icon")))
                    .child(icon.slotted(14., text))
                    .tooltip(Tooltip::text(column.caption.clone()))
                    .into_any_element(),
                None => clipped(format!("{key}-caption"), column.caption.to_uppercase(), false).into_any_element(),
            };
            let divider = (i < last).then(|| self.divider(i, column, cx));
            cells.push(
                fill(div().relative().flex().items_center(), i == last, *width)
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
            .text_size(size)
            .text_color(text)
            .on_drag_move(self.follow_drag(cx))
            .when_some(self.header_menu.clone(), |header, menu| {
                header.on_mouse_down(gpui::MouseButton::Right, cx.listener(move |this: &mut V, e: &gpui::MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    menu(this, e.position, window, cx);
                }))
            })
            .children(cells)
    }

    fn follow_drag(&self, cx: &mut Context<V>) -> impl Fn(&DragMoveEvent<DraggedDivider>, &mut Window, &mut App) + 'static {
        let prefix = format!("{}-divider-", self.id);
        let (state, columns, preferred) = (self.state.clone(), self.columns.clone(), self.widths.clone());
        cx.listener(move |_: &mut V, e: &DragMoveEvent<DraggedDivider>, _, cx| {
            let dragged = e.drag(cx);
            if !dragged.0.starts_with(&prefix) {
                return;
            }
            let mut inner = state.inner.borrow_mut();
            inner.current = drag(&columns, &inner.current, &preferred, dragged.1, e.event.position.x - e.bounds.left());
            cx.notify();
        })
    }

    fn divider(&self, i: usize, column: &Column, cx: &mut Context<V>) -> Stateful<Div> {
        let key = format!("{}-divider-{}", self.id, column.id);
        let (end, reset) = (self.on_resize.clone(), self.on_resize.clone());
        let (columns, preferred, state) = (self.columns.clone(), self.widths.clone(), self.state.clone());
        let (ended, ending) = (preferred.clone(), state.clone());
        handle(
            key.clone().into(),
            i,
            move |this, cx| {
                let kept = keep(&ending, &ended, i);
                if let Some(f) = &end {
                    f(this, kept, cx);
                }
            },
            move |this, cx| {
                {
                    let mut inner = state.inner.borrow_mut();
                    let x = inner.current[..i].iter().fold(columns[i].default, |s, w| s + *w);
                    inner.current = drag(&columns, &inner.current, &preferred, i, x);
                }
                let kept = keep(&state, &preferred, i);
                if let Some(f) = &reset {
                    f(this, kept, cx);
                }
                cx.notify();
            },
            cx,
        )
        .right(px(-4.))
        .child(hit_target(&self.hits, key))
    }

    fn body(self, widths: Rc<[Pixels]>, cx: &mut Context<V>) -> gpui::UniformList {
        let scroll = self.state.scroll.clone();
        let Self { id, state, hits, rows, row_height, inset, cell, band, on_select, near_end, on_click, row_key, .. } = self;
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
            let focused = state.focus.is_focused(window) || state.focus.contains_focused(window, cx);
            let (selection, hover) = (crate::themes::selection(colors, focused), colors.ghost_element_hover);
            let mut out = Vec::with_capacity(range.len());
            if range.len() > 1 || range.start > 0 {
                state.inner.borrow_mut().shown_selection = selected.filter(|s| range.contains(s)).map(|_| selection);
            }
            for ix in range {
                let part = row_key.as_ref().map_or_else(|| ix.to_string(), |k| k(ix));
                let (key, fill_key) = (format!("{id}-row-{part}"), format!("{id}-fill-{part}"));
                let background = band.as_ref().and_then(|b| b(this, ix, cx));
                let cells: Vec<AnyElement> = widths
                    .iter()
                    .enumerate()
                    .map(|(col, width)| {
                        let content = cell(this, ix, col, window, cx);
                        fill(div().flex().items_center().h_full().px(rems(CELL_PAD)).overflow_hidden(), col + 1 == widths.len(), *width).child(content).into_any_element()
                    })
                    .collect();
                let (state, on_select, on_click) = (state.clone(), on_select.clone(), on_click.clone());
                let fill = div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(inset)
                    .bottom(inset)
                    .when_some(background, |d, bg| d.bg(bg))
                    .map(|d| if selected == Some(ix) { d.bg(selection) } else { d.group_hover(group.clone(), |s| s.bg(hover)) })
                    .child(hit_target(&hits, fill_key));
                out.push(
                    div()
                        .id(SharedString::from(key.clone()))
                        .group(group.clone())
                        .relative()
                        .flex()
                        .w_full()
                        .h(row_height)
                        .child(fill)
                        .child(hit_target(&hits, key))
                        .children(cells)
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            state.select(Some(ix));
                            window.focus(&state.focus, cx);
                            if let Some(f) = &on_select {
                                f(this, ix, window, cx);
                            }
                            if let Some(f) = &on_click {
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

/// A cell `width` wide; the last column's takes the rest of the row, so it fills it before the width is measured.
fn fill(cell: Div, last: bool, width: Pixels) -> Div {
    if last { cell.flex_1().min_w_0() } else { cell.flex_none().w(width) }
}

/// The owner's preferred widths with column `i`'s set to its current one.
fn keep(state: &TableState, preferred: &[Pixels], i: usize) -> Vec<Pixels> {
    let inner = state.inner.borrow();
    let mut kept = preferred.to_vec();
    if let (Some(slot), Some(width)) = (kept.get_mut(i), inner.current.get(i)) {
        *slot = *width;
    }
    kept
}

/// The row the view ends at, from the scroll as of this frame: the row painter is also called with
/// `0..1` to measure a row, so its range is not the view.
fn visible_end(scroll: &UniformListScrollHandle, row_height: Pixels) -> Option<usize> {
    let state = scroll.0.borrow();
    let viewport = state.last_item_size?.item.height;
    let top = -state.base_handle.offset().y;
    Some(((top + viewport) / row_height).ceil() as usize)
}

/// Records the table's width for the next frame's [`fit`], and asks for that frame when it changed.
fn width_probe(state: TableState) -> impl IntoElement {
    canvas(
        move |bounds, window, cx| {
            let mut inner = state.inner.borrow_mut();
            if inner.width != bounds.size.width {
                inner.width = bounds.size.width;
                // gpui drops a refresh asked for while it draws
                window.defer(cx, |window, _| window.refresh());
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// The header's captions: the theme's secondary shade, a step under the rows' `text_xs`.
fn caption(colors: &theme::ThemeColors) -> (Hsla, Pixels) {
    (crate::themes::secondary(colors), px(11.))
}

#[cfg(test)]
mod tests {
    use gpui::{Entity, TestAppContext, VisualTestContext};

    use super::*;

    fn columns() -> Vec<Column> {
        vec![
            Column::new("branch", "Branch / Tag", px(32.), px(130.)).icon(Icon::Tag, px(55.)),
            Column::new("graph", "Graph", px(54.), px(150.)).icon(Icon::GitGraph, px(56.)),
            Column::new("message", "Commit Message", px(50.), px(300.)).icon(Icon::Chat, px(55.)),
            Column::new("date", "Commit Date / Time", px(50.), px(170.)).icon(Icon::Clock, px(55.)),
            Column::new("sha", "Sha", px(50.), px(130.)).icon(Icon::Hash, px(50.)),
        ]
    }

    fn sum(widths: &[Pixels]) -> Pixels {
        widths.iter().fold(px(0.), |s, w| s + *w)
    }

    fn near(a: Pixels, b: f32) -> bool {
        (a - px(b)).abs() <= px(1.)
    }

    #[test]
    fn the_header_captions_are_the_secondary_shade_a_step_smaller() {
        let theme = crate::themes::dark_modern().theme;
        let colors = theme.colors();
        assert_eq!(caption(colors), (colors.text.opacity(crate::themes::SECONDARY), px(11.)));
    }

    #[test]
    fn the_header_shows_the_icon_at_its_threshold_and_narrower() {
        let sha = &columns()[4];
        assert_eq!(sha.header_icon(px(51.)), None);
        assert_eq!(sha.header_icon(px(50.)), Some(Icon::Hash));
        assert_eq!(Column::new("x", "X", px(10.), px(20.)).header_icon(px(10.)), None, "a column without an icon keeps its caption");
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
        resized: usize,
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
            .on_resize(|this, widths, cx| {
                this.widths = widths;
                this.resized += 1;
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
            let widths = vec![px(130.), px(150.), px(300.), px(170.), px(130.)];
            Grid { state, hits: HitTargets::default(), rows, painted: Vec::new(), selected: Vec::new(), near: 0, widths, resized: 0 }
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
        grid.update(cx, |g, _| {
            g.state.select(Some(0));
            g.selected.clear();
        });
        cx.simulate_keystrokes("up");
        grid.read_with(cx, |g, _| assert!(g.selected.is_empty(), "an Up on the first row selects nothing anew: {:?}", g.selected));
    }

    #[gpui::test]
    fn the_selection_is_the_active_fill_only_while_the_table_has_the_keys(cx: &mut TestAppContext) {
        let (grid, cx) = grid(100, cx);
        let colors = cx.update(|_, cx| cx.theme().colors().clone());
        let (active, inactive) = (crate::themes::selection(&colors, true), crate::themes::selection(&colors, false));
        let shown = |cx: &mut VisualTestContext| {
            redraw(&grid, cx);
            grid.read_with(cx, |g, _| g.state.selection_shown())
        };
        cx.simulate_keystrokes("down down");
        assert_eq!(shown(cx), Some(active));
        let elsewhere = cx.update(|window, cx| {
            let other = cx.focus_handle();
            window.focus(&other, cx);
            other
        });
        assert_eq!(shown(cx), Some(inactive), "another handle has the keys");
        cx.simulate_keystrokes("down");
        grid.read_with(cx, |g, _| assert_eq!(g.state.selected(), Some(1), "a key the table does not have moves nothing"));
        drop(elsewhere);
        let handle = grid.read_with(cx, |g, _| g.state.focus_handle().clone());
        cx.update(|window, cx| window.focus(&handle, cx));
        assert_eq!(shown(cx), Some(active));
        cx.simulate_keystrokes("down");
        grid.read_with(cx, |g, _| assert_eq!(g.state.selected(), Some(2)));
    }

    #[gpui::test]
    fn a_click_on_a_row_gives_the_table_the_keys(cx: &mut TestAppContext) {
        let (grid, cx) = grid(100, cx);
        let _elsewhere = cx.update(|window, cx| {
            let other = cx.focus_handle();
            window.focus(&other, cx);
            other
        });
        redraw(&grid, cx);
        let row = grid.read_with(cx, |g, _| g.hits.borrow().get("t-row-3").copied()).expect("row 3 paints under its key");
        cx.simulate_click(row.bounds.center(), gpui::Modifiers::none());
        cx.simulate_keystrokes("down");
        grid.read_with(cx, |g, _| assert_eq!(g.state.selected(), Some(4)));
        let colors = cx.update(|_, cx| cx.theme().colors().clone());
        redraw(&grid, cx);
        assert_eq!(grid.read_with(cx, |g, _| g.state.selection_shown()), Some(crate::themes::selection(&colors, true)));
    }

    #[gpui::test]
    fn page_keys_move_a_page_of_rows_and_ctrl_keys_go_to_the_ends(cx: &mut TestAppContext) {
        let (grid, cx) = grid(1_000, cx);
        redraw(&grid, cx);
        let page = grid.read_with(cx, |g, _| g.state.page());
        assert_eq!(page, 10, "280 px of 28 px rows");
        let selected = |cx: &mut VisualTestContext| grid.read_with(cx, |g, _| g.state.selected());
        cx.simulate_keystrokes("j j k pagedown");
        assert_eq!(selected(cx), Some(10), "J and K are Down and Up; Page Down a page further");
        cx.simulate_keystrokes("pagedown pageup pageup pageup");
        assert_eq!(selected(cx), Some(0), "Page Up stops at the first row");
        cx.simulate_keystrokes("ctrl-end");
        assert_eq!(selected(cx), Some(999));
        let painted = redraw(&grid, cx);
        assert!(painted.contains(&999), "the last row in view: {painted:?}");
        cx.simulate_keystrokes("pagedown ctrl-home pagedown ctrl-down");
        assert_eq!(selected(cx), Some(999), "Page Down stops at the last row; Ctrl+Down is Ctrl+End");
        cx.simulate_keystrokes("ctrl-up");
        assert_eq!(selected(cx), Some(0));
        grid.read_with(cx, |g, _| assert_eq!(g.selected[..4], [0, 1, 0, 10]));
    }

    struct Footed {
        state: TableState,
        hits: HitTargets,
    }

    impl Render for Footed {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let table = Table::new("f", &self.state, &self.hits, columns(), &[px(100.); 5], 6, |_: &mut Footed, row, _, _, _| div().child(format!("{row}")).into_any_element()).footer(1);
            div().w(px(900.)).h(px(300.)).child(table.render(cx))
        }
    }

    #[gpui::test]
    fn no_key_selects_the_footer(cx: &mut TestAppContext) {
        cx.update(|cx| crate::init_globals(cx, "test"));
        let (footed, cx) = cx.add_window_view(|window, cx| {
            let state = TableState::new(cx);
            window.focus(state.focus_handle(), cx);
            Footed { state, hits: HitTargets::default() }
        });
        cx.run_until_parked();
        cx.simulate_keystrokes("ctrl-end");
        assert_eq!(footed.read_with(cx, |f, _| f.state.selected()), Some(4), "the last of five rows above the footer");
        cx.simulate_keystrokes("down pagedown");
        assert_eq!(footed.read_with(cx, |f, _| f.state.selected()), Some(4));
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

    fn drag_by(grid: &Entity<Grid>, divider: &str, by: f32, cx: &mut VisualTestContext) -> Vec<Pixels> {
        use gpui::{Modifiers, MouseButton, point};
        let key = format!("t-divider-{divider}");
        let hit = grid.read_with(cx, |g, _| g.hits.borrow().get(&key).copied()).expect("the divider paints under its key");
        let start = hit.bounds.center();
        cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(start + point(px(by / 2.), px(0.)), MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(start + point(px(by), px(0.)), MouseButton::Left, Modifiers::none());
        let during = grid.read_with(cx, |g, _| g.state.inner.borrow().current.clone());
        cx.simulate_mouse_up(start + point(px(by), px(0.)), MouseButton::Left, Modifiers::none());
        redraw(grid, cx);
        during
    }

    #[gpui::test]
    fn dragging_a_divider_moves_only_its_column_and_those_right_of_it(cx: &mut TestAppContext) {
        let (grid, cx) = grid(100, cx);
        redraw(&grid, cx);
        let before = grid.read_with(cx, |g, _| g.state.inner.borrow().current.clone());
        assert_eq!(before, [px(130.), px(150.), px(300.), px(170.), px(150.)], "the rest of the 900 px to the last column");
        let during = drag_by(&grid, "message", 40., cx);
        assert_eq!(during[..2], before[..2], "left of the divider: unchanged");
        assert!(near(during[2], 340.) && near(during[3], 170.) && near(during[4], 110.), "the sha gives first: {during:?}");
        assert_eq!(sum(&during), px(900.));
        grid.read_with(cx, |g, _| {
            assert_eq!(g.resized, 1, "one call, at the end");
            assert!(near(g.widths[2], 340.), "the message's preferred width is where it was let go: {:?}", g.widths);
            assert_eq!((&g.widths[..2], &g.widths[3..]), (&[px(130.), px(150.)][..], &[px(170.), px(130.)][..]), "the others' preferred widths stay");
        });
        assert!(!grid.read_with(cx, |g, _| g.hits.borrow().contains_key("t-col-sha-icon")));
        let before = grid.read_with(cx, |g, _| g.state.inner.borrow().current.clone());
        let during = drag_by(&grid, "date", 200., cx);
        assert_eq!(during[..3], before[..3], "left of the date: unchanged");
        assert_eq!(during[4], px(50.), "the sha down to its min");
        grid.read_with(cx, |g, _| {
            assert!(g.hits.borrow().contains_key("t-col-sha-icon"), "at its threshold the sha's header is its icon");
            assert!(!g.hits.borrow().contains_key("t-col-date-icon"));
        });
    }
}
