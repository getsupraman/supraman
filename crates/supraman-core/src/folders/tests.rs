use std::path::{Path, PathBuf};

use super::*;
use migrate::write_workspace;

fn top(path: &str) -> Entry {
    Entry::Folder(FolderEntry::new(path))
}

fn group(name: &str, paths: &[&str]) -> Entry {
    Entry::Group { name: name.into(), folders: paths.iter().map(|p| FolderEntry::new(*p)).collect() }
}

fn sample() -> Entries {
    Entries::new(vec![top(r"C:\a"), group("gaia", &[r"C:\g", r"C:\g\api"]), top(r"C:\b")])
}

#[test]
fn the_file_reads_back_what_was_saved() {
    let home = tempfile::tempdir().unwrap();
    let mut e = sample();
    if let Entry::Folder(f) = &mut e.list[0] {
        f.show_subfolders = true;
    }
    let (saved, ()) = edit(home.path(), |entries| {
        *entries = e.clone();
        Ok(())
    })
    .unwrap();
    assert_eq!(saved, e);
    assert_eq!(load(home.path()).unwrap(), e);
    let text = std::fs::read_to_string(path(home.path())).unwrap();
    assert!(text.contains("[[entry]]") && text.contains("group = \"gaia\"") && text.contains("[[entry.folders]]"), "{text}");
    assert!(text.contains("show_subfolders = true") && text.matches("show_subfolders").count() == 1, "{text}");
}

#[test]
fn a_broken_file_is_an_error_and_stays_untouched() {
    let home = tempfile::tempdir().unwrap();
    for bad in ["entry = ", "[[entry]]\npath = 'C:\\a'\ngroup = \"g\"\n", "[[entry]]\nshow_subfolders = true\n"] {
        std::fs::write(path(home.path()), bad).unwrap();
        assert!(matches!(load(home.path()), Err(CoreError::Parse { .. })), "{bad}");
        let refused = edit(home.path(), |entries| entries.add_group("x", 0));
        assert!(matches!(refused, Err(CoreError::Parse { .. })), "{bad}");
        assert_eq!(std::fs::read_to_string(path(home.path())).unwrap(), bad, "never overwritten");
    }
}

#[test]
fn a_refused_edit_writes_nothing() {
    let home = tempfile::tempdir().unwrap();
    edit(home.path(), |entries| entries.add_group("gaia", 0)).unwrap();
    let before = std::fs::read_to_string(path(home.path())).unwrap();
    assert!(matches!(edit(home.path(), |entries| entries.add_group("gaia", 1)), Err(CoreError::AlreadyExists(_))));
    assert_eq!(std::fs::read_to_string(path(home.path())).unwrap(), before);
}

#[test]
fn edits_hold_the_lock_so_two_writers_keep_both_changes() {
    let home = tempfile::tempdir().unwrap();
    let h = home.path().to_path_buf();
    let threads: Vec<_> = (0..8)
        .map(|i| {
            let h = h.clone();
            std::thread::spawn(move || edit(&h, |entries| entries.add_folder(Path::new(&format!(r"C:\f{i}")), &Place::Top(usize::MAX))).unwrap())
        })
        .collect();
    threads.into_iter().for_each(|t| {
        t.join().unwrap();
    });
    assert_eq!(load(&h).unwrap().list.len(), 8);
}

#[test]
fn group_names_follow_the_name_rule_and_are_unique() {
    let mut e = sample();
    assert!(matches!(e.add_group("Bad Name", 0), Err(CoreError::InvalidName(_))));
    assert!(matches!(e.add_group("gaia", 0), Err(CoreError::AlreadyExists(_))));
    e.add_group("tools", 1).unwrap();
    assert_eq!(e.list[1], group("tools", &[]));
    assert!(matches!(e.rename_group("tools", "gaia"), Err(CoreError::AlreadyExists(_))));
    assert!(matches!(e.rename_group("tools", "-x"), Err(CoreError::InvalidName(_))));
    assert!(matches!(e.rename_group("nope", "x"), Err(CoreError::NoGroup(_))));
    e.rename_group("tools", "kit").unwrap();
    assert_eq!(e.list[1].group_name(), Some("kit"));
}

#[test]
fn removing_a_group_drops_its_folders_or_puts_them_at_the_top_level_where_it_was() {
    let mut with = sample();
    with.remove_group("gaia", true).unwrap();
    assert_eq!(with.list, [top(r"C:\a"), top(r"C:\b")]);

    let mut without = sample();
    without.list.push(top(r"C:\g"));
    without.remove_group("gaia", false).unwrap();
    assert_eq!(without.list, [top(r"C:\a"), top(r"C:\g\api"), top(r"C:\b"), top(r"C:\g")], "a folder the top level lists already is not listed twice");
    assert!(matches!(without.remove_group("gaia", false), Err(CoreError::NoGroup(_))));
}

