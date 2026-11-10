//! GitMan's layout per repository, `<home>/gitman/layout.json`, keyed by the main checkout; its one reader and writer.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use futures::FutureExt as _;
use futures::future::Shared;
use gpui::{App, Global, Task};
use serde::{Deserialize, Serialize};
use supraman_core::paths::{main_checkout, path_key};
use supraman_core::store::{JsonStore, Save};

use super::GitSizes;

const LOST: &str = "Git tabs open at their default layout, and none is kept until the file is fixed or removed";

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct File {
    /// The layout last kept in any repo, without its hidden and soloed refs, collapsed folders and section heights: a new repo's start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) last: Option<GitSizes>,
    #[serde(default)]
    pub(crate) repos: BTreeMap<String, GitSizes>,
    /// The graph's columns the header menu hides, in every repo.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) hidden_columns: Vec<String>,
}

/// What a Git tab opens at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Opening {
    pub(crate) sizes: GitSizes,
    /// The graph column's width is the repo's own, not one to fit to the lanes.
    pub(crate) own_graph: bool,
    /// A saved tab's sizes taken in: the repo's entry from now on.
    pub(crate) adopt: bool,
}

impl File {
    /// The repo's entry, else `saved` (a tab's own sizes), else the last layout used.
    pub(crate) fn opening(&self, key: &str, saved: &GitSizes) -> Opening {
        if let Some(own) = self.repos.get(key) {
            return Opening { sizes: own.clone(), own_graph: has_graph(own), adopt: false };
        }
        if *saved != GitSizes::default() {
            return Opening { sizes: saved.clone(), own_graph: has_graph(saved), adopt: true };
        }
        Opening { sizes: self.last.clone().unwrap_or_default(), own_graph: false, adopt: false }
    }

    /// Keeps `sizes` as `key`'s and as the last used; whether anything changed.
    pub(crate) fn remember(&mut self, key: &str, sizes: &GitSizes) -> bool {
        let last = GitSizes { hidden: Vec::new(), collapsed: Vec::new(), solo: Vec::new(), sections: Vec::new(), ..sizes.clone() };
        let changed = self.repos.get(key) != Some(sizes) || self.last.as_ref() != Some(&last);
        self.repos.insert(key.to_string(), sizes.clone());
        self.last = Some(last);
        changed
    }
}

fn has_graph(sizes: &GitSizes) -> bool {
    sizes.widths.get(1).is_some_and(|w| *w > 0)
}

pub(crate) fn path(home: &Path) -> PathBuf {
    home.join("gitman").join("layout.json")
}

/// The repo's key: its main checkout's, so its worktrees share one layout. Reads the `.git` walk: off the UI thread.
pub(crate) fn key(folder: &Path) -> String {
    path_key(&main_checkout(folder).unwrap_or_else(|| folder.to_path_buf()))
}

/// An unreadable file is kept as it is: the store lives in memory until the next start.
pub(crate) fn load(home: Option<&Path>) -> (JsonStore<File>, Option<String>) {
    JsonStore::load_kept(home.map(path), LOST)
}

pub(crate) type Ready = Shared<Task<()>>;

struct Layouts {
    store: Option<JsonStore<File>>,
    ready: Ready,
    notice: Option<String>,
}

impl Global for Layouts {}

fn layouts(cx: &mut App) -> Option<&mut Layouts> {
    if cx.has_global::<Layouts>() { Some(cx.global_mut::<Layouts>()) } else { None }
}

/// Done once the file is read; the first call reads it on the background executor.
pub(crate) fn ready(home: Option<PathBuf>, cx: &mut App) -> Ready {
    if let Some(layouts) = cx.try_global::<Layouts>() {
        return layouts.ready.clone();
    }
    let read = cx.background_executor().spawn(async move { load(home.as_deref()) });
    let ready = cx
        .spawn(async move |cx| {
            let (store, notice) = read.await;
            cx.update(|cx| {
                let layouts = cx.global_mut::<Layouts>();
                layouts.store = Some(store);
                layouts.notice = notice;
            });
        })
        .shared();
    cx.set_global(Layouts { store: None, ready: ready.clone(), notice: None });
    ready
}

/// What `key` opens at, once [`ready`]; the save of a saved tab's sizes taken in.
pub(crate) fn opening(key: &str, saved: &GitSizes, cx: &mut App) -> (Opening, Option<Save>) {
    let Some(store) = layouts(cx).and_then(|l| l.store.as_mut()) else {
        return (File::default().opening(key, saved), None);
    };
    let opening = store.get().opening(key, saved);
    let save = if opening.adopt { store.edit(|f| f.remember(key, &opening.sizes)) } else { None };
    (opening, save)
}

