use std::path::{Path, PathBuf};

use gpui::TestAppContext;
use supraman_core::folders::{self, Entry, FolderEntry};
use supraman_ui::confirm::Answer;

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

impl gpui::Render for GroupName {
    fn render(&mut self, _: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl gpui::IntoElement {
        use gpui::ParentElement as _;
        gpui::div().child(self.field().clone())
    }
}

type Seen = std::rc::Rc<std::cell::RefCell<Vec<GroupNameEvent>>>;

fn field<'a>(cx: &'a mut TestAppContext, renaming: Option<&str>) -> (gpui::Entity<GroupName>, &'a mut gpui::VisualTestContext, Seen) {
    cx.update(|cx| {
        supraman_ui::init_globals(cx, "test");
        supraman_ui::text_input::bind_keys(cx);
    });
    let renaming = renaming.map(str::to_string);
    let (name, cx) = cx.add_window_view(|window, cx| GroupName::new(renaming, vec!["demo".into(), "other".into()], window, cx));
    let seen: Seen = Default::default();
    let sink = seen.clone();
    cx.update(|_, cx| cx.subscribe(&name, move |_, e: &GroupNameEvent, _| sink.borrow_mut().push(e.clone())).detach());
    cx.run_until_parked();
    (name, cx, seen)
}

/// `typed` over the whole name, then Enter: what the field showed and what it said.
fn named(cx: &mut TestAppContext, renaming: Option<&str>, typed: &str) -> (Option<String>, Vec<GroupNameEvent>) {
    let (name, cx, seen) = field(cx, renaming);
    if !typed.is_empty() {
        cx.simulate_input(typed);
    }
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let why = name.read_with(cx, |n, cx| n.shown_why(cx));
    let said = seen.borrow().clone();
    (why, said)
}

#[gpui::test]
fn rename_opens_with_the_whole_name_selected_and_focused(cx: &mut TestAppContext) {
    let (name, cx, _) = field(cx, Some("demo"));
    let (text, selection, focused) = name.update_in(cx, |n, window, cx| {
        let i = n.field().read(cx);
        (i.text().to_string(), i.line.selection(), i.is_focused(window))
    });
    assert_eq!((text.as_str(), selection, focused), ("demo", 0..4, true));
}

#[gpui::test]
fn enter_renames_to_a_new_name(cx: &mut TestAppContext) {
    assert_eq!(named(cx, Some("demo"), "work"), (None, vec![GroupNameEvent::Named(FolderEdit::RenameGroup { from: "demo".into(), to: "work".into() })]));
}

#[gpui::test]
fn enter_on_the_unchanged_name_closes_without_a_write(cx: &mut TestAppContext) {
    assert_eq!(named(cx, Some("demo"), ""), (None, vec![GroupNameEvent::Closed]));
}

#[gpui::test]
fn esc_cancels_and_names_nothing(cx: &mut TestAppContext) {
    let (_, cx, seen) = field(cx, Some("demo"));
    cx.simulate_input("work");
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert_eq!(*seen.borrow(), [GroupNameEvent::Closed]);
}

#[gpui::test]
fn a_name_another_group_has_is_refused_in_place_with_its_why(cx: &mut TestAppContext) {
    let why = "A group named other exists already";
    assert_eq!(named(cx, Some("demo"), "other"), (Some(why.into()), vec![GroupNameEvent::Refused(why.into())]));
}

#[gpui::test]
fn a_refused_name_marks_the_field_and_an_edit_to_a_free_one_clears_it(cx: &mut TestAppContext) {
    let (name, cx, _) = field(cx, None);
    cx.simulate_input("other");
    cx.run_until_parked();
    assert!(name.read_with(cx, |n, cx| n.field().read(cx).invalid()), "a taken name shows as typed");
    cx.simulate_input("s");
    cx.run_until_parked();
    assert!(!name.read_with(cx, |n, cx| n.field().read(cx).invalid()));
}

#[gpui::test]
fn a_name_the_rule_does_not_allow_is_refused(cx: &mut TestAppContext) {
    let (why, said) = named(cx, None, "-x");
    assert!(why.as_deref().is_some_and(|w| w.contains("invalid name")), "{why:?}");
    assert!(matches!(said.as_slice(), [GroupNameEvent::Refused(_)]), "{said:?}");
}

