//! A workspace's folder list: each folder with its home and remove buttons, and a row that adds a path.

use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use gpui::{ClickEvent, Context, Div, Entity, SharedString, Window, div, prelude::*, px};
use supraman_core::paths::path_key;
use supraman_core::{Folder, Workspace, owners};
use supraman_ui::button;
use supraman_ui::clipped::clipped;
use supraman_ui::hit::{HitTargets, hit_target};
use supraman_ui::icon::Icon;
use supraman_ui::text_input::{TEXT_LEFT, TextInput};
use supraman_ui::tooltip::Tooltip;
use theme::ActiveTheme as _;

use super::left::{secondary, tag};

pub const HOME_TIP: &str = "Home: the workspace's own session runs here";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderRow {
    pub folder: Folder,
    /// `Some` only in a parent folder's list, where an unticked folder is left out.
    pub included: Option<bool>,
    pub home: bool,
    /// The workspace that supervises this folder.
    pub elsewhere: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowAction {
    Include(usize),
    Home(usize),
    Remove(usize),
    Add,
    Browse,
}

/// The rows, the folders removed from them, and who supervises each folder elsewhere ([`owners`]).
#[derive(Debug, Clone, Default)]
pub struct FolderList {
    pub rows: Vec<FolderRow>,
    pub removed: Vec<Folder>,
    /// Folders a Rescan found gone from disk, taken out of the rows; Save drops them.
    pub gone: Vec<String>,
    /// With no row marked, the first row is home.
    pub first_is_home: bool,
    owners: HashMap<String, String>,
}

impl FolderList {
    /// `others`: every workspace but the one this list is of.
    pub fn new(others: &[Workspace], first_is_home: bool) -> Self {
        Self { rows: Vec::new(), removed: Vec::new(), gone: Vec::new(), first_is_home, owners: owners(others) }
    }

    pub fn of_workspace(w: &Workspace, others: &[Workspace]) -> Self {
        let mut list = Self::new(others, w.home.is_none());
        for f in &w.folders {
            list.push(f.clone(), None);
        }
        if let Some(home) = &w.home {
            list.mark_home(home);
        }
        list
    }

    /// False when the list already holds `folder`'s path.
    pub fn push(&mut self, folder: Folder, included: Option<bool>) -> bool {
        if self.rows.iter().any(|r| path_key(&r.folder.path) == path_key(&folder.path)) {
            return false;
        }
        let elsewhere = self.owners.get(&path_key(&folder.path)).cloned();
        self.rows.push(FolderRow { folder, included, home: false, elsewhere });
        self.settle_home();
        true
    }

    pub fn mark_home(&mut self, path: &std::path::Path) {
        for r in &mut self.rows {
            r.home = path_key(&r.folder.path) == path_key(path);
        }
        self.settle_home();
    }

    pub fn toggle(&mut self, i: usize) {
        if let Some(on) = self.rows.get_mut(i).and_then(|r| r.included.as_mut()) {
            *on = !*on;
        }
    }

    /// Row `i` becomes home; a home row clicked again gives home back to the default.
    pub fn set_home(&mut self, i: usize) {
        let Some(was) = self.rows.get(i).map(|r| r.home) else {
            return;
        };
        for (j, r) in self.rows.iter_mut().enumerate() {
            r.home = j == i && !was;
        }
        self.settle_home();
    }

    pub fn remove(&mut self, i: usize) {
        if i < self.rows.len() {
            let row = self.rows.remove(i);
            self.removed.push(row.folder);
            self.settle_home();
        }
    }

    pub fn drop_gone(&mut self, names: &[String]) {
        self.rows.retain(|r| !names.contains(&r.folder.name));
        self.gone.extend(names.iter().cloned());
        self.settle_home();
    }

    fn settle_home(&mut self) {
        if self.first_is_home && !self.rows.iter().any(|r| r.home) {
            if let Some(first) = self.rows.first_mut() {
                first.home = true;
            }
        }
    }

    pub fn home_index(&self) -> Option<usize> {
        self.rows.iter().position(|r| r.home)
    }

    pub fn folders(&self) -> Vec<Folder> {
        self.rows.iter().filter(|r| r.included != Some(false)).map(|r| r.folder.clone()).collect()
    }

    pub fn unticked(&self) -> impl Iterator<Item = &Folder> {
        self.rows.iter().filter(|r| r.included == Some(false)).map(|r| &r.folder)
    }
}

/// The path typed in the add row, unquoted; `None` when empty.
pub fn typed(path: &Entity<TextInput>, cx: &gpui::App) -> Option<PathBuf> {
    let text = path.read(cx).text().trim().trim_matches('"').to_string();
    (!text.is_empty()).then(|| PathBuf::from(text))
}

pub type OnRow<V> = Rc<dyn Fn(&mut V, RowAction, &mut Window, &mut Context<V>)>;

/// The rows and the add row; every control paints under `<prefix>-…` and runs `on` with its [`RowAction`].
pub fn render<V: 'static>(
    list: &FolderList,
    path: &Entity<TextInput>,
    prefix: &str,
    hits: &HitTargets,
    on: OnRow<V>,
    cx: &mut Context<V>,
) -> Div {
    let mut out = div().flex().flex_col().gap(px(2.)).min_w_0();
    for (i, row) in list.rows.iter().enumerate() {
        let click = (act(&on, RowAction::Include(i), cx), act(&on, RowAction::Home(i), cx), act(&on, RowAction::Remove(i), cx));
        out = out.child(row_element(row, prefix, hits, click, cx));
    }
    let empty = typed(path, cx).is_none();
    let field = div()
        .id(SharedString::from(format!("{prefix}-new-path")))
        .relative()
        .flex_1()
        .min_w_0()
        .child(path.clone())
        .child(hit_target(hits, format!("{prefix}-new-path")))
        .child(div().absolute().top_0().bottom_0().left(TEXT_LEFT).right(TEXT_LEFT).child(hit_target(hits, format!("{prefix}-new-path-text"))));
    let browse = button::bordered(hits, format!("{prefix}-browse"), "Browse", cx).on_click(act(&on, RowAction::Browse, cx));
    let add = button::bordered(hits, format!("{prefix}-add"), "Add", cx).on_click(act(&on, RowAction::Add, cx));
    let add = if empty { button::dimmed(add) } else { add };
    out.child(div().flex().items_center().gap_1().min_w_0().pt_1().child(field).child(browse).child(add))
}

