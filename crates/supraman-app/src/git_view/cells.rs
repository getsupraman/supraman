//! The Branch / Tag, Commit Message, Date and Sha cells.

use gpui::{AnyElement, AppContext as _, ClickEvent, Context, Div, MouseButton, MouseDownEvent, SharedString, Stateful, deferred, div, prelude::*, px, relative};
use supraman_gitman::{RefGroup, RefKind, Row, short_sha};
use supraman_ui::clipped::clipped;
use supraman_ui::fonts;
use supraman_ui::hit::{HitTargets, hit_target};
use supraman_ui::icon::Icon;

use super::GitView;
use super::branch::DraggedPill;
use super::colours::Palette;
use super::graph::{BAR, CELL_PAD, GAP};
use super::load::GraphRow;

/// The hover list shows at most this many groups.
const LISTED: usize = 100;
const PILL: f32 = 18.;
/// The body gives up its width long before the summary does.
const BODY_SHRINK: f32 = 1000.;

/// What a row's Branch / Tag cell shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefCellView {
    pub first: Option<RefGroup>,
    /// The `+N`; 0 while the list is open.
    pub more: usize,
    /// Every group, while the pointer rests on the cell of a row with more than one.
    pub list: Vec<RefGroup>,
    /// `first` is the nearest descendant's, shown dimmed while the row is hovered or selected.
    pub ghost: bool,
}

pub fn ref_cell(row: &Row, listed: bool) -> RefCellView {
    match (row.refs.first(), row.ghost.as_ref().and_then(|g| g.first())) {
        (Some(first), _) => {
            let open = listed && row.refs.len() > 1;
            RefCellView {
                first: Some(first.clone()),
                more: if open { 0 } else { row.more() },
                list: if open { row.refs.iter().take(LISTED).cloned().collect() } else { Vec::new() },
                ghost: false,
            }
        }
        (None, Some(ghost)) => RefCellView { first: Some(ghost.clone()), more: 0, list: Vec::new(), ghost: true },
        (None, None) => RefCellView { first: None, more: 0, list: Vec::new(), ghost: false },
    }
}

fn kind_icon(kind: RefKind) -> Icon {
    match kind {
        RefKind::Worktree => Icon::Worktree,
        RefKind::Local => Icon::Local,
        RefKind::Remote => Icon::Remote,
        RefKind::Tag => Icon::Tag,
    }
}

/// A name's box in a lane's pill fill, as a ref shows on the graph.
pub fn pill_box(colour: usize, palette: &Palette) -> Div {
    div().flex().items_center().gap_1().min_w_0().h(px(PILL)).px_1p5().rounded(px(4.)).bg(palette.pill(colour)).text_color(palette.text).text_xs()
}

/// The HEAD mark, the name, then the kinds of its refs.
fn pill(key: String, group: &RefGroup, colour: usize, palette: &Palette) -> Div {
    let mut kinds: Vec<RefKind> = group.refs.iter().map(|r| r.kind).collect();
    kinds.dedup();
    pill_box(colour, palette)
        .when(group.head(), |d| d.child(Icon::Check.svg(12., palette.text)))
        .child(clipped(key, group.name.clone(), false))
        .children(kinds.into_iter().map(|k| div().flex_none().child(kind_icon(k).svg(12., palette.text))))
}

/// A pill of row `ix`'s group `group`: its right-click menu, its double-click checkout, dragged onto another pill
/// and taking one dropped on it.
fn acting(pill: Div, key: String, ix: usize, group: &RefGroup, at: usize, sha: SharedString, palette: &Palette, cx: &mut Context<GitView>) -> Stateful<Div> {
    let drop_border = palette.text;
    let dragged = DraggedPill { group: group.clone(), sha };
    pill.id(SharedString::from(format!("{key}-act")))
        .on_mouse_down(MouseButton::Right, cx.listener(move |this, e: &MouseDownEvent, _, cx| {
            cx.stop_propagation();
            this.pill_menu(ix, at, e.position, cx);
        }))
        .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
            if e.click_count() == 2 {
                this.pill_double_clicked(ix, at, window.mouse_position(), cx);
            }
        }))
        .on_drag(dragged, |pill, _, _, cx| cx.new(|_| pill.clone()))
        .drag_over::<DraggedPill>(move |style, _, _, _| style.border_1().border_color(drop_border))
        .on_drop(cx.listener(move |this, pill: &DraggedPill, window, cx| this.pill_dropped(pill, ix, at, window, cx)))
}

/// The cell of a row whose name field stands in place of its pills.
pub fn naming(ix: usize, field: gpui::Entity<supraman_ui::text_input::TextInput>, hits: &HitTargets) -> AnyElement {
    div().relative().size_full().flex().items_center().child(hit_target(hits, format!("git-refs-{ix}"))).child(div().w_full().child(field)).into_any_element()
}

