use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, SubsecRound, Utc};
use serde::{Deserialize, Serialize};

use crate::paths::{path_key, within};

/// A name, the folder its own sessions run in, and the folders it holds. A file in the root-based shape
/// (`root` and `[[repos]]`) loads as one scan of the root plus the repos found, with the root as home.
// `scans` and `folders` stay last: TOML emits them as arrays of tables, which must follow the plain keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "Stored")]
pub struct Workspace {
    pub name: String,
    /// `None`: the first folder's path ([`Self::home`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub home: Option<PathBuf>,
    pub created_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub scans: Vec<Scan>,
    pub folders: Vec<Folder>,
}

/// A parent folder whose git repos, at most `depth` levels down, [`crate::rescan`] adds as folders.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scan {
    pub path: PathBuf,
    pub depth: usize,
    /// Folders removed by hand: a rescan does not add them again.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<PathBuf>,
}

impl Scan {
    pub fn new(path: impl Into<PathBuf>, depth: usize) -> Self {
        Self { path: path.into(), depth, exclude: Vec::new() }
    }
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
    /// Unique within the workspace by its slug: the `<repo>` of `fm-<ws>-<repo>`. A scan names a folder by
    /// its path below the scanned folder, `/`-separated ([`crate::scan`]).
    pub name: String,
    pub path: PathBuf,
    #[serde(default)]
    pub kind: FolderKind,
    #[serde(default)]
    pub has_claude_md: bool,
    #[serde(default)]
    pub has_agents_md: bool,
    /// `None` for a plain folder or a detached HEAD.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Shown with its sessions and may have a supervised session; one workspace at most has a folder enabled
    /// ([`owners`]).
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Added by hand rather than found by a scan: a rescan keeps it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub added: bool,
}

fn yes() -> bool {
    true
}

fn is_false(b: &bool) -> bool {
    !b
}

#[derive(Deserialize)]
struct Stored {
    name: String,
    #[serde(default)]
    home: Option<PathBuf>,
    #[serde(default)]
    root: Option<PathBuf>,
    created_at: DateTime<Utc>,
    #[serde(default)]
    scans: Vec<Scan>,
    #[serde(default, alias = "repos")]
    folders: Vec<Folder>,
}

impl From<Stored> for Workspace {
    fn from(s: Stored) -> Self {
        let mut scans = s.scans;
        if let Some(root) = &s.root
            && scans.is_empty()
        {
            let depth = s.folders.iter().map(|f| f.name.split('/').count()).max().unwrap_or(1).max(1);
            scans.push(Scan::new(root.clone(), depth));
        }
        Self {
            name: s.name,
            home: s.home.or(s.root),
            created_at: s.created_at,
            scans,
            folders: s.folders,
        }
    }
}

impl Workspace {
    pub fn new(name: impl Into<String>, home: impl Into<PathBuf>, folders: Vec<Folder>) -> Self {
        Self {
            name: name.into(),
            home: Some(home.into()),
            created_at: Utc::now().trunc_subsecs(0),
            scans: Vec::new(),
            folders,
        }
    }

    /// Where the workspace and relay sessions run: `home`, else the first folder.
    pub fn home(&self) -> Option<&Path> {
        self.home.as_deref().or_else(|| self.folders.first().map(|f| f.path.as_path()))
    }

    /// `dir` lies in the home or in one of the folders.
    pub fn contains(&self, dir: &Path) -> bool {
        self.home().is_some_and(|h| within(dir, h)) || self.folders.iter().any(|f| within(dir, &f.path))
    }

    /// The folders and scans outside the home, once each: the workspace session's `--add-dir`s.
    pub fn other_dirs(&self) -> Vec<PathBuf> {
        let home = self.home();
        let mut out: Vec<PathBuf> = Vec::new();
        let dirs = self.folders.iter().map(|f| &f.path).chain(self.scans.iter().map(|s| &s.path));
        for dir in dirs {
            let inside = home.is_some_and(|h| within(dir, h)) || out.iter().any(|o| within(dir, o));
            if !inside {
                out.retain(|o| !within(o, dir));
                out.push(dir.clone());
            }
        }
        out
    }

