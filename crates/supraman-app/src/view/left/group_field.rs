//! A group's row edited in place: Rename… and F2 on the selected group's row, Add Group's new row at the top level's end.

use gpui::{AnyElement, AppContext as _, ClickEvent, Context, Entity, FontWeight, KeyBinding, MouseButton, SharedString, Subscription, Window, div, prelude::*, px};
use supraman_ui::hit::hit_target;
use supraman_ui::icon::Icon;
use supraman_ui::text_input;
use supraman_ui::tooltip::Tooltip;
use theme::ActiveTheme as _;

use super::super::folder_edits::{FIELD, FolderEdit, GroupName, GroupNameEvent};
use super::{FleetView, PendingReveal, TOP_ROW, disclosure, grid, icon_slot, top_text};

gpui::actions!(sidebar, [RenameSelected]);

/// The sidebar's key context: F2.
pub const KEY_CONTEXT: &str = "Sidebar";
/// Add Group's new row's key in the tree.
pub const NEW_ROW: &str = "group-new";
const HINT: &str = "Enter names the group, Esc cancels";

pub fn bind_keys(cx: &mut gpui::App) {
    cx.bind_keys([KeyBinding::new("f2", RenameSelected, Some(KEY_CONTEXT))]);
}

/// The group name being typed in place, and the owner's subscription to it.
pub struct GroupEdit {
    pub name: Entity<GroupName>,
    _sub: Subscription,
}

impl FleetView {
    /// Add Group's new row with an empty field, or Rename on `renaming`'s row with its name selected.
    pub fn open_group_name(&mut self, renaming: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let taken = self.listed.entries.groups().map(|(n, _)| n.to_string()).collect();
        let row = renaming.as_ref().map_or_else(|| NEW_ROW.to_string(), |n| super::fold::group(n));
        let name = cx.new(|cx| GroupName::new(renaming, taken, window, cx));
        let sub = cx.subscribe_in(&name, window, |this, _, e: &GroupNameEvent, window, cx| this.group_named(e, window, cx));
        self.sidebar.group_edit = Some(GroupEdit { name, _sub: sub });
        self.reveal_new(PendingReveal::Row(row));
        cx.notify();
    }

    fn group_named(&mut self, e: &GroupNameEvent, window: &mut Window, cx: &mut Context<Self>) {
        let edit = match e {
            GroupNameEvent::Refused(why) => return self.toast.show_why(why.clone(), |v: &mut Self| &mut v.toast, cx),
            GroupNameEvent::Named(edit) => Some(edit.clone()),
            GroupNameEvent::Closed => None,
        };
        let held = self.sidebar.group_edit.take().is_some_and(|g| g.name.read(cx).field().read(cx).is_focused(window));
        if held {
            self.sidebar.focus.focus(window, cx);
        }
        if let Some(edit) = edit {
            if let FolderEdit::AddGroup(to) | FolderEdit::RenameGroup { to, .. } = &edit {
                self.sidebar.left_list.select_key(Some(super::fold::group(to).into()));
            }
            self.edit_folders(edit, cx);
        }
        cx.notify();
    }

    pub(super) fn rename_selected(&mut self, _: &RenameSelected, window: &mut Window, cx: &mut Context<Self>) {
        let Some(name) = self.selected_group() else { return };
        if self.listed.entries.groups().any(|(n, _)| n == name) {
            self.open_group_name(Some(name), window, cx);
        }
    }

    /// The row whose name is typed in place: `Some(None)` Add Group's new row, `Some(Some(group))` a rename.
    #[cfg(feature = "selftest")]
    pub fn group_editing(&self, cx: &gpui::App) -> Option<Option<String>> {
        self.sidebar.group_edit.as_ref().map(|g| g.name.read(cx).renaming().map(str::to_string))
    }

    /// What the field in place holds.
    #[cfg(feature = "selftest")]
    pub fn group_name_typed(&self, cx: &gpui::App) -> Option<String> {
        self.sidebar.group_edit.as_ref().map(|g| g.name.read(cx).field().read(cx).text().to_string())
    }

    /// Whether Add Group's new row shows.
    pub(super) fn adding_group(&self, cx: &gpui::App) -> bool {
        self.sidebar.group_edit.as_ref().is_some_and(|g| g.name.read(cx).renaming().is_none())
    }

    /// The field in the name's place on `renaming`'s row (`None`: the new row's), while that row is edited.
    pub(super) fn group_name_field(&self, renaming: Option<&str>, cx: &gpui::App) -> Option<AnyElement> {
        let edit = self.sidebar.group_edit.as_ref().filter(|g| g.name.read(cx).renaming() == renaming)?;
        let name = edit.name.clone();
        let field = name.read(cx).field().clone();
        Some(
            div()
                .id(FIELD)
                .relative()
                .flex_1()
                .min_w_0()
                .ml(-text_input::INLINE_TEXT_LEFT)
                .h(px(TOP_ROW - 4.))
                .font_weight(FontWeight::BOLD)
                .child(field)
                .child(hit_target(&self.hits, FIELD.into()))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(|_: &ClickEvent, _, cx| cx.stop_propagation())
                .tooltip(move |window, cx| {
                    let why = name.read(cx).shown_why(cx).unwrap_or_else(|| HINT.into());
                    Tooltip::text(why)(window, cx)
                })
                .into_any_element(),
        )
    }

    /// Add Group's row at the top level's end: a group's row holding the field.
    pub(super) fn render_new_group_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let line = div()
            .flex()
            .items_center()
            .gap(grid::gap())
            .h(px(TOP_ROW))
            .child(disclosure(None, colors.text_muted))
            .child(icon_slot(&self.hits, NEW_ROW, Some(Icon::Group.slotted(grid::ICON, colors.text_muted).into_any_element())))
            .children(self.group_name_field(None, cx));
        div().id(SharedString::from(NEW_ROW)).flex().flex_col().flex_none().pl(grid::indent(0)).pr_2().map(top_text).child(line).into_any_element()
    }
}
