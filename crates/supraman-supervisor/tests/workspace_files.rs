//! Running sessions are matched by name, so a workspace file in the root-based shape must derive the same
//! session names when it loads into the folders model and after it is saved in the new shape.

use std::path::PathBuf;

use supraman_core::{FolderKind, Registry, Workspace};
use supraman_supervisor::{relay_session_name, repo_session_name, workspace_session_name};

const LEGACY: &str = r#"name = "plat"
root = 'C:\Plat'
created_at = "2026-10-22T20:36:39Z"

[[repos]]
name = "Plat-root"
path = 'C:\Plat'
has_claude_md = false
has_agents_md = false
branch = "main"
enabled = true

[[repos]]
name = "Nomos.Pdf"
path = 'C:\Plat\Nomos.Pdf'
has_claude_md = true
has_agents_md = true
branch = "main"
enabled = true

[[repos]]
name = "_Logo"
path = 'C:\Plat\_Logo'
has_claude_md = false
has_agents_md = false
enabled = false

[[repos]]
name = "group/deep"
path = 'C:\Plat\group\deep'
has_claude_md = false
has_agents_md = false
branch = "copilot/fix-5739e0dd"
enabled = true
"#;

/// The names as the root-based file states them, read without the folders model.
fn names_from_text(text: &str) -> (String, PathBuf, Vec<(String, PathBuf, bool)>) {
    let raw: toml::Value = toml::from_str(text).unwrap();
    let ws = raw["name"].as_str().unwrap().to_string();
    let root = PathBuf::from(raw["root"].as_str().unwrap());
    let repos = raw["repos"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r["name"].as_str().unwrap().to_string(), PathBuf::from(r["path"].as_str().unwrap()), r["enabled"].as_bool().unwrap()))
        .collect();
    (ws, root, repos)
}

fn session_names(ws: &str, repos: impl Iterator<Item = String>) -> Vec<String> {
    let mut out = vec![workspace_session_name(ws), relay_session_name(ws)];
    out.extend(repos.map(|r| repo_session_name(ws, &r).unwrap()));
    out
}

fn check(text: &str) {
    let (ws, root, repos) = names_from_text(text);
    let before = session_names(&ws, repos.iter().map(|r| r.0.clone()));

    let home = tempfile::tempdir().unwrap();
    let reg = Registry::at(home.path());
    std::fs::create_dir_all(reg.workspaces_dir()).unwrap();
    let file = reg.path_of(&ws).unwrap();
    std::fs::write(&file, text).unwrap();

    let loaded = reg.load(&ws).unwrap();
    assert_eq!(loaded.home(), Some(root.as_path()), "{ws}: the root is the home");
    assert_eq!(loaded.scans.len(), 1, "{ws}: one scan of the root");
    assert_eq!(loaded.scans[0].path, root);
    let folders: Vec<(String, PathBuf, bool)> = loaded.folders.iter().map(|f| (f.name.clone(), f.path.clone(), f.enabled)).collect();
    assert_eq!(folders, repos, "{ws}: every repo is a folder, in order, enabled as it was");
    assert!(loaded.folders.iter().all(|f| f.kind == FolderKind::Git));
    let after_load = session_names(&loaded.name, loaded.folders.iter().map(|f| f.name.clone()));
    assert_eq!(after_load, before, "{ws}: the names a loaded file derives");

    reg.save(&loaded, true).unwrap();
    let saved = std::fs::read_to_string(&file).unwrap();
    assert!(saved.contains("[[folders]]") && saved.contains("[[scans]]") && !saved.contains("[[repos]]"), "{saved}");
    let again: Workspace = reg.load(&ws).unwrap();
    assert_eq!(again, loaded, "{ws}: the new shape reads back the same");
    let after_save = session_names(&again.name, again.folders.iter().map(|f| f.name.clone()));
    assert_eq!(after_save, before, "{ws}: the names after a save in the new shape");
}

#[test]
fn a_root_based_file_keeps_its_session_names_through_load_and_save() {
    check(LEGACY);
}
