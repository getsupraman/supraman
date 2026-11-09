//! The other worktrees' work in progress of a schedule's repository, read when a tab opens or becomes the active one,
//! at each fetch tick and on [`refresh`]; never on a timer of its own, never by a watcher. A read the repository's
//! locks hold off waits for the next of these, as a fetch does.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{App, AsyncApp, Context, EntityId, Entity};
use supraman_core::paths::path_key;
use supraman_gitman::WorktreeWip;
use supraman_gitman::write::{LockSet, Writer};
use supraman_ui::nudge::Nudged;

use super::{Background, IDLE, Step};
use crate::git_view::GitView;

/// The last read of a repository's worktrees; observe it (`cx.observe`) to hear of the next.
#[derive(Default)]
pub(crate) struct Wips {
    read: Option<Result<Arc<[WorktreeWip]>, String>>,
    reading: bool,
    /// The last read the repository's locks held off.
    skipped: bool,
    #[cfg(test)]
    pub(crate) reads: usize,
}

impl Wips {
    /// Every worktree but the one at `top`, as last read; none before the first read.
    pub(crate) fn others(&self, top: &Path) -> Vec<WorktreeWip> {
        let key = path_key(top);
        match &self.read {
            Some(Ok(all)) => all.iter().filter(|w| path_key(w.path()) != key).cloned().collect(),
            _ => Vec::new(),
        }
    }

    /// The worktree at `path`, as last read.
    pub(crate) fn get(&self, path: &Path) -> Option<&WorktreeWip> {
        let key = path_key(path);
        match &self.read {
            Some(Ok(all)) => all.iter().find(|w| path_key(w.path()) == key),
            _ => None,
        }
    }

    /// Whether a read is running now.
    pub(crate) fn reading(&self) -> bool {
        self.reading
    }

    /// Whether a read has landed.
    pub(crate) fn landed(&self) -> bool {
        self.read.is_some()
    }

    /// Why the last read failed as a whole.
    pub(crate) fn error(&self) -> Option<&str> {
        self.read.as_ref()?.as_ref().err().map(String::as_str)
    }
}

/// The [`Wips`] of the tab `id`'s repository.
pub(crate) fn wips(id: EntityId, cx: &App) -> Option<Entity<Wips>> {
    let bg = cx.try_global::<Background>()?;
    bg.repos.values().find(|s| s.tabs.iter().any(|(t, _)| *t == id)).map(|s| s.wips.clone())
}

/// The tab `id`'s repository reads its worktrees again now.
pub(crate) fn refresh(id: EntityId, cx: &App) {
    let Some(bg) = cx.try_global::<Background>() else { return };
    for schedule in bg.repos.values().filter(|s| s.tabs.iter().any(|(t, _)| *t == id)) {
        schedule.wip.wake();
    }
}

impl GitView {
    /// The other worktrees' work in progress, as last read.
    pub(crate) fn other_wips(&self, cx: &Context<Self>) -> Vec<WorktreeWip> {
        let Some(top) = self.remote.read(cx).writer().map(|w| w.dirs().top.clone()) else { return Vec::new() };
        wips(cx.entity_id(), cx).map(|w| w.read(cx).others(&top)).unwrap_or_default()
    }
}

/// The tab `id`'s write ended: a read its repository's locks held off runs now.
pub(crate) fn after_write(id: EntityId, cx: &App) {
    let Some(bg) = cx.try_global::<Background>() else { return };
    for schedule in bg.repos.values().filter(|s| s.tabs.iter().any(|(t, _)| *t == id)) {
        if schedule.wips.read(cx).skipped {
            schedule.wip.wake();
        }
    }
}

/// A writer of the schedule `key`, and the worktrees to leave out: the one every tab shows, none when its tabs show
/// different ones.
fn job(key: &str, cx: &App) -> Step<(Writer, Vec<PathBuf>)> {
    let Some(bg) = cx.try_global::<Background>() else { return Step::End };
    let views = bg.views(key);
    if views.is_empty() {
        return Step::End;
    }
    if !bg.worktree_wips() {
        return Step::Skip;
    }
    let writers: Vec<_> = views.iter().filter_map(|v| v.read(cx).remote.read(cx).writer().cloned()).collect();
    let Some(first) = writers.first() else { return Step::Skip };
    let top = &first.dirs().top;
    let shown = if writers.iter().all(|w| path_key(&w.dirs().top) == path_key(top)) { vec![top.clone()] } else { Vec::new() };
    Step::Run((first.clone(), shown))
}

pub(super) async fn run_loop(key: String, mut nudged: Nudged, cx: &mut AsyncApp) {
    let executor = cx.background_executor().clone();
    loop {
        if !nudged.wait(&executor, IDLE).await {
            continue;
        }
        let (writer, shown) = match cx.update(|cx| job(&key, cx)) {
            Step::End => return,
            Step::Skip => continue,
            Step::Run(job) => job,
        };
        cx.update(|cx| reading(&key, true, cx));
        let read = executor
            .spawn(async move {
                let _index = writer.scheduler().try_acquire(LockSet::refresh())?;
                Some(supraman_gitman::wips(writer.repo(), &shown))
            })
            .await;
        let Some(read) = read else {
            cx.update(|cx| reading(&key, false, cx));
            continue;
        };
        cx.update(|cx| read_into(&key, read.map(Arc::from).map_err(|e| e.to_string()), cx));
    }
}

/// A read started, or ended with nothing read (`false`: the locks held it off).
fn reading(key: &str, on: bool, cx: &mut App) {
    let Some(schedule) = cx.try_global::<Background>().and_then(|bg| bg.repos.get(key)) else { return };
    let (entity, views) = (schedule.wips.clone(), cx.global::<Background>().views(key));
    entity.update(cx, |w, cx| {
        w.reading = on;
        if !on {
            w.skipped = true;
        }
        cx.notify();
    });
    for view in views {
        view.update(cx, |v, cx: &mut Context<GitView>| v.hand_wips(cx));
    }
}

fn read_into(key: &str, read: Result<Arc<[WorktreeWip]>, String>, cx: &mut App) {
    if let Err(e) = &read {
        log::warn!("git tab: the other worktrees' work in progress: {e}");
    }
    let Some(schedule) = cx.try_global::<Background>().and_then(|bg| bg.repos.get(key)) else { return };
    let (entity, views) = (schedule.wips.clone(), cx.global::<Background>().views(key));
    entity.update(cx, |w, cx| {
        w.read = Some(read);
        (w.reading, w.skipped) = (false, false);
        #[cfg(test)]
        {
            w.reads += 1;
        }
        cx.notify();
    });
    for view in views {
        view.update(cx, |v, cx: &mut Context<GitView>| v.worktrees_read(cx));
    }
}