fn act<V: 'static>(on: &OnRow<V>, action: RowAction, cx: &mut Context<V>) -> impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static {
    let on = on.clone();
    cx.listener(move |this: &mut V, _: &ClickEvent, window, cx| {
        cx.stop_propagation();
        on(this, action, window, cx);
    })
}

fn row_element<V: 'static, F: Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static>(
    row: &FolderRow,
    prefix: &str,
    hits: &HitTargets,
    (include, home, remove): (F, F, F),
    cx: &mut Context<V>,
) -> impl IntoElement {
    let colors = cx.theme().colors().clone();
    let f = &row.folder;
    let off = row.included == Some(false);
    let named = div()
        .flex()
        .items_center()
        .gap_2()
        .min_w_0()
        .children(row.included.map(|on| button::checkbox(hits, format!("{prefix}-include-{}", f.name), on, cx).on_click(include)))
        .child(clipped(format!("{prefix}-name-{}", f.name), f.name.clone(), false).when(off, |d| d.text_color(secondary(cx))))
        .children(f.branch.clone().map(|b| clipped(format!("{prefix}-branch-{}", f.name), format!("({b})"), false).flex_shrink(8.).text_xs().text_color(secondary(cx))))
        .children(row.elsewhere.as_ref().map(|owner| {
            tag(format!("supervised in {owner}"), cx)
                .relative()
                .flex_shrink(1.)
                .min_w_0()
                .overflow_hidden()
                .child(hit_target(hits, format!("{prefix}-elsewhere-{}", f.name)))
        }));
    let path = clipped(format!("{prefix}-folder-path-{}", f.name), f.path.display().to_string(), true).text_xs().text_color(colors.text_muted);
    let home_color = if row.home { colors.text_accent } else { colors.text_muted };
    let home_button = button::icon(hits, format!("{prefix}-home-{}", f.name), Icon::Home.svg(12., home_color), px(20.), cx)
        .when(row.home, |d| d.bg(colors.ghost_element_selected))
        .tooltip(Tooltip::text(HOME_TIP))
        .on_click(home);
    let remove_button =
        button::icon(hits, format!("{prefix}-remove-{}", f.name), Icon::Close.svg(12., colors.text_muted), px(20.), cx).tooltip(Tooltip::text("Remove")).on_click(remove);
    div()
        .id(SharedString::from(format!("{prefix}-folder-{}", f.name)))
        .relative()
        .flex()
        .items_center()
        .gap_2()
        .min_w_0()
        .px_1()
        .text_sm()
        .child(hit_target(hits, format!("{prefix}-folder-{}", f.name)))
        .child(div().flex().flex_col().flex_1().min_w_0().child(named).child(div().flex().min_w_0().when(row.included.is_some(), |d| d.pl_5()).child(path)))
        .child(div().flex().flex_none().items_center().gap_1().child(home_button).child(remove_button))
}

