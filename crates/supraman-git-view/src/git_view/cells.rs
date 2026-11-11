//! The Branch / Tag, Commit Message, Date and Sha cells.

use gpui::{AnyElement, AppContext as _, ClickEvent, Context, Div, Hsla, MouseButton, MouseDownEvent, SharedString, Stateful, Svg, deferred, div, prelude::*, px, relative};
use supraman_gitman::{RefGroup, RefKind, Row, short_sha};
use supraman_ui::clipped::clipped;
use supraman_ui::fonts;
use supraman_ui::hit::{HitTargets, hit_target};
use supraman_ui::icon::Icon;
use supraman_ui::ink;

use super::GitView;
use super::branch::DraggedPill;
use super::colours::Palette;
use super::graph::{BAR, CELL_PAD, GAP, LINE};
use super::load::GraphRow;

/// The hover list shows at most this many groups.
const LISTED: usize = 100;
/// A pill's icons and HEAD mark.
pub(crate) const PILL_ICON: f32 = 16.;
const PILL_PAD: f32 = 8.;
const PILL_GAP: f32 = 6.;
/// A cut name keeps its first letter and the ellipsis.
const NAME_MIN: f32 = 22.;
const ICON_SLACK: f32 = 2.;
/// The body gives up its width long before the summary does.
const BODY_SHRINK: f32 = 1000.;

/// What a row's Branch / Tag cell shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RefCellView {
    pub(crate) first: Option<RefGroup>,
    /// The `+N`; 0 while the list is open.
    pub(crate) more: usize,
    /// Every group, while the pointer rests on the cell of a row with more than one.
    pub(crate) list: Vec<RefGroup>,
    /// `first` is the nearest descendant's, shown dimmed while the row is hovered or selected.
    pub(crate) ghost: bool,
}

pub(crate) fn ref_cell(row: &Row, listed: bool) -> RefCellView {
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

/// What a pill shows: HEAD's mark, the name, then its icons.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Face {
    pub(crate) head: bool,
    pub(crate) name: SharedString,
    pub(crate) icons: Vec<Icon>,
}

impl Face {
    pub(crate) fn of(group: &RefGroup) -> Face {
        Face { head: group.head(), name: group.name.clone().into(), icons: kind_icons(group) }
    }

    pub(crate) fn named(name: impl Into<SharedString>) -> Face {
        Face { head: false, name: name.into(), icons: Vec::new() }
    }
}

/// A pill's fill in lane `colour`, stronger while hovered.
pub(crate) fn pill_fill(colour: usize, hovered: bool, palette: &Palette) -> Hsla {
    if hovered { palette.pill_hover(colour) } else { palette.pill(colour) }
}

/// Every ref pill: the row band's height in lane `colour`, its name cut under `key` and painted under `<key>-text`, each icon under `<key>-icon-<n>`.
pub(crate) fn pill(key: impl Into<String>, face: Face, colour: usize, palette: &Palette, hits: &HitTargets) -> Div {
    let key = key.into();
    let (text, hover) = (palette.text, pill_fill(colour, true, palette));
    let name = clipped(key.clone(), face.name, false).min_w(px(NAME_MIN)).flex_shrink(BODY_SHRINK).child(hit_target(hits, format!("{key}-text")));
    let least = 2. * PILL_PAD + NAME_MIN + if face.head { PILL_ICON + PILL_GAP } else { 0. };
    div()
        .flex()
        .items_center()
        .gap(px(PILL_GAP))
        .min_w(px(least))
        .overflow_hidden()
        .h(px(LINE))
        .px(px(PILL_PAD))
        .rounded(px(4.))
        .bg(pill_fill(colour, false, palette))
        .hover(move |s| s.bg(hover))
        .text_color(text)
        .text_xs()
        .when(face.head, |d| d.child(div().flex_none().child(current_mark(text, PILL_ICON))))
        .child(ink::text(name))
        .when(!face.icons.is_empty(), |d| d.child(icons(&key, face.icons, text, hits)))
}

