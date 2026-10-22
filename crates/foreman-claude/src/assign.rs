use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use foreman_core::{Platform, Repo};

use crate::Session;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Assignment {
    /// Keyed by [`Repo::name`]; repos without sessions are absent.
    pub by_repo: BTreeMap<String, Vec<Session>>,
    pub unassigned: Vec<Session>,
}

pub fn assign(sessions: &[Session], platform: &Platform) -> Assignment {
    let mut out = Assignment::default();
    for s in sessions {
        match repo_of(&s.cwd, platform) {
            Some(repo) => out.by_repo.entry(repo.name.clone()).or_default().push(s.clone()),
            None => out.unassigned.push(s.clone()),
        }
    }
    out
}

/// The repo whose path is `cwd` or an ancestor of it (deepest wins), else the repo owning the
/// linked worktree `cwd` lies in, found through the worktree's `.git` file.
pub fn repo_of<'a>(cwd: &Path, platform: &'a Platform) -> Option<&'a Repo> {
    let by_prefix = |dir: &str| {
        platform
            .repos
            .iter()
            .filter(|r| is_within(dir, &norm(&r.path)))
            .max_by_key(|r| norm(&r.path).len())
    };
    let cwd_norm = norm(cwd);
    by_prefix(&cwd_norm).or_else(|| {
        let main = worktree_main(cwd)?;
        platform.repos.iter().find(|r| norm(&r.path) == main)
    })
}

/// Walks up from `dir` to the first `.git`; when it is a file with `gitdir: <repo>/.git/worktrees/<n>`,
/// returns `<repo>`, normalised. A `.git` directory means a plain repo, not a worktree.
fn worktree_main(dir: &Path) -> Option<String> {
    for ancestor in dir.ancestors() {
        let git = ancestor.join(".git");
        if git.is_dir() {
            return None;
        }
        if !git.is_file() {
            continue;
        }
        let text = fs::read_to_string(&git).ok()?;
        let gitdir = text.lines().find_map(|l| l.trim().strip_prefix("gitdir:"))?.trim();
        let mut gitdir = norm(&ancestor.join(gitdir));
        let at = gitdir.rfind("/.git/worktrees/")?;
        gitdir.truncate(at);
        return Some(gitdir);
    }
    None
}

fn is_within(dir: &str, root: &str) -> bool {
    dir == root || dir.strip_prefix(root).is_some_and(|rest| rest.starts_with('/'))
}

/// `/`-separated, no trailing `/`, `.`/`..` folded, no `\\?\`; lowercase on Windows.
fn norm(path: &Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    let s = s.strip_prefix("//?/").unwrap_or(&s);
    let mut parts: Vec<&str> = Vec::new();
    for (i, part) in s.split('/').enumerate() {
        match part {
            "" if i > 0 => {}
            "." => {}
            ".." if parts.len() > 1 => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    let joined = parts.join("/");
    if cfg!(windows) { joined.to_lowercase() } else { joined }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::parse;

    /// tmp/
    ///   Plat/Alpha/.git/          repo
    ///   Plat/Alpha/src/
    ///   Plat/Alpha.Ext/.git/      repo whose name shares a prefix with Alpha
    ///   Plat/Alpha.worktrees/feat/.git  file → ../../Alpha/.git/worktrees/feat
    ///   Plat/Alpha.worktrees/abs/.git   file → <abs Alpha>/.git/worktrees/abs
    ///   Elsewhere/
    fn layout() -> (tempfile::TempDir, Platform) {
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
        let platform = Platform::new("plat", &plat, vec![repo("Alpha"), repo("Alpha.Ext")]);
        (tmp, platform)
    }

    fn session(id: &str, cwd: &Path) -> Session {
        let json = serde_json::json!([{
            "id": id, "sessionId": format!("{id}-0000"), "name": format!("fm-{id}"),
            "cwd": cwd, "kind": "background", "state": "working",
        }]);
        parse(&json.to_string()).unwrap().remove(0)
    }

    fn repo_name<'a>(cwd: &Path, p: &'a Platform) -> Option<&'a str> {
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

    #[test]
    fn worktree_maps_to_its_repo() {
        let (tmp, p) = layout();
        let wts = tmp.path().join("Plat").join("Alpha.worktrees");
        assert_eq!(repo_name(&wts.join("feat"), &p), Some("Alpha"), "relative gitdir");
        assert_eq!(repo_name(&wts.join("feat").join("deep"), &p), Some("Alpha"), "below a worktree");
        assert_eq!(repo_name(&wts.join("abs"), &p), Some("Alpha"), "absolute gitdir");
    }

    #[test]
    fn outside_every_repo_is_unassigned() {
        let (tmp, p) = layout();
        assert_eq!(repo_name(&tmp.path().join("Elsewhere"), &p), None);
        assert_eq!(repo_name(&tmp.path().join("Plat"), &p), None, "the platform root itself");
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
