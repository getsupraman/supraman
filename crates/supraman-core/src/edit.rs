//! One change to a registered workspace, read, checked and saved under the registry's lock.

use std::path::{Path, PathBuf};

use crate::paths::{path_key, within};
use crate::{CoreError, Folder, Registry, Result, Scan, Workspace, owners, rescan};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edit {
    /// `enable`: the folder may have a supervised session, unless another workspace owns it.
    AddFolder { path: PathBuf, enable: bool },
    /// As [`Workspace::find_folder`] finds it; a scan of it skips it.
    RemoveFolder(String),
    SetEnabled { folder: String, enabled: bool },
    /// `None`: the first folder.
    SetHome(Option<PathBuf>),
    /// Runs the scans again; a scan of the same path takes the new depth.
    AddScan { path: PathBuf, depth: usize },
    /// The folders only it found go with it.
    RemoveScan(PathBuf),
    /// Of these folders, those a [`rescan`] does not find go; no scan skips them.
    DropGone(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edited {
    pub workspace: Workspace,
    pub added: Vec<String>,
    /// Their supervised sessions are the caller's to stop.
    pub removed: Vec<Folder>,
    /// `(folder, owner)`: disabled here since `owner` supervises it ([`Registry::claim_folders`]).
    pub claimed: Vec<(String, String)>,
}

fn directory(path: &Path) -> Result<PathBuf> {
    let abs = std::path::absolute(path).map_err(|_| CoreError::NotADirectory(path.to_path_buf()))?;
    if !abs.is_dir() {
        return Err(CoreError::NotADirectory(abs));
    }
    Ok(abs)
}

fn no_folder(w: &Workspace, folder: &str) -> CoreError {
    let known: Vec<&str> = w.folders.iter().map(|f| f.name.as_str()).collect();
    CoreError::NoFolder {
        workspace: w.name.clone(),
        folder: folder.to_string(),
        known: if known.is_empty() { "none".into() } else { known.join(", ") },
    }
}

impl Workspace {
    /// [`Self::add_folder`] of `path` made absolute, refused unless it is a folder.
    pub fn add_directory(&mut self, path: &Path) -> Result<&Folder> {
        let path = directory(path)?;
        Ok(self.add_folder(&path))
    }

    /// By name, then ignoring case, then by slug, then by path.
    pub fn find_folder(&self, which: &str) -> Option<&Folder> {
        let slug = crate::slugify(which);
        self.folders
            .iter()
            .find(|f| f.name == which)
            .or_else(|| self.folders.iter().find(|f| f.name.eq_ignore_ascii_case(which)))
            .or_else(|| self.folders.iter().find(|f| crate::slugify(&f.name) == slug))
            .or_else(|| self.folder_at(Path::new(which)))
    }
}

impl Registry {
    /// Under the registry's lock, not the workspace's: an enable reads every other workspace's folders.
    pub fn edit(&self, name: &str, edit: Edit) -> Result<Edited> {
        let path = self.path_of(name)?;
        let lock = path.with_file_name("registry.lock");
        let _held = crate::fs::lock_file(&lock).map_err(|e| CoreError::io(&lock, e))?;
        let mut w = self.load(name)?;
        let mut out = Edited { workspace: w.clone(), added: Vec::new(), removed: Vec::new(), claimed: Vec::new() };
        match edit {
            Edit::AddFolder { path, enable } => {
                let path = directory(&path)?;
                for s in &mut w.scans {
                    s.exclude.retain(|x| path_key(x) != path_key(&path));
                }
                if w.folder_at(&path).is_none() {
                    let folder = w.add_folder(&path);
                    out.added.push(folder.name.clone());
                    let i = w.folders.len() - 1;
                    w.folders[i].enabled = enable;
                }
                let added = out.added.clone();
                out.claimed = self.claim_folders(&mut w, |f| added.contains(&f.name))?;
            }
            Edit::RemoveFolder(which) => {
                let found = w.find_folder(&which).map(|f| f.name.clone()).ok_or_else(|| no_folder(&w, &which))?;
                let i = w.folders.iter().position(|f| f.name == found).unwrap();
                let folder = w.folders.remove(i);
                for s in w.scans.iter_mut().filter(|s| within(&folder.path, &s.path)) {
                    if !s.exclude.iter().any(|x| path_key(x) == path_key(&folder.path)) {
                        s.exclude.push(folder.path.clone());
                    }
                }
                out.removed.push(folder);
            }
            Edit::SetEnabled { folder, enabled } => {
                let found = w.find_folder(&folder).map(|f| f.name.clone()).ok_or_else(|| no_folder(&w, &folder))?;
                let f = w.folders.iter_mut().find(|x| x.name == found).unwrap();
                if enabled {
                    let others: Vec<Workspace> = self.list()?.into_iter().filter(|o| o.name != name).collect();
                    if let Some(owner) = owners(&others).get(&path_key(&f.path)) {
                        return Err(CoreError::SupervisedElsewhere { folder: f.name.clone(), owner: owner.clone() });
                    }
                }
                f.enabled = enabled;
            }
            Edit::SetHome(home) => w.home = home.as_deref().map(directory).transpose()?,
            Edit::AddScan { path, depth } => {
                let path = directory(&path)?;
                match w.scans.iter_mut().find(|s| path_key(&s.path) == path_key(&path)) {
                    Some(s) => s.depth = depth,
                    None => w.scans.push(Scan::new(path, depth)),
                }
                let (next, added, removed) = rescan(&w)?;
                out.removed = w.folders.iter().filter(|f| removed.contains(&f.name)).cloned().collect();
                w = next;
                out.claimed = self.claim_folders(&mut w, |f| added.contains(&f.name))?;
                out.added = added;
            }
            Edit::RemoveScan(path) => {
                let key = path_key(&std::path::absolute(&path).unwrap_or(path.clone()));
                let i = w.scans.iter().position(|s| path_key(&s.path) == key).ok_or_else(|| CoreError::NoScan { workspace: w.name.clone(), path: path.clone() })?;
                let scan = w.scans.remove(i);
                let only_it = |f: &Folder| !f.added && within(&f.path, &scan.path) && !w.scans.iter().any(|s| within(&f.path, &s.path));
                let (gone, kept): (Vec<Folder>, Vec<Folder>) = w.folders.iter().cloned().partition(only_it);
                w.folders = kept;
                out.removed = gone;
            }
            Edit::DropGone(seen) => {
                let (_, _, gone) = rescan(&w)?;
                let (removed, kept): (Vec<Folder>, Vec<Folder>) = w.folders.drain(..).partition(|f| gone.contains(&f.name) && seen.contains(&f.name));
                w.folders = kept;
                out.removed = removed;
            }
        }
        self.save(&w, true)?;
        out.workspace = w;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::FolderKind;

    fn repo_dir(path: &Path) {
        fs::create_dir_all(path.join(".git")).unwrap();
        fs::write(path.join(".git").join("HEAD"), "ref: refs/heads/main\n").unwrap();
    }

    fn setup() -> (tempfile::TempDir, Registry) {
        let tmp = tempfile::tempdir().unwrap();
        let reg = Registry::at(tmp.path().join("home"));
        let mut empty = Workspace::new("v", tmp.path(), Vec::new());
        empty.home = None;
        reg.save(&empty, false).unwrap();
        (tmp, reg)
    }

    fn names(w: &Workspace) -> Vec<(&str, FolderKind, bool)> {
        w.folders.iter().map(|f| (f.name.as_str(), f.kind, f.enabled)).collect()
    }

    #[test]
    fn an_empty_workspace_takes_folders_of_either_kind_a_home_and_loses_one() {
        let (tmp, reg) = setup();
        let (git, plain) = (tmp.path().join("Gaia"), tmp.path().join("NodeApi"));
        repo_dir(&git);
        fs::create_dir_all(&plain).unwrap();
        let e = reg.edit("v", Edit::AddFolder { path: git.clone(), enable: true }).unwrap();
        assert_eq!((e.added.as_slice(), e.claimed.len()), (["Gaia".to_string()].as_slice(), 0));
        reg.edit("v", Edit::AddFolder { path: plain.clone(), enable: false }).unwrap();
        assert!(reg.edit("v", Edit::AddFolder { path: plain.clone(), enable: true }).unwrap().added.is_empty(), "one per path");
        reg.edit("v", Edit::SetEnabled { folder: "nodeapi".into(), enabled: true }).unwrap();
        let w = reg.load("v").unwrap();
        assert_eq!(names(&w), [("Gaia", FolderKind::Git, true), ("NodeApi", FolderKind::Plain, true)]);
        assert_eq!(w.home(), Some(git.as_path()), "home defaults to the first folder");

        reg.edit("v", Edit::SetHome(Some(tmp.path().to_path_buf()))).unwrap();
        let e = reg.edit("v", Edit::RemoveFolder("Gaia".into())).unwrap();
        assert_eq!(e.removed.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["Gaia"]);
        let w = reg.load("v").unwrap();
        assert_eq!((w.home(), names(&w)), (Some(tmp.path()), vec![("NodeApi", FolderKind::Plain, true)]));
        reg.edit("v", Edit::SetHome(None)).unwrap();
        assert_eq!(reg.load("v").unwrap().home(), Some(plain.as_path()));

        assert!(matches!(reg.edit("v", Edit::RemoveFolder("Gaia".into())), Err(CoreError::NoFolder { known, .. }) if known == "NodeApi"));
        assert!(matches!(reg.edit("v", Edit::AddFolder { path: tmp.path().join("missing"), enable: true }), Err(CoreError::NotADirectory(_))));
        assert!(matches!(reg.edit("v", Edit::SetHome(Some(tmp.path().join("missing")))), Err(CoreError::NotADirectory(_))));
    }

    #[test]
    fn a_folder_owned_elsewhere_is_added_disabled_and_cannot_be_enabled() {
        let (tmp, reg) = setup();
        let shared = tmp.path().join("Skills");
        repo_dir(&shared);
        reg.save(&Workspace::new("a", tmp.path(), vec![Folder::new("Skills", &shared)]), false).unwrap();
        let e = reg.edit("v", Edit::AddFolder { path: shared.clone(), enable: true }).unwrap();
        assert_eq!(e.claimed, [("Skills".to_string(), "a".to_string())]);
        assert!(!e.workspace.folders[0].enabled);
        let refused = reg.edit("v", Edit::SetEnabled { folder: "Skills".into(), enabled: true });
        assert!(matches!(&refused, Err(CoreError::SupervisedElsewhere { owner, .. }) if owner == "a"), "{refused:?}");
        assert!(refused.unwrap_err().is_user_error());
        assert!(!reg.load("v").unwrap().folders[0].enabled, "nothing written");
    }

    #[test]
    fn two_workspaces_enabling_one_folder_at_once_leave_it_supervised_by_one() {
        let (tmp, reg) = setup();
        let shared = tmp.path().join("Skills");
        repo_dir(&shared);
        for w in ["a", "b"] {
            let mut ws = Workspace::new(w, tmp.path(), vec![Folder::new("Skills", &shared)]);
            ws.folders[0].enabled = false;
            reg.save(&ws, false).unwrap();
        }
        for _ in 0..20 {
            for w in ["a", "b"] {
                reg.edit(w, Edit::SetEnabled { folder: "Skills".into(), enabled: false }).unwrap();
            }
            let results: Vec<Result<Edited>> = std::thread::scope(|scope| {
                let enable = |w: &'static str| {
                    let reg = &reg;
                    scope.spawn(move || reg.edit(w, Edit::SetEnabled { folder: "Skills".into(), enabled: true }))
                };
                let (a, b) = (enable("a"), enable("b"));
                vec![a.join().unwrap(), b.join().unwrap()]
            });
            assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1, "{results:?}");
            assert!(results.iter().any(|r| matches!(r, Err(CoreError::SupervisedElsewhere { .. }))), "{results:?}");
            let enabled = ["a", "b"].iter().filter(|w| reg.load(w).unwrap().folders[0].enabled).count();
            assert_eq!(enabled, 1, "one owner for the folder");
        }
    }

    #[test]
    fn scans_add_their_repos_keep_a_removed_one_out_and_take_theirs_when_removed() {
        let (tmp, reg) = setup();
        let parent = tmp.path().join("P");
        for r in ["A", "B"] {
            repo_dir(&parent.join(r));
        }
        let by_hand = tmp.path().join("Own");
        fs::create_dir_all(&by_hand).unwrap();
        reg.edit("v", Edit::AddFolder { path: by_hand, enable: true }).unwrap();
        let e = reg.edit("v", Edit::AddScan { path: parent.clone(), depth: 1 }).unwrap();
        assert_eq!(e.added, ["A", "B"]);
        reg.edit("v", Edit::RemoveFolder("A".into())).unwrap();
        let again = reg.edit("v", Edit::AddScan { path: parent.clone(), depth: 1 }).unwrap();
        assert!(again.added.is_empty(), "a removed folder stays out of its scan");
        assert_eq!(again.workspace.scans, [Scan { exclude: vec![parent.join("A")], ..Scan::new(&parent, 1) }]);
        let back = reg.edit("v", Edit::AddFolder { path: parent.join("A"), enable: true }).unwrap();
        assert!(back.workspace.scans[0].exclude.is_empty(), "added again by hand: no longer skipped");

        let e = reg.edit("v", Edit::RemoveScan(parent.clone())).unwrap();
        assert_eq!(e.removed.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["B"]);
        let w = reg.load("v").unwrap();
        assert!(w.scans.is_empty());
        assert_eq!(w.folders.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["Own", "A"], "folders added by hand stay");
        assert!(matches!(reg.edit("v", Edit::RemoveScan(parent)), Err(CoreError::NoScan { .. })));
    }

    #[test]
    fn drop_gone_takes_only_the_scanned_folders_gone_from_disk_and_skips_none() {
        let (tmp, reg) = setup();
        let parent = tmp.path().join("P");
        for r in ["A", "B"] {
            repo_dir(&parent.join(r));
        }
        let by_hand = tmp.path().join("Own");
        fs::create_dir_all(&by_hand).unwrap();
        reg.edit("v", Edit::AddFolder { path: by_hand.clone(), enable: true }).unwrap();
        reg.edit("v", Edit::AddScan { path: parent.clone(), depth: 1 }).unwrap();
        fs::remove_dir_all(parent.join("A")).unwrap();
        repo_dir(&parent.join("C"));
        let e = reg.edit("v", Edit::DropGone(vec!["A".into()])).unwrap();
        assert_eq!(e.removed.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["A"]);
        assert!(e.added.is_empty(), "a new folder is not taken");
        let w = reg.load("v").unwrap();
        assert_eq!(w.folders.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["B", "Own"]);
        assert!(w.scans[0].exclude.is_empty());
    }

    #[test]
    fn add_directory_reads_the_folder_names_it_apart_and_refuses_a_missing_one() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, other) = (tmp.path().join("a").join("Repo"), tmp.path().join("b").join("Repo"));
        repo_dir(&repo);
        fs::create_dir_all(&other).unwrap();
        let mut w = Workspace::new("v", tmp.path(), Vec::new());
        let first = w.add_directory(&repo).unwrap().clone();
        assert_eq!((first.name.as_str(), first.branch.as_deref(), first.added), ("Repo", Some("main"), true));
        assert_eq!(w.add_directory(&other).unwrap().name, "Repo-2");
        assert_eq!(w.add_directory(&repo).unwrap().name, "Repo", "the folder already there");
        assert_eq!(w.folders.len(), 2);
        assert!(matches!(w.add_directory(&tmp.path().join("gone")), Err(CoreError::NotADirectory(_))));
    }
}