/// A panel's header branch: its pill in lane `colour`, or `Detached HEAD`, then [`super::ref_panel::track`]'s words; painted under `<key>-branch` and `<key>-upstream`.
pub(crate) fn branch_header(key: &str, branch: Option<&str>, track: &str, colour: usize, palette: &Palette, hits: &HitTargets) -> Div {
    let shown = match branch {
        Some(name) => pill(format!("{key}-branch-name"), Face::named(name.to_string()), colour, palette, hits),
        None => div().flex_none().text_color(palette.muted).child("Detached HEAD"),
    };
    div()
        .flex()
        .items_center()
        .gap_1p5()
        .min_w_0()
        .child(shown.relative().child(hit_target(hits, format!("{key}-branch"))))
        .when(!track.is_empty(), |d| d.child(div().relative().flex_none().whitespace_nowrap().text_color(palette.muted).child(SharedString::from(track.to_string())).child(hit_target(hits, format!("{key}-upstream")))))
}

/// The icons after a pill's name: its refs' kinds, a linked worktree's last (GitKraken's order).
pub(crate) fn kind_icons(group: &RefGroup) -> Vec<Icon> {
    let mut kinds: Vec<RefKind> = group.refs.iter().map(|r| r.kind).collect();
    kinds.dedup();
    kinds.sort_by_key(|k| *k == RefKind::Worktree);
    kinds.into_iter().map(kind_icon).collect()
}

/// The current branch's mark: the graph's HEAD pill and the ref panel's current row.
pub(crate) fn current_mark(colour: Hsla, size: f32) -> Svg {
    Icon::Check.svg(size, colour)
}

/// A pill's icons laid out from the last after an empty first item: one without room wraps whole onto the hidden second line, the slack before any.
fn icons(key: &str, icons: Vec<Icon>, colour: Hsla, hits: &HitTargets) -> Div {
    let last = icons.len().saturating_sub(1);
    let shown = icons.into_iter().enumerate().rev().map(|(n, i)| {
        div().relative().flex_none().when(n < last, |d| d.mr(px(PILL_GAP))).child(i.svg(PILL_ICON, colour)).child(hit_target(hits, format!("{key}-icon-{n}")))
    });
    div().flex().flex_row_reverse().flex_wrap().justify_end().min_w_0().h(px(PILL_ICON)).overflow_hidden().child(div().flex_none().w(px(0.)).h(px(PILL_ICON))).children(shown).child(div().flex_none().w(px(ICON_SLACK)).h(px(PILL_ICON)))
}

/// A pill of row `ix`'s group `group`: its menu, double-click checkout, drag and drop.
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
pub(crate) fn naming(ix: usize, field: gpui::Entity<supraman_ui::text_input::TextInput>, hits: &HitTargets) -> AnyElement {
    div().relative().size_full().flex().items_center().child(hit_target(hits, format!("git-refs-{ix}"))).child(div().w_full().child(field)).into_any_element()
}

pub(crate) fn refs(ix: usize, row: &GraphRow, view: RefCellView, selected: bool, palette: &Palette, hits: &HitTargets, cx: &mut Context<GitView>) -> AnyElement {
    let colour = row.row.colour;
    let group = supraman_ui::table::row_group(super::TABLE);
    let mut cell = div()
        .id(SharedString::from(format!("git-refs-{ix}")))
        .relative()
        .flex()
        .items_center()
        .gap(px(GAP))
        .size_full()
        .child(hit_target(hits, format!("git-refs-{ix}")))
        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
            this.hover_ref = if *hovered { Some(ix) } else { this.hover_ref.filter(|r| *r != ix) };
            cx.notify();
        }));
    let Some(first) = &view.first else { return cell.into_any_element() };
    let shown = pill(format!("git-ref-{ix}"), Face::of(first), colour, palette, hits);
    if view.ghost {
        let dim = palette.ghost();
        let shown = if selected { shown.opacity(dim) } else { shown.opacity(0.).group_hover(group, move |s| s.opacity(dim)) };
        return cell.child(shown.relative().child(hit_target(hits, format!("git-ghost-{ix}")))).into_any_element();
    }
    let shown = acting(shown.relative().child(hit_target(hits, format!("git-pill-{ix}"))), format!("git-pill-{ix}"), ix, first, 0, row.sha.clone(), palette, cx);
    let shown = if view.list.is_empty() { shown } else { shown.child(deferred(list(ix, row, &view.list, colour, palette, hits, cx)).with_priority(supraman_ui::menu::FLOAT)) };
    cell = cell.child(shown).when(view.more > 0, |d| {
        let more = SharedString::from(format!("+{}", view.more));
        d.child(more_chip(more, colour, palette).child(hit_target(hits, format!("git-more-{ix}"))))
    });
    let connector = div().flex_1().min_w(px(4.)).mr(px(-CELL_PAD)).h(px(1.5)).bg(palette.connector(colour));
    cell.child(connector).into_any_element()
}

