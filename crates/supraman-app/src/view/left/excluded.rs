use gpui::{AnyElement, Context, MouseButton, MouseDownEvent, SharedString, div, prelude::*, px};

use super::{REPO_ROW, disclosure, folder_level, grid, icon_slot, secondary, tree_label};
use crate::fleet::{EntryNode, FolderNode};
use crate::view::FleetView;
use crate::view::menu::MenuTarget;
use supraman_ui::icon::Icon;
use supraman_ui::hit::hit_target;

/// An excluded sub folder's row: shown only with the stopped sessions, this see-through.
pub(super) const OPACITY: f32 = 0.45;

impl FleetView {
    /// A sub folder its parent excludes: no sessions, no actions but its menu's Include Folder.
    pub(super) fn render_excluded_row(&self, entry: &EntryNode, folder: &FolderNode, cx: &mut Context<Self>) -> AnyElement {
        let key = folder.key.clone();
        let tree_key = format!("repo-{key}");
        let icon = if folder.kind == supraman_core::FolderKind::Git { Icon::GitFolder } else { Icon::Folder };
        div()
            .id(SharedString::from(format!("excluded:{key}")))
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(grid::gap())
            .h(px(REPO_ROW))
            .pl(grid::indent(folder_level(entry, folder)))
            .pr_2()
            .text_sm()
            .opacity(OPACITY)
            .child(disclosure(None, secondary(cx)))
            .child(icon_slot(&self.hits, &tree_key, Some(icon.slotted(grid::ICON, secondary(cx)).into_any_element())))
            .child(tree_label(&self.hits, &tree_key, div().flex_none().whitespace_nowrap().child(SharedString::from(folder.name.clone()))))
            .child(hit_target(&self.hits, format!("excluded-row-{key}")))
            .on_mouse_down(MouseButton::Right, cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                this.open_menu(MenuTarget::Folder(key.clone()), e.position, cx);
            }))
            .into_any_element()
    }
}