pub(crate) fn remember(key: &str, sizes: &GitSizes, cx: &mut App) -> Option<Save> {
    layouts(cx)?.store.as_mut()?.edit(|f| f.remember(key, sizes))
}

/// The graph's columns hidden, once [`ready`].
pub(crate) fn hidden_columns(cx: &mut App) -> Vec<String> {
    layouts(cx).and_then(|l| l.store.as_ref()).map(|s| s.get().hidden_columns.clone()).unwrap_or_default()
}

pub(crate) fn hide_columns(hidden: Vec<String>, cx: &mut App) -> Option<Save> {
    layouts(cx)?.store.as_mut()?.edit(|f| {
        let changed = f.hidden_columns != hidden;
        f.hidden_columns = hidden;
        changed
    })
}

pub(crate) fn notice(cx: &App) -> Option<String> {
    cx.try_global::<Layouts>()?.notice.clone()
}

pub(crate) fn set_notice(notice: Option<String>, cx: &mut App) {
    if let Some(layouts) = layouts(cx) {
        layouts.notice = notice;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sizes(graph: u32) -> GitSizes {
        GitSizes {
            widths: vec![120, graph, 300, 150, 90],
            panel: Some(400),
            message: Some(120),
            refs: Some(200),
            refs_shut: false,
            hidden: vec!["refs/heads/topic".into()],
            collapsed: vec!["local/topic".into()],
            solo: vec!["refs/remotes/origin/topic".into()],
            sections: Vec::new(),
        }
    }

    #[test]
    fn git_layout_round_trips_per_repo_and_a_new_repo_takes_the_last_widths_without_its_refs() {
        let home = tempfile::tempdir().unwrap();
        let (mut store, notice) = load(Some(home.path()));
        assert!(notice.is_none() && store.get().repos.is_empty());
        let (a, b) = (sizes(80), GitSizes { widths: vec![100, 60, 250, 140, 80], refs_shut: true, hidden: Vec::new(), collapsed: Vec::new(), solo: Vec::new(), ..sizes(0) });
        store.edit(|f| f.remember("p:/a", &a)).expect("a change").write().unwrap();
        store.edit(|f| f.remember("p:/b", &b)).expect("a change").write().unwrap();
        assert!(store.edit(|f| f.remember("p:/b", &b)).is_none(), "no change, no save");
        let (back, _) = load(Some(home.path()));
        let file = back.get();
        assert_eq!(file.opening("p:/a", &GitSizes::default()), Opening { sizes: a.clone(), own_graph: true, adopt: false });
        assert_eq!(file.opening("p:/b", &a).sizes, b, "the repo's entry wins over a saved tab's sizes");
        let new = file.opening("p:/c", &GitSizes::default());
        assert_eq!(new, Opening { sizes: b.clone(), own_graph: false, adopt: false }, "the last widths, its graph still fitted");
        let mut file = file.clone();
        file.remember("p:/a", &a);
        let fresh = file.opening("p:/c", &GitSizes::default()).sizes;
        assert!(fresh.hidden.is_empty() && fresh.collapsed.is_empty() && fresh.solo.is_empty() && fresh.widths == a.widths, "{fresh:?}");
        assert!(home.path().join("gitman").join("layout.json").is_file());
    }

    #[test]
    fn git_layout_adopts_a_saved_tabs_sizes_once_when_the_repo_has_no_entry() {
        let saved = sizes(90);
        let opening = File::default().opening("p:/a", &saved);
        assert_eq!(opening, Opening { sizes: saved.clone(), own_graph: true, adopt: true });
        let mut file = File::default();
        assert!(file.remember("p:/a", &opening.sizes));
        assert!(!file.opening("p:/a", &GitSizes::default()).adopt);
    }

    #[test]
    fn git_layout_of_a_corrupt_file_is_a_notice_and_the_file_is_never_written_over() {
        let home = tempfile::tempdir().unwrap();
        let file = path(home.path());
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "{ not json").unwrap();
        let (mut store, notice) = load(Some(home.path()));
        assert!(notice.is_some_and(|n| n.contains("layout.json") && n.contains("default layout")));
        assert!(store.edit(|f| f.remember("p:/a", &sizes(80))).is_none(), "kept in memory only");
        assert_eq!(store.get().opening("p:/a", &GitSizes::default()).sizes, sizes(80));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "{ not json");
    }

    #[test]
    fn git_layout_keys_a_worktree_by_its_main_checkout() {
        let f = supraman_gitman::fixture::Fixture::copy();
        let worktree = f.temp.path().join("wt");
        assert!(worktree.join(".git").is_file(), "the fixture's linked worktree");
        assert_eq!(key(&worktree), key(&f.dir));
        assert_eq!(key(&f.dir), path_key(&f.dir));
    }
}
