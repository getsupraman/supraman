use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use gpui::Context;
use supraman_core::folders::{self, Entries, OnDisk, Place};
use supraman_core::{Registry, Workspace};

use super::FleetView;
use crate::fleet;

/// The sidebar's entries with what the disk says of their folders, and the workspaces schedules read.
#[derive(Default)]
pub struct Listed {
    pub entries: Entries,
    pub disk: HashMap<String, OnDisk>,
    /// The schedules' bridge (folders/schedules removes it): only schedule code reads these.
    pub schedule_workspaces: Vec<Workspace>,
    pub error: Option<String>,
    /// The workspace files the schedules' bridge could not read.
    pub skipped: Vec<String>,
}

/// One read off the UI thread: folders.toml with its folders' kinds, and the schedules' workspaces.
pub struct Read {
    entries: Result<(Entries, HashMap<String, OnDisk>), String>,
    workspaces: (Vec<Workspace>, Vec<String>),
}

impl Read {
    pub fn workspaces(&self) -> &[Workspace] {
        &self.workspaces.0
    }
}

pub fn read(home: Option<&Path>) -> Read {
    let Some(home) = home else {
        return Read { entries: Err("no Supraman home".into()), workspaces: Default::default() };
    };
    let entries = folders::load(home).map(|e| {
        let disk = fleet::on_disk(&e);
        (e, disk)
    }).map_err(|e| e.to_string());
    let workspaces = match Registry::at(home).load_all() {
        Ok((loaded, failed)) => (loaded, failed.iter().map(ToString::to_string).collect()),
        Err(e) => (Vec::new(), vec![e.to_string()]),
    };
    Read { entries, workspaces }
}

impl Listed {
    /// `read`, unless a folders.toml write from this window came after it was started (`read_at` against `writes`).
    pub fn apply(&mut self, read: Read, read_at: u64, writes: u64) -> bool {
        let before = (self.entries.clone(), self.disk.clone());
        match read.entries {
            Ok(_) if read_at != writes => {}
            Ok((entries, disk)) => {
                self.entries = entries;
                self.disk = disk;
                self.error = None;
            }
            Err(e) => self.error = Some(e),
        }
        (self.schedule_workspaces, self.skipped) = read.workspaces;
        before != (self.entries.clone(), self.disk.clone())
    }
}

impl FleetView {
    /// `path` added at the top level's end, written off the UI thread; a refusal is the action's error.
    pub fn add_top_folder(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let Some(home) = self.home.clone() else {
            self.action_error = Some("Cannot add a folder: no Supraman home".into());
            return;
        };
        let generation = self.polls.folder_writes.fetch_add(1, Ordering::SeqCst) + 1;
        let added = path.clone();
        let write = cx.background_executor().spawn(async move {
            folders::edit(&home, |e| e.add_folder(&added, &Place::Top(usize::MAX))).map_err(|e| e.to_string())?;
            Ok::<_, String>(read(Some(&home)))
        });
        cx.spawn(async move |this, cx| {
            let done = write.await;
            this.update(cx, |this, cx| {
                match done {
                    Ok(read) => {
                        let writes = this.polls.folder_writes.load(Ordering::SeqCst);
                        if this.listed.apply(read, generation, writes) {
                            this.listed_changed();
                        }
                        let name = supraman_core::paths::folder_name(&path);
                        this.reveal_new(super::left::PendingReveal::Row(format!("repo:{name}")));
                        this.show_toast(format!("Added {name}."), cx);
                    }
                    Err(e) => this.action_error = Some(format!("Cannot add {}: {e}", path.display())),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The entry (a group, or a top-level folder) whose folder a session in `cwd` shows under.
    pub(super) fn entry_of(&self, cwd: &Path) -> Option<String> {
        let folder = supraman_claude::holding(cwd, &self.listed.entries, |_| None)?;
        let key = supraman_core::paths::path_key(&folder.path);
        self.tree().folders().find(|(_, f)| supraman_core::paths::path_key(&f.path) == key).map(|(e, _)| e.name.clone())
    }

    /// Every entry's name in the tree's order: a group's, a top-level folder's key.
    pub(super) fn entry_names(&self) -> Vec<String> {
        self.tree().entries.iter().map(|e| e.name.clone()).collect()
    }

    /// The names this home's scheduled runs are named after (the schedules' bridge).
    pub(super) fn own_names(&self) -> Vec<String> {
        self.listed.schedule_workspaces.iter().map(|w| w.name.clone()).collect()
    }

    pub(super) fn listed_changed(&mut self) {
        self.polls.transcripts_nudge.wake();
        self.invalidate_tree();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_read_from_before_this_windows_write_keeps_the_written_list() {
        let entries = Entries::new(vec![folders::Entry::Folder(folders::FolderEntry::new(r"C:\a"))]);
        let read = |e: Entries| Read { entries: Ok((e, HashMap::new())), workspaces: Default::default() };
        let mut listed = Listed { entries: entries.clone(), ..Listed::default() };
        assert!(!listed.apply(read(Entries::default()), 1, 2), "read before the write");
        assert_eq!(listed.entries, entries);
        assert!(listed.apply(read(Entries::default()), 2, 2));
        assert_eq!(listed.entries, Entries::default());
        listed.apply(Read { entries: Err("unreadable".into()), workspaces: (Vec::new(), vec!["broken.toml".into()]) }, 2, 2);
        assert_eq!((listed.error.as_deref(), listed.skipped.as_slice()), (Some("unreadable"), &["broken.toml".to_string()][..]));
    }

    #[test]
    fn a_broken_folders_toml_is_the_reads_error() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(folders::path(home.path()), "entry = ").unwrap();
        assert!(read(Some(home.path())).entries.is_err());
        assert!(read(None).entries.is_err());
    }
}
