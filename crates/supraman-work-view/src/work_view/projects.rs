//! A Work tab's Workman projects: those bound to the repo of one of its folders (Workman's `project_repo`).

use std::path::{Path, PathBuf};

use supraman_supervisor::MintRepo;
use supraman_workman::{Overview, OverviewProject, OverviewRun, repo};

use super::state::closed;

pub type RepoOf = fn(&Path) -> Option<MintRepo>;

/// Each folder's repo keys; runs git, so never on the UI thread.
pub(crate) fn folder_keys(folders: &[PathBuf], repo_of: RepoOf) -> Vec<(&'static str, String)> {
    let mut keys = Vec::new();
    for found in folders.iter().filter_map(|f| repo_of(f)) {
        for key in repo::keys(found.remote.as_deref(), Some(&found.root)) {
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
    }
    keys
}

pub(crate) fn bound(overview: &Overview, keys: &[(&str, String)]) -> Vec<OverviewProject> {
    overview.projects.iter().filter(|p| p.archived_at.is_none() && repo::binds(&p.repos, keys)).cloned().collect()
}

pub(crate) fn active(project: &OverviewProject) -> impl Iterator<Item = &OverviewRun> {
    project.runs.iter().filter(|r| !closed(r.item.status))
}

pub(crate) fn closed_runs(project: &OverviewProject) -> impl Iterator<Item = &OverviewRun> {
    project.runs.iter().filter(|r| closed(r.item.status))
}

pub(crate) fn no_project_message(target: &super::Target, keys: &[(&str, String)]) -> String {
    let name = target.name();
    let (of, which) = match target {
        super::Target::Folder(_) => (format!("{name} is not a git repo"), format!("{name}'s repo")),
        super::Target::Group(_) => (format!("None of {name}'s folders is a git repo"), format!("the repos of {name}'s folders")),
    };
    if keys.is_empty() {
        return format!("{of}, so no Workman project is bound to it.");
    }
    format!("No Workman project is bound to {which}. Bind one with work_update(ref:\"<project>\", repo:{{…}}).")
}

#[cfg(test)]
mod tests {
    use supraman_workman::{ItemRef, ItemStatus, ProjectRepo};

    use super::*;

    fn project(id: &str, repos: &[(&str, &str)]) -> OverviewProject {
        OverviewProject {
            item: ItemRef { id: id.into(), ..Default::default() },
            repos: repos.iter().map(|(k, v)| ProjectRepo { kind: k.to_string(), key: v.to_string() }).collect(),
            ..Default::default()
        }
    }

    fn repo_of(dir: &Path) -> Option<MintRepo> {
        match dir.to_str()? {
            "a" => Some(MintRepo { root: r"P:\W\Alpha".into(), remote: Some("git@github.com:Acme/alpha.git".into()) }),
            "a-wt" => Some(MintRepo { root: r"P:\W\Alpha".into(), remote: Some("https://github.com/acme/alpha".into()) }),
            "b" => Some(MintRepo { root: r"P:\W\Beta".into(), remote: None }),
            _ => None,
        }
    }

    #[test]
    fn a_folder_binds_the_project_of_its_repos_remote_or_root() {
        let keys = folder_keys(&["a".into(), "a-wt".into(), "b".into(), "plain".into()], repo_of);
        assert_eq!(keys, [("remote", "github.com/acme/alpha".to_string()), ("root", "p:/w/alpha".into()), ("root", "p:/w/beta".into())], "a worktree adds nothing new");
        let mut archived = project("old", &[("root", "p:/w/beta")]);
        archived.archived_at = Some("2026-11-01T00:00:00Z".into());
        let overview = Overview {
            projects: vec![project("pa", &[("remote", "github.com/acme/alpha")]), project("other", &[("root", "p:/w/gamma")]), archived, project("pb", &[("root", "p:/w/beta")])],
            ..Default::default()
        };
        let ids: Vec<String> = bound(&overview, &keys).into_iter().map(|p| p.item.id).collect();
        assert_eq!(ids, ["pa", "pb"]);
        assert!(no_project_message(&super::super::Target::Group("w".into()), &[]).contains("None of w's folders is a git repo"));
        assert!(no_project_message(&super::super::Target::Folder("C:/a/shop".into()), &[]).starts_with("shop is not a git repo"));
    }

    #[test]
    fn closed_runs_are_apart() {
        let run = |id: &str, status| OverviewRun { item: ItemRef { id: id.into(), status, ..Default::default() }, ..Default::default() };
        let p = OverviewProject { runs: vec![run("1", ItemStatus::Done), run("2", ItemStatus::Open), run("3", ItemStatus::Cancelled)], ..Default::default() };
        assert_eq!(active(&p).map(|r| r.item.id.as_str()).collect::<Vec<_>>(), ["2"]);
        assert_eq!(closed_runs(&p).count(), 2);
    }
}
