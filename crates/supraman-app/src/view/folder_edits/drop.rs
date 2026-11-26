use std::path::PathBuf;

use gpui::SharedString;
use supraman_core::folders::{Entries, Entry, Place, Pos};

/// A listed folder's row being dragged: its place in folders.toml while it is still `path` there.
#[derive(Clone)]
pub struct DraggedFolder {
    pub pos: Pos,
    pub path: PathBuf,
    pub title: SharedString,
}

impl supraman_ui::dnd::Payload for DraggedFolder {
    fn title(&self) -> SharedString {
        self.title.clone()
    }
}

/// A group's row being dragged.
#[derive(Clone)]
pub struct DraggedGroup {
    pub name: String,
    pub title: SharedString,
}

impl supraman_ui::dnd::Payload for DraggedGroup {
    fn title(&self) -> SharedString {
        self.title.clone()
    }
}

/// Where a dragged folder or group row is let go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A group's row: the end of its folders.
    Group(String),
    /// A listed folder's row: its place, the dropped folder going before it.
    Folder(Pos),
    /// Below the tree: the end of the top level.
    End,
}

/// Where a folder at `from` dropped on `target` goes, counted with it taken out ([`Entries::move_folder`]);
/// `None` when that is where it is already, or the target is gone.
pub fn landing(entries: &Entries, from: Pos, target: &Target) -> Option<Place> {
    let place = match target {
        Target::End => Place::Top(entries.list.len() - usize::from(from.folder.is_none())),
        Target::Group(name) => {
            let e = entries.list.iter().position(|x| x.group_name() == Some(name))?;
            let len = entries.list[e].folders().len();
            Place::Group { name: name.clone(), index: len - usize::from(from.entry == e && from.folder.is_some()) }
        }
        Target::Folder(to) => match (&entries.list.get(to.entry)?, to.folder) {
            (Entry::Folder(_), None) => Place::Top(to.entry - usize::from(from.folder.is_none() && from.entry < to.entry)),
            (Entry::Group { name, .. }, Some(i)) => Place::Group { name: name.clone(), index: i - usize::from(from.entry == to.entry && from.folder.is_some_and(|f| f < i)) },
            _ => return None,
        },
    };
    (place != here(entries, from)?).then_some(place)
}

/// Where the group `name` dropped on `target` goes among the top-level entries, counted with it taken out
/// ([`Entries::move_group`]): before a top-level row's entry, or last below the tree; a row inside a group is no target.
/// `None` when that is where it is already, or the group or target is gone.
pub fn group_landing(entries: &Entries, name: &str, target: &Target) -> Option<usize> {
    let from = entries.list.iter().position(|e| e.group_name() == Some(name))?;
    let before = match target {
        Target::End => entries.list.len(),
        Target::Group(other) => entries.list.iter().position(|e| e.group_name() == Some(other))?,
        Target::Folder(Pos { entry, folder: None }) if matches!(entries.list.get(*entry), Some(Entry::Folder(_))) => *entry,
        Target::Folder(_) => return None,
    };
    let to = before - usize::from(from < before);
    (to != from).then_some(to)
}

/// The place of the folder at `from`, as [`landing`] counts it.
fn here(entries: &Entries, from: Pos) -> Option<Place> {
    match (entries.list.get(from.entry)?, from.folder) {
        (Entry::Folder(_), None) => Some(Place::Top(from.entry)),
        (Entry::Group { name, folders }, Some(i)) if i < folders.len() => Some(Place::Group { name: name.clone(), index: i }),
        _ => None,
    }
}
