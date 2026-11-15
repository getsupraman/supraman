//! `--selftest` step 7b's folder rows: one dragged into a group and back out to the top level, the Add menu on a
//! right-click below the tree, then its sub folders shown, followed as they come and go, and hidden. A drag needs the window's own mouse dispatch, its drag state and the
//! rows' drop listeners, which only a running FleetView has.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, bail};
use gpui::{AsyncWindowContext, Entity, Modifiers, MouseButton, point};
use supraman_core::folders::{self, Entries, Entry};

use crate::selftest::{click, key, press_at, scrolled_into_view, until};
use crate::selftest_manage::{menu_open, press_painted, shot};
use crate::selftest_tabs::drag_from;
use crate::view::FleetView;
use crate::view::folder_edits::FolderEdit;
const END: &str = "tree-drop-end";
const UNLISTED: (&str, &str) = ("unlisted-row", "tree-label-unlisted");

const GROUP: &str = "selftest-drag";
const WITHIN: Duration = Duration::from_secs(10);
const TOP: f32 = crate::view::TOP_ROW;

/// The top level's folders and the group's, as folders.toml holds them.
fn places(entries: &Entries) -> (Vec<PathBuf>, Option<Vec<PathBuf>>) {
    let top = entries.list.iter().filter_map(|e| match e {
        Entry::Folder(f) => Some(f.path.clone()),
        Entry::Group { .. } => None,
    });
    let group = entries.groups().find(|(n, _)| *n == GROUP).map(|(_, f)| f.iter().map(|f| f.path.clone()).collect());
    (top.collect(), group)
}


/// `root`'s row (a top-level folder named `name`) dragged onto a new group's row, then from there below the tree.
pub(crate) async fn drags(view: &Entity<FleetView>, root: &Path, name: &str, home: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    view.update(cx, |v, cx| v.edit_folders(FolderEdit::AddGroup(GROUP.into()), cx));
    let group_row = format!("group-row-{GROUP}");
    until(view, cx, WITHIN, "the new group's row", |v| v.shown(&group_row).then_some(())).await?;
    let onto = scrolled_into_view(view, &group_row, cx).await?.center();
    let rhythm = top_rhythm(view, &[(&group_row, &format!("tree-label-group-{GROUP}")), (&format!("repo-row-{name}"), &format!("tree-label-repo-{name}"))], cx).await?;
    drag_from(view, &format!("repo-row-{name}"), onto, Some("folder-drag"), cx).await?;
    let member = format!("repo-row-{GROUP}/{name}");
    until(view, cx, WITHIN, "the folder's row in the group after the drop", |v| v.shown(&member).then_some(())).await?;
    let (top, group) = places(&folders::load(home)?);
    if top.iter().any(|p| p == root) || group.as_deref() != Some(&[root.to_path_buf()][..]) {
        bail!("dropped on {GROUP}'s row: folders.toml's top level {top:?}, {GROUP} {group:?}; expected {} moved into {GROUP}", root.display());
    }

    let end = scrolled_into_view(view, END, cx).await?.center();
    drag_from(view, &member, end, None, cx).await?;
    let top_row = format!("repo-row-{name}");
    until(view, cx, WITHIN, "the folder's row at the top level after the drop below the tree", |v| (v.shown(&top_row) && !v.shown(&member)).then_some(())).await?;
    let (top, group) = places(&folders::load(home)?);
    if top.last().map(PathBuf::as_path) != Some(root) || group.as_ref().is_none_or(|g| !g.is_empty()) {
        bail!("dropped below the tree: folders.toml's top level {top:?}, {GROUP} {group:?}; expected {} last at the top level", root.display());
    }
    view.update(cx, |v, cx| v.edit_folders(FolderEdit::RemoveGroup { name: GROUP.into(), with_folders: true }, cx));
    until(view, cx, WITHIN, "the group gone from the tree, so the rows' positions are folders.toml's", |v| v.tree().entry(GROUP).is_none().then_some(())).await?;
    Ok(format!("{rhythm}; {name} dragged onto {GROUP}'s row: into {GROUP}; then below the tree: the top level's end; folders.toml checked after each"))
}

/// A right-click on the drop row, then on the empty room under it, each opens the Add menu at the pointer. The room has no
/// hit key and the listener is the tree area's, so only the window's own mouse dispatch over a painted FleetView reaches it.
pub(crate) async fn room_menu(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let end = scrolled_into_view(view, END, cx).await?;
    let area = view.read_with(cx, |v, _| v.hits.borrow().get("tree-area").map(|h| h.visible)).ok_or_else(|| anyhow!("the tree area is not painted"))?;
    let room = area.bottom() - end.bottom();
    if f32::from(room) < 8. {
        bail!("{} px of room under the drop row; the step needs some to right-click", f32::from(room));
    }
    let below = point(end.center().x, end.bottom() + room / 2.);
    let mut out = Vec::new();
    for (place, at) in [("the drop row", end.center()), ("the room below it", below)] {
        crate::selftest::drawn(cx).await?;
        press_at(at, MouseButton::Right, 1, cx)?;
        let keys = menu_open(view, "Add", cx).await.map_err(|e| e.context(format!("right-click on {place}")))?;
        if keys != ["add-folder", "add-group"] {
            bail!("right-click on {place}: the menu offers {keys:?}, expected add-folder and add-group");
        }
        let origin = view.read_with(cx, |v, _| v.dialogs.menu.as_ref().map(|m| m.position));
        if origin != Some(at) {
            bail!("right-click on {place} at {at:?}: the menu opened at {origin:?}");
        }
        key(cx, "escape", Modifiers::default())?;
        until(view, cx, WITHIN, "the Add menu closed on Esc", |v| v.dialogs.menu.is_none().then_some(())).await?;
        out.push(place);
    }
    Ok(format!("a right-click on {}: the Add menu (Add Folder…, Add Group…) at the pointer, closed by Esc", out.join(" and on ")))
}

