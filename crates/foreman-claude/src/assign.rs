use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use foreman_core::paths::path_key;
use foreman_core::{Repo, Workspace};

use crate::Session;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Assignment {
    /// Keyed by [`Repo::name`]; repos without sessions are absent.
    pub by_repo: BTreeMap<String, Vec<Session>>,
    pub unassigned: Vec<Session>,
}

pub fn assign(sessions: &[Session], workspace: &Workspace) -> Assignment {
    assign_with(sessions, workspace, main_repo)
}

/// [`assign`] with `main_of` in place of [`main_repo`]: the app renders every frame from a table
/// filled once per poll, so the filesystem walk stays off the UI thread.
pub fn assign_with(sessions: &[Session], workspace: &Workspace, main_of: impl Fn(&Path) -> Option<PathBuf>) -> Assignment {
    let mut out = Assignment::default();
    for s in sessions {
        match repo_of_with(&s.cwd, workspace, &main_of) {
            Some(repo) => out.by_repo.entry(repo.name.clone()).or_default().push(s.clone()),
            None => out.unassigned.push(s.clone()),
        }
    }
    out
}

/// The repo whose path is `cwd` or an ancestor of it (deepest wins), else the repo owning the
/// linked worktree `cwd` lies in, found through the worktree's `.git` file.
pub fn repo_of<'a>(cwd: &Path, workspace: &'a Workspace) -> Option<&'a Repo> {
    repo_of_with(cwd, workspace, main_repo)
}

fn repo_of_with<'a>(cwd: &Path, workspace: &'a Workspace, main_of: impl Fn(&Path) -> Option<PathBuf>) -> Option<&'a Repo> {
    if workspace.repos.is_empty() {
        return None;
    }
    let by_prefix = |dir: &str| {
        workspace
            .repos
            .iter()
            .filter(|r| is_within(dir, &path_key(&r.path)))
            .max_by_key(|r| path_key(&r.path).len())
    };
    let cwd_norm = path_key(cwd);
    by_prefix(&cwd_norm).or_else(|| {
        let main = path_key(&main_of(cwd)?);
        workspace.repos.iter().find(|r| path_key(&r.path) == main)
    })
}

static MAIN_REPO_WALKS: AtomicU64 = AtomicU64::new(0);

/// How many times [`main_repo`] has walked the filesystem in this process; `--selftest` checks that
/// idle frames add none.
pub fn main_repo_walks() -> u64 {
    MAIN_REPO_WALKS.load(Ordering::Relaxed)
}

/// [`foreman_core::paths::main_checkout`], counted by [`main_repo_walks`].
pub fn main_repo(dir: &Path) -> Option<PathBuf> {
    MAIN_REPO_WALKS.fetch_add(1, Ordering::Relaxed);
    foreman_core::paths::main_checkout(dir)
}