    pub fn folder(&self, name: &str) -> Option<&Folder> {
        self.folders.iter().find(|f| f.name == name)
    }

    pub fn folder_at(&self, path: &Path) -> Option<&Folder> {
        let key = path_key(path);
        self.folders.iter().find(|f| path_key(&f.path) == key)
    }

    /// `base`, or `base-2`, `base-3`, … : the first whose slug no folder's name has.
    pub fn unique_folder_name(&self, base: &str) -> String {
        let taken = |n: &str| self.folders.iter().any(|f| crate::slugify(&f.name) == crate::slugify(n));
        if !taken(base) {
            return base.to_string();
        }
        (2..).map(|i| format!("{base}-{i}")).find(|n| !taken(n)).unwrap()
    }

    /// `path` as a folder added by hand, named after its folder ([`Self::unique_folder_name`]), its kind read
    /// from the disk; the folder already there when the workspace has it.
    pub fn add_folder(&mut self, path: &Path) -> &Folder {
        if let Some(i) = self.folders.iter().position(|f| path_key(&f.path) == path_key(path)) {
            return &self.folders[i];
        }
        let base = path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_else(|| "folder".into());
        let name = self.unique_folder_name(&base);
        self.folders.push(Folder { added: true, ..crate::scan::folder_at(name, path.to_path_buf()) });
        self.folders.last().unwrap()
    }
}

impl Folder {
    /// A git folder with no branch, no CLAUDE.md or AGENTS.md, enabled; nothing is read from the disk.
    pub fn new(name: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self {
            name: name.into(),
            path: path.into(),
            kind: FolderKind::Git,
            has_claude_md: false,
            has_agents_md: false,
            branch: None,
            enabled: true,
            added: false,
        }
    }

    pub fn is_git(&self) -> bool {
        self.kind == FolderKind::Git
    }
}

/// Which workspace a folder's supervised session belongs to, by the folder's [`path_key`]: the first
/// workspace in `workspaces` that has it enabled.
pub fn owners(workspaces: &[Workspace]) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for w in workspaces {
        for f in w.folders.iter().filter(|f| f.enabled) {
            out.entry(path_key(&f.path)).or_insert_with(|| w.name.clone());
        }
    }
    out
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

[[repos]]
name = "Alpha"
path = 'C:\Plat\Alpha'
has_claude_md = true
has_agents_md = false
branch = "master"
enabled = true

