//! The Git tab following its repository: the git dirs and the work dir watched with [`supraman_core::watch`],
//! each batch sorted into the one refresh it asks for (research §2 "Watching").

use std::collections::HashSet;
use std::path::{Component, Path};
use std::time::Duration;

use futures::channel::mpsc::{UnboundedReceiver, unbounded};
use supraman_core::paths::within;
use supraman_core::watch::{Batch, Watch, Watcher};
use supraman_gitman::write::{self, Settled};
use supraman_gitman::{Dirs, Repo};

/// How long the git dirs' and the work dir's watchers gather changes into one batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Debounces {
    pub git: Duration,
    pub work: Duration,
}

#[cfg_attr(test, allow(dead_code))]
pub const DEBOUNCES: Debounces = Debounces { git: Duration::from_millis(200), work: Duration::from_secs(2) };
/// After the window comes back to the front, the one full refresh waits this long.
pub const FOCUS_DELAY: Duration = Duration::from_millis(500);

/// What a batch of changes asks of the tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refresh {
    /// HEAD or a ref moved: the refs, the graph and the WIP.
    Graph,
    /// The index or the work dir changed: the WIP; `files` the changed ones from the top, `None` for any.
    Wip { files: Option<HashSet<String>> },
}

/// The watchers of one tab; dropping it stops them and closes the refreshes' channel.
pub struct Live {
    _watchers: Vec<Watcher>,
    _settled: Settled,
}

impl Live {
    #[cfg(test)]
    pub fn stopped(&self) -> Vec<supraman_core::watch::Stopped> {
        self._watchers.iter().map(Watcher::stopped).collect()
    }
}

/// Watches the repository of `repo`; blocking (it asks git where its dirs are). While GitMan's own writes run on
/// it the batches are dropped, and one graph refresh follows the last.
pub fn start(repo: &Repo, debounces: Debounces) -> Result<(Live, UnboundedReceiver<Refresh>), String> {
    let dirs = supraman_gitman::dirs(repo).map_err(|e| e.to_string())?;
    let (tx, rx) = unbounded();
    let quiet = write::quiet(&dirs);
    let settled = {
        let tx = tx.clone();
        quiet.on_settled(move || {
            tx.unbounded_send(Refresh::Graph).ok();
        })
    };
    let mut git_dirs = vec![Watch::tree(&dirs.common_dir)];
    if !within(&dirs.git_dir, &dirs.common_dir) {
        git_dirs.push(Watch::tree(&dirs.git_dir));
    }
    let git = {
        let (tx, dirs, quiet) = (tx.clone(), dirs.clone(), quiet.clone());
        Watcher::new(&git_dirs, debounces.git, move |batch| {
            if quiet.paused() {
                return;
            }
            if let Some(r) = git_refresh(&dirs, &batch) {
                tx.unbounded_send(r).ok();
            }
        })
    };
    let work = {
        let top = Repo::new(repo.git().clone(), &dirs.top);
        Watcher::new(&[Watch::tree(&dirs.top)], debounces.work, move |batch| {
            if quiet.paused() {
                return;
            }
            if let Some(r) = work_refresh(&top, &dirs, &batch) {
                tx.unbounded_send(r).ok();
            }
        })
    };
    let watchers = vec![git.map_err(|e| e.to_string())?, work.map_err(|e| e.to_string())?];
    Ok((Live { _watchers: watchers, _settled: settled }, rx))
}

/// `path` below `base`, `/`-separated; `None` outside it.
fn below(base: &Path, path: &Path) -> Option<String> {
    let rest = path.strip_prefix(base).ok()?;
    Some(rest.components().filter_map(|c| match c {
        Component::Normal(n) => Some(n.to_string_lossy()),
        _ => None,
    })
    .collect::<Vec<_>>()
    .join("/"))
}

/// The refresh one change in a git dir asks for; nothing for objects, logs of refs and lock files.
pub fn git_class(dirs: &Dirs, path: &Path) -> Option<Refresh> {
    if path.extension().is_some_and(|e| e == "lock") {
        return None;
    }
    let operation = |p: &str| matches!(p, "MERGE_HEAD" | "CHERRY_PICK_HEAD" | "REVERT_HEAD") || p.starts_with("rebase-merge") || p.starts_with("rebase-apply");
    match below(&dirs.git_dir, path).as_deref() {
        Some("HEAD" | "logs/HEAD") => return Some(Refresh::Graph),
        Some("index") => return Some(Refresh::Wip { files: None }),
        Some(p) if operation(p) => return Some(Refresh::Wip { files: None }),
        _ => {}
    }
    let common = below(&dirs.common_dir, path)?;
    let parts: Vec<&str> = common.split('/').collect();
    match parts.as_slice() {
        ["HEAD" | "packed-refs"] | ["logs", "HEAD"] | ["refs", ..] | ["worktrees", _, "HEAD"] => Some(Refresh::Graph),
        _ => None,
    }
}

/// The batch's refresh: the graph's when any change asks for it, it covers the WIP too.
pub fn git_refresh(dirs: &Dirs, batch: &Batch) -> Option<Refresh> {
    if batch.rescan {
        return Some(Refresh::Graph);
    }
    let classes: Vec<Refresh> = batch.paths.iter().filter_map(|p| git_class(dirs, p)).collect();
    if classes.contains(&Refresh::Graph) { Some(Refresh::Graph) } else { classes.into_iter().next() }
}

/// The work dir's changed files from the top, without the git dirs inside it.
pub fn work_files(dirs: &Dirs, batch: &Batch) -> Vec<String> {
    let in_git = |p: &Path| p.components().any(|c| c.as_os_str() == ".git") || within(p, &dirs.git_dir) || within(p, &dirs.common_dir);
    batch.paths.iter().filter(|p| !in_git(p)).filter_map(|p| below(&dirs.top, p)).filter(|p| !p.is_empty()).collect()
}

/// The WIP refresh of the files git does not ignore; ignored-file noise (`target/`) asks for none. Runs git on
/// the watcher's thread.
fn work_refresh(top: &Repo, dirs: &Dirs, batch: &Batch) -> Option<Refresh> {
    if batch.rescan {
        return Some(Refresh::Wip { files: None });
    }
    let files = work_files(dirs, batch);
    let ignored = supraman_gitman::ignored(top, &files).unwrap_or_default();
    let files: HashSet<String> = files.into_iter().filter(|f| !ignored.contains(f)).collect();
    (!files.is_empty()).then_some(Refresh::Wip { files: Some(files) })
}