#[gpui::test]
fn an_emptied_rename_is_refused_as_needing_a_name(cx: &mut TestAppContext) {
    let (_, cx, seen) = field(cx, Some("demo"));
    cx.simulate_keystrokes("backspace enter");
    cx.run_until_parked();
    assert_eq!(*seen.borrow(), [GroupNameEvent::Refused(super::group_name::NEEDS_NAME.into())]);
}

#[gpui::test]
fn add_groups_field_takes_a_names_letters_and_enter_adds_it(cx: &mut TestAppContext) {
    assert_eq!(named(cx, None, "Big Ideas"), (None, vec![GroupNameEvent::Named(FolderEdit::AddGroup("bigideas".into()))]), "typed in the name's letters");
}

#[gpui::test]
fn enter_on_an_empty_new_row_closes_it(cx: &mut TestAppContext) {
    assert_eq!(named(cx, None, ""), (None, vec![GroupNameEvent::Closed]));
}

fn lose_focus(cx: &mut gpui::VisualTestContext) {
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    cx.update(|window, cx| window.blur(cx));
    cx.run_until_parked();
}

#[gpui::test]
fn focus_lost_names_a_valid_name(cx: &mut TestAppContext) {
    let (_, cx, seen) = field(cx, Some("demo"));
    cx.simulate_input("work");
    lose_focus(cx);
    assert_eq!(*seen.borrow(), [GroupNameEvent::Named(FolderEdit::RenameGroup { from: "demo".into(), to: "work".into() })]);
}

#[gpui::test]
fn focus_lost_on_a_refused_name_cancels(cx: &mut TestAppContext) {
    let (_, cx, seen) = field(cx, Some("demo"));
    cx.simulate_input("other");
    lose_focus(cx);
    assert_eq!(*seen.borrow(), [GroupNameEvent::Closed]);
}

fn top(path: &str) -> Entry {
    Entry::Folder(FolderEntry::new(path))
}

fn paths(list: &[&str]) -> Vec<PathBuf> {
    list.iter().map(PathBuf::from).collect()
}

/// a, [g: x, y], b, [h: ]
fn dropping() -> Entries {
    let g = Entry::Group { name: "g".into(), folders: vec![FolderEntry::new(r"C:\x"), FolderEntry::new(r"C:\y")] };
    Entries::new(vec![top(r"C:\a"), g, top(r"C:\b"), Entry::Group { name: "h".into(), folders: vec![] }])
}

/// The top level's folders and each group's, by path, after `from` lands on `target`.
fn after(from: Pos, target: Target) -> (Vec<PathBuf>, Vec<(String, Vec<PathBuf>)>) {
    let mut e = dropping();
    let to = landing(&e, from, &target).unwrap_or_else(|| panic!("{from:?} on {target:?} changes nothing"));
    e.move_folder(from, &to).unwrap();
    let top = e.list.iter().filter_map(|x| match x {
        Entry::Folder(f) => Some(f.path.clone()),
        Entry::Group { .. } => None,
    });
    (top.collect(), e.groups().map(|(n, f)| (n.to_string(), f.iter().map(|f| f.path.clone()).collect())).collect())
}

#[test]
fn a_folder_dropped_on_a_group_row_goes_to_its_end_on_a_folder_row_before_it_below_the_tree_to_the_top_levels_end() {
    let (a, x, y) = (Pos::top(0), Pos::member(1, 0), Pos::member(1, 1));
    assert_eq!(after(a, Target::Group("h".into())), (paths(&[r"C:\b"]), vec![("g".into(), paths(&[r"C:\x", r"C:\y"])), ("h".into(), paths(&[r"C:\a"]))]), "onto a group row: into it");
    assert_eq!(after(a, Target::Group("g".into())).1[0], ("g".into(), paths(&[r"C:\x", r"C:\y", r"C:\a"])), "at its end");
    assert_eq!(after(x, Target::Folder(Pos::top(2))), (paths(&[r"C:\a", r"C:\x", r"C:\b"]), vec![("g".into(), paths(&[r"C:\y"])), ("h".into(), vec![])]), "onto a top-level row: out of the group, before it");
    assert_eq!(after(a, Target::Folder(Pos::member(1, 1))).1[0], ("g".into(), paths(&[r"C:\x", r"C:\a", r"C:\y"])), "onto a row in a group: before it in that group");
    assert_eq!(after(y, Target::Folder(x)).1[0], ("g".into(), paths(&[r"C:\y", r"C:\x"])), "reordered in its group");
    assert_eq!(after(a, Target::Folder(Pos::top(2))).0, paths(&[r"C:\a", r"C:\b"]), "before the next one is where it is");
    assert_eq!(after(a, Target::End).0, paths(&[r"C:\b", r"C:\a"]), "below the tree: the top level's end");
    assert_eq!(after(y, Target::End).0, paths(&[r"C:\a", r"C:\b", r"C:\y"]));
}

