//! The one reader of the earlier `<home>/workspaces/*.toml`: folders.toml's first entries and the schedules'
//! move to their folders. Compat code removed in 0.1.19 with `end_controllers` (backlog/end-controllers-removal).

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::paths::path_key;
use crate::validate_name;

use super::{Entries, Entry, FolderEntry};

/// A workspace file as an earlier release wrote it; a root-based one (`root`, `[[repos]]`) too.
#[derive(Deserialize)]
struct Workspace {
    name: String,
    #[serde(default, alias = "repos")]
    folders: Vec<Folder>,
}

#[derive(Deserialize)]
struct Folder {
    name: String,
    path: PathBuf,
}

fn workspaces_dir(home: &Path) -> PathBuf {
    home.join("workspaces")
}

/// Every workspace file that loads, by name; one that does not is left out.
fn load_all(home: &Path) -> Vec<Workspace> {
    let Ok(entries) = fs::read_dir(workspaces_dir(home)) else { return Vec::new() };
    let mut out: Vec<Workspace> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "toml") && p.file_stem().and_then(|s| s.to_str()).is_some_and(|s| validate_name(s).is_ok()))
        .filter_map(|p| toml::from_str(&fs::read_to_string(p).ok()?).ok())
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Each workspace that loads, by name, as a group of its folders; a path already placed keeps its first place.
pub(super) fn from_workspaces(home: &Path) -> Entries {
    let mut placed = HashSet::new();
    let list = load_all(home)
        .into_iter()
        .map(|w| Entry::Group {
            folders: w.folders.into_iter().filter(|f| placed.insert(path_key(&f.path))).map(|f| FolderEntry::new(f.path)).collect(),
            name: w.name,
        })
        .collect();
    Entries::new(list)
}

/// Each workspace's folders as `(name, path)`, by workspace name: what a schedule of that workspace named.
pub(crate) fn workspace_folders(home: &Path) -> Vec<(String, Vec<(String, PathBuf)>)> {
    load_all(home).into_iter().map(|w| (w.name, w.folders.into_iter().map(|f| (f.name, f.path)).collect())).collect()
}

/// A workspace file in the earlier shape, for a test of a migration.
#[cfg(test)]
pub(crate) fn write_workspace(home: &Path, name: &str, folders: &[&Path]) {
    let mut text = format!("name = \"{name}\"\ncreated_at = \"2026-10-28T07:07:16Z\"\n");
    for f in folders {
        text.push_str(&format!("\n[[folders]]\nname = \"{}\"\npath = '{}'\n", crate::paths::folder_name(f), f.display()));
    }
    fs::create_dir_all(workspaces_dir(home)).unwrap();
    fs::write(workspaces_dir(home).join(format!("{name}.toml")), text).unwrap();
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
branch = "master"
"#;

    #[test]
    fn a_root_based_file_reads_its_repos_as_folders() {
        let home = tempfile::tempdir().unwrap();
        fs::create_dir_all(workspaces_dir(home.path())).unwrap();
        fs::write(workspaces_dir(home.path()).join("plat.toml"), LEGACY).unwrap();
        fs::write(workspaces_dir(home.path()).join("broken.toml"), "name = ").unwrap();
        assert_eq!(workspace_folders(home.path()), [("plat".to_string(), vec![("Alpha".to_string(), PathBuf::from(r"C:\Plat\Alpha"))])]);
    }
}
