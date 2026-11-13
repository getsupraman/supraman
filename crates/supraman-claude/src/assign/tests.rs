use std::fs;
use std::path::PathBuf;

use supraman_core::folders::{Entry, FolderEntry, Shown};
use supraman_core::paths::{folder_name, normalize};

use super::*;
use crate::parse;

/// tmp/
///   Plat/Alpha/.git/          repo
///   Plat/Alpha/src/
///   Plat/Alpha.Ext/.git/      repo whose name shares a prefix with Alpha
///   Plat/Alpha.worktrees/feat/.git  file → ../../Alpha/.git/worktrees/feat
///   Plat/Alpha.worktrees/abs/.git   file → <abs Alpha>/.git/worktrees/abs
///   Elsewhere/
fn layout() -> (tempfile::TempDir, Entries) {
    let tmp = tempfile::tempdir().unwrap();
    let plat = tmp.path().join("Plat");
    let alpha = plat.join("Alpha");
    fs::create_dir_all(alpha.join(".git").join("worktrees")).unwrap();
    fs::create_dir_all(alpha.join("src")).unwrap();
    fs::create_dir_all(plat.join("Alpha.Ext").join(".git")).unwrap();
    let wts = plat.join("Alpha.worktrees");
    fs::create_dir_all(wts.join("feat").join("deep")).unwrap();
    fs::write(wts.join("feat").join(".git"), "gitdir: ../../Alpha/.git/worktrees/feat\n").unwrap();
    fs::create_dir_all(wts.join("abs")).unwrap();
    let abs = format!("gitdir: {}\n", alpha.join(".git").join("worktrees").join("abs").display());
    fs::write(wts.join("abs").join(".git"), abs).unwrap();
    fs::create_dir_all(tmp.path().join("Elsewhere")).unwrap();
    let group = Entry::Group { name: "plat".into(), folders: vec![FolderEntry::new(alpha), FolderEntry::new(plat.join("Alpha.Ext"))] };
    (tmp, Entries::new(vec![group]))
}

fn session(id: &str, cwd: &Path) -> Session {
    let json = serde_json::json!([{
        "id": id, "sessionId": format!("{id}-0000"), "name": format!("fm-{id}"),
        "cwd": cwd, "kind": "background", "state": "working",
    }]);
    parse(&json.to_string()).unwrap().remove(0)
}

fn folder_of(cwd: &Path, entries: &Entries) -> Option<String> {
    holding(cwd, entries, main_repo).map(|f| folder_name(&f.path))
}

#[test]
fn repo_path_and_below() {
    let (tmp, p) = layout();
    let plat = tmp.path().join("Plat");
    assert_eq!(folder_of(&plat.join("Alpha"), &p).as_deref(), Some("Alpha"));
    assert_eq!(folder_of(&plat.join("Alpha").join("src"), &p).as_deref(), Some("Alpha"));
    assert_eq!(folder_of(&plat.join("Alpha.Ext"), &p).as_deref(), Some("Alpha.Ext"));
    assert_eq!(folder_of(&plat.join("Alpha").join("not-created-yet"), &p).as_deref(), Some("Alpha"));
}

#[test]
fn the_deepest_listed_folder_takes_a_session_wherever_it_is_listed() {
    let (tmp, mut p) = layout();
    let plat = tmp.path().join("Plat");
    p.list.insert(0, Entry::Folder(FolderEntry::new(&plat)));
    assert_eq!(folder_of(&plat, &p).as_deref(), Some("Plat"));
    assert_eq!(folder_of(&plat.join("docs"), &p).as_deref(), Some("Plat"));
    assert_eq!(folder_of(&plat.join("Alpha").join("src"), &p).as_deref(), Some("Alpha"), "never the parent");
    assert_eq!(folder_of(&plat.join("Alpha.Ext"), &p).as_deref(), Some("Alpha.Ext"));
    assert_eq!(folder_of(&tmp.path().join("Elsewhere"), &p), None);
}

#[test]
fn worktree_maps_to_its_main_checkout_before_a_listed_parent() {
    let (tmp, mut p) = layout();
    let plat = tmp.path().join("Plat");
    let wts = plat.join("Alpha.worktrees");
    assert_eq!(folder_of(&wts.join("feat"), &p).as_deref(), Some("Alpha"), "relative gitdir");
    assert_eq!(folder_of(&wts.join("feat").join("deep"), &p).as_deref(), Some("Alpha"), "below a worktree");
    assert_eq!(folder_of(&wts.join("abs"), &p).as_deref(), Some("Alpha"), "absolute gitdir");
    p.list.push(Entry::Folder(FolderEntry::new(&plat)));
    assert_eq!(folder_of(&wts.join("feat"), &p).as_deref(), Some("Alpha"), "the main checkout is deeper than the parent holding the worktree");
}

