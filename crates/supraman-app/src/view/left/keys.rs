//! The sidebar on the list core: its rows' chrome and its one selection, moved by the list keys, Enter opening or
//! folding the selected row.

use gpui::{Context, Div, KeyDownEvent, MouseButton, MouseDownEvent, SharedString, Stateful, Window, prelude::*};
use supraman_ui::button::RowKeys;
use supraman_ui::list::{self, Chrome};

use super::{LeftRow, fold, other_schedules, unlisted};
use crate::fleet;
use crate::view::FleetView;

impl FleetView {
    /// A sidebar row painted by the list core: selected while the sidebar's selection is `row`, a press selecting it.
    pub(in crate::view) fn sidebar_row(&self, keys: RowKeys, row: Option<&str>, cx: &mut Context<Self>) -> Stateful<Div> {
        self.sidebar_row_with(keys, row, Chrome::new(), cx)
    }

    /// [`FleetView::sidebar_row`] over `chrome`'s band and hover.
    pub(in crate::view) fn sidebar_row_with(&self, keys: RowKeys, row: Option<&str>, chrome: Chrome, cx: &mut Context<Self>) -> Stateful<Div> {
        let selected = row.is_some_and(|r| self.sidebar.left_list.is_selected(r));
        let el = list::row(&self.hits, keys, sidebar_chrome(chrome, selected, self.sidebar.keys_held), cx);
        let Some(row) = row.map(|r| SharedString::from(r.to_string())) else { return el };
        el.on_mouse_down(MouseButton::Left, cx.listener(move |this, _: &MouseDownEvent, _, cx| {
            this.sidebar.left_list.select_key(Some(row.clone()));
            cx.notify();
        }))
    }

    /// The group whose row is selected: F2 renames it.
    pub(super) fn selected_group(&self) -> Option<String> {
        self.sidebar.left_list.selected_key()?.strip_prefix(&fold::group("")).map(str::to_string)
    }

    /// A key while the sidebar itself holds the keys (not its search box or a name typed in place).
    pub(super) fn sidebar_key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.sidebar.focus.is_focused(window) || self.search_query(cx).len() > 0 {
            return;
        }
        let Some(key) = list::key_of(&e.keystroke) else { return };
        cx.stop_propagation();
        let rows = self.sidebar.left_rows.clone();
        if key == list::Key::Activate {
            let at = self.sidebar.left_list.selected_key().and_then(|k| self.sidebar.left_list.index_of(k));
            if let Some(row) = at.and_then(|ix| rows.1.get(ix).copied()) {
                self.activate_left(&rows.0, row, cx);
            }
            return;
        }
        self.sidebar.left_list.set_selectable(rows.1.iter().map(|r| !matches!(r, LeftRow::NewGroup | LeftRow::DropEnd)).collect());
        let texts: Vec<String> = match key {
            list::Key::Typed(_) => rows.1.iter().map(|r| row_text(&rows.0, *r)).collect(),
            _ => Vec::new(),
        };
        self.sidebar.left_list.key(&key, |ix| texts.get(ix).cloned().unwrap_or_default());
        cx.notify();
    }

    /// Enter on a row: a session opens, a group or folder folds or opens, More shows the closed sessions.
    fn activate_left(&mut self, tree: &fleet::Tree, row: LeftRow, cx: &mut Context<Self>) {
        match row {
            LeftRow::Group(e) => self.toggle(fold::group(&tree.entries[e].name), cx),
            LeftRow::Folder(e, f) if !tree.entries[e].folders[f].excluded => self.toggle(fold::folder(&tree.entries[e].folders[f].key), cx),
            LeftRow::Schedules(e, f) => self.toggle(format!("schedules:{}", supraman_core::schedule::folder_key(&tree.entries[e].folders[f].path)), cx),
            LeftRow::Session(e, f, i) => {
                let folder = &tree.node(e).folders[f];
                let row = &folder.sessions[i];
                self.open_row(&folder.row_place(row).replace('/', " · "), row, cx);
            }
            LeftRow::More(e, f) => {
                self.sidebar.closed_expanded.insert(tree.node(e).folders[f].key.clone());
                self.invalidate_tree();
                cx.notify();
            }
            LeftRow::Unlisted => self.toggle(unlisted::KEY.to_string(), cx),
            LeftRow::Others => self.toggle(other_schedules::KEY.to_string(), cx),
            LeftRow::Folder(..) | LeftRow::NewGroup | LeftRow::DropEnd => {}
        }
    }
}

/// A sidebar row's chrome: selected while the sidebar's selection is on it, the active selection while the sidebar holds the keys.
pub(in crate::view) fn sidebar_chrome(chrome: Chrome, selected: bool, held: bool) -> Chrome {
    chrome.selected(selected).focused(held)
}

/// A schedule row's chrome: a pending schedule's warning tint at rest.
pub(in crate::view) fn schedule_chrome(pending: bool, cx: &gpui::App) -> Chrome {
    use theme::ActiveTheme as _;
    Chrome::new().band(pending.then(|| cx.theme().status().warning.opacity(PENDING_TINT)))
}

const PENDING_TINT: f32 = 0.08;

/// A row's type-ahead text: its name.
fn row_text(tree: &fleet::Tree, row: LeftRow) -> String {
    match row {
        LeftRow::Group(e) => tree.entries[e].name.clone(),
        LeftRow::Folder(e, f) => tree.entries[e].folders[f].name.clone(),
        LeftRow::Session(e, f, i) => fleet::display_name(&tree.node(e).folders[f].sessions[i].session).to_string(),
        LeftRow::Schedules(..) => "Schedules".into(),
        LeftRow::Unlisted => fleet::UNLISTED_TITLE.into(),
        LeftRow::Others => other_schedules::TITLE.into(),
        LeftRow::More(..) | LeftRow::NewGroup | LeftRow::DropEnd => String::new(),
    }
}

#[cfg(test)]
mod tests;
