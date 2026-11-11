//! A list of changed files for the commit and WIP panels: Path's flat rows or the Tree, marks, badges, sort and toggle.

use std::rc::Rc;

use gpui::{AnyElement, App, ClickEvent, Context, Div, Hsla, MouseButton, MouseDownEvent, Pixels, Point, SharedString, Stateful, Window, div, prelude::*, px};
use supraman_gitman::ChangeKind;
use supraman_ui::button;
use supraman_ui::clipped::{clipped, path_label};
use supraman_ui::hit::{HitTargets, hit_target};
use supraman_ui::icon::Icon;
use supraman_ui::ink;
use supraman_ui::segmented::{Segment, segmented};
use supraman_ui::table::{Column, Table, TableState};
use supraman_ui::tooltip::Tooltip;
use supraman_ui::tree::{Item, LEAD, Order, Row, TreeList, TreeState};
use theme::ActiveTheme as _;

use super::{FileView, Mark};
use supraman_gitman::WipCounts;
use supraman_ui::themes::secondary;

const ROW: f32 = 24.;
const ICON: f32 = 12.;
/// A count sits close to its icon, kinds apart, and a folder's counts a fixed step after its name.
const ICON_GAP: f32 = 3.;
const KIND_GAP: f32 = 14.;
const NAME_GAP: f32 = 12.;

/// A file row's buttons at its right end, shown while its row (hover group `group`) is hovered: the owner's for `path`.
pub(crate) type Tail<V> = Rc<dyn Fn(&mut V, &str, ChangeKind, SharedString, &mut Context<V>) -> AnyElement>;

/// A row right-clicked: its path, whether it is a folder, and where.
pub(crate) type OnMenu<V> = Rc<dyn Fn(&mut V, &str, bool, Point<Pixels>, &mut Context<V>)>;

/// `row` taking a right-click for `menu`.
fn menu_row<V: 'static>(row: AnyElement, menu: &Option<OnMenu<V>>, path: &str, folder: bool, cx: &mut Context<V>) -> AnyElement {
    let Some(menu) = menu.clone() else { return row };
    let path = path.to_string();
    div()
        .size_full()
        .flex()
        .items_center()
        .min_w_0()
        .on_mouse_down(MouseButton::Right, cx.listener(move |this: &mut V, e: &MouseDownEvent, _, cx| {
            cx.stop_propagation();
            menu(this, &path, folder, e.position, cx);
        }))
        .child(row)
        .into_any_element()
}

/// A change's icon in the theme's status colour.
pub(crate) fn mark_icon(kind: ChangeKind, cx: &App) -> (Icon, Hsla) {
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

pub(crate) fn kind_word(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Added | ChangeKind::Copied => "added",
        ChangeKind::Deleted => "deleted",
        ChangeKind::Renamed => "renamed",
        ChangeKind::Modified | ChangeKind::TypeChanged => "modified",
        ChangeKind::Untracked => "untracked",
        ChangeKind::Unmerged => "conflicted",
    }
}

/// A file's mark, painted under `<list>-mark-<path>`; none on an unchanged file.
fn mark(mark: &Mark, key: String, hits: &HitTargets, cx: &App) -> Option<Div> {
    let (icon, colour) = mark_icon((*mark)?, cx);
    Some(div().relative().flex().child(icon.svg(ICON, colour)).child(hit_target(hits, key)))
}

/// Each kind's icon and its words: a folder's badges and the panel's summary line.
pub(crate) fn tally(parts: impl IntoIterator<Item = (ChangeKind, String)>, cx: &App) -> Div {
    div().flex().flex_none().items_center().gap(px(KIND_GAP)).text_xs().children(parts.into_iter().map(|(kind, words)| {
        let (icon, colour) = mark_icon(kind, cx);
        div().flex().items_center().gap(px(ICON_GAP)).child(icon.svg(ICON, colour)).child(SharedString::from(words))
    }))
}

/// A closed folder's counts, painted under `<list>-badges-<path>`.
fn badges(counts: &WipCounts, key: String, hits: &HitTargets, cx: &App) -> Div {
    tally(counts.parts().into_iter().map(|(kind, n)| (kind, n.to_string())), cx).relative().ml(px(NAME_GAP)).text_color(cx.theme().colors().text_muted).child(hit_target(hits, key))
}

/// A row's name, painted under `<list>-name-<path>`.
fn name(list: &str, row: &Row<'_, Mark, WipCounts>, colour: Hsla, hits: &HitTargets) -> Stateful<Div> {
    let kind = if matches!(row.item, Item::File(_)) { "file" } else { "folder" };
    clipped(format!("{list}-{kind}-{}", row.path), row.name.clone(), false).text_color(colour).child(hit_target(hits, format!("{list}-name-{}", row.path)))
}

/// A folder's name muted, its badges only while it is closed, and a file's bright.
fn tree_row(list: &str, row: Row<'_, Mark, WipCounts>, hits: &HitTargets, cx: &App) -> AnyElement {
    let row_div = div().flex().items_center().min_w_0();
    match row.item {
        Item::File(_) => row_div.child(name(list, &row, cx.theme().colors().text, hits)).into_any_element(),
        Item::Folder { aggregate, open } => row_div.child(name(list, &row, secondary(cx.theme().colors()), hits)).when(!open, |d| d.child(badges(aggregate, format!("{list}-badges-{}", row.path), hits, cx))).into_any_element(),
    }
}