[[repos]]
name = "group/deep"
path = 'C:\Plat\group\deep'
has_claude_md = false
has_agents_md = false
enabled = false
"#;

    #[test]
    fn a_root_based_file_is_one_scan_of_its_root_with_the_root_as_home() {
        let w: Workspace = toml::from_str(LEGACY).unwrap();
        assert_eq!(w.home(), Some(Path::new(r"C:\Plat")));
        assert_eq!(w.scans, [Scan::new(r"C:\Plat", 2)], "deep enough for group/deep");
        let names: Vec<(&str, FolderKind, bool)> = w.folders.iter().map(|f| (f.name.as_str(), f.kind, f.enabled)).collect();
        assert_eq!(names, [("Alpha", FolderKind::Git, true), ("group/deep", FolderKind::Git, false)]);
        assert_eq!(w.folders[0].branch.as_deref(), Some("master"));

        let text = toml::to_string(&w).unwrap();
        assert!(text.contains("home = ") && text.contains("[[scans]]") && text.contains("[[folders]]"), "{text}");
        assert!(!text.contains("root") && !text.contains("[[repos]]"), "{text}");
        assert_eq!(toml::from_str::<Workspace>(&text).unwrap(), w, "the new shape reads back the same");
    }

    #[test]
    fn home_defaults_to_the_first_folder() {
        let mut w = Workspace::new("v", r"C:\x", vec![Folder::new("A", r"C:\a"), Folder::new("B", r"C:\b")]);
        w.home = None;
        assert_eq!(w.home(), Some(Path::new(r"C:\a")));
        let text = toml::to_string(&w).unwrap();
        assert!(!text.contains("home"), "{text}");
        assert_eq!(toml::from_str::<Workspace>(&text).unwrap(), w);
        w.folders.clear();
        assert_eq!(w.home(), None);
    }

    #[test]
    fn contains_and_other_dirs_cover_the_home_and_every_folder() {
        let mut w = Workspace::new(
            "gaia",
            r"C:\P\Gaia",
            vec![Folder::new("Gaia", r"C:\P\Gaia"), Folder::new("NodeApi", r"C:\P\NodeApi"), Folder::new("sub", r"C:\P\NodeApi\sub")],
        );
        assert!(w.contains(Path::new(r"C:\P\NodeApi\src")));
        assert!(w.contains(Path::new(r"C:\P\Gaia\x")));
        assert!(!w.contains(Path::new(r"C:\P\Gaia.Ext")));
        assert_eq!(w.other_dirs(), [PathBuf::from(r"C:\P\NodeApi")], "the home and nested folders need none");
        w.scans.push(Scan::new(r"C:\Q", 1));
        assert_eq!(w.other_dirs(), [PathBuf::from(r"C:\P\NodeApi"), PathBuf::from(r"C:\Q")]);
    }

    #[test]
    fn folder_names_are_unique_by_slug() {
        let w = Workspace::new("w", r"C:\w", vec![Folder::new("Nomos.Pdf", r"C:\w\a"), Folder::new("x-2", r"C:\w\b")]);
        assert_eq!(w.unique_folder_name("nomos-pdf"), "nomos-pdf-2");
        assert_eq!(w.unique_folder_name("x"), "x");
        let w2 = Workspace::new("w", r"C:\w", vec![Folder::new("x", r"C:\w\a"), Folder::new("x-2", r"C:\w\b")]);
        assert_eq!(w2.unique_folder_name("X"), "X-3");
    }

    #[test]
    fn add_folder_reads_its_kind_and_keeps_one_per_path() {
        let tmp = tempfile::tempdir().unwrap();
        let (git, plain) = (tmp.path().join("Gaia"), tmp.path().join("NodeApi"));
        std::fs::create_dir_all(git.join(".git")).unwrap();
        std::fs::write(git.join(".git").join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::create_dir_all(&plain).unwrap();
        let mut w = Workspace::new("gaia", &git, Vec::new());
        assert_eq!((w.add_folder(&git).kind, w.folders[0].branch.as_deref()), (FolderKind::Git, Some("main")));
        let p = w.add_folder(&plain).clone();
        assert_eq!((p.name.as_str(), p.kind, p.branch, p.added, p.enabled), ("NodeApi", FolderKind::Plain, None, true, true));
        w.add_folder(&plain);
        assert_eq!(w.folders.len(), 2);
    }

    #[test]
    fn a_folder_enabled_in_two_workspaces_belongs_to_the_first() {
        let a = Workspace::new("a", r"C:\P", vec![Folder::new("Skills", r"C:\P\Skills")]);
        let mut b = Workspace::new("b", r"C:\X", vec![Folder::new("Skills", r"c:/p/skills"), Folder::new("Own", r"C:\X\Own")]);
        let o = owners(&[a.clone(), b.clone()]);
        assert_eq!(o[&path_key(Path::new(r"C:\P\Skills"))], "a");
        assert_eq!(o[&path_key(Path::new(r"C:\X\Own"))], "b");
        b.folders[0].enabled = false;
        let mut a_off = a.clone();
        a_off.folders[0].enabled = false;
        assert!(!owners(&[a_off, b]).contains_key(&path_key(Path::new(r"C:\P\Skills"))));
    }
}