/// A group's `+N`: its own fill, [`GAP`] from the pill, as two graph stripes stand apart.
fn more_chip(more: SharedString, colour: usize, palette: &Palette) -> Div {
    div().relative().flex_none().flex().items_center().h(px(LINE)).px_1p5().rounded(px(4.)).bg(palette.pill(colour)).text_xs().text_color(palette.text).child(ink::text(more))
}

/// Every group in one block over the row's pill, as wide as its widest group and never narrower than the pill.
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
            let hover = palette.pill_hover_solid(colour);
            let shown = pill(format!("{key}-name"), Face::of(g), colour, palette, hits)
                .relative()
                .rounded(px(0.))
                .rounded_t(top)
                .rounded_b(bottom)
                .bg(palette.pill_solid(colour))
                .hover(move |s| s.bg(hover))
                .child(hit_target(hits, key.clone()));
            acting(shown, key, ix, g, i, row.sha.clone(), palette, cx)
        }))
        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
            this.over_list = if *hovered { Some(ix) } else { this.over_list.filter(|r| *r != ix) };
            cx.notify();
        }))
        .into_any_element()
}

/// The summary, then the body muted (the WIP row's `// WIP` too), after a bar in the row's lane colour.
pub(crate) fn message(ix: usize, row: &GraphRow, wip: Option<&str>, palette: &Palette, hits: &HitTargets) -> AnyElement {
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
        .line_height(px(LINE))
        .overflow_hidden()
        .child(clipped(format!("git-msg-{ix}"), summary, false).flex_shrink(1.).text_color(colour).child(hit_target(hits, format!("git-text-{ix}"))))
        .when(!body.is_empty(), |d| d.child(clipped(format!("git-body-{ix}"), body, false).flex_shrink(BODY_SHRINK).text_color(palette.muted)));
    div().relative().size_full().child(bar).child(text).into_any_element()
}

/// The WIP row's message cell: the summary's `field`, then the counts muted, after the lane's bar.
pub(crate) fn wip_message(row: &GraphRow, field: impl IntoElement, counts: impl IntoElement, palette: &Palette) -> AnyElement {
    let bar = div().absolute().left(px(-CELL_PAD)).w(px(BAR)).top(px(GAP / 2.)).bottom(px(GAP / 2.)).bg(palette.lane(row.row.colour));
    let counts = div().flex_none().whitespace_nowrap().text_color(palette.muted).child(counts);
    div().relative().size_full().flex().items_center().gap_2().overflow_hidden().child(bar).child(field).child(counts).into_any_element()
}

pub(crate) fn date_text(row: &GraphRow) -> String {
    if row.commit() { supraman_core::time::stamp(row.time_ms) } else { String::new() }
}

pub(crate) fn sha_text(row: &GraphRow) -> &str {
    short_sha(&row.sha)
}

/// The date and sha columns' letters: a step under the body's `text_sm`, muted, in [`fonts::figures`] so their digits align.
pub(crate) fn figure_text<E: Styled>(element: E, palette: &Palette) -> E {
    fonts::figures(element).text_xs().text_color(palette.muted)
}

fn figures(text: String, palette: &Palette, key: String, hits: &HitTargets) -> AnyElement {
    let line = figure_text(div(), palette).relative().whitespace_nowrap().line_height(px(LINE)).child(SharedString::from(text)).child(hit_target(hits, key));
    div().h_full().flex().items_center().child(line).into_any_element()
}

pub(crate) fn date(ix: usize, row: &GraphRow, palette: &Palette, hits: &HitTargets) -> AnyElement {
    figures(date_text(row), palette, format!("git-date-{ix}"), hits)
}

pub(crate) fn sha(ix: usize, row: &GraphRow, palette: &Palette, hits: &HitTargets) -> AnyElement {
    figures(sha_text(row).to_string(), palette, format!("git-sha-{ix}"), hits)
}

#[cfg(test)]
mod tests;
