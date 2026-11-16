use std::path::{Path, PathBuf};

use super::*;

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
fn move_group_counts_its_index_with_the_group_taken_out_and_refuses_an_unknown_name() {
    let mut e = sample();
    e.move_group("gaia", 0).unwrap();
    assert_eq!(e.list, [group("gaia", &[r"C:\g", r"C:\g\api"]), top(r"C:\a"), top(r"C:\b")], "to the front");
    e.move_group("gaia", 2).unwrap();
    assert_eq!(e.list, [top(r"C:\a"), top(r"C:\b"), group("gaia", &[r"C:\g", r"C:\g\api"])], "to the end");
    let before = e.clone();
    e.move_group("gaia", 2).unwrap();
    assert_eq!(e, before, "where it is already");
    e.move_group("gaia", 9).unwrap();
    assert_eq!(e, before, "past the end appends");
    assert!(matches!(e.move_group("no", 0), Err(CoreError::NoGroup(_))));
    assert_eq!(e, before, "a refused move changes nothing");
}

#[test]
fn no_workspaces_is_an_empty_list() {
    let home = tempfile::tempdir().unwrap();
    assert_eq!(load(home.path()).unwrap(), Entries::default());
}

#[test]
fn a_fresh_home_is_an_empty_tree_and_a_workspaces_folder_in_it_is_never_read() {
    let home = tempfile::tempdir().unwrap();
    let workspaces = home.path().join("workspaces");
    std::fs::create_dir_all(&workspaces).unwrap();
    std::fs::write(workspaces.join("gaia.toml"), "name = \"gaia\"\n\n[[folders]]\nname = \"gaia\"\npath = 'P:/gaia'\n").unwrap();

    assert_eq!(load(home.path()).unwrap(), Entries::default());
    assert_eq!(read(home.path()).unwrap(), Some(Entries::default()), "the empty tree is written");
    let (entries, ()) = edit(home.path(), |_| Ok(())).unwrap();
    assert_eq!(entries, Entries::default());
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

#[test]
fn subfolders_are_the_direct_child_folders_by_name_without_dot_hidden_or_skipped_ones() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for d in ["gaia", "Alpha", ".git", ".claude", "node_modules", "Target", "obj", "src/deep"] {
        std::fs::create_dir_all(root.join(d)).unwrap();
    }
    std::fs::write(root.join("README.md"), "x").unwrap();
    let names: Vec<String> = subfolders(root).iter().map(|p| crate::paths::folder_name(p)).collect();
    assert_eq!(names, ["Alpha", "gaia", "src"], "sorted by name ignoring case, one level, folders only");
    assert!(subfolders(&root.join("missing")).is_empty());
}

#[cfg(windows)]
#[test]
fn a_folder_with_the_hidden_attribute_is_no_subfolder() {
    let dir = tempfile::tempdir().unwrap();
    let hidden = dir.path().join("secret");
    std::fs::create_dir(&hidden).unwrap();
    let ran = crate::process::run(crate::process::command("attrib").arg("+h").arg(&hidden), std::time::Duration::from_secs(30), crate::process::Kill::Tree).unwrap();
    assert_eq!(ran.code, Some(0));
    assert!(subfolders(dir.path()).is_empty());
}

#[test]
fn show_subfolders_is_set_by_position_and_shown_children_join_the_placed_folders() {
    let mut e = sample();
    e.set_show_subfolders(Pos::member(1, 0), true).unwrap();
    assert!(e.folder_at(Pos::member(1, 0)).unwrap().show_subfolders);
    assert!(e.set_show_subfolders(Pos::top(1), true).is_err(), "a group is no folder");
    assert_eq!(e.showing(), [PathBuf::from(r"C:\g")]);
    let shown = Shown::from([(crate::paths::path_key(Path::new(r"C:\g")), vec![PathBuf::from(r"C:\g\web")]), (crate::paths::path_key(Path::new(r"C:\a")), vec![PathBuf::from(r"C:\a\x")])]);
    let placed: Vec<&Path> = e.placed(&shown).collect();
    assert_eq!(placed, [Path::new(r"C:\a"), Path::new(r"C:\g"), Path::new(r"C:\g\api"), Path::new(r"C:\b"), Path::new(r"C:\g\web")], "a folder not showing its own adds none");
    e.move_folder(Pos::member(1, 0), &Place::Top(0)).unwrap();
    assert!(e.folder_at(Pos::top(0)).unwrap().show_subfolders, "a move keeps the flag");
}

