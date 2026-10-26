//! What `fm workspace add` and the app's Add workspace share: a folder → a workspace to register, the
//! checks before registering it, and a rescan of a registered one.

use std::path::Path;

use crate::{CoreError, Registry, Result, Workspace, scan, slugify, validate_name};

/// `dir` made absolute, named `name` or the slug of its folder name, with the repos [`scan`] finds.
/// Nothing is written.
pub fn discover(dir: &Path, name: Option<&str>, depth: usize) -> Result<Workspace> {
    let root = std::path::absolute(dir).map_err(|_| CoreError::NotADirectory(dir.to_path_buf()))?;
    let name = match name {
        Some(name) => name.to_string(),
        None => slugify(&root.file_name().map(|f| f.to_string_lossy()).unwrap_or_default()),
    };
    validate_name(&name)?;
    let repos = scan(&root, depth)?;
    Ok(Workspace::new(name, root, repos))
}

/// The registered workspace scanned again: repos found now, each keeping the `enabled` it had; repos
/// gone from the disk are dropped. Returns it with the names added and removed. Nothing is written.
pub fn rescan(workspace: &Workspace, depth: usize) -> Result<(Workspace, Vec<String>, Vec<String>)> {
    let mut repos = scan(&workspace.root, depth)?;
    let mut added = Vec::new();
    for repo in &mut repos {
        match workspace.repos.iter().find(|r| r.name == repo.name) {
            Some(old) => repo.enabled = old.enabled,
            None => added.push(repo.name.clone()),
        }
    }
    let removed = workspace
        .repos
        .iter()
        .filter(|old| !repos.iter().any(|r| r.name == old.name))
        .map(|r| r.name.clone())
        .collect();
    Ok((Workspace { repos, ..workspace.clone() }, added, removed))
}

/// Windows paths compare ignoring case, separator style and a trailing separator.
pub fn same_root(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| p.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_lowercase();
    norm(a) == norm(b)
}

impl Registry {
    /// Why `workspace` cannot be registered as a new one: its name is taken, or its root already is
    /// another workspace's.
    pub fn check_new(&self, workspace: &Workspace) -> Result<()> {
        if self.exists(&workspace.name)? {
            return Err(CoreError::AlreadyExists(workspace.name.clone()));
        }
        if let Some(other) = self.list()?.into_iter().find(|w| same_root(&w.root, &workspace.root)) {
            return Err(CoreError::RootTaken { root: workspace.root.clone(), by: other.name });
        }
        Ok(())
    }

    /// Load, `edit`, save over it.
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
    use std::path::PathBuf;

    use super::*;

    fn repo_dir(root: &Path, name: &str) {
        fs::create_dir_all(root.join(name).join(".git")).unwrap();
        fs::write(root.join(name).join(".git").join("HEAD"), "ref: refs/heads/main\n").unwrap();
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
        assert_eq!(w.repos.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["Alpha", "Beta"]);
        assert_eq!(discover(&root, Some("x"), 1).unwrap().name, "x");
        assert!(matches!(discover(&root, Some("Bad Name"), 1), Err(CoreError::InvalidName(_))));
        assert!(matches!(discover(&root.join("nope"), None, 1), Err(CoreError::NotADirectory(_))));
    }

    #[test]
    fn check_new_refuses_a_taken_name_or_root() {
        let dir = tempfile::tempdir().unwrap();
        let reg = Registry::at(dir.path().join("home"));
        let root = dir.path().join("ws");
        repo_dir(&root, "Alpha");
        let w = discover(&root, Some("ws"), 1).unwrap();
        reg.check_new(&w).unwrap();
        reg.save(&w, false).unwrap();
        assert!(matches!(reg.check_new(&w), Err(CoreError::AlreadyExists(n)) if n == "ws"));
        let upper = PathBuf::from(root.to_string_lossy().to_uppercase() + "\\");
        let other = Workspace::new("other", upper, vec![]);
        let err = reg.check_new(&other).unwrap_err();
        assert!(matches!(&err, CoreError::RootTaken { by, .. } if by == "ws"), "{err}");
        assert!(err.is_user_error());
    }

    #[test]
    fn rescan_keeps_enabled_and_reports_changes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("ws");
        repo_dir(&root, "Alpha");
        repo_dir(&root, "Beta");
        let mut w = discover(&root, Some("ws"), 1).unwrap();
        w.repos[1].enabled = false;
        fs::remove_dir_all(root.join("Alpha")).unwrap();
        repo_dir(&root, "Gamma");
        let (after, added, removed) = rescan(&w, 1).unwrap();
        let names: Vec<(&str, bool)> = after.repos.iter().map(|r| (r.name.as_str(), r.enabled)).collect();
        assert_eq!(names, [("Beta", false), ("Gamma", true)]);
        assert_eq!(added, ["Gamma"]);
        assert_eq!(removed, ["Alpha"]);
        assert_eq!(after.created_at, w.created_at);
    }

    #[test]
    fn update_edits_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let reg = Registry::at(dir.path());
        let root = dir.path().join("ws");
        repo_dir(&root, "Alpha");
        reg.save(&discover(&root, Some("ws"), 1).unwrap(), false).unwrap();
        reg.update("ws", |w| w.repos[0].enabled = false).unwrap();
        assert!(!reg.load("ws").unwrap().repos[0].enabled);
        assert!(matches!(reg.update("nope", |_| {}), Err(CoreError::NotFound(_))));
    }
}
