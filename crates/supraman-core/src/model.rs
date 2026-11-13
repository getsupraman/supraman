//! A workspace file's shape: the schedules' bridge (folders/schedules removes it) and folders.toml's one-time migration.

use std::path::{Path, PathBuf};

use chrono::{DateTime, SubsecRound, Utc};
use serde::{Deserialize, Serialize};

use crate::paths::{path_key, within};

/// A name, the folder its own sessions run in, and the folders it holds. A file in the root-based shape
/// (`root` and `[[repos]]`) loads with the root as home.
// `folders` stays last: TOML emits it as an array of tables, which must follow the plain keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "Stored")]
pub struct Workspace {
    pub name: String,
    /// `None`: the first folder's path ([`Self::home`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub home: Option<PathBuf>,
    pub created_at: DateTime<Utc>,
    pub folders: Vec<Folder>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FolderKind {
    #[default]
    Git,
    Plain,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Folder {
    /// Unique within the workspace: what a schedule's `folder` names.
    pub name: String,
    pub path: PathBuf,
    #[serde(default)]
    pub kind: FolderKind,
    /// `None` for a plain folder or a detached HEAD.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

#[derive(Deserialize)]
struct Stored {
    name: String,
    #[serde(default)]
    home: Option<PathBuf>,
    #[serde(default)]
    root: Option<PathBuf>,
    created_at: DateTime<Utc>,
    #[serde(default, alias = "repos")]
    folders: Vec<Folder>,
}

impl From<Stored> for Workspace {
    fn from(s: Stored) -> Self {
        Self { name: s.name, home: s.home.or(s.root), created_at: s.created_at, folders: s.folders }
    }
}

impl Workspace {
    pub fn new(name: impl Into<String>, home: impl Into<PathBuf>, folders: Vec<Folder>) -> Self {
        Self { name: name.into(), home: Some(home.into()), created_at: Utc::now().trunc_subsecs(0), folders }
    }

    /// Where the workspace's own sessions run: `home`, else the first folder.
    pub fn home(&self) -> Option<&Path> {
        self.home.as_deref().or_else(|| self.folders.first().map(|f| f.path.as_path()))
    }

    /// `dir` lies in the home or in one of the folders.
    pub fn contains(&self, dir: &Path) -> bool {
        self.home().is_some_and(|h| within(dir, h)) || self.folders.iter().any(|f| within(dir, &f.path))
    }

    pub fn folder(&self, name: &str) -> Option<&Folder> {
        self.folders.iter().find(|f| f.name == name)
    }

    /// By name, then ignoring case, then by slug, then by path.
    pub fn find_folder(&self, which: &str) -> Option<&Folder> {
        let slug = crate::slugify(which);
        let at = path_key(Path::new(which));
        self.folders
            .iter()
            .find(|f| f.name == which)
            .or_else(|| self.folders.iter().find(|f| f.name.eq_ignore_ascii_case(which)))
            .or_else(|| self.folders.iter().find(|f| crate::slugify(&f.name) == slug))
            .or_else(|| self.folders.iter().find(|f| path_key(&f.path) == at))
    }
}

impl Folder {
    /// A git folder with no branch; nothing is read from the disk.
    pub fn new(name: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self { name: name.into(), path: path.into(), kind: FolderKind::Git, branch: None }
    }

    pub fn is_git(&self) -> bool {
        self.kind == FolderKind::Git
    }
}

/// The workspace whose home or folder holds `cwd`, the deepest match winning.
pub fn holder_of<'a>(cwd: &Path, workspaces: &'a [Workspace]) -> Option<&'a Workspace> {
    let depth = |w: &Workspace| {
        w.home()
            .into_iter()
            .chain(w.folders.iter().map(|f| f.path.as_path()))
            .filter(|d| within(cwd, d))
            .map(|d| path_key(d).len())
            .max()
    };
    workspaces.iter().filter_map(|w| depth(w).map(|d| (d, w))).max_by_key(|(d, _)| *d).map(|(_, w)| w)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEGACY: &str = r#"
name = "plat"
root = 'C:\Plat'
created_at = "2026-10-28T07:07:16Z"

[[scans]]
path = 'C:\Plat'
depth = 2

[[repos]]
name = "Alpha"
path = 'C:\Plat\Alpha'
has_claude_md = true
branch = "master"
enabled = true
"#;

    #[test]
    fn a_root_based_file_loads_with_the_root_as_home_and_its_repos_as_folders() {
        let w: Workspace = toml::from_str(LEGACY).unwrap();
        assert_eq!(w.home(), Some(Path::new(r"C:\Plat")));
        assert_eq!(w.folders, [Folder { branch: Some("master".into()), ..Folder::new("Alpha", r"C:\Plat\Alpha") }]);
        let text = toml::to_string(&w).unwrap();
        assert_eq!(toml::from_str::<Workspace>(&text).unwrap(), w, "the new shape reads back the same");
    }

    #[test]
    fn home_defaults_to_the_first_folder() {
        let mut w = Workspace::new("v", r"C:\x", vec![Folder::new("A", r"C:\a"), Folder::new("B", r"C:\b")]);
        w.home = None;
        assert_eq!(w.home(), Some(Path::new(r"C:\a")));
        assert!(w.contains(Path::new(r"C:\b\src")) && !w.contains(Path::new(r"C:\bee")));
        w.folders.clear();
        assert_eq!(w.home(), None);
    }

    #[test]
    fn the_deepest_workspace_holds_a_folder() {
        let outer = Workspace::new("outer", r"P:\Projects", vec![]);
        let inner = Workspace::new("inner", r"P:\Projects\Gaia", vec![Folder::new("Api", r"Q:\Api")]);
        let all = [outer, inner];
        assert_eq!(holder_of(Path::new(r"P:\Projects\Gaia\src"), &all).map(|w| w.name.as_str()), Some("inner"));
        assert_eq!(holder_of(Path::new(r"Q:\Api\x"), &all).map(|w| w.name.as_str()), Some("inner"));
        assert_eq!(holder_of(Path::new(r"P:\Projects\Other"), &all).map(|w| w.name.as_str()), Some("outer"));
        assert!(holder_of(Path::new(r"D:\x"), &all).is_none());
    }
}
