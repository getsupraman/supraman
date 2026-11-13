use supraman_gitman::fixture::{self, BINARY, Fixture, git_in};
use supraman_gitman::*;

#[test]
fn refs_name_branches_with_ahead_behind_tags_remotes_and_worktrees() {
    let f = Fixture::shared();
    let r = refs(&f.repo()).unwrap();
    assert_eq!(r.head, Head::Branch { name: "main".into(), sha: Some(f.sha("ahead").into()) });
    let branch = |name: &str| r.branches.iter().find(|b| b.name == name).unwrap_or_else(|| panic!("no {name} in {:?}", r.branches));
    assert_eq!(branch("main").upstream, Some(Upstream { name: "origin/main".into(), ahead: 1, behind: 0, gone: false }));
    assert_eq!(branch("stale").upstream, Some(Upstream { name: "origin/main".into(), ahead: 0, behind: 2, gone: false }));
    assert_eq!(branch("feature").upstream, None);
    assert_eq!(branch("side").sha, f.sha("second"));
    assert_eq!(r.remotes, [RemoteBranch { name: "origin/main".into(), sha: f.sha("merge").into() }]);
    let tag = |name: &str| r.tags.iter().find(|t| t.name == name).unwrap();
    assert!(tag("v1").annotated && tag("v1").sha != f.sha("second"));
    assert_eq!(tag("v1").commit.as_deref(), Some(f.sha("second")));
    assert_eq!((tag("light").annotated, tag("light").commit.as_deref()), (false, Some(f.sha("main_work"))));
    assert_eq!(r.worktrees.len(), 2, "{:?}", r.worktrees);
    assert_eq!(r.worktrees[0].branch.as_deref(), Some("main"));
    assert_eq!((r.worktrees[1].branch.as_deref(), r.worktrees[1].head.as_deref()), (Some("side"), Some(f.sha("second"))));
    let tips = r.tips();
    for label in ["ahead", "merge", "rename", "main_work", "second", "stash_wip_index", "stash_message_index", "stash_feature_index"] {
        assert!(tips.iter().any(|t| t == f.sha(label)), "{label} is a tip");
    }
    assert_eq!(tips.len(), 8, "each once: {tips:?}");
    assert!(!tips.iter().any(|t| t == f.sha("stash_wip")), "a stash's own commit is no tip");
}

#[test]
fn a_page_of_history_has_parents_and_says_whether_more_follow() {
    let f = Fixture::shared();
    let repo = f.repo();
    let tips = refs(&repo).unwrap().tips();
    let first = history(&repo, &tips, 0, 4).unwrap();
    let shas: Vec<&str> = first.commits.iter().map(|c| c.sha.as_str()).collect();
    assert_eq!(shas, [f.sha("ahead"), f.sha("stash_wip_index"), f.sha("stash_message_index"), f.sha("merge")]);
    assert!(first.more);
    assert_eq!(first.commits[1].parents, [f.sha("merge")], "a stash's index commit has the base for its one parent");
    assert_eq!(first.commits[3].parents, [f.sha("main_work"), f.sha("rename")]);
    assert_eq!(first.commits[3].message, "Merge feature\n");
    let author = &first.commits[0].author;
    assert_eq!((author.name.as_str(), author.email.as_str(), author.time), ("Ada Fixture", "ada@example.test", 1_700_000_360));
    let rest = history(&repo, &tips, 4, 5).unwrap();
    let rest_shas: Vec<&str> = rest.commits.iter().map(|c| c.sha.as_str()).collect();
    assert_eq!(rest_shas, [f.sha("main_work"), f.sha("stash_feature_index"), f.sha("rename"), f.sha("second"), f.sha("root")]);
    assert!(!rest.more);
    assert!(rest.commits[4].parents.is_empty());
    assert_eq!(history(&repo, &tips, 0, 100).unwrap().commits.len(), 9);
}

#[test]
fn details_split_the_message_and_name_the_co_authors() {
    let f = Fixture::shared();
    let d = details(&f.repo(), f.sha("rename")).unwrap();
    assert_eq!(d.summary, "Move old.txt to new.txt");
    assert!(d.body.starts_with("The old name no longer fits.") && d.body.ends_with("<cy@example.test>"), "{:?}", d.body);
    let co: Vec<(&str, &str)> = d.co_authors.iter().map(|p| (p.name.as_str(), p.email.as_str())).collect();
    assert_eq!(co, [("Bea Two", "bea@example.test"), ("Cy Three", "cy@example.test")]);
    assert_eq!(d.commit.parents, [f.sha("second")]);
    assert!(details(&f.repo(), f.sha("root")).unwrap().co_authors.is_empty());
}

