//! Every reader on a fixture leaves the repository as it found it (C146).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use supraman_gitman::fixture::{Fixture, git_in};
use supraman_gitman::*;

/// Every file under `root` (the `.git` dir with its index, refs and reflogs, and the worktree): bytes and mtime.
fn files(root: &Path) -> BTreeMap<PathBuf, (Vec<u8>, SystemTime)> {
    let mut out = BTreeMap::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            } else {
                let modified = entry.metadata().unwrap().modified().unwrap();
                out.insert(path.clone(), (std::fs::read(&path).unwrap(), modified));
            }
        }
    }
    out
}

#[derive(Debug, PartialEq)]
struct Snapshot {
    files: BTreeMap<PathBuf, (Vec<u8>, SystemTime)>,
    head: String,
    refs: String,
    status: String,
}

fn snapshot(f: &Fixture) -> Snapshot {
    let files = files(f.temp.path());
    let git = |args: &[&str]| git_in(&f.git, &f.dir, args, &[], None);
    Snapshot {
        files,
        head: git(&["rev-parse", "HEAD"]),
        refs: git(&["for-each-ref"]),
        status: git(&["--no-optional-locks", "status", "--porcelain"]),
    }
}

fn read_everything(f: &Fixture, repo: &Repo) {
    let refs = refs(repo).unwrap();
    history(repo, &refs.tips(), 0, FIRST_PAGE).unwrap();
    details(repo, f.sha("rename")).unwrap();
    for label in ["root", "rename", "merge"] {
        changed_files(repo, f.sha(label)).unwrap();
    }
    changed_between(repo, f.sha("root"), f.sha("ahead")).unwrap();
    tree(repo, "HEAD").unwrap();
    blob(repo, "HEAD", "bin.dat").unwrap();
    staged_blob(repo, "a.txt").unwrap();
    worktree_file(repo, "a.txt").unwrap();
    let rev = f.sha("root");
    for of in [DiffOf::Commit(f.sha("merge")), DiffOf::Revs(rev, "HEAD"), DiffOf::RevToWorktree(rev), DiffOf::RevToIndex(rev), DiffOf::IndexToWorktree] {
        patch(repo, of, &[]).unwrap();
    }
    status(repo).unwrap();
    dirs(repo).unwrap();
    ignored(repo, &["a.txt".into(), "target/x".into()]).unwrap();
}

#[test]
fn every_reader_leaves_refs_index_head_reflog_and_worktree_unchanged() {
    let f = Fixture::new();
    let before = snapshot(&f);
    assert!(before.files.keys().any(|p| p.ends_with(Path::new(".git").join("index"))));
    assert!(before.files.keys().any(|p| p.ends_with(Path::new("logs").join("HEAD"))), "the reflog is in the snapshot");
    read_everything(&f, &f.repo());
    read_everything(&f, &Repo::new(f.git.clone(), &f.worktree));
    let after = snapshot(&f);
    let changed: Vec<&PathBuf> = before.files.keys().chain(after.files.keys()).filter(|p| before.files.get(*p) != after.files.get(*p)).collect();
    assert!(changed.is_empty(), "the readers changed {changed:?}");
    assert_eq!(before, after);
}
