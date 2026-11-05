use supraman_gitman::fixture::{BINARY, Fixture, git_in};
use supraman_gitman::*;

#[test]
fn refs_name_branches_with_ahead_behind_tags_remotes_and_worktrees() {
    let f = Fixture::new();
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
    for label in ["ahead", "merge", "rename", "main_work", "second"] {
        assert!(tips.iter().any(|t| t == f.sha(label)), "{label} is a tip");
    }
    assert_eq!(tips.len(), 5, "each once: {tips:?}");
}

#[test]
fn a_page_of_history_has_parents_and_says_whether_more_follow() {
    let f = Fixture::new();
    let repo = f.repo();
    let tips = refs(&repo).unwrap().tips();
    let first = history(&repo, &tips, 0, 3).unwrap();
    let shas: Vec<&str> = first.commits.iter().map(|c| c.sha.as_str()).collect();
    assert_eq!(shas, [f.sha("ahead"), f.sha("merge"), f.sha("main_work")]);
    assert!(first.more);
    assert_eq!(first.commits[1].parents, [f.sha("main_work"), f.sha("rename")]);
    assert_eq!(first.commits[1].message, "Merge feature\n");
    let author = &first.commits[0].author;
    assert_eq!((author.name.as_str(), author.email.as_str(), author.time), ("Ada Fixture", "ada@example.test", 1_700_000_360));
    let rest = history(&repo, &tips, 3, 3).unwrap();
    assert_eq!(rest.commits.iter().map(|c| c.sha.as_str()).collect::<Vec<_>>(), [f.sha("rename"), f.sha("second"), f.sha("root")]);
    assert!(!rest.more);
    assert!(rest.commits[2].parents.is_empty());
    assert_eq!(history(&repo, &tips, 0, 100).unwrap().commits.len(), 6);
}

#[test]
fn details_split_the_message_and_name_the_co_authors() {
    let f = Fixture::new();
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
    let f = Fixture::new();
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
    let f = Fixture::new();
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
fn patches_of_a_commit_two_revs_and_the_work_in_progress() {
    let f = Fixture::new();
    let repo = f.repo();
    let lines = |p: &[FilePatch]| -> Vec<(LineKind, String)> { p[0].hunks.iter().flat_map(|h| h.lines.iter().map(|l| (l.kind, l.text.clone()))).collect() };
    let second = patch(&repo, DiffOf::Commit(f.sha("second")), &["a.txt"]).unwrap();
    assert_eq!(lines(&second), [(LineKind::Context, "one".into()), (LineKind::Added, "two".into())]);
    assert_eq!((second[0].hunks[0].old_start, second[0].hunks[0].new_lines), (1, 2));
    let rename = patch(&repo, DiffOf::Commit(f.sha("rename")), &["old.txt", "new.txt"]).unwrap();
    assert_eq!((rename.len(), rename[0].old_path.as_deref(), rename[0].new_path.as_deref()), (1, Some("old.txt"), Some("new.txt")));
    assert!(rename[0].hunks.is_empty());
    let root_bin = patch(&repo, DiffOf::Commit(f.sha("root")), &["bin.dat"]).unwrap();
    assert!(root_bin[0].binary && root_bin[0].old_path.is_none());
    assert_eq!(patch(&repo, DiffOf::Revs(f.sha("root"), f.sha("main_work")), &[]).unwrap().len(), 2);
    let unstaged = patch(&repo, DiffOf::IndexToWorktree, &["a.txt"]).unwrap();
    assert_eq!(lines(&unstaged), [(LineKind::Context, "staged".into()), (LineKind::Added, "unstaged".into())]);
    let staged = patch(&repo, DiffOf::RevToIndex("HEAD"), &["a.txt"]).unwrap();
    assert!(lines(&staged).contains(&(LineKind::Added, "staged".into())) && lines(&staged).contains(&(LineKind::Removed, "three".into())));
    let both = patch(&repo, DiffOf::RevToWorktree("HEAD"), &["a.txt"]).unwrap();
    assert!(lines(&both).contains(&(LineKind::Added, "unstaged".into())));
}

#[test]
fn work_in_progress_entries_and_a_merge_under_way() {
    let f = Fixture::new();
    let s = status(&f.repo()).unwrap();
    assert_eq!(s.branch, BranchStatus { oid: Some(f.sha("ahead").into()), head: Some("main".into()), upstream: Some("origin/main".into()), ahead: 1, behind: 0 });
    let entry = |path: &str| s.entries.iter().find(|e| e.path == path).unwrap_or_else(|| panic!("no {path} in {:?}", s.entries));
    assert_eq!((entry("a.txt").kind, entry("a.txt").staged, entry("a.txt").unstaged), (EntryKind::Ordinary, Some(ChangeKind::Modified), Some(ChangeKind::Modified)));
    assert_eq!((entry("s.txt").staged, entry("s.txt").unstaged), (Some(ChangeKind::Added), None));
    assert_eq!((entry("renamed.txt").kind, entry("renamed.txt").orig_path.as_deref()), (EntryKind::Renamed { score: 100 }, Some("new.txt")));
    assert_eq!((entry("b.txt").staged, entry("b.txt").unstaged), (None, Some(ChangeKind::Deleted)));
    assert_eq!(entry("u.txt").kind, EntryKind::Untracked);
    assert_eq!(s.entries.len(), 5, "bin.dat is unchanged: {:?}", s.entries);
    assert_eq!(s.operation, None);
    let env = [("GIT_AUTHOR_DATE", "@1700001000 +0000".to_string()), ("GIT_COMMITTER_DATE", "@1700001000 +0000".to_string())];
    git_in(&f.git, &f.worktree, &["merge", "-q", "--no-ff", "--no-commit", "stale"], &env, None);
    let in_worktree = status(&Repo::new(f.git.clone(), &f.worktree)).unwrap();
    assert_eq!((in_worktree.operation, in_worktree.branch.head.as_deref()), (Some(Operation::Merge), Some("side")));
    assert_eq!(status(&f.repo()).unwrap().operation, None, "the merge is the worktree's own");
}
