//! A listed folder's row dragged onto a group's row, another listed folder's row or below the tree, a group's row onto a
//! top-level row or below the tree; the room below the tree's rows and its right-click.

use gpui::{AnyElement, App, Axis, Context, Div, MouseButton, MouseDownEvent, Pixels, SharedString, Stateful, Window, div, prelude::*, px};
use supraman_ui::dnd::{self, DropZone, Edge, Sortable};
use supraman_ui::hit::{HitTargets, hit_target};
use theme::ActiveTheme as _;

use super::super::FleetView;
use super::super::folder_edits::{DraggedFolder, DraggedGroup, Target};
use super::{REPO_ROW, grid};
use crate::fleet::FolderNode;

/// Hit key of the drop row below the tree.
pub(crate) const END: &str = "tree-drop-end";

fn folders() -> Sortable<DraggedFolder> {
    Sortable::new("tree-folders", Axis::Vertical)
}

fn groups() -> Sortable<DraggedGroup> {
    Sortable::new("tree-groups", Axis::Vertical)
}

impl FleetView {
    /// A listed folder's row: dragged by its chip, and taking another folder before it; a top-level one a group too.
    pub(super) fn folder_drag(&self, row: Stateful<Div>, folder: &FolderNode, cx: &mut Context<Self>) -> Stateful<Div> {
        let Some(pos) = folder.pos else { return row };
        let key = format!("repo-row-{}", folder.key);
        let dragged = DraggedFolder { pos, path: folder.path.clone(), title: SharedString::from(folder.place()) };
        let dropped = cx.listener(move |this, d: &DraggedFolder, _, cx| this.drop_folder(d, &Target::Folder(pos), cx));
        let row = folders().item(row, key.clone(), move |d: &DraggedFolder, _: &App| (d.pos != pos).then_some(Edge::Before), dropped, cx);
        let row = dnd::source(row, key.clone(), dragged, cx);
        if pos.folder.is_some() {
            return row;
        }
        let dropped = cx.listener(move |this, d: &DraggedGroup, _, cx| this.drop_group(d, &Target::Folder(pos), cx));
        groups().item(row, key, |_: &DraggedGroup, _: &App| Some(Edge::Before), dropped, cx)
    }

    /// A group's row: dragged by its chip; a folder dropped on it goes to the end of its folders, another group before it.
    pub(super) fn group_drag(&self, row: Stateful<Div>, name: &str, cx: &mut Context<Self>) -> Stateful<Div> {
        let key = format!("group-row-{name}");
        let target = Target::Group(name.to_string());
        let dragged = DraggedGroup { name: name.to_string(), title: SharedString::from(name.to_string()) };
        let own = name.to_string();
        let into = {
            let target = target.clone();
            cx.listener(move |this, d: &DraggedFolder, _, cx| this.drop_folder(d, &target, cx))
        };
        let row = DropZone::new(key.clone(), row).take::<DraggedFolder>(|_, _, _| true, into).element();
        let dropped = cx.listener(move |this, d: &DraggedGroup, _, cx| this.drop_group(d, &target, cx));
        let row = groups().item(row, key.clone(), move |d: &DraggedGroup, _: &App| (d.name != own).then_some(Edge::Before), dropped, cx);
        dnd::source(row, key, dragged, cx)
    }

    /// The row below the tree: a folder or group dropped there goes to the end of the top level; its words show while one is dragged.
    pub(super) fn render_drop_end(&self, cx: &mut Context<Self>) -> AnyElement {
        let carried = dnd::carrying::<DraggedFolder>(cx) || dnd::carrying::<DraggedGroup>(cx);
        let text = if carried { cx.theme().colors().text_muted } else { gpui::transparent_black() };
        let row = div()
            .id(END)
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .h(px(REPO_ROW))
            .pl(grid::indent(0) + px(grid::LABEL))
            .text_xs()
            .text_color(text)
            .child(SharedString::from("Move to the top level"))
            .child(hit_target(&self.hits, END.into()));
        DropZone::new(END, row)
            .take::<DraggedFolder>(|_, _, _| true, cx.listener(|this, d: &DraggedFolder, _, cx| this.drop_folder(d, &Target::End, cx)))
            .take::<DraggedGroup>(|_, _, _| true, cx.listener(|this, d: &DraggedGroup, _, cx| this.drop_group(d, &Target::End, cx)))
            .element()
            .into_any_element()
    }

    /// The tree's area below the drop row's top, the empty room under the tree: a folder let go there lands at the top
    /// level's end, and a right-click there opens the Add menu; a drag near its edge scrolls it.
    pub(super) fn tree_room(&self, area: Stateful<Div>, cx: &mut Context<Self>) -> Stateful<Div> {
        let (folder_hits, group_hits) = (self.hits.clone(), self.hits.clone());
        let below = move |hits: &HitTargets, window: &Window| below_rows(hits, window.mouse_position().y);
        let area = DropZone::unmarked(area)
            .take::<DraggedFolder>(move |_, window, _| below(&folder_hits, window), cx.listener(|this, d: &DraggedFolder, _, cx| this.drop_folder(d, &Target::End, cx)))
            .take::<DraggedGroup>(move |_, window, _| below(&group_hits, window), cx.listener(|this, d: &DraggedGroup, _, cx| this.drop_group(d, &Target::End, cx)))
            .element();
        let list = self.sidebar.left_list.state();
        groups()
            .scrolls(folders().scrolls(area, list.clone()), list)
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, e: &MouseDownEvent, _, cx| {
                    if below_rows(&this.hits, e.position.y) {
                        cx.stop_propagation();
                        this.show_add_menu(e.position, cx);
                    }
                }),
            )
    }
}

fn below_rows(hits: &HitTargets, y: Pixels) -> bool {
    hits.borrow().get(END).is_some_and(|h| y >= h.visible.top())
}
