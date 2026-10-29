//! What `fm workspace add` and the app's Add workspace share: a folder → a workspace to register, the
//! checks before registering it, and a rescan of a registered one.

use std::path::Path;

use crate::paths::{path_key, within};
use crate::{CoreError, Folder, Registry, Result, Scan, Workspace, owners, scan, slugify, validate_name};

/// `dir` made absolute as the home and one scan `depth` deep, named `name` or the slug of its folder name,
/// with the repos [`scan`] finds. Nothing is written.
pub fn discover(dir: &Path, name: Option<&str>, depth: usize) -> Result<Workspace> {
    let root = std::path::absolute(dir).map_err(|_| CoreError::NotADirectory(dir.to_path_buf()))?;
    let name = match name {
        Some(name) => name.to_string(),
        None => slugify(&root.file_name().map(|f| f.to_string_lossy()).unwrap_or_default()),
    };
    validate_name(&name)?;
    let mut workspace = Workspace::new(name, &root, Vec::new());
    for found in scan(&root, depth)? {
        let name = workspace.unique_folder_name(&found.name);
        workspace.folders.push(Folder { name, ..found });
    }
    workspace.scans.push(Scan { path: root, depth });
    Ok(workspace)
}

/// The registered workspace with each scan run again: a folder found now keeps the name and `enabled` it
/// had, a new one is added, and a scanned folder gone from its scan is dropped; folders added by hand stay.
/// Returns it with the names added and removed. Nothing is written.
pub fn rescan(workspace: &Workspace) -> Result<(Workspace, Vec<String>, Vec<String>)> {
    let mut found: Vec<Folder> = Vec::new();
    for s in &workspace.scans {
        for f in scan(&s.path, s.depth)? {
            if !found.iter().any(|o| path_key(&o.path) == path_key(&f.path)) {
                found.push(f);
            }
        }
    }
    let mut next = Workspace { folders: Vec::new(), ..workspace.clone() };
    let mut added = Vec::new();
    for f in found {
        match workspace.folder_at(&f.path) {
            Some(old) if old.added => {}
            Some(old) => next.folders.push(Folder { name: old.name.clone(), enabled: old.enabled, ..f }),
            None => {
                let name = next.unique_folder_name(&f.name);
                added.push(name.clone());
                next.folders.push(Folder { name, ..f });
            }
        }
    }
    let scanned = |f: &Folder| workspace.scans.iter().any(|s| within(&f.path, &s.path));
    let mut removed = Vec::new();
    for old in &workspace.folders {
        if old.added || !scanned(old) {
            next.folders.push(old.clone());
        } else if next.folder_at(&old.path).is_none() {
            removed.push(old.name.clone());
        }
    }
    Ok((next, added, removed))
}

pub fn same_path(a: &Path, b: &Path) -> bool {
    path_key(a) == path_key(b)
}

impl Registry {
    /// Why `workspace` cannot be registered as a new one: its name is taken.
    pub fn check_new(&self, workspace: &Workspace) -> Result<()> {
        if self.exists(&workspace.name)? {
            return Err(CoreError::AlreadyExists(workspace.name.clone()));
        }
        Ok(())
    }

    /// Disables each folder of `workspace` that `which` picks and another registered workspace has enabled,
    /// since a folder's supervised session belongs to one workspace ([`owners`]). Returns `(folder, owner)`
    /// for each.
    pub fn claim_folders(&self, workspace: &mut Workspace, which: impl Fn(&Folder) -> bool) -> Result<Vec<(String, String)>> {
        let others: Vec<Workspace> = self.list()?.into_iter().filter(|w| w.name != workspace.name).collect();
        let owners = owners(&others);
        let mut taken = Vec::new();
        for f in workspace.folders.iter_mut().filter(|f| f.enabled && which(f)) {
            if let Some(owner) = owners.get(&path_key(&f.path)) {
                f.enabled = false;
                taken.push((f.name.clone(), owner.clone()));
            }
        }
        Ok(taken)
    }

