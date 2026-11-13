//! The sidebar's edits of folders.toml: Add Folder, Add Group, Rename, Remove Folder and Remove Group, each written off the UI thread.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use gpui::{AppContext as _, Context, Entity, Window};
use supraman_core::folders::{self, Entries, Place, Pos};
use supraman_core::paths::{folder_name, path_key};
use supraman_core::{CoreError, text};

use super::FleetView;
use super::left::PendingReveal;
use super::listed;
use super::menu::{Confirm, Confirmed, MenuAction};

mod group_name;

pub use group_name::{GroupName, GroupNameEvent};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FolderEdit {
    /// At the end of the top level, or of the group named.
    AddFolder { path: PathBuf, group: Option<String> },
    AddGroup(String),
    RenameGroup { from: String, to: String },
    /// The folder at `pos`, while it is still `path` there.
    RemoveFolder { pos: Pos, path: PathBuf },
    RemoveGroup { name: String, with_folders: bool },
}

impl FolderEdit {
    /// `entries` changed, and the words that say what was done.
    pub fn apply(&self, entries: &mut Entries) -> supraman_core::Result<String> {
        match self {
            FolderEdit::AddFolder { path, group } => {
                let name = folder_name(path);
                match group {
                    Some(g) => {
                        entries.add_folder(path, &Place::Group { name: g.clone(), index: usize::MAX })?;
                        Ok(format!("Added {name} to {g}."))
                    }
                    None => {
                        entries.add_folder(path, &Place::Top(usize::MAX))?;
                        Ok(format!("Added {name}."))
                    }
                }
            }
            FolderEdit::AddGroup(name) => {
                entries.add_group(name, usize::MAX)?;
                Ok(format!("Added group {name}."))
            }
            FolderEdit::RenameGroup { from, to } => {
                entries.rename_group(from, to)?;
                Ok(format!("Renamed group {from} to {to}."))
            }
            FolderEdit::RemoveFolder { pos, path } => {
                if listed_at(entries, *pos).is_none_or(|p| path_key(p) != path_key(path)) {
                    return Err(CoreError::NoEntry(path.display().to_string()));
                }
                entries.remove_folder(*pos)?;
                Ok(format!("Removed {} from the list; its files and sessions stay.", folder_name(path)))
            }
            FolderEdit::RemoveGroup { name, with_folders } => {
                let held = entries.groups().find(|(n, _)| n == name).map_or(0, |(_, f)| f.len());
                entries.remove_group(name, *with_folders)?;
                Ok(match with_folders {
                    true => format!("Removed group {name} and its {}.", text::count(held, "folder", "folders")),
                    false => format!("Removed group {name}; its folders are at the top level."),
                })
            }
        }
    }

    /// What a refusal of it starts with.
    fn refused(&self) -> String {
        match self {
            FolderEdit::AddFolder { path, .. } => format!("Cannot add {}", folder_name(path)),
            FolderEdit::AddGroup(name) => format!("Cannot add group {name}"),
            FolderEdit::RenameGroup { from, .. } => format!("Cannot rename group {from}"),
            FolderEdit::RemoveFolder { path, .. } => format!("Cannot remove {}", folder_name(path)),
            FolderEdit::RemoveGroup { name, .. } => format!("Cannot remove group {name}"),
        }
    }

    /// The tree row it makes, scrolled into view once read back.
    fn row(&self) -> Option<String> {
        match self {
            FolderEdit::AddFolder { path, group: None } => Some(format!("repo:{}", folder_name(path))),
            FolderEdit::AddFolder { path, group: Some(g) } => Some(format!("repo:{g}/{}", folder_name(path))),
            FolderEdit::AddGroup(name) | FolderEdit::RenameGroup { to: name, .. } => Some(format!("group:{name}")),
            FolderEdit::RemoveFolder { .. } | FolderEdit::RemoveGroup { .. } => None,
        }
    }
}

fn listed_at(entries: &Entries, pos: Pos) -> Option<&Path> {
    let entry = entries.list.get(pos.entry)?;
    match (entry, pos.folder) {
        (folders::Entry::Folder(f), None) => Some(&f.path),
        (folders::Entry::Group { folders, .. }, Some(i)) => folders.get(i).map(|f| f.path.as_path()),
        _ => None,
    }
}