/// Each top-level row's list item is its painted row alone, `TOP_ROW` high, and its name's line one height: no margin or
/// text size of its own sets one kind apart; Other sessions' joins them while it is painted. The item's bounds are the
/// sidebar list's layout and the name's line its text size's, which only a painted FleetView has.
async fn top_rhythm(view: &Entity<FleetView>, rows: &[(&str, &str)], cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let mut rows = rows.iter().map(|(r, l)| (r.to_string(), l.to_string())).collect::<Vec<_>>();
    if view.read_with(cx, |v, _| v.hits.borrow().contains_key(UNLISTED.0)) {
        rows.push((UNLISTED.0.into(), UNLISTED.1.into()));
    }
    let mut names = Vec::new();
    for (row, label) in &rows {
        scrolled_into_view(view, row, cx).await?;
        let (painted, item, name) = view.read_with(cx, |v, _| {
            let list = v.left_list();
            let hits = v.hits.borrow();
            (hits.get(row.as_str()).map(|h| h.bounds), list.row_of(row).and_then(|ix| list.state().bounds_for_item(ix)), hits.get(label.as_str()).map(|h| h.bounds))
        });
        let (Some(painted), Some(item), Some(name)) = (painted, item, name) else { bail!("{row}: painted {painted:?}, its list item {item:?}, its name {name:?}") };
        let high = f32::from(item.size.height);
        if (high - TOP).abs() > 0.5 || (f32::from(painted.size.height) - high).abs() > 0.5 {
            bail!("{row}: its list item {high} px high, the row painted {} px; every top-level row is {TOP} px with no margin", f32::from(painted.size.height));
        }
        names.push((row.clone(), f32::from(name.size.height)));
    }
    let (first, line) = names[0].clone();
    if let Some((row, other)) = names.iter().find(|(_, h)| (h - line).abs() > 0.5) {
        bail!("{row}'s name is a {other} px line, {first}'s {line} px; every top-level name is one text size");
    }
    Ok(format!("{} each a {TOP} px row with no margin, the name a {line} px line", names.iter().map(|(r, _)| r.as_str()).collect::<Vec<_>>().join(", ")))
}

/// Show sub folders from `root`'s menu, a folder made and one removed under it, then Hide sub folders.
pub(crate) async fn subfolders(view: &Entity<FleetView>, root: &Path, name: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let (a, b) = (root.join("sub-a"), root.join("sub-b"));
    let _ = std::fs::remove_dir_all(&b);
    std::fs::create_dir_all(&a)?;
    let row = format!("repo-row-{name}");
    let child = |c: &str| format!("repo-row-{name}/{c}");
    press_painted(view, &row, MouseButton::Right, cx).await?;
    menu_open(view, name, cx).await?;
    let label = view.read_with(cx, |v, _| v.dialogs.menu.as_ref().and_then(|m| m.items.iter().find(|i| i.key == "subfolders").map(|i| i.label.clone())));
    if label.as_deref() != Some("Show sub folders") {
        bail!("{name}'s menu offers {label:?}, expected Show sub folders");
    }
    click(view, "menu-subfolders", cx).await?;
    until(view, cx, WITHIN, "sub-a's row under its parent", |v| v.shown(&child("sub-a")).then_some(())).await.map_err(|e| {
        let state = view.read_with(cx, |v, _| {
            let flag = v.listed.entries.folders().filter(|f| f.path == root).map(|f| f.show_subfolders).collect::<Vec<_>>();
            let rows: Vec<String> = v.tree().folders().map(|(_, f)| f.key.clone()).collect();
            format!("toast {:?}; {name} show_subfolders {flag:?}; shown {:?}; rows {rows:?}", v.toast.text(), v.listed.disk.shown)
        });
        e.context(state)
    })?;
    std::fs::create_dir_all(&b)?;
    until(view, cx, WITHIN, "sub-b's row after it was made", |v| v.shown(&child("sub-b")).then_some(())).await?;
    shot("folder-subfolders", cx).await;
    std::fs::remove_dir(&a)?;
    until(view, cx, WITHIN, "sub-a's row gone after it was removed", |v| (!v.shown(&child("sub-a"))).then_some(())).await?;

    press_painted(view, &row, MouseButton::Right, cx).await?;
    menu_open(view, name, cx).await?;
    let label = view.read_with(cx, |v, _| v.dialogs.menu.as_ref().and_then(|m| m.items.iter().find(|i| i.key == "subfolders").map(|i| i.label.clone())));
    if label.as_deref() != Some("Hide sub folders") {
        bail!("{name}'s menu, its sub folders shown, offers {label:?}, expected Hide sub folders");
    }
    click(view, "menu-subfolders", cx).await?;
    until(view, cx, WITHIN, "sub-b's row gone once hidden", |v| (!v.shown(&child("sub-b"))).then_some(())).await?;
    std::fs::remove_dir(&b).map_err(|e| anyhow!("removing {}: {e}", b.display()))?;
    Ok(format!("Show sub folders on {name}: sub-a's row; sub-b made: its row; sub-a removed: its row gone; Hide sub folders: none"))
}