fn is_within(dir: &str, root: &str) -> bool {
    dir == root || dir.strip_prefix(root).is_some_and(|rest| rest.starts_with('/'))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use foreman_core::paths::normalize;

    use super::*;
    use crate::parse;

    /// tmp/
    ///   Plat/Alpha/.git/          repo
    ///   Plat/Alpha/src/
    ///   Plat/Alpha.Ext/.git/      repo whose name shares a prefix with Alpha
    ///   Plat/Alpha.worktrees/feat/.git  file → ../../Alpha/.git/worktrees/feat
    ///   Plat/Alpha.worktrees/abs/.git   file → <abs Alpha>/.git/worktrees/abs
    ///   Elsewhere/
    fn layout() -> (tempfile::TempDir, Workspace) {
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
        let repo = |name: &str| Repo {
            name: name.into(),
            path: plat.join(name),
            has_claude_md: false,
            has_agents_md: false,
            branch: Some("main".into()),
            enabled: true,
        };
        let workspace = Workspace::new("plat", &plat, vec![repo("Alpha"), repo("Alpha.Ext")]);
        (tmp, workspace)
    }

    fn session(id: &str, cwd: &Path) -> Session {
        let json = serde_json::json!([{
            "id": id, "sessionId": format!("{id}-0000"), "name": format!("fm-{id}"),
            "cwd": cwd, "kind": "background", "state": "working",
        }]);
        parse(&json.to_string()).unwrap().remove(0)
    }

    fn repo_name<'a>(cwd: &Path, p: &'a Workspace) -> Option<&'a str> {
        repo_of(cwd, p).map(|r| r.name.as_str())
    }

    #[test]
    fn repo_path_and_below() {
        let (tmp, p) = layout();
        let plat = tmp.path().join("Plat");
        assert_eq!(repo_name(&plat.join("Alpha"), &p), Some("Alpha"));
        assert_eq!(repo_name(&plat.join("Alpha").join("src"), &p), Some("Alpha"));
        assert_eq!(repo_name(&plat.join("Alpha.Ext"), &p), Some("Alpha.Ext"));
        assert_eq!(repo_name(&plat.join("Alpha").join("not-created-yet"), &p), Some("Alpha"));
    }

    /// m5/26: a workspace whose root is a repo too: its children keep their sessions, the rest of the
    /// root is the root repo's.
    #[test]
    fn a_root_repo_takes_what_no_child_repo_does() {
        let (tmp, mut p) = layout();
        let plat = tmp.path().join("Plat");
        p.repos.insert(0, Repo { name: "Plat".into(), path: plat.clone(), ..p.repos[0].clone() });
        assert_eq!(repo_name(&plat, &p), Some("Plat"));
        assert_eq!(repo_name(&plat.join("docs"), &p), Some("Plat"));
        assert_eq!(repo_name(&plat.join("Alpha").join("src"), &p), Some("Alpha"));
        assert_eq!(repo_name(&plat.join("Alpha.Ext"), &p), Some("Alpha.Ext"));
        assert_eq!(repo_name(&tmp.path().join("Elsewhere"), &p), None);
    }

    #[test]
    fn worktree_maps_to_its_repo() {
        let (tmp, p) = layout();
        let wts = tmp.path().join("Plat").join("Alpha.worktrees");
        assert_eq!(repo_name(&wts.join("feat"), &p), Some("Alpha"), "relative gitdir");
        assert_eq!(repo_name(&wts.join("feat").join("deep"), &p), Some("Alpha"), "below a worktree");
        assert_eq!(repo_name(&wts.join("abs"), &p), Some("Alpha"), "absolute gitdir");
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
    fn outside_every_repo_is_unassigned() {
        let (tmp, p) = layout();
        assert_eq!(repo_name(&tmp.path().join("Elsewhere"), &p), None);
        assert_eq!(repo_name(&tmp.path().join("Plat"), &p), None, "the workspace root itself");
        assert_eq!(repo_name(&tmp.path().join("Plat").join("Alpha.worktrees"), &p), None);
        assert_eq!(repo_name(&tmp.path().join("Plat").join("Alph"), &p), None);
    }

    #[cfg(windows)]
    #[test]
    fn case_and_slashes_do_not_matter_on_windows() {
        let (tmp, p) = layout();
        let raw = tmp.path().join("Plat").join("Alpha").join("src").to_string_lossy().into_owned();
        let shouted = PathBuf::from(raw.to_uppercase().replace('\\', "/") + "/");
        assert_eq!(repo_name(&shouted, &p), Some("Alpha"));
        let verbatim = PathBuf::from(format!(r"\\?\{}", tmp.path().join("plat").join("alpha.ext").display()));
        assert_eq!(repo_name(&verbatim, &p), Some("Alpha.Ext"));
    }

    #[test]
    fn assign_groups_by_repo_and_keeps_the_rest() {
        let (tmp, p) = layout();
        let plat = tmp.path().join("Plat");
        let sessions = vec![
            session("a1", &plat.join("Alpha")),
            session("a2", &plat.join("Alpha.worktrees").join("feat")),
            session("e1", &plat.join("Alpha.Ext")),
            session("x1", &tmp.path().join("Elsewhere")),
        ];
        let a = assign(&sessions, &p);
        let ids = |name: &str| a.by_repo[name].iter().map(|s| s.id.as_str()).collect::<Vec<_>>();
        assert_eq!(ids("Alpha"), ["a1", "a2"]);
        assert_eq!(ids("Alpha.Ext"), ["e1"]);
        assert_eq!(a.unassigned.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["x1"]);
    }
}