#[test]
fn a_drop_that_changes_nothing_has_no_landing_so_nothing_is_written() {
    let e = dropping();
    let (a, x, y) = (Pos::top(0), Pos::member(1, 0), Pos::member(1, 1));
    assert_eq!(landing(&e, a, &Target::Folder(a)), None, "onto itself");
    assert_eq!(landing(&e, x, &Target::Folder(y)), None, "before the one after it");
    assert_eq!(landing(&e, y, &Target::Group("g".into())), None, "the last of its group onto its group");
    assert_eq!(landing(&e, Pos::top(2), &Target::Folder(Pos::top(3))), None, "a group row's place is no folder");
    let end = Entries::new(vec![top(r"C:\a"), top(r"C:\b")]);
    assert_eq!(landing(&end, Pos::top(1), &Target::End), None, "the top level's last below the tree");
    assert_eq!(landing(&e, Pos::top(9), &Target::End), None, "a position no longer there");
}

#[test]
fn a_move_or_show_subfolders_of_a_folder_no_longer_at_its_position_is_refused() {
    let home = tempfile::tempdir().unwrap();
    let (a, b) = (PathBuf::from(r"C:\demo\alpha"), PathBuf::from(r"C:\demo\beta"));
    run(home.path(), &FolderEdit::AddGroup("demo".into())).unwrap();
    add(home.path(), &a, None).unwrap();
    let moved = FolderEdit::MoveFolder { from: Pos::top(1), path: a.clone(), to: Place::Group { name: "demo".into(), index: 0 } };
    assert_eq!(run(home.path(), &moved).unwrap(), "Moved alpha into demo.");
    assert_eq!(listed(home.path()), [group("demo", &[&a])]);
    assert!(run(home.path(), &moved).unwrap_err().starts_with("Cannot move alpha"), "alpha is no longer at the top level");
    add(home.path(), &b, None).unwrap();
    let stale = FolderEdit::MoveFolder { from: Pos::top(1), path: a.clone(), to: Place::Top(0) };
    assert!(run(home.path(), &stale).unwrap_err().starts_with("Cannot move alpha"), "beta took its place");
    assert_eq!(run(home.path(), &FolderEdit::MoveFolder { from: Pos::member(0, 0), path: a.clone(), to: Place::Top(usize::MAX) }).unwrap(), "Moved alpha to the top level.");

    let show = FolderEdit::ShowSubfolders { pos: Pos::top(2), path: a.clone(), show: true };
    assert_eq!(run(home.path(), &show).unwrap(), "Showing the sub folders of alpha.");
    assert!(folders::load(home.path()).unwrap().folder_at(Pos::top(2)).unwrap().show_subfolders);
    assert_eq!(run(home.path(), &FolderEdit::ShowSubfolders { pos: Pos::top(2), path: a.clone(), show: false }).unwrap(), "Hid the sub folders of alpha.");
    assert!(!folders::load(home.path()).unwrap().folder_at(Pos::top(2)).unwrap().show_subfolders);
    assert!(run(home.path(), &FolderEdit::ShowSubfolders { pos: Pos::top(1), path: a, show: true }).unwrap_err().starts_with("Cannot show the sub folders of alpha"));
}

