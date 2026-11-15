use std::path::{Path, PathBuf};

use supraman_core::folders::Pos;
use supraman_core::schedule::folder_key;

use super::*;
use supraman_core::FolderKind;

use crate::fleet::{EntryNode, FolderNode};

fn row(key: &str, path: &Path, pos: Option<Pos>, parent: Option<&str>) -> FolderNode {
    FolderNode { pos, parent: parent.map(str::to_string), ..FolderNode::bare(key.into(), folder_name(path), path.to_path_buf(), FolderKind::Plain, None) }
}

fn entry(name: &str, group: bool, folders: Vec<FolderNode>) -> EntryNode {
    EntryNode { name: name.into(), group, folders }
}

fn tree(entries: Vec<EntryNode>) -> Tree {
    Tree { entries, ..Tree::default() }
}

struct Paths {
    infra: PathBuf,
    child: PathBuf,
    other: PathBuf,
}

fn paths() -> Paths {
    Paths { infra: PathBuf::from(r"C:\demo\infra"), child: PathBuf::from(r"C:\demo\infra\api"), other: PathBuf::from(r"C:\demo\other") }
}

/// `(folder key, id)` of each `(path, id)`.
fn scheduled(at: &[(&Path, &str)]) -> Vec<(String, String)> {
    at.iter().map(|(p, id)| (folder_key(p), id.to_string())).collect()
}

fn lost(edit: &FolderEdit, tree: &Tree, schedules: &[(String, String)]) -> Vec<(String, String)> {
    orphans(edit, tree, schedules.iter().map(|(k, id)| (k.as_str(), id.as_str())))
}

fn remove(pos: Pos, path: &Path) -> FolderEdit {
    FolderEdit::RemoveFolder { pos, path: path.to_path_buf() }
}

#[test]
fn a_removed_folder_leaves_its_own_and_its_shown_childrens_schedules_without_a_row() {
    let p = paths();
    let t = tree(vec![
        entry("infra", false, vec![row("infra", &p.infra, Some(Pos::top(0)), None), row("infra/api", &p.child, None, Some("infra"))]),
        entry("other", false, vec![row("other", &p.other, Some(Pos::top(1)), None)]),
    ]);
    let schedules = scheduled(&[(&p.infra, "gate"), (&p.child, "nightly"), (&p.other, "kept")]);
    assert_eq!(lost(&remove(Pos::top(0), &p.infra), &t, &schedules), scheduled(&[(&p.infra, "gate"), (&p.child, "nightly")]));
    assert_eq!(lost(&remove(Pos::top(1), &p.other), &t, &schedules), scheduled(&[(&p.other, "kept")]));
    assert!(lost(&remove(Pos::top(1), &p.infra), &t, &schedules).is_empty(), "a position no longer holding the path takes nothing");
}

#[test]
fn a_path_with_a_row_elsewhere_keeps_its_schedules() {
    let p = paths();
    let t = tree(vec![
        entry("infra", false, vec![row("infra", &p.infra, Some(Pos::top(0)), None), row("infra/api", &p.child, None, Some("infra"))]),
        entry("demo", true, vec![row("demo/infra", &p.infra, Some(Pos::member(1, 0)), None), row("demo/api", &p.child, Some(Pos::member(1, 1)), None)]),
    ]);
    let schedules = scheduled(&[(&p.infra, "gate"), (&p.child, "nightly")]);
    assert!(lost(&remove(Pos::top(0), &p.infra), &t, &schedules).is_empty(), "listed twice, and the child listed in the group");
}

#[test]
fn an_excluded_child_has_no_row_to_lose_or_keep() {
    let p = paths();
    let mut excluded = row("infra/api", &p.child, None, Some("infra"));
    excluded.excluded = true;
    let t = tree(vec![
        entry("infra", false, vec![row("infra", &p.infra, Some(Pos::top(0)), None), excluded.clone()]),
        entry("other", false, vec![row("other", &p.other, Some(Pos::top(1)), None), FolderNode { parent: Some("other".into()), ..excluded }]),
    ]);
    let schedules = scheduled(&[(&p.child, "nightly")]);
    assert!(lost(&remove(Pos::top(0), &p.infra), &t, &schedules).is_empty(), "already under Other schedules");
}

#[test]
fn remove_group_moving_its_folders_loses_nothing_and_removing_them_loses_theirs() {
    let p = paths();
    let t = tree(vec![
        entry("demo", true, vec![row("demo/infra", &p.infra, Some(Pos::member(0, 0)), None), row("demo/infra/api", &p.child, None, Some("demo/infra"))]),
        entry("other", false, vec![row("other", &p.other, Some(Pos::top(1)), None)]),
    ]);
    let schedules = scheduled(&[(&p.infra, "gate"), (&p.child, "nightly"), (&p.other, "kept")]);
    let group = |with_folders| FolderEdit::RemoveGroup { name: "demo".into(), with_folders };
    assert!(lost(&group(false), &t, &schedules).is_empty());
    assert_eq!(lost(&group(true), &t, &schedules), scheduled(&[(&p.infra, "gate"), (&p.child, "nightly")]));
    assert!(lost(&FolderEdit::RemoveGroup { name: "other".into(), with_folders: true }, &t, &schedules).is_empty(), "a top-level folder is no group");
}

#[test]
fn the_toast_says_what_went_and_where_the_kept_ones_are() {
    let p = paths();
    let two = |delete| Leaving { schedules: vec![(p.infra.clone(), "gate".into()), (p.child.clone(), "nightly".into())], delete };
    let edit = remove(Pos::top(0), &p.infra);
    assert_eq!(two(true).words(&edit).unwrap(), "Removed infra and 2 scheduled tasks.");
    assert_eq!(two(false).words(&edit).unwrap(), "Removed infra; its 2 scheduled tasks are under Other schedules.");
    let one = Leaving { schedules: vec![(p.infra.clone(), "gate".into())], delete: false };
    assert_eq!(one.words(&edit).unwrap(), "Removed infra; its 1 scheduled task is under Other schedules.");
    let group = FolderEdit::RemoveGroup { name: "demo".into(), with_folders: true };
    assert_eq!(two(true).words(&group).unwrap(), "Removed group demo with its folders and 2 scheduled tasks.");
    assert_eq!(Leaving::default().words(&edit), None);
}

#[gpui::test]
fn only_the_answer_the_tick_box_belongs_to_deletes(cx: &mut gpui::TestAppContext) {
    let p = paths();
    let schedules = vec![(p.infra.clone(), "gate".to_string())];
    let ask = |delete| {
        let focus = cx.update(|cx| cx.focus_handle());
        RemoveQuestion { delete, ..RemoveQuestion::group("demo", 1, schedules.clone(), focus) }
    };
    assert_eq!(ask(true).take(0).unwrap().1, Leaving::default(), "Move its folders to the top level");
    assert_eq!(ask(true).take(1).unwrap().1, Leaving { schedules: schedules.clone(), delete: true });
    assert_eq!(ask(false).take(1).unwrap().1, Leaving { schedules: schedules.clone(), delete: false });
}
