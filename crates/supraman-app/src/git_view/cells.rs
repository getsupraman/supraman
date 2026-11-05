//! The Branch / Tag, Commit Message, Date and Sha cells.

use gpui::{AnyElement, Context, Div, SharedString, anchored, deferred, div, prelude::*, px};
use supraman_gitman::{RefGroup, RefKind, Row};
use supraman_ui::clipped::clipped;
use supraman_ui::fonts;
use supraman_ui::hit::{HitTargets, hit_target};
use supraman_ui::icon::Icon;
use theme::ActiveTheme as _;

use super::GitView;
use super::colours::Palette;
use super::graph::CELL_PAD;
use super::load::GraphRow;

/// The hover list shows at most this many groups.
const LISTED: usize = 100;

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
    div().flex().items_center().gap_1().min_w_0().h(px(18.)).px_1p5().rounded(px(4.)).bg(palette.pill(colour)).text_color(palette.text).text_xs()
}

fn pill(key: String, group: &RefGroup, colour: usize, palette: &Palette) -> Div {
    let mut kinds: Vec<RefKind> = group.refs.iter().map(|r| r.kind).collect();
    kinds.dedup();
    pill_box(colour, palette)
        .when(group.head(), |d| d.child(Icon::Check.svg(12., palette.text)))
        .children(kinds.into_iter().map(|k| kind_icon(k).svg(12., palette.text)))
        .child(clipped(key, group.name.clone(), false))
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
    cell = cell.child(shown).when(view.more > 0, |d| {
        let more = SharedString::from(format!("+{}", view.more));
        d.child(div().relative().flex_none().text_xs().text_color(palette.muted).child(more).child(hit_target(hits, format!("git-more-{ix}"))))
    });
    let connector = div().flex_1().min_w(px(4.)).mr(px(-CELL_PAD)).h(px(1.5)).bg(palette.connector(colour));
    cell = cell.child(connector);
    if !view.list.is_empty() {
        cell = cell.child(div().absolute().top_0().right(px(-CELL_PAD)).child(deferred(anchored().child(list(ix, &view.list, colour, palette, hits, cx))).with_priority(1)));
    }
    cell.into_any_element()
}

fn list(ix: usize, groups: &[RefGroup], colour: usize, palette: &Palette, hits: &HitTargets, cx: &mut Context<GitView>) -> AnyElement {
    let colors = cx.theme().colors().clone();
    div()
        .id(SharedString::from(format!("git-reflist-{ix}")))
        .occlude()
        .flex()
        .flex_col()
        .gap_1()
        .p_1()
        .max_w(px(320.))
        .rounded_md()
        .border_1()
        .border_color(colors.border)
        .bg(colors.elevated_surface_background)
        .shadow_md()
        .children(groups.iter().enumerate().map(|(i, g)| {
            let key = format!("git-reflist-{ix}-{i}");
            pill(format!("{key}-name"), g, colour, palette).relative().child(hit_target(hits, key))
        }))
        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
            this.over_list = if *hovered { Some(ix) } else { this.over_list.filter(|r| *r != ix) };
            cx.notify();
        }))
        .into_any_element()
}

pub fn message(row: &GraphRow, wip: Option<&str>, palette: &Palette) -> AnyElement {
    let (summary, body): (SharedString, SharedString) = match wip {
        Some(counts) => ("// WIP".into(), counts.to_string().into()),
        None => (row.summary.clone(), row.body.clone()),
    };
    div()
        .flex()
        .items_center()
        .gap_2()
        .size_full()
        .overflow_hidden()
        .whitespace_nowrap()
        .child(div().flex_none().text_color(palette.text).child(summary))
        .child(div().flex_none().text_color(palette.muted).child(body))
        .into_any_element()
}

pub fn date_text(row: &GraphRow, now_ms: i64) -> String {
    if row.wip() { String::new() } else { supraman_core::time::short(row.time_ms, now_ms).unwrap_or_default() }
}

pub fn sha_text(row: &GraphRow) -> &str {
    row.sha.get(..7).unwrap_or(&row.sha)
}

pub fn date(row: &GraphRow, now_ms: i64, palette: &Palette) -> AnyElement {
    div().whitespace_nowrap().text_color(palette.muted).child(SharedString::from(date_text(row, now_ms))).into_any_element()
}

pub fn sha(row: &GraphRow, palette: &Palette) -> AnyElement {
    div().whitespace_nowrap().font_family(fonts::MONO).text_color(palette.muted).child(SharedString::from(sha_text(row).to_string())).into_any_element()
}