#[test]
fn main_repo_from_git_dir_and_worktree_files() {
    let (tmp, _) = layout();
    let plat = tmp.path().join("Plat");
    let alpha = normalize(&plat.join("Alpha"));
    let found = |dir: &Path| main_repo(dir).map(|p| p.to_string_lossy().into_owned());
    assert_eq!(found(&plat.join("Alpha")).as_deref(), Some(alpha.as_str()), ".git dir");
    assert_eq!(found(&plat.join("Alpha").join("src")).as_deref(), Some(alpha.as_str()), "below a .git dir");
    let wts = plat.join("Alpha.worktrees");
    assert_eq!(found(&wts.join("abs")).as_deref(), Some(alpha.as_str()), "absolute gitdir");
    assert_eq!(found(&wts.join("feat").join("deep")).as_deref(), Some(alpha.as_str()), "relative gitdir");
    assert!(alpha.ends_with("/Plat/Alpha"), "case is kept: {alpha}");
}

#[test]
fn main_repo_without_git_is_none() {
    let (tmp, _) = layout();
    if main_repo(tmp.path()).is_some() {
        return; // the temp dir itself lies inside some repo; nothing to assert
    }
    assert_eq!(main_repo(&tmp.path().join("Elsewhere")), None);
    assert_eq!(main_repo(&tmp.path().join("Plat").join("Alpha.worktrees")), None);
}

#[test]
fn outside_every_listed_folder_is_unplaced() {
    let (tmp, p) = layout();
    assert_eq!(folder_of(&tmp.path().join("Elsewhere"), &p), None);
    assert_eq!(folder_of(&tmp.path().join("Plat"), &p), None);
    assert_eq!(folder_of(&tmp.path().join("Plat").join("Alpha.worktrees"), &p), None);
    assert_eq!(folder_of(&tmp.path().join("Plat").join("Alph"), &p), None);
}

#[cfg(windows)]
#[test]
fn case_and_slashes_do_not_matter_on_windows() {
    let (tmp, p) = layout();
    let raw = tmp.path().join("Plat").join("Alpha").join("src").to_string_lossy().into_owned();
    let shouted = PathBuf::from(raw.to_uppercase().replace('\\', "/") + "/");
    assert_eq!(folder_of(&shouted, &p).as_deref(), Some("Alpha"));
    let verbatim = PathBuf::from(format!(r"\\?\{}", tmp.path().join("plat").join("alpha.ext").display()));
    assert_eq!(folder_of(&verbatim, &p).as_deref(), Some("Alpha.Ext"));
}

#[test]
fn place_groups_by_folder_and_keeps_the_rest() {
    let (tmp, p) = layout();
    let plat = tmp.path().join("Plat");
    let sessions = vec![
        session("a1", &plat.join("Alpha")),
        session("a2", &plat.join("Alpha.worktrees").join("feat")),
        session("e1", &plat.join("Alpha.Ext")),
        session("x1", &tmp.path().join("Elsewhere")),
    ];
    let placed = place(&sessions, &p, &Shown::new());
    let ids = |dir: PathBuf| placed.by_folder[&path_key(&dir)].iter().map(|s| s.id.as_str()).collect::<Vec<_>>();
    assert_eq!(ids(plat.join("Alpha")), ["a1", "a2"]);
    assert_eq!(ids(plat.join("Alpha.Ext")), ["e1"]);
    assert_eq!(placed.unplaced.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["x1"]);
}

#[test]
fn a_session_under_a_shown_child_is_placed_under_the_child_not_its_parent() {
    let tmp = tempfile::tempdir().unwrap();
    let larch = tmp.path().join("LarchSys");
    let gaia = larch.join("gaia");
    fs::create_dir_all(gaia.join("src")).unwrap();
    let mut parent = FolderEntry::new(&larch);
    parent.show_subfolders = true;
    let entries = Entries::new(vec![Entry::Folder(parent)]);
    let shown = Shown::from([(path_key(&larch), supraman_core::folders::subfolders(&larch))]);
    let sessions = vec![session("g1", &gaia.join("src")), session("l1", &larch)];
    let placed = place_with(&sessions, &entries, &shown, |_| None);
    let ids = |dir: &Path| placed.by_folder.get(&path_key(dir)).map(|v| v.iter().map(|s| s.id.as_str()).collect::<Vec<_>>()).unwrap_or_default();
    assert_eq!(ids(&gaia), ["g1"], "a session in gaia/src shows under the shown gaia");
    assert_eq!(ids(&larch), ["l1"]);
    let hidden = Entries::new(vec![Entry::Folder(FolderEntry::new(&larch))]);
    assert_eq!(place_with(&sessions, &hidden, &shown, |_| None).by_folder[&path_key(&larch)].len(), 2, "a parent not showing them keeps them");
}
