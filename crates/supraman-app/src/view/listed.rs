use std::path::{Path, PathBuf};

use supraman_core::folders::{self, Entries};

use gpui::Context;

use super::FleetView;
use crate::fleet;

mod subfolders;

pub use subfolders::SubfolderWatch;

/// The sidebar's entries with what the disk says of their folders.
#[derive(Default)]
pub struct Listed {
    pub entries: Entries,
    pub disk: fleet::Disk,
    pub error: Option<String>,
}

/// One read off the UI thread: folders.toml with its folders' kinds and shown children.
pub struct Read {
    entries: Result<(Entries, fleet::Disk), String>,
}

pub fn read(home: Option<&Path>) -> Read {
    let Some(home) = home else {
        return Read { entries: Err("no Supraman home".into()) };
    };
    let entries = folders::load(home).map(|e| {
        let disk = fleet::on_disk(&e);
        prune_gone(home, &e, &disk);
        (e, disk)
    }).map_err(|e| e.to_string());
    Read { entries }
}

/// The excluded paths this read finds gone dropped from folders.toml; the next read shows it.
fn prune_gone(home: &Path, entries: &Entries, disk: &fleet::Disk) {
    match folders::prune_excluded(home, &folders::gone_excluded(entries, &disk.shown)) {
        Ok(0) => {}
        Ok(n) => log::info!("folders.toml: {n} excluded folder(s) gone, dropped"),
        Err(e) => log::warn!("folders.toml: the excluded folders gone stay listed: {e}"),
    }
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
        before != (self.entries.clone(), self.disk.clone())
    }
}

impl FleetView {
    /// The entry (a group, or a top-level folder) a session in `cwd` belongs to ([`fleet::Tree::holding`]).
    pub(super) fn entry_of(&self, cwd: &Path) -> Option<String> {
        self.entry_and_folder(cwd).map(|(entry, _)| entry)
    }

    pub(super) fn entry_and_folder(&self, cwd: &Path) -> Option<(String, PathBuf)> {
        self.tree().holding(&self.listed.entries, &self.listed.disk.shown, cwd)
    }

    /// Every entry's name in the tree's order: a group's, a top-level folder's key.
    pub(super) fn entry_names(&self) -> Vec<String> {
        self.tree().entries.iter().map(|e| e.name.clone()).collect()
    }

    /// The folder keys this home's scheduled runs are named after: every listed folder's and every schedule file's.
    pub(super) fn own_names(&self) -> Vec<String> {
        let listed = self.listed.entries.folders().map(|f| supraman_core::schedule::folder_key(&f.path));
        let mut out: Vec<String> = listed.chain(self.scheduling.schedules.keys().cloned()).collect();
        out.sort();
        out.dedup();
        out
    }

    pub(super) fn listed_changed(&mut self, cx: &mut Context<Self>) {
        self.polls.transcripts_nudge.wake();
        self.invalidate_tree();
        let (showing, fleet) = (self.listed.entries.showing(), cx.entity().downgrade());
        self.sidebar.subfolders.update(cx, |w, cx| w.sync(showing, fleet, cx));
    }

    /// folders.toml and its folders read again off the UI thread: a shown folder's children changed.
    pub(super) fn read_listed(&mut self, cx: &mut Context<Self>) {
        let home = self.home.clone();
        let generation = self.polls.folder_writes.load(std::sync::atomic::Ordering::SeqCst);
        let read = cx.background_executor().spawn(async move { read(home.as_deref()) });
        cx.spawn(async move |this, cx| {
            let read = read.await;
            this.update(cx, |this, cx| {
                let writes = this.polls.folder_writes.load(std::sync::atomic::Ordering::SeqCst);
                if this.listed.apply(read, generation, writes) {
                    this.listed_changed(cx);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_read_from_before_this_windows_write_keeps_the_written_list() {
        let entries = Entries::new(vec![folders::Entry::Folder(folders::FolderEntry::new(r"C:\a"))]);
        let read = |e: Entries| Read { entries: Ok((e, fleet::Disk::default())) };
        let mut listed = Listed { entries: entries.clone(), ..Listed::default() };
        assert!(!listed.apply(read(Entries::default()), 1, 2), "read before the write");
        assert_eq!(listed.entries, entries);
        assert!(listed.apply(read(Entries::default()), 2, 2));
        assert_eq!(listed.entries, Entries::default());
        listed.apply(Read { entries: Err("unreadable".into()) }, 2, 2);
        assert_eq!(listed.error.as_deref(), Some("unreadable"));

    }

    #[test]
    fn a_read_drops_an_excluded_folder_only_once_its_parent_was_read_without_it() {
        let (home, root) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let (web, missing) = (root.path().join("web"), root.path().join("missing"));
        std::fs::create_dir(&web).unwrap();
        let parent = |path: &Path, child: &Path| folders::FolderEntry { path: path.into(), show_subfolders: true, excluded: vec![child.into()] };
        let listed = Entries::new(vec![folders::Entry::Folder(parent(root.path(), &web)), folders::Entry::Folder(parent(&missing, &missing.join("x")))]);
        folders::edit(home.path(), |e| {
            *e = listed.clone();
            Ok(())
        })
        .unwrap();
        let excluded = || folders::load(home.path()).unwrap().folders().map(|f| f.excluded.len()).collect::<Vec<_>>();
        let file = folders::path(home.path());
        let written = std::fs::metadata(&file).unwrap().modified().unwrap();
        read(Some(home.path()));
        assert_eq!(std::fs::metadata(&file).unwrap().modified().unwrap(), written, "nothing gone, nothing written");
        std::fs::remove_dir(&web).unwrap();
        let first = read(Some(home.path()));
        assert_eq!(first.entries.unwrap().0, listed, "this read keeps what it read");
        assert_eq!(excluded(), [0, 1], "the next sees it dropped; an unreadable parent drops nothing");
    }

    #[test]
    fn a_broken_folders_toml_is_the_reads_error() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(folders::path(home.path()), "entry = ").unwrap();
        assert!(read(Some(home.path())).entries.is_err());
        assert!(read(None).entries.is_err());
    }
}
