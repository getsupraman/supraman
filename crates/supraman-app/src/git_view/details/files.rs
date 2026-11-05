//! The commit panel's file list: Path's flat rows, the Tree, and the change marks and folder badges.

use std::rc::Rc;

use gpui::{AnyElement, App, Context, Div, Hsla, SharedString, div, prelude::*, px};
use supraman_gitman::ChangeKind;
use supraman_ui::clipped::clipped;
use supraman_ui::hit::{HitTargets, hit_target};
use supraman_ui::icon::Icon;
use supraman_ui::table::{Column, Table};
use supraman_ui::tree::{Item, Row, TreeList};
use theme::ActiveTheme as _;

use super::{CommitPanel, Counts, Mark, Shown};

const ROW: f32 = 24.;

/// A change's icon in the theme's status colour.
pub fn mark_icon(kind: ChangeKind, cx: &App) -> (Icon, Hsla) {
    let status = cx.theme().status();
    match kind {
        ChangeKind::Added | ChangeKind::Copied => (Icon::Plus, status.created),
        ChangeKind::Deleted => (Icon::Minus, status.deleted),
        ChangeKind::Renamed => (Icon::Renamed, status.renamed),
        ChangeKind::Modified | ChangeKind::TypeChanged | ChangeKind::Unmerged => (Icon::Modified, status.modified),
    }
}

pub fn kind_word(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Added | ChangeKind::Copied => "added",
        ChangeKind::Deleted => "deleted",
        ChangeKind::Renamed => "renamed",
        ChangeKind::Modified | ChangeKind::TypeChanged | ChangeKind::Unmerged => "modified",
    }
}

/// A file's mark, painted under `commit-mark-<path>`; an empty slot of the same width on an unchanged file.
fn mark(mark: &Mark, path: &str, hits: &HitTargets, cx: &App) -> Div {
    let slot = div().relative().flex().flex_none().items_center().justify_center().w(px(16.));
    match mark {
        Some(kind) => {
            let (icon, colour) = mark_icon(*kind, cx);
            slot.child(icon.svg(12., colour)).child(hit_target(hits, format!("commit-mark-{path}")))
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

fn tree_row(this: &mut CommitPanel, row: Row<'_, Mark, Counts>, cx: &mut Context<CommitPanel>) -> AnyElement {
    let colors = cx.theme().colors().clone();
    match row.item {
        Item::File(m) => div()
            .flex()
            .items_center()
            .gap_1()
            .min_w_0()
            .child(mark(m, row.path, &this.hits, cx))
            .child(clipped(format!("commit-file-{}", row.path), row.name.clone(), false).text_color(colors.text))
            .into_any_element(),
        Item::Folder { aggregate, .. } => div()
            .flex()
            .items_center()
            .min_w_0()
            .child(clipped(format!("commit-folder-{}", row.path), row.name.clone(), false).text_color(colors.text_muted))
            .child(badges(aggregate, cx))
            .into_any_element(),
    }
}

/// A Path row: the mark, the folder part muted and the file name bright.
fn path_row(change: &supraman_gitman::FileChange, hits: &HitTargets, cx: &App) -> AnyElement {
    let colors = cx.theme().colors().clone();
    let (dir, name) = match change.path.rsplit_once('/') {
        Some((dir, name)) => (format!("{dir}/"), name.to_string()),
        None => (String::new(), change.path.clone()),
    };
    div()
        .flex()
        .items_center()
        .gap_1()
        .w_full()
        .min_w_0()
        .whitespace_nowrap()
        .child(mark(&Some(change.kind), &change.path, hits, cx))
        .when(!dir.is_empty(), |d| d.child(clipped(format!("commit-dir-{}", change.path), dir, true).flex_shrink(1.).text_color(colors.text_muted)))
        .child(div().flex_none().text_color(colors.text).child(SharedString::from(name)))
        .into_any_element()
}

impl CommitPanel {
    /// The tree on show, with View all files the whole one; rows under `commit-tree-row-<path>`.
    pub(super) fn render_tree(&self, cx: &mut Context<Self>) -> AnyElement {
        let (tree, _) = self.tree();
        TreeList::new("commit-tree", tree, &self.hits, |this: &mut CommitPanel, row, _, cx| tree_row(this, row, cx))
            .row_height(px(ROW))
            .on_select(|this: &mut CommitPanel, path, _, cx| this.file_clicked(path, cx))
            .render(cx)
            .flex_1()
            .min_h_0()
            .into_any_element()
    }

    /// The changed files, one path a row, under `commit-files-row-<path>`.
    pub(super) fn render_paths(&self, shown: &Rc<Shown>, cx: &mut Context<Self>) -> AnyElement {
        let (paint, keys, picked) = (shown.clone(), shown.clone(), shown.clone());
        let column = [Column::new("path", "", px(0.), px(0.)).flexible()];
        Table::new("commit-files", &self.paths, &self.hits, column, &[px(0.)], shown.changes.len(), move |this: &mut CommitPanel, ix, _, _, cx| {
            paint.changes.get(ix).map_or_else(|| div().into_any_element(), |c| path_row(c, &this.hits, cx))
        })
        .headless()
        .row_height(px(ROW))
        .row_key(move |ix| keys.changes.get(ix).map_or_else(String::new, |c| c.path.clone()))
        .on_select(move |this: &mut CommitPanel, ix, _, cx| {
            if let Some(c) = picked.changes.get(ix) {
                this.file_clicked(&c.path.clone().into(), cx);
            }
        })
        .render(cx)
        .flex_1()
        .min_h_0()
        .into_any_element()
    }
}