#[test]
fn changed_files_of_a_rename_a_merge_against_its_first_parent_and_the_root() {
    let f = Fixture::shared();
    let repo = f.repo();
    let renamed = FileChange { kind: ChangeKind::Renamed, path: "new.txt".into(), old_path: Some("old.txt".into()), similarity: Some(100) };
    assert_eq!(changed_files(&repo, f.sha("rename")).unwrap(), [renamed.clone()]);
    assert_eq!(changed_files(&repo, f.sha("merge")).unwrap(), [renamed], "only what the merge brought to main");
    let root = changed_files(&repo, f.sha("root")).unwrap();
    assert!(root.iter().all(|c| c.kind == ChangeKind::Added), "{root:?}");
    assert_eq!(root.iter().map(|c| c.path.as_str()).collect::<Vec<_>>(), ["a.txt", "b.txt", "bin.dat", "old.txt"]);
    let between = changed_between(&repo, f.sha("root"), f.sha("main_work")).unwrap();
    assert_eq!(between.iter().map(|c| (c.kind, c.path.as_str())).collect::<Vec<_>>(), [(ChangeKind::Modified, "a.txt"), (ChangeKind::Modified, "b.txt")]);
}

#[test]
fn the_tree_and_its_blobs_text_and_binary() {
    let f = Fixture::shared();
    let repo = f.repo();
    let files = tree(&repo, f.sha("merge")).unwrap();
    assert_eq!(files.iter().map(|t| (t.path.as_str(), t.mode)).collect::<Vec<_>>(), [("a.txt", 0o100644), ("b.txt", 0o100644), ("bin.dat", 0o100644), ("new.txt", 0o100644)]);
    let text = blob(&repo, f.sha("second"), "a.txt").unwrap();
    assert_eq!((text.bytes.as_slice(), text.binary), (&b"one\ntwo\n"[..], false));
    let binary = blob(&repo, f.sha("merge"), "bin.dat").unwrap();
    assert_eq!((binary.bytes.as_slice(), binary.binary), (BINARY, true));
    assert!(blob(&repo, f.sha("root"), "new.txt").is_err());
}

#[test]
fn work_in_progress_entries_and_a_merge_under_way() {
    let f = Fixture::copy();
    let s = status(&f.repo()).unwrap();
    assert_eq!(s.branch, BranchStatus { oid: Some(f.sha("ahead").into()), head: Some("main".into()), upstream: Some("origin/main".into()), ahead: 1, behind: 0 });
    let entry = |path: &str| s.entries.iter().find(|e| e.path == path).unwrap_or_else(|| panic!("no {path} in {:?}", s.entries));
    assert_eq!((entry("a.txt").kind, entry("a.txt").staged, entry("a.txt").unstaged), (EntryKind::Ordinary, Some(ChangeKind::Modified), Some(ChangeKind::Modified)));
    assert_eq!((entry("s.txt").staged, entry("s.txt").unstaged), (Some(ChangeKind::Added), None));
    assert_eq!((entry("renamed.txt").kind, entry("renamed.txt").orig_path.as_deref()), (EntryKind::Renamed { score: 100 }, Some("new.txt")));
    assert_eq!((entry("b.txt").staged, entry("b.txt").unstaged), (None, Some(ChangeKind::Deleted)));
    assert_eq!((entry("u.txt").kind, entry("u.txt").unstaged), (EntryKind::Untracked, Some(ChangeKind::Untracked)));
    assert_eq!(s.entries.len(), 5, "bin.dat is unchanged: {:?}", s.entries);
    assert_eq!(s.operation, None);
    let env = [("GIT_AUTHOR_DATE", "@1700001000 +0000".to_string()), ("GIT_COMMITTER_DATE", "@1700001000 +0000".to_string())];
    git_in(&f.git, &f.worktree, &["merge", "-q", "--no-ff", "--no-commit", "stale"], &env, None);
    let in_worktree = status(&Repo::new(f.git.clone(), &f.worktree)).unwrap();
    assert_eq!((in_worktree.operation, in_worktree.branch.head.as_deref()), (Some(Operation::Merge), Some("side")));
    assert_eq!(status(&f.repo()).unwrap().operation, None, "the merge is the worktree's own");
}