#[test]
fn folders_are_added_removed_and_moved_by_position() {
    let mut e = sample();
    e.add_folder(Path::new(r"C:\c"), &Place::Top(1)).unwrap();
    e.add_folder(Path::new(r"C:\a"), &Place::Group { name: "gaia".into(), index: 0 }).unwrap();
    assert!(matches!(e.add_folder(Path::new(r"c:/A"), &Place::Top(0)), Err(CoreError::FolderListed(_))), "the top level has it");
    assert!(matches!(e.add_folder(Path::new(r"C:\x"), &Place::Group { name: "no".into(), index: 0 }), Err(CoreError::NoGroup(_))));
    assert_eq!(e.list, [top(r"C:\a"), top(r"C:\c"), group("gaia", &[r"C:\a", r"C:\g", r"C:\g\api"]), top(r"C:\b")]);

    assert_eq!(e.remove_folder(Pos::member(2, 0)).unwrap(), FolderEntry::new(r"C:\a"));
    assert!(matches!(e.remove_folder(Pos::top(2)), Err(CoreError::NoEntry(_))), "a group is not a folder");
    assert!(matches!(e.remove_folder(Pos::member(0, 0)), Err(CoreError::NoEntry(_))));
    assert_eq!(e.remove_folder(Pos::top(1)).unwrap(), FolderEntry::new(r"C:\c"));

    e.move_folder(Pos::top(2), &Place::Group { name: "gaia".into(), index: 1 }).unwrap();
    assert_eq!(e.list, [top(r"C:\a"), group("gaia", &[r"C:\g", r"C:\b", r"C:\g\api"])]);
    e.move_folder(Pos::member(1, 2), &Place::Top(0)).unwrap();
    e.move_folder(Pos::member(2, 0), &Place::Group { name: "gaia".into(), index: 9 }).unwrap();
    assert_eq!(e.list, [top(r"C:\g\api"), top(r"C:\a"), group("gaia", &[r"C:\b", r"C:\g"])], "reordered inside the group");
    let before = e.clone();
    e.add_folder(Path::new(r"C:\b"), &Place::Top(9)).unwrap();
    assert!(matches!(e.move_folder(Pos::member(2, 0), &Place::Top(0)), Err(CoreError::FolderListed(_))));
    assert!(matches!(e.move_folder(Pos::top(0), &Place::Group { name: "no".into(), index: 0 }), Err(CoreError::NoGroup(_))));
    assert_eq!(e.list[..3], before.list[..], "a refused move changes nothing");
}


#[test]
fn the_first_load_makes_a_group_of_each_workspace_with_a_duplicate_at_its_first_place_once() {
    let home = tempfile::tempdir().unwrap();
    let larchsys = PathBuf::from(r"P:\Projects\LarchSys");
    let gaia = larchsys.join("gaia");
    let old = larchsys.join("old");
    write_workspace(home.path(), "larchsys", &[&larchsys, &gaia, &old]);
    write_workspace(home.path(), "gaia", &[&gaia, &gaia.join("api")]);
    std::fs::write(home.path().join("workspaces").join("broken.toml"), "name = ").unwrap();

    let entries = load(home.path()).unwrap();
    let gaia_s = gaia.to_str().unwrap();
    let api = gaia.join("api");
    assert_eq!(
        entries.list,
        [group("gaia", &[gaia_s, api.to_str().unwrap()]), group("larchsys", &[larchsys.to_str().unwrap(), larchsys.join("old").to_str().unwrap()])],
        "by name; gaia is placed in its first group only; the home is no folder"
    );
    assert!(path(home.path()).is_file());

    write_workspace(home.path(), "zeta", &[Path::new(r"C:\z")]);

    edit(home.path(), |e| e.remove_group("larchsys", true)).unwrap();
    let again = load(home.path()).unwrap();
    assert_eq!(again.list.len(), 1, "folders.toml is read, never migrated again: {:?}", again.list);
}

#[test]
fn no_workspaces_is_an_empty_list() {
    let home = tempfile::tempdir().unwrap();
    assert_eq!(load(home.path()).unwrap(), Entries::default());
}

#[test]
fn on_disk_reads_git_and_branch_or_plain() {
    let tmp = tempfile::tempdir().unwrap();
    let git = tmp.path().join("g");
    std::fs::create_dir_all(git.join(".git")).unwrap();
    std::fs::write(git.join(".git").join("HEAD"), "ref: refs/heads/main\n").unwrap();
    assert_eq!(on_disk(&git), OnDisk { kind: FolderKind::Git, branch: Some("main".into()) });
    assert_eq!(on_disk(tmp.path()), OnDisk { kind: FolderKind::Plain, branch: None });
}
