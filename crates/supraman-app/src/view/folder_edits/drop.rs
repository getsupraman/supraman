use std::path::PathBuf;

use gpui::{Context, SharedString, Window, prelude::*};
use supraman_core::folders::{Entries, Entry, Place, Pos};

/// A listed folder's row being dragged: its place in folders.toml while it is still `path` there.
#[derive(Clone)]
pub struct DraggedFolder {
    pub pos: Pos,
    pub path: PathBuf,
    pub title: SharedString,
}

impl Render for DraggedFolder {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        supraman_ui::tabs::chip(self.title.clone(), cx)
    }
}

/// Where a dragged folder row is let go.
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

/// The place of the folder at `from`, as [`landing`] counts it.
fn here(entries: &Entries, from: Pos) -> Option<Place> {
    match (entries.list.get(from.entry)?, from.folder) {
        (Entry::Folder(_), None) => Some(Place::Top(from.entry)),
        (Entry::Group { name, folders }, Some(i)) if i < folders.len() => Some(Place::Group { name: name.clone(), index: i }),
        _ => None,
    }
}
