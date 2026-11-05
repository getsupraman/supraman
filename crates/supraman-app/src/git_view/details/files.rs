//! A list of changed files, for the commit panel and the WIP panel: Path's flat rows, the Tree, the change
//! marks and folder badges, and the sort button and Path | Tree toggle above them.

use std::rc::Rc;

use gpui::{AnyElement, App, ClickEvent, Context, Div, Hsla, SharedString, Stateful, Window, div, prelude::*, px};
use supraman_gitman::ChangeKind;
use supraman_ui::button;
use supraman_ui::clipped::clipped;
use supraman_ui::hit::{HitTargets, hit_target};
use supraman_ui::icon::Icon;
use supraman_ui::segmented::{Segment, segmented};
use supraman_ui::table::{Column, Table, TableState};
use supraman_ui::tooltip::Tooltip;
use supraman_ui::tree::{Item, Order, Row, TreeList, TreeState};
use theme::ActiveTheme as _;

use super::{Counts, FileView, Mark};

const ROW: f32 = 24.;

/// A change's icon in the theme's status colour.
pub fn mark_icon(kind: ChangeKind, cx: &App) -> (Icon, Hsla) {
    let status = cx.theme().status();
    match kind {
        ChangeKind::Added | ChangeKind::Copied => (Icon::Plus, status.created),
        ChangeKind::Deleted => (Icon::Minus, status.deleted),
        ChangeKind::Renamed => (Icon::Renamed, status.renamed),
        ChangeKind::Modified | ChangeKind::TypeChanged => (Icon::Modified, status.modified),
        ChangeKind::Untracked => (Icon::Untracked, status.created),
        ChangeKind::Unmerged => (Icon::Conflict, status.conflict),
    }
}

pub fn kind_word(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Added | ChangeKind::Copied => "added",
        ChangeKind::Deleted => "deleted",
        ChangeKind::Renamed => "renamed",
        ChangeKind::Modified | ChangeKind::TypeChanged => "modified",
        ChangeKind::Untracked => "untracked",
        ChangeKind::Unmerged => "conflicted",
    }
}

/// A file's mark, painted under `<list>-mark-<path>`; an empty slot of the same width on an unchanged file.
fn mark(mark: &Mark, key: String, hits: &HitTargets, cx: &App) -> Div {
    let slot = div().relative().flex().flex_none().items_center().justify_center().w(px(16.));
    match mark {
        Some(kind) => {
            let (icon, colour) = mark_icon(*kind, cx);
            slot.child(icon.svg(12., colour)).child(hit_target(hits, key))
        }
        None => slot,
    }
}

/// A folder's counts of changes below it, each its icon and number.
fn badges(counts: &Counts, cx: &App) -> Div {
    let text = cx.theme().colors().text_muted;
    div().flex().flex_none().items_center().gap_2().ml_2().text_xs().text_color(text).children(counts.parts().into_iter().map(|(kind, n)| {
        let (icon, colour) = mark_icon(kind, cx);
        div().flex().items_center().gap_0p5().child(icon.svg(11., colour)).child(SharedString::from(n.to_string()))
    }))
}

fn tree_row(list: &str, row: Row<'_, Mark, Counts>, hits: &HitTargets, cx: &App) -> AnyElement {
    let colors = cx.theme().colors().clone();
    match row.item {
        Item::File(m) => div()
            .flex()
            .items_center()
            .gap_1()
            .min_w_0()
            .child(mark(m, format!("{list}-mark-{}", row.path), hits, cx))
            .child(clipped(format!("{list}-file-{}", row.path), row.name.clone(), false).text_color(colors.text))
            .into_any_element(),
        Item::Folder { aggregate, .. } => div()
            .flex()
            .items_center()
            .min_w_0()
            .child(clipped(format!("{list}-folder-{}", row.path), row.name.clone(), false).text_color(colors.text_muted))
            .child(badges(aggregate, cx))
            .into_any_element(),
    }
}

