use gpui::{AnyElement, ClickEvent, Context, MouseButton, MouseDownEvent, SharedString, div, prelude::*, px};

use super::{REPO_ROW, disclosure, folder_level, grid, icon_slot, secondary, tree_label};
use crate::fleet::{EntryNode, FolderNode};
use crate::view::FleetView;
use crate::view::menu::MenuTarget;
use supraman_ui::icon::Icon;

/// An excluded sub folder's mark, icon and name: shown only with the stopped sessions, this see-through.
pub(super) const OPACITY: f32 = 0.45;

impl FleetView {
    /// A sub folder its parent excludes: no sessions, nothing to fold; a click or right-click opens its menu (Include Folder).
    pub(super) fn render_excluded_row(&self, entry: &EntryNode, folder: &FolderNode, cx: &mut Context<Self>) -> AnyElement {
        let key = folder.key.clone();
        let tree_key = format!("repo-{key}");
        let icon = if folder.kind == supraman_core::FolderKind::Git { Icon::GitFolder } else { Icon::Folder };
        let menu_key = key.clone();
        let keys = supraman_ui::button::RowKeys { id: format!("excluded:{key}").into(), group: format!("excluded-group-{key}").into(), hit: format!("excluded-row-{key}").into() };
        self.sidebar_row(keys, Some(&super::fold::folder(&key)), cx)
            .flex()
            .flex_none()
            .items_center()
            .gap(grid::gap())
            .h(px(REPO_ROW))
            .pl(grid::indent(folder_level(entry, folder)))
            .pr_2()
            .text_sm()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(grid::gap())
                    .opacity(OPACITY)
                    .child(disclosure(None, secondary(cx)))
                    .child(icon_slot(&self.hits, &tree_key, Some(icon.slotted(grid::ICON, secondary(cx)).into_any_element())))
                    .child(tree_label(&self.hits, &tree_key, div().flex_none().whitespace_nowrap().child(SharedString::from(folder.name.clone())))),
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.open_menu(MenuTarget::Folder(menu_key.clone()), Some(format!("excluded-row-{menu_key}")), window.mouse_position(), cx);
            }))
            .on_mouse_down(MouseButton::Right, cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                this.open_menu(MenuTarget::Folder(key.clone()), Some(format!("excluded-row-{key}")), e.position, cx);
            }))
            .into_any_element()
    }
}
