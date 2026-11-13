use std::path::{Path, PathBuf};

use gpui::TestAppContext;
use supraman_core::folders::{self, Entry, FolderEntry};
use supraman_ui::confirm::Answer;
use supraman_ui::hit::HitTargets;

use super::*;

fn run(home: &Path, edit: &FolderEdit) -> Result<String, String> {
    write(home, edit).map(|(said, _)| said)
}

fn add(home: &Path, path: &Path, group: Option<&str>) -> Result<String, String> {
    run(home, &FolderEdit::AddFolder { path: path.to_path_buf(), group: group.map(str::to_string) })
}

fn listed(home: &Path) -> Vec<Entry> {
    folders::load(home).unwrap().list
}

fn group(name: &str, paths: &[&Path]) -> Entry {
    Entry::Group { name: name.into(), folders: paths.iter().map(|p| FolderEntry::new(*p)).collect() }
}

#[test]
fn add_folder_writes_the_top_level_or_a_group_and_refuses_a_second_at_the_same_place() {
    let home = tempfile::tempdir().unwrap();
    let (a, b) = (PathBuf::from(r"C:\demo\alpha"), PathBuf::from(r"C:\demo\beta"));
    run(home.path(), &FolderEdit::AddGroup("demo".into())).unwrap();
    assert_eq!(add(home.path(), &a, None).unwrap(), "Added alpha.");
    assert_eq!(add(home.path(), &b, Some("demo")).unwrap(), "Added beta to demo.");
    assert_eq!(add(home.path(), &a, Some("demo")).unwrap(), "Added alpha to demo.", "a path listed elsewhere is allowed");
    assert_eq!(listed(home.path()), [group("demo", &[&b, &a]), Entry::Folder(FolderEntry::new(&a))]);

    let again = add(home.path(), &a, None).unwrap_err();
    assert!(again.starts_with("Cannot add alpha:") && again.contains("listed there already"), "{again}");
    let in_group = add(home.path(), &b, Some("demo")).unwrap_err();
    assert!(in_group.contains("listed there already"), "{in_group}");
    assert_eq!(listed(home.path()), [group("demo", &[&b, &a]), Entry::Folder(FolderEntry::new(&a))], "nothing written");
}

#[test]
fn a_group_is_added_and_renamed_by_the_name_rule_and_never_twice() {
    let home = tempfile::tempdir().unwrap();
    assert_eq!(run(home.path(), &FolderEdit::AddGroup("demo".into())).unwrap(), "Added group demo.");
    for bad in ["Demo", "a b", ""] {
        assert!(run(home.path(), &FolderEdit::AddGroup(bad.into())).unwrap_err().starts_with(&format!("Cannot add group {bad}:")));
    }
    assert!(run(home.path(), &FolderEdit::AddGroup("demo".into())).unwrap_err().contains("already exists"));
    run(home.path(), &FolderEdit::AddGroup("other".into())).unwrap();
    assert!(run(home.path(), &FolderEdit::RenameGroup { from: "demo".into(), to: "other".into() }).unwrap_err().contains("already exists"));
    assert_eq!(run(home.path(), &FolderEdit::RenameGroup { from: "demo".into(), to: "work".into() }).unwrap(), "Renamed group demo to work.");
    assert_eq!(listed(home.path()), [group("work", &[]), group("other", &[])]);
}

#[test]
fn remove_folder_takes_that_one_entry_while_it_is_still_there() {
    let home = tempfile::tempdir().unwrap();
    let (a, b) = (PathBuf::from(r"C:\demo\alpha"), PathBuf::from(r"C:\demo\beta"));
    run(home.path(), &FolderEdit::AddGroup("demo".into())).unwrap();
    add(home.path(), &a, Some("demo")).unwrap();
    add(home.path(), &b, Some("demo")).unwrap();
    add(home.path(), &a, None).unwrap();
    let stale = run(home.path(), &FolderEdit::RemoveFolder { pos: Pos::member(0, 0), path: b.clone() }).unwrap_err();
    assert!(stale.starts_with("Cannot remove beta"), "a position holding another folder is refused: {stale}");
    let said = run(home.path(), &FolderEdit::RemoveFolder { pos: Pos::member(0, 0), path: a.clone() }).unwrap();
    assert_eq!(said, "Removed alpha from the list; its files and sessions stay.");
    assert_eq!(listed(home.path()), [group("demo", &[&b]), Entry::Folder(FolderEntry::new(&a))], "the top level's alpha stays");
    run(home.path(), &FolderEdit::RemoveFolder { pos: Pos::top(1), path: a }).unwrap();
    assert_eq!(listed(home.path()), [group("demo", &[&b])]);
}