#[test]
fn a_shown_child_is_excluded_and_included_by_its_parents_position_once_and_only_a_direct_child() {
    let mut e = sample();
    let web = PathBuf::from(r"C:\g\web");
    e.set_excluded(Pos::member(1, 0), &web, true).unwrap();
    e.set_excluded(Pos::member(1, 0), Path::new(r"c:/G/WEB"), true).unwrap();
    assert_eq!(e.folder_at(Pos::member(1, 0)).unwrap().excluded, [web.clone()], "idempotent, by path_key");
    assert!(e.folder_at(Pos::member(1, 0)).unwrap().excludes(Path::new(r"c:\g\Web")));
    for not_child in [r"C:\g\web\deep", r"C:\a\web", r"C:\g"] {
        assert!(matches!(e.set_excluded(Pos::member(1, 0), Path::new(not_child), true), Err(CoreError::NotAChild { .. })), "{not_child}");
    }
    assert!(matches!(e.set_excluded(Pos::top(1), &web, true), Err(CoreError::NoEntry(_))), "a group is no folder");
    e.set_excluded(Pos::member(1, 0), &web, false).unwrap();
    e.set_excluded(Pos::member(1, 0), &web, false).unwrap();
    assert!(e.folder_at(Pos::member(1, 0)).unwrap().excluded.is_empty());
}

#[test]
fn excluded_paths_are_saved_only_when_there_are_some_and_kept_while_sub_folders_are_hidden() {
    let home = tempfile::tempdir().unwrap();
    let web = PathBuf::from(r"C:\g\web");
    edit(home.path(), |entries| {
        *entries = sample();
        entries.set_show_subfolders(Pos::member(1, 0), true)?;
        entries.set_excluded(Pos::member(1, 0), &web, true)?;
        entries.set_show_subfolders(Pos::member(1, 0), false)
    })
    .unwrap();
    let text = std::fs::read_to_string(path(home.path())).unwrap();
    assert_eq!(text.matches("excluded").count(), 1, "{text}");
    assert_eq!(load(home.path()).unwrap().folder_at(Pos::member(1, 0)).unwrap().excluded, [web.clone()], "hiding sub folders keeps it");
    edit(home.path(), |entries| entries.set_excluded(Pos::member(1, 0), &web, false)).unwrap();
    let text = std::fs::read_to_string(path(home.path())).unwrap();
    assert!(!text.contains("excluded"), "{text}");
    edit(home.path(), |entries| {
        entries.set_show_subfolders(Pos::top(0), true)?;
        entries.set_excluded(Pos::top(0), Path::new(r"C:\a\x"), true)
    })
    .unwrap();
    assert_eq!(load(home.path()).unwrap().folder_at(Pos::top(0)).unwrap().excluded, [PathBuf::from(r"C:\a\x")], "a top-level folder's too");
}

#[test]
fn placed_leaves_an_excluded_child_out() {
    let mut e = sample();
    e.set_show_subfolders(Pos::member(1, 0), true).unwrap();
    let (web, docs) = (PathBuf::from(r"C:\g\web"), PathBuf::from(r"C:\g\docs"));
    let shown = Shown::from([(crate::paths::path_key(Path::new(r"C:\g")), vec![docs.clone(), web.clone()])]);
    e.set_excluded(Pos::member(1, 0), &web, true).unwrap();
    let placed: Vec<&Path> = e.placed(&shown).collect();
    assert_eq!(placed, [Path::new(r"C:\a"), Path::new(r"C:\g"), Path::new(r"C:\g\api"), Path::new(r"C:\b"), docs.as_path()]);
    let g = e.folder_at(Pos::member(1, 0)).unwrap();
    assert_eq!(g.read_children(&shown), [docs, web], "the read children keep it for its row");
}