#[test]
fn exclude_and_include_write_the_childs_path_while_its_parent_still_holds_it() {
    let home = tempfile::tempdir().unwrap();
    let (a, b) = (PathBuf::from(r"C:\demolpha"), PathBuf::from(r"C:\demoeta"));
    add(home.path(), &a, None).unwrap();
    let web = a.join("web");
    assert_eq!(run(home.path(), &FolderEdit::ExcludeFolder { parent: Pos::top(0), path: web.clone() }).unwrap(), "Excluded web.");
    assert_eq!(folders::load(home.path()).unwrap().folder_at(Pos::top(0)).unwrap().excluded, [web.clone()]);
    let stale = FolderEdit::IncludeFolder { parent: Pos::top(1), path: web.clone() };
    assert!(run(home.path(), &stale).unwrap_err().starts_with("Cannot include web"), "no folder there");
    run(home.path(), &FolderEdit::RemoveFolder { pos: Pos::top(0), path: a.clone() }).unwrap();
    add(home.path(), &b, None).unwrap();
    assert!(run(home.path(), &FolderEdit::IncludeFolder { parent: Pos::top(0), path: web.clone() }).unwrap_err().starts_with("Cannot include web"), "beta took alpha's place");
    add(home.path(), &a, None).unwrap();
    run(home.path(), &FolderEdit::ExcludeFolder { parent: Pos::top(1), path: web.clone() }).unwrap();
    assert_eq!(run(home.path(), &FolderEdit::IncludeFolder { parent: Pos::top(1), path: web }).unwrap(), "Included web.");
    assert!(!std::fs::read_to_string(folders::path(home.path())).unwrap().contains("excluded"));
}

/// The top-level entries' names (a group's, a folder's path) after the group `name` lands on `target`.
fn group_after(name: &str, target: Target) -> Vec<String> {
    let mut e = dropping();
    let to = group_landing(&e, name, &target).unwrap_or_else(|| panic!("{name} on {target:?} changes nothing"));
    e.move_group(name, to).unwrap();
    e.list.iter().map(|x| x.group_name().map_or_else(|| x.folders()[0].path.display().to_string(), str::to_string)).collect()
}

#[test]
fn a_group_dropped_on_a_top_level_row_goes_before_that_entry_and_below_the_tree_to_the_end() {
    assert_eq!(group_after("h", Target::Group("g".into())), [r"C:\a", "h", "g", r"C:\b"], "onto a group row: before it");
    assert_eq!(group_after("h", Target::Folder(Pos::top(0))), ["h", r"C:\a", "g", r"C:\b"], "onto the first row: first");
    assert_eq!(group_after("g", Target::Folder(Pos::top(0))), ["g", r"C:\a", r"C:\b", "h"], "onto an earlier top-level folder row: before it");
    assert_eq!(group_after("g", Target::Group("h".into())), [r"C:\a", r"C:\b", "g", "h"], "onto a later group row: before it");
    assert_eq!(group_after("g", Target::End), [r"C:\a", r"C:\b", "h", "g"], "below the tree: the end");
}

#[test]
fn a_group_dropped_where_it_is_or_inside_a_group_has_no_landing() {
    let e = dropping();
    assert_eq!(group_landing(&e, "g", &Target::Group("g".into())), None, "onto its own row");
    assert_eq!(group_landing(&e, "g", &Target::Folder(Pos::top(2))), None, "before the entry after it");
    assert_eq!(group_landing(&e, "h", &Target::End), None, "the last below the tree");
    assert_eq!(group_landing(&e, "h", &Target::Folder(Pos::member(1, 0))), None, "a row inside a group is no target");
    assert_eq!(group_landing(&e, "h", &Target::Folder(Pos::top(1))), None, "a group's place is no folder row");
    assert_eq!(group_landing(&e, "gone", &Target::End), None, "a group no longer there");
}

#[test]
fn move_group_writes_the_move_and_refuses_a_group_gone_since() {
    let home = tempfile::tempdir().unwrap();
    let a = PathBuf::from(r"C:\demo\alpha");
    add(home.path(), &a, None).unwrap();
    run(home.path(), &FolderEdit::AddGroup("demo".into())).unwrap();
    assert_eq!(run(home.path(), &FolderEdit::MoveGroup { name: "demo".into(), to: 0 }).unwrap(), "Moved group demo.");
    assert_eq!(listed(home.path()), [group("demo", &[]), Entry::Folder(FolderEntry::new(&a))]);
    assert!(run(home.path(), &FolderEdit::MoveGroup { name: "gone".into(), to: 0 }).unwrap_err().starts_with("Cannot move group gone"));
}