#[cfg(test)]
mod tests {
    use supraman_core::FolderKind;

    use super::*;

    fn folder(name: &str) -> Folder {
        Folder { kind: FolderKind::Plain, ..Folder::new(name, format!(r"C:\P\{name}")) }
    }

    #[test]
    fn the_first_row_is_home_until_another_is_marked_and_a_mark_clicked_again_gives_it_back() {
        let mut list = FolderList::new(&[], true);
        assert!(list.push(folder("a"), None) && list.push(folder("b"), None));
        assert!(!list.push(folder("a"), None), "a path once");
        assert_eq!(list.home_index(), Some(0));
        list.set_home(1);
        assert_eq!(list.home_index(), Some(1));
        list.set_home(1);
        assert_eq!(list.home_index(), Some(0));
        list.set_home(1);
        list.remove(1);
        assert_eq!((list.home_index(), list.removed.len()), (Some(0), 1), "the home row removed: the first again");
    }

    #[test]
    fn a_parent_list_has_no_home_row_until_one_is_marked_and_leaves_unticked_folders_out() {
        let mut list = FolderList::new(&[], false);
        list.push(folder("a"), Some(true));
        list.push(folder("b"), Some(true));
        assert_eq!(list.home_index(), None);
        list.toggle(1);
        assert_eq!(list.folders(), [folder("a")]);
        assert_eq!(list.unticked().cloned().collect::<Vec<_>>(), [folder("b")]);
        list.set_home(0);
        list.set_home(0);
        assert_eq!(list.home_index(), None);
    }

    #[test]
    fn a_row_names_the_workspace_that_supervises_its_folder() {
        let other = Workspace::new("larchsys", r"C:\P", vec![folder("a"), Folder { enabled: false, ..folder("b") }]);
        let mut w = Workspace::new("new", r"C:\P\b", vec![folder("a"), folder("b")]);
        let list = FolderList::of_workspace(&w, std::slice::from_ref(&other));
        let said: Vec<Option<&str>> = list.rows.iter().map(|r| r.elsewhere.as_deref()).collect();
        assert_eq!(said, [Some("larchsys"), None]);
        assert_eq!(list.home_index(), Some(1));
        w.home = None;
        assert_eq!(FolderList::of_workspace(&w, &[]).home_index(), Some(0));
    }
}