#[test]
fn the_staged_and_the_worktree_side_of_a_file() {
    let f = Fixture::copy();
    let repo = f.repo();
    assert_eq!(staged_blob(&repo, "a.txt").unwrap().bytes, b"staged
");
    assert_eq!(worktree_file(&repo, "a.txt").unwrap().bytes, b"staged
unstaged
");
    assert_eq!(worktree_file(&repo, "u.txt").unwrap().bytes, b"untracked
");
    assert!(staged_blob(&repo, "u.txt").is_err(), "an untracked file has no staged side");
    assert!(matches!(worktree_file(&repo, "b.txt"), Err(GitError::Read { .. })), "b.txt is deleted");
    let inside = Repo::new(f.git.clone(), f.dir.join("sub"));
    std::fs::create_dir(f.dir.join("sub")).unwrap();
    assert_eq!(worktree_file(&inside, "a.txt").unwrap().bytes, b"staged
unstaged
", "paths are from the top");
    assert!(worktree_file(&repo, "bin.dat").unwrap().binary);
}

#[test]
fn a_conflicted_file_is_unmerged_in_the_status() {
    let f = Fixture::copy();
    f.conflict();
    let s = status(&Repo::new(f.git.clone(), &f.worktree)).unwrap();
    let a = s.entries.iter().find(|e| e.path == "a.txt").expect("a.txt is listed");
    assert_eq!((a.kind, a.staged, a.unstaged), (EntryKind::Unmerged, Some(ChangeKind::Unmerged), Some(ChangeKind::Unmerged)));
    assert_eq!(s.operation, Some(Operation::Merge));
}

#[test]
fn dirs_of_a_checkout_and_a_linked_worktree() {
    let f = Fixture::shared();
    let same = |a: &std::path::Path, b: &std::path::Path| a.canonicalize().unwrap() == b.canonicalize().unwrap();
    let main = dirs(&f.repo()).unwrap();
    assert!(same(&main.top, &f.dir) && same(&main.git_dir, &f.dir.join(".git")) && same(&main.common_dir, &main.git_dir), "{main:?}");
    let wt = dirs(&Repo::new(f.git.clone(), &f.worktree)).unwrap();
    assert!(same(&wt.top, &f.worktree) && same(&wt.common_dir, &main.git_dir), "{wt:?}");
    assert!(same(&wt.git_dir, &f.dir.join(".git").join("worktrees").join("wt")), "a worktree's own git dir: {wt:?}");
}

#[test]
fn ignored_names_the_ignored_paths_but_never_a_tracked_file() {
    let f = Fixture::copy();
    f.write(".gitignore", b"target/\n*.log\na.txt\n");
    std::fs::create_dir(f.dir.join("target")).unwrap();
    f.write("target/out.bin", b"x");
    let asked: Vec<String> = ["target/out.bin", "x.log", "a.txt", "u.txt", "sub/deep.log"].map(String::from).into();
    let mut got: Vec<String> = ignored(&f.repo(), &asked).unwrap().into_iter().collect();
    got.sort();
    assert_eq!(got, ["sub/deep.log", "target/out.bin", "x.log"], "a.txt is tracked");
    assert!(ignored(&f.repo(), &[]).unwrap().is_empty());
}

#[test]
fn stashes_from_the_reflog_newest_first_with_their_index_commit_base_branch_and_message() {
    let f = Fixture::shared();
    let all = stashes(&f.repo()).unwrap();
    let shas: Vec<&str> = all.iter().map(|s| s.sha.as_str()).collect();
    assert_eq!(shas, fixture::STASHES.map(|l| f.sha(l)));
    let [wip, message, feature] = &all[..] else { panic!("three stashes: {all:?}") };
    assert_eq!((message.branch.as_str(), message.label(), message.base.as_str()), ("main", fixture::MAIN_STASH.to_string(), f.sha("merge")));
    assert_eq!(message.index, f.sha("stash_message_index"));
    let short = &f.sha("merge")[..7];
    assert_eq!((wip.branch.as_str(), wip.label()), ("main", format!("{short} Merge feature")));
    assert_eq!((feature.branch.as_str(), feature.label(), feature.base.as_str()), ("feature", fixture::FEATURE_STASH.to_string(), f.sha("rename")));
    assert_eq!(wip.time - message.time, 20);
    assert_eq!(refs(&f.repo()).unwrap().stashes, all);
    let changed = changed_between(&f.repo(), &message.base, &message.sha).unwrap();
    assert_eq!(changed.iter().map(|c| (c.path.as_str(), c.kind)).collect::<Vec<_>>(), [("a.txt", ChangeKind::Modified)]);
}

#[test]
fn no_stashes_without_a_stash_ref() {
    let f = Fixture::copy();
    git_in(&f.git, &f.dir, &["update-ref", "-d", "refs/stash"], &[], None);
    assert!(stashes(&f.repo()).unwrap().is_empty());
}

#[test]
fn head_detached_and_on_a_branch_with_no_commit_yet() {
    let f = Fixture::copy();
    git_in(&f.git, &f.worktree, &["checkout", "-q", "--detach"], &[], None);
    let wt = refs(&Repo::new(f.git.clone(), &f.worktree)).unwrap();
    assert_eq!(wt.head, Head::Detached { sha: f.sha("second").into() });
    assert_eq!(refs(&f.repo()).unwrap().head, Head::Branch { name: "main".into(), sha: Some(f.sha("ahead").into()) }, "the main checkout's own HEAD");
    let empty = f.temp.path().join("empty");
    git_in(&f.git, f.temp.path(), &["init", "-q", "-b", "fresh", "empty"], &[], None);
    assert_eq!(refs(&Repo::new(f.git.clone(), &empty)).unwrap().head, Head::Branch { name: "fresh".into(), sha: None });
}

#[test]
fn a_repo_that_knows_its_dirs_reads_them_instead_of_asking() {
    let f = Fixture::copy();
    let env = [("GIT_AUTHOR_DATE", "@1700001000 +0000".to_string()), ("GIT_COMMITTER_DATE", "@1700001000 +0000".to_string())];
    git_in(&f.git, &f.worktree, &["merge", "-q", "--no-ff", "--no-commit", "stale"], &env, None);
    let theirs = dirs(&Repo::new(f.git.clone(), &f.worktree)).unwrap();
    let told = f.repo().with_dirs(theirs.clone());
    assert_eq!(dirs(&told).unwrap(), theirs);
    assert_eq!(status(&told).unwrap().operation, Some(Operation::Merge), "the operation of the git dir it was told");
    std::fs::write(theirs.top.join("a.txt"), b"theirs\n").unwrap();
    assert_eq!(worktree_file(&told, "a.txt").unwrap().bytes, b"theirs\n", "the file under the top it was told");
}

#[test]
fn a_head_read_takes_the_classify_window_and_ends_its_git() {
    use std::time::Duration;
    use supraman_core::platform::process::snapshot;
    use supraman_core::process::alive;
    use supraman_core::testing::{eventually, polled};
    use supraman_core::text::BINARY_PROBE;

    let f = Fixture::copy();
    let repo = f.repo();
    let big: Vec<u8> = (0..4 * 1024 * 1024u32).map(|i| (i % 251) as u8).collect();
    let prose = "a line of text\n".repeat(2000);
    f.write("big.bin", &big);
    f.write("prose.txt", prose.as_bytes());
    f.git(&["add", "big.bin", "prose.txt"]);

    let head = staged_blob_head(&repo, "big.bin").unwrap();
    let ours = std::process::id();
    let gits: Vec<u32> = snapshot().into_iter().filter(|p| p.parent == ours && p.exe.starts_with("git")).map(|p| p.pid).collect();
    assert_eq!((head.bytes.as_slice(), head.binary), (&big[..BINARY_PROBE], true));
    eventually(Duration::from_secs(20), "the head read's git is gone", polled(Duration::from_millis(20)), || gits.iter().all(|p| !alive(*p)).then_some(()));

    let text = staged_blob_head(&repo, "prose.txt").unwrap();
    assert_eq!((text.bytes.as_slice(), text.binary), (&prose.as_bytes()[..BINARY_PROBE], false));
    assert_eq!(staged_blob(&repo, "prose.txt").unwrap().bytes, prose.as_bytes(), "a text blob still reads whole");
    let small = blob_head(&repo, f.sha("second"), "a.txt").unwrap();
    assert_eq!((small.bytes.as_slice(), small.binary), (&b"one\ntwo\n"[..], false), "a blob under the window is read to its end");
    let file = worktree_head(&repo, "big.bin").unwrap();
    assert_eq!((file.bytes.len(), file.binary), (BINARY_PROBE, true));
}
