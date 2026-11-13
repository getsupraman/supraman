use std::path::Path;

use crate::paths::path_key;
use crate::{CoreError, Result, validate_name};

use super::{Entries, Entry, FolderEntry};

/// Where a folder goes: at an index of the top level, or of a group's folders; an index past the end appends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    Top(usize),
    Group { name: String, index: usize },
}

/// A listed folder: the entry at `entry`, or the `folder`th of the group there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pos {
    pub entry: usize,
    pub folder: Option<usize>,
}

impl Pos {
    pub fn top(entry: usize) -> Self {
        Self { entry, folder: None }
    }

    pub fn member(entry: usize, folder: usize) -> Self {
        Self { entry, folder: Some(folder) }
    }
}

impl Entries {
    fn group_index(&self, name: &str) -> Result<usize> {
        self.list.iter().position(|e| e.group_name() == Some(name)).ok_or_else(|| CoreError::NoGroup(name.to_string()))
    }

    fn check_new_group_name(&self, name: &str) -> Result<()> {
        validate_name(name)?;
        if self.list.iter().any(|e| e.group_name() == Some(name)) {
            return Err(CoreError::AlreadyExists(name.to_string()));
        }
        Ok(())
    }

    /// `path` added at `place`; refused when that list already holds it.
    pub fn add_folder(&mut self, path: &Path, place: &Place) -> Result<()> {
        self.insert(FolderEntry::new(path), place)
    }

    fn insert(&mut self, folder: FolderEntry, place: &Place) -> Result<()> {
        let key = path_key(&folder.path);
        let taken = |list: &[FolderEntry]| list.iter().any(|f| path_key(&f.path) == key);
        match place {
            Place::Top(index) => {
                if self.list.iter().any(|e| matches!(e, Entry::Folder(f) if path_key(&f.path) == key)) {
                    return Err(CoreError::FolderListed(folder.path));
                }
                let at = (*index).min(self.list.len());
                self.list.insert(at, Entry::Folder(folder));
            }
            Place::Group { name, index } => {
                let g = self.group_index(name)?;
                let Entry::Group { folders, .. } = &mut self.list[g] else { unreachable!() };
                if taken(folders) {
                    return Err(CoreError::FolderListed(folder.path));
                }
                let at = (*index).min(folders.len());
                folders.insert(at, folder);
            }
        }
        Ok(())
    }

    /// The folder at `pos`, taken out of its list.
    pub fn remove_folder(&mut self, pos: Pos) -> Result<FolderEntry> {
        let missing = || CoreError::NoEntry(format!("{pos:?}"));
        match (self.list.get_mut(pos.entry), pos.folder) {
            (Some(Entry::Folder(_)), None) => match self.list.remove(pos.entry) {
                Entry::Folder(f) => Ok(f),
                Entry::Group { .. } => unreachable!(),
            },
            (Some(Entry::Group { folders, .. }), Some(i)) if i < folders.len() => Ok(folders.remove(i)),
            _ => Err(missing()),
        }
    }

    /// The folder at `from` moved to `to`; `to`'s index counts with the folder already taken out.
    pub fn move_folder(&mut self, from: Pos, to: &Place) -> Result<()> {
        if let Place::Group { name, .. } = to {
            self.group_index(name)?;
        }
        let before = self.clone();
        let folder = self.remove_folder(from)?;
        if let Err(e) = self.insert(folder, to) {
            *self = before;
            return Err(e);
        }
        Ok(())
    }

    pub fn add_group(&mut self, name: &str, index: usize) -> Result<()> {
        self.check_new_group_name(name)?;
        let at = index.min(self.list.len());
        self.list.insert(at, Entry::Group { name: name.to_string(), folders: Vec::new() });
        Ok(())
    }

    pub fn rename_group(&mut self, from: &str, to: &str) -> Result<()> {
        let g = self.group_index(from)?;
        if from == to {
            return Ok(());
        }
        self.check_new_group_name(to)?;
        if let Entry::Group { name, .. } = &mut self.list[g] {
            *name = to.to_string();
        }
        Ok(())
    }

    /// The group gone; `with_folders` false puts its folders at the top level where it was, but those
    /// the top level already lists.
    pub fn remove_group(&mut self, name: &str, with_folders: bool) -> Result<()> {
        let g = self.group_index(name)?;
        let Entry::Group { folders, .. } = self.list.remove(g) else { unreachable!() };
        if !with_folders {
            let mut at = g;
            for f in folders {
                if self.insert(f, &Place::Top(at)).is_ok() {
                    at += 1;
                }
            }
        }
        Ok(())
    }
}