/// A Path row: the mark, the folder part muted and the file name bright.
fn path_row(list: &str, path: &str, kind: ChangeKind, hits: &HitTargets, cx: &App) -> AnyElement {
    let colors = cx.theme().colors().clone();
    let (dir, name) = match path.rsplit_once('/') {
        Some((dir, name)) => (format!("{dir}/"), name.to_string()),
        None => (String::new(), path.to_string()),
    };
    div()
        .flex()
        .items_center()
        .gap_1()
        .w_full()
        .min_w_0()
        .whitespace_nowrap()
        .child(mark(&Some(kind), format!("{list}-mark-{path}"), hits, cx))
        .when(!dir.is_empty(), |d| d.child(clipped(format!("{list}-dir-{path}"), dir, true).flex_shrink(1.).text_color(colors.text_muted)))
        .child(div().flex_none().text_color(colors.text).child(SharedString::from(name)))
        .into_any_element()
}

/// `tree` under `<list>-tree-row-<path>`; a file's or folder's selection goes to `picked`.
pub fn tree_list<V: 'static>(
    list: &'static str,
    tree: &TreeState<Mark, Counts>,
    hits: &HitTargets,
    picked: impl Fn(&mut V, &SharedString, &mut Context<V>) + 'static,
    cx: &mut Context<V>,
) -> Stateful<Div> {
    let painter = hits.clone();
    TreeList::new(format!("{list}-tree"), tree, hits, move |_: &mut V, row, _, cx| tree_row(list, row, &painter, cx))
        .row_height(px(ROW))
        .on_select(move |this: &mut V, path, _, cx| picked(this, path, cx))
        .render(cx)
}

/// `len` files one path a row, `file(ix)` the path and change of a row, under `<list>-files-row-<path>`.
pub fn path_list<V: 'static>(
    list: &'static str,
    state: &TableState,
    hits: &HitTargets,
    len: usize,
    file: impl Fn(usize) -> Option<(String, ChangeKind)> + 'static,
    picked: impl Fn(&mut V, String, &mut Context<V>) + 'static,
    cx: &mut Context<V>,
) -> Stateful<Div> {
    let file = Rc::new(file);
    let (paint, keys, painter) = (file.clone(), file.clone(), hits.clone());
    let column = [Column::new("path", "", px(0.), px(0.)).flexible()];
    Table::new(format!("{list}-files"), state, hits, column, &[px(0.)], len, move |_: &mut V, ix, _, _, cx| {
        paint(ix).map_or_else(|| div().into_any_element(), |(path, kind)| path_row(list, &path, kind, &painter, cx))
    })
    .headless()
    .row_height(px(ROW))
    .row_key(move |ix| keys(ix).map_or_else(String::new, |(path, _)| path))
    .on_select(move |this: &mut V, ix, _, cx| {
        if let Some((path, _)) = file(ix) {
            picked(this, path, cx);
        }
    })
    .render(cx)
}

/// The sort button (`<list>-sort`), turning `order` over on a click.
pub fn sort_button<V: 'static>(hits: &HitTargets, list: &str, order: Order, set: impl Fn(&mut V, Order, &mut Context<V>) + 'static, cx: &mut Context<V>) -> Stateful<Div> {
    let muted = cx.theme().colors().text_muted;
    let (icon, words) = match order {
        Order::Ascending => (Icon::SortDown, "Names A–Z; click for Z–A"),
        Order::Descending => (Icon::SortUp, "Names Z–A; click for A–Z"),
    };
    button::icon(hits, format!("{list}-sort"), icon.svg(14., muted), px(24.), cx)
        .tooltip(Tooltip::text(words))
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| set(this, order.reversed(), cx)))
}

/// Path | Tree under `<list>-view-path` and `<list>-view-tree`.
pub fn view_toggle<V: 'static>(hits: &HitTargets, list: &str, view: FileView, set: impl Fn(&mut V, FileView, &mut Context<V>) + 'static, cx: &mut Context<V>) -> AnyElement {
    let segments = [Segment::new("path").icon(Icon::ListFlat).label("Path"), Segment::new("tree").icon(Icon::ListTree).label("Tree")];
    let chosen = usize::from(view == FileView::Tree);
    let set = Rc::new(set);
    segmented(hits, &format!("{list}-view"), &segments, chosen, move |this: &mut V, i, _: &mut Window, cx| set(this, if i == 0 { FileView::Path } else { FileView::Tree }, cx), cx)
        .into_any_element()
}
