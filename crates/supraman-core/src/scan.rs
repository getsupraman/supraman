use std::fs;
use std::path::Path;

use crate::{CoreError, Folder, FolderKind, Result};

pub const DEFAULT_DEPTH: usize = 1;

/// The folders under `root`: at `depth` 1 every direct subfolder, git or plain; deeper, the git repos at most
/// `depth` levels down. Hidden folders, `*.worktrees` folders and linked worktrees are skipped. A `root` that is
/// itself a repo (a `.git` folder, or the `.git` file of a linked worktree) is one too, listed first named after
/// its folder ([`root_repo_name`]), its path the root; a plain `root` scanned at depth 1 with no subfolder is its
/// own one folder.
///
/// Read-only: it never runs git and never writes inside a scanned folder.
pub fn scan(root: &Path, depth: usize) -> Result<Vec<Folder>> {
    if !root.is_dir() {
        return Err(CoreError::NotADirectory(root.to_path_buf()));
    }
    let entries = fs::read_dir(root).map_err(|e| CoreError::io(root, e))?;
    let mut folders = Vec::new();
    // Only a depth-1 scan lists plain folders; a deeper one stays a search for repos (add-workspace/1).
    let every = depth == 1;
    walk(entries, "", 1, depth, every, &mut folders);
    folders.sort_by(|a, b| a.name.cmp(&b.name));
    let git = root.join(".git");
    if git.exists() {
        let name = root_repo_name(root, &folders);
        let git_dir = if git.is_dir() { Some(git) } else { crate::paths::linked_git_dir(&git) };
        folders.insert(0, repo_at(name, root.to_path_buf(), git_dir.as_deref()));
    } else if every && folders.is_empty() {
        folders.push(folder_at(root_repo_name(root, &[]), root.to_path_buf()));
    }
    Ok(folders)
}

/// The root repo's name: its folder's name, `<folder>-root` when a repo below already has that name.
fn root_repo_name(root: &Path, below: &[Folder]) -> String {
    let folder = root.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_else(|| "root".into());
    if below.iter().any(|r| r.name.eq_ignore_ascii_case(&folder)) { format!("{folder}-root") } else { folder }
}

fn repo_at(name: String, path: std::path::PathBuf, git_dir: Option<&Path>) -> Folder {
    Folder {
        has_claude_md: path.join("CLAUDE.md").is_file(),
        has_agents_md: path.join("AGENTS.md").is_file(),
        branch: git_dir.and_then(read_branch),
        ..Folder::new(name, path)
    }
}

/// `path` as a folder named `name`: a git folder when it holds a `.git` (a folder, or a linked worktree's
/// file), else a plain one. Read-only.
pub(crate) fn folder_at(name: String, path: std::path::PathBuf) -> Folder {
    let git = path.join(".git");
    if git.is_dir() {
        repo_at(name, path, Some(&git))
    } else if git.exists() {
        let git_dir = crate::paths::linked_git_dir(&git);
        repo_at(name, path, git_dir.as_deref())
    } else {
        Folder { kind: FolderKind::Plain, ..repo_at(name, path, None) }
    }
}

fn walk(entries: fs::ReadDir, prefix: &str, level: usize, depth: usize, every: bool, found: &mut Vec<Folder>) {
    if level > depth {
        return;
    }
    for entry in entries.flatten() {
        // Symlinks and junctions report as non-dirs here, so the walk cannot loop.
        if !entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let Ok(folder) = entry.file_name().into_string() else {
            continue;
        };
        if folder.starts_with('.') || folder.ends_with(".worktrees") || crate::platform::fs::hidden(&entry) {
            continue;
        }
        let path = entry.path();
        let name = format!("{prefix}{folder}");
        let git = path.join(".git");
        if git.is_dir() {
            found.push(repo_at(name, path, Some(&git)));
        } else if git.exists() {
            // A `.git` file: a linked worktree of a repo that lives elsewhere.
            continue;
        } else if every {
            found.push(Folder { kind: FolderKind::Plain, ..repo_at(name, path, None) });
        } else if let Ok(children) = fs::read_dir(&path) {
            walk(children, &format!("{name}/"), level + 1, depth, every, found);
        }
    }
}

/// The branch checked out at `dir`, from the nearest `.git` at or above it (a folder, or a linked
/// worktree's file); `None` when HEAD is detached or there is no repo.
pub fn branch_at(dir: &Path) -> Option<String> {
    let git = crate::paths::git_at(dir)?;
    let git_dir = if git.is_dir() { git } else { crate::paths::linked_git_dir(&git)? };
    read_branch(&git_dir)
}