#[test]
fn remove_group_moves_its_folders_up_by_default_removes_them_on_the_second_answer_and_cancel_writes_nothing() {
    let (a, b) = (PathBuf::from(r"C:\demo\alpha"), PathBuf::from(r"C:\demo\beta"));
    let setup = || {
        let home = tempfile::tempdir().unwrap();
        add(home.path(), &b, None).unwrap();
        run(home.path(), &FolderEdit::AddGroup("demo".into())).unwrap();
        add(home.path(), &a, Some("demo")).unwrap();
        home
    };
    let question = remove_group_question("demo", 1);
    assert_eq!(question.yes, "Move its folders to the top level");
    assert_eq!(question.default_answer(), Answer::Yes, "Enter moves them");
    assert_eq!(question.alt.as_ref().map(|(w, _)| w.as_str()), Some("Remove its folders too"));
    assert!(!question.danger && !question.alt_danger, "nothing is lost: the files and sessions stay");
    let run = |answer| {
        let home = setup();
        if let Some(Confirmed::EditFolders(edit)) = question.clone().take(answer) {
            run(home.path(), &edit).unwrap();
        }
        listed(home.path())
    };
    assert_eq!(run(Answer::Yes), [Entry::Folder(FolderEntry::new(&b)), Entry::Folder(FolderEntry::new(&a))]);
    assert_eq!(run(Answer::Alt), [Entry::Folder(FolderEntry::new(&b))]);
    assert_eq!(run(Answer::No), [Entry::Folder(FolderEntry::new(&b)), group("demo", &[&a])]);
    let empty = remove_group_question("demo", 0);
    assert!(empty.alt.is_none() && empty.text.contains("empty group"));
}

fn named(cx: &mut TestAppContext, renaming: Option<&str>, typed: &str) -> (Option<String>, Vec<FolderEdit>, bool) {
    cx.update(|cx| {
        supraman_ui::init_globals(cx, "test");
        supraman_ui::text_input::bind_keys(cx);
    });
    let hits = HitTargets::default();
    let renaming = renaming.map(str::to_string);
    let (form, cx) = cx.add_window_view(|window, cx| GroupName::new(renaming, vec!["demo".into(), "other".into()], hits.clone(), window, cx));
    let seen = std::rc::Rc::new(std::cell::RefCell::new((Vec::new(), false)));
    let sink = seen.clone();
    cx.update(|_, cx| {
        cx.subscribe(&form, move |_, e: &GroupNameEvent, _| match e {
            GroupNameEvent::Named(edit) => sink.borrow_mut().0.push(edit.clone()),
            GroupNameEvent::Closed => sink.borrow_mut().1 = true,
        })
        .detach()
    });
    form.update_in(cx, |f, window, cx| f.field().update(cx, |i, cx| i.focus_all(window, cx)));
    cx.simulate_input(typed);
    cx.run_until_parked();
    let why = form.read_with(cx, |f, cx| f.why_not(cx));
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(hits.borrow().contains_key("group-name-ok"), "the form paints its OK");
    let (edits, closed) = seen.borrow().clone();
    (why, edits, closed)
}

#[gpui::test]
fn add_groups_field_takes_a_names_letters_and_enter_adds_only_a_new_name(cx: &mut TestAppContext) {
    assert_eq!(named(cx, None, "Big Ideas"), (None, vec![FolderEdit::AddGroup("bigideas".into())], false), "typed in the name's letters");
}

#[gpui::test]
fn add_group_refuses_a_name_another_group_has(cx: &mut TestAppContext) {
    let (why, edits, _) = named(cx, None, "other");
    assert_eq!((why.as_deref(), edits), (Some("A group named other exists already"), vec![]));
}

#[gpui::test]
fn add_group_refuses_a_name_the_rule_does_not_allow(cx: &mut TestAppContext) {
    let (why, edits, _) = named(cx, None, "-x");
    assert!(why.is_some_and(|w| w.contains("invalid name")) && edits.is_empty());
}

#[gpui::test]
fn rename_keeps_its_own_name_and_renames_to_a_new_one(cx: &mut TestAppContext) {
    assert_eq!(named(cx, Some("demo"), "work"), (None, vec![FolderEdit::RenameGroup { from: "demo".into(), to: "work".into() }], false));
}

#[gpui::test]
fn rename_to_the_same_name_closes_without_a_write(cx: &mut TestAppContext) {
    assert_eq!(named(cx, Some("demo"), "demo"), (None, vec![], true));
}