/// `edit` written under folders.toml's lock, then the list read back; blocking.
pub fn write(home: &Path, edit: &FolderEdit) -> Result<(String, listed::Read), String> {
    let (_, said) = folders::edit(home, |e| edit.apply(e)).map_err(|e| format!("{}: {e}", edit.refused()))?;
    Ok((said, listed::read(Some(home))))
}

/// Remove Group's question: its folders moved to the top level (the default), or removed with it; Cancel.
pub fn remove_group_question(name: &str, folders: usize) -> Confirm {
    let edit = |with_folders| Confirmed::EditFolders(FolderEdit::RemoveGroup { name: name.to_string(), with_folders });
    if folders == 0 {
        return Confirm::ask(format!("Remove the empty group {name}?"), "Remove Group", edit(false));
    }
    Confirm::ask(
        format!("Remove the group {name}? Its {} can move to the top level or go from the list with it; no files or sessions are touched.", text::count(folders, "folder", "folders")),
        "Move its folders to the top level",
        edit(false),
    )
    .or("Remove its folders too", edit(true))
}

impl FleetView {
    /// `edit` written off the UI thread; what it did, or why not, is the toast's.
    pub fn edit_folders(&mut self, edit: FolderEdit, cx: &mut Context<Self>) {
        let Some(home) = self.home.clone() else {
            return self.toast.show_why(format!("{}: no Supraman home", edit.refused()), |v: &mut Self| &mut v.toast, cx);
        };
        let generation = self.polls.folder_writes.fetch_add(1, Ordering::SeqCst) + 1;
        let row = edit.row();
        let write = cx.background_executor().spawn(async move { write(&home, &edit) });
        cx.spawn(async move |this, cx| {
            let done = write.await;
            this.update(cx, |this, cx| {
                match done {
                    Ok((said, read)) => {
                        let writes = this.polls.folder_writes.load(Ordering::SeqCst);
                        if this.listed.apply(read, generation, writes) {
                            this.listed_changed();
                        }
                        if let Some(row) = row {
                            this.reveal_new(PendingReveal::Row(row));
                        }
                        this.show_toast(said, cx);
                    }
                    Err(why) => this.toast.show_why(why, |v: &mut Self| &mut v.toast, cx),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// A folder picked, added at the end of the top level or of `group`.
    pub fn add_folder(&mut self, group: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.dialogs.menu = None;
        if let Some(path) = self.dialogs.pick_folder.take() {
            return self.edit_folders(FolderEdit::AddFolder { path, group }, cx);
        }
        let title = match &group {
            Some(g) => format!("Add a folder to {g}"),
            None => "Add a folder to Supraman".to_string(),
        };
        supraman_ui::folder_picker::pick_folder(&title, window, cx, move |this, path, _, cx| this.edit_folders(FolderEdit::AddFolder { path, group: group.clone() }, cx));
    }

    /// Add Group, or Rename with `renaming`'s name filled in.
    pub fn open_group_name(&mut self, renaming: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let taken = self.listed.entries.groups().map(|(n, _)| n.to_string()).collect();
        let hits = self.hits.clone();
        let form = cx.new(|cx| GroupName::new(renaming, taken, hits, window, cx));
        let sub = cx.subscribe(&form, |this, _, e: &GroupNameEvent, cx| {
            this.dialogs.group_name = None;
            if let GroupNameEvent::Named(edit) = e {
                this.edit_folders(edit.clone(), cx);
            }
            cx.notify();
        });
        self.dialogs.group_name = Some((form, sub));
        cx.notify();
    }

    pub(super) fn group_name_form(&self) -> Option<&Entity<GroupName>> {
        self.dialogs.group_name.as_ref().map(|(form, _)| form)
    }

    fn ask_remove_group(&mut self, name: String) {
        let folders = self.listed.entries.groups().find(|(n, _)| *n == name).map_or(0, |(_, f)| f.len());
        self.dialogs.confirm = Some(remove_group_question(&name, folders));
    }

    pub(super) fn run_folder_action(&mut self, action: MenuAction, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            MenuAction::AddFolder(group) => self.add_folder(group, window, cx),
            MenuAction::AddGroup => self.open_group_name(None, window, cx),
            MenuAction::RenameGroup(name) => self.open_group_name(Some(name), window, cx),
            MenuAction::RemoveFolder { pos, path } => self.edit_folders(FolderEdit::RemoveFolder { pos, path }, cx),
            MenuAction::RemoveGroup(name) => self.ask_remove_group(name),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests;