fn excluding(home: &Path, parent: &Path, show: bool, children: &[&str]) -> Vec<PathBuf> {
    let excluded: Vec<PathBuf> = children.iter().map(|c| parent.join(c)).collect();
    let folder = FolderEntry { path: parent.to_path_buf(), show_subfolders: show, excluded: excluded.clone() };
    edit(home, |entries| {
        *entries = Entries::new(vec![Entry::Folder(folder)]);
        Ok(())
    })
    .unwrap();
    excluded
}

fn prune_read(home: &Path) -> usize {
    let entries = load(home).unwrap();
    prune_excluded(home, &gone_excluded(&entries, &shown(&entries))).unwrap()
}

fn excluded_now(home: &Path) -> Vec<PathBuf> {
    load(home).unwrap().folders().next().unwrap().excluded.clone()
}

#[test]
fn an_excluded_folder_deleted_is_dropped_and_one_still_there_kept_whether_sub_folders_show_or_not() {
    for show in [true, false] {
        let (home, root) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        for d in ["web", "docs"] {
            std::fs::create_dir(root.path().join(d)).unwrap();
        }
        let [web, docs] = <[PathBuf; 2]>::try_from(excluding(home.path(), root.path(), show, &["web", "docs"])).unwrap();
        assert_eq!(prune_read(home.path()), 0, "both there");
        std::fs::remove_dir(&web).unwrap();
        assert_eq!(prune_read(home.path()), 1, "show {show}");
        assert_eq!(excluded_now(home.path()), [docs], "show {show}");
    }
}

#[test]
fn a_parent_that_cannot_be_read_drops_nothing() {
    let (home, root) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let parent = root.path().join("gone");
    for show in [true, false] {
        let excluded = excluding(home.path(), &parent, show, &["web"]);
        assert_eq!(prune_read(home.path()), 0, "show {show}");
        assert_eq!(prune_excluded(home.path(), &excluded).unwrap(), 0, "re-checked under the lock, show {show}");
        assert_eq!(excluded_now(home.path()), excluded, "show {show}");
    }
}

#[test]
fn a_renamed_folder_drops_its_old_path_and_shows_under_its_new_name() {
    let (home, root) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    std::fs::create_dir(root.path().join("web")).unwrap();
    excluding(home.path(), root.path(), true, &["web"]);
    std::fs::rename(root.path().join("web"), root.path().join("site")).unwrap();
    assert_eq!(prune_read(home.path()), 1);
    let entries = load(home.path()).unwrap();
    let shown = shown(&entries);
    let f = entries.folders().next().unwrap();
    assert!(f.excluded.is_empty());
    assert_eq!(f.children(&shown).collect::<Vec<_>>(), [root.path().join("site").as_path()]);
}

#[test]
fn a_folder_made_again_before_the_edit_is_kept_and_nothing_is_written() {
    let (home, root) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    std::fs::create_dir(root.path().join("web")).unwrap();
    let excluded = excluding(home.path(), root.path(), true, &["web"]);
    std::fs::remove_dir(&excluded[0]).unwrap();
    let entries = load(home.path()).unwrap();
    let gone = gone_excluded(&entries, &shown(&entries));
    assert_eq!(gone, excluded);
    std::fs::create_dir(&excluded[0]).unwrap();
    let file = path(home.path());
    let before = (std::fs::read(&file).unwrap(), std::fs::metadata(&file).unwrap().modified().unwrap());
    assert_eq!(prune_excluded(home.path(), &gone).unwrap(), 0);
    assert_eq!(excluded_now(home.path()), excluded);
    let after = (std::fs::read(&file).unwrap(), std::fs::metadata(&file).unwrap().modified().unwrap());
    assert_eq!(before, after, "no write when nothing is dropped");
}