    pub fn update(&self, name: &str, edit: impl FnOnce(&mut Workspace)) -> Result<Workspace> {
        let mut workspace = self.load(name)?;
        edit(&mut workspace);
        workspace.name = name.to_string();
        self.save(&workspace, true)?;
        Ok(workspace)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::FolderKind;

    fn repo_dir(root: &Path, name: &str) {
        fs::create_dir_all(root.join(name).join(".git")).unwrap();
        fs::write(root.join(name).join(".git").join("HEAD"), "ref: refs/heads/main\n").unwrap();
    }

    fn names(w: &Workspace) -> Vec<(&str, bool)> {
        w.folders.iter().map(|f| (f.name.as_str(), f.enabled)).collect()
    }

    #[test]
    fn discover_names_by_folder_and_finds_repos() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("My Projects");
        repo_dir(&root, "Alpha");
        repo_dir(&root, "Beta");
        fs::create_dir_all(root.join("plain")).unwrap();
        let w = discover(&root, None, 1).unwrap();
        assert_eq!(w.name, "my-projects");
        assert_eq!(names(&w), [("Alpha", true), ("Beta", true)]);
        assert_eq!(w.home(), Some(root.as_path()));
        assert_eq!(w.scans, [Scan { path: root.clone(), depth: 1 }]);
        assert_eq!(discover(&root, Some("x"), 1).unwrap().name, "x");
        assert!(matches!(discover(&root, Some("Bad Name"), 1), Err(CoreError::InvalidName(_))));
        assert!(matches!(discover(&root.join("nope"), None, 1), Err(CoreError::NotADirectory(_))));
    }

    #[test]
    fn check_new_refuses_a_taken_name_and_a_second_workspace_on_a_folder_gets_it_disabled() {
        let dir = tempfile::tempdir().unwrap();
        let reg = Registry::at(dir.path().join("home"));
        let root = dir.path().join("ws");
        repo_dir(&root, "Alpha");
        repo_dir(&root, "Beta");
        let w = discover(&root, Some("ws"), 1).unwrap();
        reg.check_new(&w).unwrap();
        reg.save(&w, false).unwrap();
        assert!(matches!(reg.check_new(&w), Err(CoreError::AlreadyExists(n)) if n == "ws"));

        let mut again = discover(&root, Some("again"), 1).unwrap();
        reg.check_new(&again).unwrap();
        assert_eq!(reg.claim_folders(&mut again, |_| true).unwrap().len(), 2, "the same folder registers again, its folders owned by ws");
        let mut other = Workspace::new("other", dir.path(), Vec::new());
        other.add_folder(&root.join("Beta"));
        other.add_folder(&dir.path().join("home"));
        let mut only_home = other.clone();
        assert!(reg.claim_folders(&mut only_home, |f| f.name == "home").unwrap().is_empty());
        assert_eq!(reg.claim_folders(&mut other, |_| true).unwrap(), [("Beta".to_string(), "ws".to_string())]);
        assert_eq!(names(&other), [("Beta", false), ("home", true)]);
    }

    #[test]
    fn rescan_keeps_enabled_and_reports_changes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("ws");
        repo_dir(&root, "Alpha");
        repo_dir(&root, "Beta");
        let mut w = discover(&root, Some("ws"), 1).unwrap();
        w.folders[1].enabled = false;
        fs::remove_dir_all(root.join("Alpha")).unwrap();
        repo_dir(&root, "Gamma");
        let (after, added, removed) = rescan(&w).unwrap();
        assert_eq!(names(&after), [("Beta", false), ("Gamma", true)]);
        assert_eq!(added, ["Gamma"]);
        assert_eq!(removed, ["Alpha"]);
        assert_eq!(after.created_at, w.created_at);
    }

    #[test]
    fn rescan_keeps_folders_added_by_hand_and_the_names_it_had() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("ws");
        repo_dir(&root, "Alpha");
        fs::create_dir_all(root.join("NodeApi")).unwrap();
        fs::create_dir_all(dir.path().join("Far")).unwrap();
        let mut w = discover(&root, Some("ws"), 1).unwrap();
        w.folders[0].name = "renamed".into();
        w.add_folder(&root.join("NodeApi"));
        w.add_folder(&dir.path().join("Far"));
        fs::remove_dir_all(dir.path().join("Far")).unwrap();
        let (after, added, removed) = rescan(&w).unwrap();
        assert_eq!(names(&after), [("renamed", true), ("NodeApi", true), ("Far", true)]);
        assert_eq!(after.folders[1].kind, FolderKind::Plain);
        assert!(added.is_empty() && removed.is_empty(), "{added:?} {removed:?}");
    }

    #[test]
    fn update_edits_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let reg = Registry::at(dir.path());
        let root = dir.path().join("ws");
        repo_dir(&root, "Alpha");
        reg.save(&discover(&root, Some("ws"), 1).unwrap(), false).unwrap();
        reg.update("ws", |w| w.folders[0].enabled = false).unwrap();
        assert!(!reg.load("ws").unwrap().folders[0].enabled);
        assert!(matches!(reg.update("nope", |_| {}), Err(CoreError::NotFound(_))));
    }
}