/// A Path row: the mark, the folder part muted and the file name bright.
fn path_row(list: &str, path: &str, kind: ChangeKind, hits: &HitTargets, cx: &App) -> Div {
    div()
        .flex()
        .items_center()
        .gap_1()
        .w_full()
        .min_w_0()
        .whitespace_nowrap()
        .child(ink::mark(div().flex().flex_none().justify_center().w(px(LEAD)).children(mark(&Some(kind), format!("{list}-mark-{path}"), hits, cx))))
        .child(path_label(format!("{list}-dir-{path}"), path, secondary(cx.theme().colors()), cx.theme().colors().text).flex_1())
}

fn with_tail<V: 'static>(this: &mut V, row: Div, tail: &Option<Tail<V>>, path: &str, kind: ChangeKind, group: &SharedString, cx: &mut Context<V>) -> AnyElement {
    match tail {
        Some(tail) => row.child(div().flex_none().pr_1().child(tail(this, path, kind, group.clone(), cx))).into_any_element(),
        None => row.into_any_element(),
    }
}

/// `tree` under `<list>-tree-row-<path>`: a selection goes to `picked`, a right-click to `menu`, a file row ends with `tail`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn tree_list<V: 'static>(
    list: &'static str,
    tree: &TreeState<Mark, WipCounts>,
    hits: &HitTargets,
    picked: impl Fn(&mut V, &SharedString, &mut Context<V>) + 'static,
    tail: Option<Tail<V>>,
    menu: Option<OnMenu<V>>,
    cx: &mut Context<V>,
) -> Stateful<Div> {
    let (painter, marker) = (hits.clone(), hits.clone());
    let id = format!("{list}-tree");
    let group = supraman_ui::table::row_group(&id);
    TreeList::new(id, tree, hits, move |this: &mut V, row, _, cx| {
        let kind = match row.item {
            Item::File(Some(kind)) => Some(*kind),
            _ => None,
        };
        let (path, folder) = (row.path.to_string(), matches!(row.item, Item::Folder { .. }));
        let painted = match (kind, &tail) {
        (Some(kind), Some(_)) => {
            let path = row.path.to_string();
            let shown = div().flex().items_center().w_full().min_w_0().child(div().flex_1().min_w_0().child(tree_row(list, row, &painter, cx)));
            with_tail(this, shown, &tail, &path, kind, &group, cx)
        }
        _ => tree_row(list, row, &painter, cx),
        };
        menu_row(painted, &menu, &path, folder, cx)
    })
        .row_height(px(ROW))
        .lead(move |_: &mut V, row, _, cx| match row.item {
            Item::File(m) => mark(m, format!("{list}-mark-{}", row.path), &marker, cx).map(IntoElement::into_any_element),
            Item::Folder { .. } => None,
        })
        .on_select(move |this: &mut V, path, _, cx| picked(this, path, cx))
        .render(cx)
}

/// `len` files a path a row (`file(ix)`), under `<list>-files-row-<path>`, each ending with `tail` and right-clicked for `menu`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn path_list<V: 'static>(
    list: &'static str,
    state: &TableState,
    hits: &HitTargets,
    len: usize,
    file: impl Fn(usize) -> Option<(String, ChangeKind)> + 'static,
    picked: impl Fn(&mut V, String, &mut Context<V>) + 'static,
    tail: Option<Tail<V>>,
    menu: Option<OnMenu<V>>,
    cx: &mut Context<V>,
) -> Stateful<Div> {
    let file = Rc::new(file);
    let (paint, keys, painter) = (file.clone(), file.clone(), hits.clone());
    let column = [Column::new("path", "", px(0.), px(0.))];
    let id = format!("{list}-files");
    let group = supraman_ui::table::row_group(&id);
    Table::new(id, state, hits, column, &[px(0.)], len, move |this: &mut V, ix, _, _, cx| {
        paint(ix).map_or_else(|| div().into_any_element(), |(path, kind)| {
            let row = with_tail(this, path_row(list, &path, kind, &painter, cx), &tail, &path, kind, &group, cx);
            menu_row(row, &menu, &path, false, cx)
        })
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
pub(crate) fn sort_button<V: 'static>(hits: &HitTargets, list: &str, order: Order, set: impl Fn(&mut V, Order, &mut Context<V>) + 'static, cx: &mut Context<V>) -> Stateful<Div> {
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
pub(crate) fn view_toggle<V: 'static>(hits: &HitTargets, list: &str, view: FileView, set: impl Fn(&mut V, FileView, &mut Context<V>) + 'static, cx: &mut Context<V>) -> AnyElement {
    let segments = [Segment::new("path").icon(Icon::ListFlat).label("Path"), Segment::new("tree").icon(Icon::ListTree).label("Tree")];
    let chosen = usize::from(view == FileView::Tree);
    let set = Rc::new(set);
    segmented(hits, &format!("{list}-view"), &segments, chosen, move |this: &mut V, i, _: &mut Window, cx| set(this, if i == 0 { FileView::Path } else { FileView::Tree }, cx), cx)
        .into_any_element()
}