fn read_branch(git_dir: &Path) -> Option<String> {
    let head = fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let reference = head.trim().strip_prefix("ref:")?.trim();
    Some(reference.strip_prefix("refs/heads/").unwrap_or(reference).to_string())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn repo(dir: &Path, head: &str) -> PathBuf {
        fs::create_dir_all(dir.join(".git")).unwrap();
        fs::write(dir.join(".git").join("HEAD"), head).unwrap();
        dir.to_path_buf()
    }

    fn names(repos: &[Folder]) -> Vec<&str> {
        repos.iter().map(|r| r.name.as_str()).collect()
    }

    /// root/
    ///   Alpha/            repo on main, CLAUDE.md
    ///   Beta/             repo, detached HEAD, AGENTS.md
    ///   Gamma/            repo on feature/x, contains a nested repo (must not be descended into)
    ///   Alpha-wt/         worktree (.git file)
    ///   Alpha.worktrees/  holds a repo and a worktree
    ///   .hidden/          holds a repo
    ///   group/deep/       repo at depth 2
    ///   plain/            no repo
    fn tree() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        let r = tmp.path();
        let alpha = repo(&r.join("Alpha"), "ref: refs/heads/main\n");
        fs::write(alpha.join("CLAUDE.md"), "x").unwrap();
        let beta = repo(&r.join("Beta"), "3f786850e387550fdab836ed7e6dc881de23001b\n");
        fs::write(beta.join("AGENTS.md"), "x").unwrap();
        let gamma = repo(&r.join("Gamma"), "ref: refs/heads/feature/x\n");
        repo(&gamma.join("inner"), "ref: refs/heads/main\n");
        fs::create_dir_all(r.join("Alpha-wt")).unwrap();
        fs::write(r.join("Alpha-wt").join(".git"), "gitdir: ../Alpha/.git/worktrees/wt\n").unwrap();
        repo(&r.join("Alpha.worktrees").join("feat"), "ref: refs/heads/feat\n");
        repo(&r.join(".hidden").join("secret"), "ref: refs/heads/main\n");
        repo(&r.join("group").join("deep"), "ref: refs/heads/dev\n");
        fs::create_dir_all(r.join("plain").join("sub")).unwrap();
        fs::write(r.join("README.md"), "x").unwrap();
        tmp
    }

    #[test]
    fn depth_one_lists_every_direct_subfolder_git_or_plain() {
        let tmp = tree();
        let repos = scan(tmp.path(), DEFAULT_DEPTH).unwrap();
        assert_eq!(names(&repos), ["Alpha", "Beta", "Gamma", "group", "plain"]);
        assert!(repos.iter().all(|r| r.enabled));
        assert_eq!(repos[0].path, tmp.path().join("Alpha"));
        let kinds: Vec<FolderKind> = repos.iter().map(|r| r.kind).collect();
        assert_eq!(kinds, [FolderKind::Git, FolderKind::Git, FolderKind::Git, FolderKind::Plain, FolderKind::Plain]);
    }

    #[test]
    fn a_plain_parent_lists_its_subfolders_as_plain_folders() {
        let tmp = tempfile::tempdir().unwrap();
        for f in ["NodeApi", "docs"] {
            fs::create_dir_all(tmp.path().join(f).join("src")).unwrap();
        }
        fs::write(tmp.path().join("NodeApi").join("CLAUDE.md"), "x").unwrap();
        let found = scan(tmp.path(), 1).unwrap();
        assert_eq!(names(&found), ["NodeApi", "docs"]);
        assert!(found.iter().all(|f| f.kind == FolderKind::Plain && f.branch.is_none()));
        assert!(found[0].has_claude_md && !found[1].has_claude_md);
        assert_eq!(found[1].path, tmp.path().join("docs"));
    }

    #[test]
    fn a_parent_without_subfolders_is_its_own_one_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("Notes");
        fs::create_dir_all(root.join(".cache")).unwrap();
        fs::write(root.join("todo.md"), "x").unwrap();
        let found = scan(&root, 1).unwrap();
        assert_eq!(names(&found), ["Notes"]);
        assert_eq!((found[0].path.as_path(), found[0].kind), (root.as_path(), FolderKind::Plain));
        assert!(scan(&root, 2).unwrap().is_empty(), "a deeper scan only searches for repos");
    }

    #[test]
    fn hidden_subfolders_are_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        for f in [".vscode", "shown", "Hidden"] {
            fs::create_dir_all(tmp.path().join(f)).unwrap();
        }
        repo(&tmp.path().join(".dotrepo"), "ref: refs/heads/main
");
        #[cfg(windows)]
        unsafe {
            use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_HIDDEN, SetFileAttributesW};
            SetFileAttributesW(&windows::core::HSTRING::from(tmp.path().join("Hidden").as_os_str()), FILE_ATTRIBUTE_HIDDEN).unwrap();
        }
        let expected: &[&str] = if cfg!(windows) { &["shown"] } else { &["Hidden", "shown"] };
        assert_eq!(names(&scan(tmp.path(), 1).unwrap()), expected);
    }

    #[test]
    fn deeper_scan_finds_nested_repo_but_never_descends_into_a_repo() {
        let tmp = tree();
        let repos = scan(tmp.path(), 2).unwrap();
        assert_eq!(names(&repos), ["Alpha", "Beta", "Gamma", "group/deep"]);
        let repos = scan(tmp.path(), 5).unwrap();
        assert_eq!(names(&repos), ["Alpha", "Beta", "Gamma", "group/deep"]);
    }

    #[test]
    fn branch_and_detached_head() {
        let tmp = tree();
        let repos = scan(tmp.path(), 1).unwrap();
        assert_eq!(repos[0].branch.as_deref(), Some("main"));
        assert_eq!(repos[1].branch, None);
        assert_eq!(repos[2].branch.as_deref(), Some("feature/x"));
    }

    #[test]
    fn missing_head_gives_no_branch() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("bare").join(".git")).unwrap();
        let repos = scan(tmp.path(), 1).unwrap();
        assert_eq!(names(&repos), ["bare"]);
        assert_eq!(repos[0].branch, None);
    }

    #[test]
    fn claude_and_agents_md_detection() {
        let tmp = tree();
        let repos = scan(tmp.path(), 1).unwrap();
        assert!(repos[0].has_claude_md && !repos[0].has_agents_md);
        assert!(!repos[1].has_claude_md && repos[1].has_agents_md);
        assert!(!repos[2].has_claude_md && !repos[2].has_agents_md);
    }

    #[test]
    fn worktrees_hidden_and_worktree_folders_are_skipped_at_any_depth() {
        let tmp = tree();
        let repos = scan(tmp.path(), 3).unwrap();
        for r in &repos {
            assert!(!r.name.contains("wt"), "{}", r.name);
            assert!(!r.name.contains("worktrees"), "{}", r.name);
            assert!(!r.name.contains("hidden"), "{}", r.name);
        }
    }

    #[test]
    fn scan_writes_nothing() {
        let tmp = tree();
        let before = snapshot(tmp.path());
        scan(tmp.path(), 3).unwrap();
        assert_eq!(snapshot(tmp.path()), before);
    }

    #[test]
    fn a_root_that_is_a_repo_is_one_listed_first_with_its_children() {
        let tmp = tempfile::tempdir().unwrap();
        let root = repo(&tmp.path().join("Gaia"), "ref: refs/heads/main
");
        let repos = scan(&root, 1).unwrap();
        assert_eq!(names(&repos), ["Gaia"]);
        assert_eq!((repos[0].path.as_path(), repos[0].branch.as_deref()), (root.as_path(), Some("main")));
        repo(&root.join("child"), "ref: refs/heads/dev
");
        repo(&root.join("gaia"), "ref: refs/heads/x
");
        let repos = scan(&root, 1).unwrap();
        assert_eq!(names(&repos), ["Gaia-root", "child", "gaia"], "a child of the same name keeps its own");
        assert_eq!(repos[0].path, root);
    }

    #[test]
    fn a_root_that_is_a_linked_worktree_is_a_repo_on_its_branch() {
        let tmp = tempfile::tempdir().unwrap();
        let main = repo(&tmp.path().join("main"), "ref: refs/heads/main
");
        let gitdir = main.join(".git").join("worktrees").join("wt");
        fs::create_dir_all(&gitdir).unwrap();
        fs::write(gitdir.join("HEAD"), "ref: refs/heads/feat/security
").unwrap();
        let wt = tmp.path().join("wt");
        fs::create_dir_all(&wt).unwrap();
        fs::write(wt.join(".git"), format!("gitdir: {}
", gitdir.display())).unwrap();
        let repos = scan(&wt, 1).unwrap();
        assert_eq!((names(&repos), repos[0].branch.as_deref()), (vec!["wt"], Some("feat/security")));
    }

    #[test]
    fn root_must_be_a_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("f");
        fs::write(&file, "x").unwrap();
        assert!(matches!(scan(&file, 1), Err(CoreError::NotADirectory(_))));
        let missing = tmp.path().join("missing");
        assert!(matches!(scan(&missing, 1), Err(CoreError::NotADirectory(_))));
    }

    // Directory mtimes are left out: NTFS refreshes a parent's cached copy of them lazily.
    fn snapshot(dir: &Path) -> Vec<(PathBuf, u64, Option<std::time::SystemTime>)> {
        let mut out = Vec::new();
        for e in fs::read_dir(dir).unwrap().flatten() {
            let m = fs::metadata(e.path()).unwrap();
            if m.is_dir() {
                out.push((e.path(), 0, None));
                out.extend(snapshot(&e.path()));
            } else {
                out.push((e.path(), m.len(), m.modified().ok()));
            }
        }
        out.sort();
        out
    }

    #[test]
    fn the_branch_comes_from_the_nearest_git() {
        let tmp = tempfile::tempdir().unwrap();
        let root = repo(tmp.path(), "ref: refs/heads/m5/29
");
        let inner = root.join("src").join("deep");
        fs::create_dir_all(&inner).unwrap();
        assert_eq!(branch_at(&inner).as_deref(), Some("m5/29"));
        fs::write(root.join(".git").join("HEAD"), "0123456789abcdef
").unwrap();
        assert_eq!(branch_at(&inner), None, "detached");
    }
}