pub fn refs(ix: usize, row: &GraphRow, view: RefCellView, selected: bool, palette: &Palette, hits: &HitTargets, cx: &mut Context<GitView>) -> AnyElement {
    let colour = row.row.colour;
    let group = supraman_ui::table::row_group(super::TABLE);
    let mut cell = div()
        .id(SharedString::from(format!("git-refs-{ix}")))
        .relative()
        .flex()
        .items_center()
        .gap_1()
        .size_full()
        .child(hit_target(hits, format!("git-refs-{ix}")))
        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
            this.hover_ref = if *hovered { Some(ix) } else { this.hover_ref.filter(|r| *r != ix) };
            cx.notify();
        }));
    let Some(first) = &view.first else { return cell.into_any_element() };
    let shown = pill(format!("git-ref-{ix}"), first, colour, palette);
    if view.ghost {
        let dim = palette.ghost();
        let shown = if selected { shown.opacity(dim) } else { shown.opacity(0.).group_hover(group, move |s| s.opacity(dim)) };
        return cell.child(shown.relative().child(hit_target(hits, format!("git-ghost-{ix}")))).into_any_element();
    }
    let shown = acting(shown.relative().child(hit_target(hits, format!("git-pill-{ix}"))), format!("git-pill-{ix}"), ix, first, 0, row.sha.clone(), palette, cx);
    let shown = if view.list.is_empty() { shown } else { shown.child(deferred(list(ix, row, &view.list, colour, palette, hits, cx)).with_priority(1)) };
    cell = cell.child(shown).when(view.more > 0, |d| {
        let more = SharedString::from(format!("+{}", view.more));
        d.child(div().relative().flex_none().text_xs().text_color(palette.muted).child(more).child(hit_target(hits, format!("git-more-{ix}"))))
    });
    let connector = div().flex_1().min_w(px(4.)).mr(px(-CELL_PAD)).h(px(1.5)).bg(palette.connector(colour));
    cell.child(connector).into_any_element()
}

/// Every group, the first on top, in one block laid over the row's pill: as wide as its widest group and never
/// narrower than the pill, each group as wide as the block.
fn list(ix: usize, row: &GraphRow, groups: &[RefGroup], colour: usize, palette: &Palette, hits: &HitTargets, cx: &mut Context<GitView>) -> AnyElement {
    let last = groups.len().saturating_sub(1);
    div()
        .id(SharedString::from(format!("git-reflist-{ix}")))
        .occlude()
        .absolute()
        .top_0()
        .left_0()
        .min_w(relative(1.))
        .max_w(px(320.))
        .flex()
        .flex_col()
        .children(groups.iter().enumerate().map(|(i, g)| {
            let key = format!("git-reflist-{ix}-{i}");
            let top = if i == 0 { px(4.) } else { px(0.) };
            let bottom = if i == last { px(4.) } else { px(0.) };
            let shown = pill(format!("{key}-name"), g, colour, palette).relative().rounded(px(0.)).rounded_t(top).rounded_b(bottom).bg(palette.pill_solid(colour)).child(hit_target(hits, key.clone()));
            acting(shown, key, ix, g, i, row.sha.clone(), palette, cx)
        }))
        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
            this.over_list = if *hovered { Some(ix) } else { this.over_list.filter(|r| *r != ix) };
            cx.notify();
        }))
        .into_any_element()
}

/// The summary, then the body muted, the WIP row's `// WIP` muted too; whichever overflows ends with an ellipsis. A bar in the row's lane colour on the
/// column's left edge closes the graph's band.
pub fn message(ix: usize, row: &GraphRow, wip: Option<&str>, palette: &Palette) -> AnyElement {
    let (summary, body, colour): (SharedString, SharedString, _) = match wip {
        Some(counts) => ("// WIP".into(), counts.to_string().into(), palette.muted),
        None => (row.summary.clone(), row.body.clone(), palette.text),
    };
    let bar = div().absolute().left(px(-CELL_PAD)).w(px(BAR)).top(px(GAP / 2.)).bottom(px(GAP / 2.)).bg(palette.lane(row.row.colour));
    let text = div()
        .flex()
        .items_center()
        .gap_2()
        .size_full()
        .overflow_hidden()
        .child(clipped(format!("git-msg-{ix}"), summary, false).flex_shrink(1.).text_color(colour))
        .when(!body.is_empty(), |d| d.child(clipped(format!("git-body-{ix}"), body, false).flex_shrink(BODY_SHRINK).text_color(palette.muted)));
    div().relative().size_full().child(bar).child(text).into_any_element()
}

/// The WIP row's message cell: the summary's `field`, then the counts muted, after the lane's bar.
pub fn wip_message(row: &GraphRow, field: impl IntoElement, counts: String, palette: &Palette) -> AnyElement {
    let bar = div().absolute().left(px(-CELL_PAD)).w(px(BAR)).top(px(GAP / 2.)).bottom(px(GAP / 2.)).bg(palette.lane(row.row.colour));
    let counts = div().flex_none().whitespace_nowrap().text_color(palette.muted).child(SharedString::from(counts));
    div().relative().size_full().flex().items_center().gap_2().overflow_hidden().child(bar).child(field).child(counts).into_any_element()
}

pub fn date_text(row: &GraphRow) -> String {
    if row.wip() { String::new() } else { supraman_core::time::stamp(row.time_ms) }
}

pub fn sha_text(row: &GraphRow) -> &str {
    short_sha(&row.sha)
}

/// Muted in [`fonts::figures`]: the date and sha columns share one face and keep their digits aligned.
fn figures(text: String, palette: &Palette) -> AnyElement {
    fonts::figures(div()).whitespace_nowrap().text_color(palette.muted).child(SharedString::from(text)).into_any_element()
}

pub fn date(row: &GraphRow, palette: &Palette) -> AnyElement {
    figures(date_text(row), palette)
}

pub fn sha(row: &GraphRow, palette: &Palette) -> AnyElement {
    figures(sha_text(row).to_string(), palette)
}
