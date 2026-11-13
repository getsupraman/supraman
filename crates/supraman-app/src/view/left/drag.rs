//! A listed folder's row dragged onto a group's row, another listed folder's row or below the tree.

use gpui::{AnyElement, Context, Div, SharedString, Stateful, div, prelude::*, px};
use supraman_ui::hit::hit_target;
use theme::ActiveTheme as _;

use super::super::FleetView;
use super::super::folder_edits::{DraggedFolder, Target};
use super::{REPO_ROW, grid};
use crate::fleet::FolderNode;

/// Hit key of the drop row below the tree.
pub(crate) const END: &str = "tree-drop-end";

impl FleetView {
    /// A listed folder's row: dragged by its chip, and taking another folder before it.
    pub(super) fn folder_drag(&self, row: Stateful<Div>, folder: &FolderNode, cx: &mut Context<Self>) -> Stateful<Div> {
        let Some(pos) = folder.pos else { return row };
        let border = cx.theme().colors().drop_target_border;
        let dragged = DraggedFolder { pos, path: folder.path.clone(), title: SharedString::from(folder.place()) };
        row.on_drag(dragged, |d, _, _, cx| cx.new(|_| d.clone()))
            .drag_over::<DraggedFolder>(move |s, d: &DraggedFolder, _, _| if d.pos == pos { s } else { s.border_t_2().border_color(border) })
            .on_drop(cx.listener(move |this, d: &DraggedFolder, _, cx| this.drop_folder(d, &Target::Folder(pos), cx)))
    }

    /// A group's row: a folder dropped on it goes to the end of its folders.
    pub(super) fn group_drop(&self, row: Stateful<Div>, name: &str, cx: &mut Context<Self>) -> Stateful<Div> {
        let colors = cx.theme().colors();
        let (fill, border) = (colors.drop_target_background, colors.drop_target_border);
        let target = Target::Group(name.to_string());
        row.drag_over::<DraggedFolder>(move |s, _, _, _| s.bg(fill).border_1().border_color(border))
            .on_drop(cx.listener(move |this, d: &DraggedFolder, _, cx| this.drop_folder(d, &target, cx)))
    }

    /// The row below the tree: a folder dropped there goes to the end of the top level; its words show only then.
    pub(super) fn render_drop_end(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors();
        let (fill, border, text) = (colors.drop_target_background, colors.drop_target_border, colors.text_muted);
        div()
            .id(END)
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .h(px(REPO_ROW))
            .pl(grid::indent(0) + px(grid::LABEL))
            .text_xs()
            .text_color(gpui::transparent_black())
            .child(SharedString::from("Move to the top level"))
            .child(hit_target(&self.hits, END.into()))
            .drag_over::<DraggedFolder>(move |s, _, _, _| s.bg(fill).border_1().border_color(border).text_color(text))
            .on_drop(cx.listener(|this, d: &DraggedFolder, _, cx| this.drop_folder(d, &Target::End, cx)))
            .into_any_element()
    }

    /// The tree's area: a folder let go below the drop row's top, in the empty room under the tree, lands at the top level's end.
    pub(super) fn tree_drop(&self, area: Stateful<Div>, cx: &mut Context<Self>) -> Stateful<Div> {
        area.on_drop(cx.listener(|this, d: &DraggedFolder, window, cx| {
            let below = this.hits.borrow().get(END).is_some_and(|h| window.mouse_position().y >= h.visible.top());
            if below {
                this.drop_folder(d, &Target::End, cx);
            }
        }))
    }
}
