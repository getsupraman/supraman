//! The tab's refreshes as the repository changes ([`live`]) or the window comes back to the front: read off
//! the UI thread, a late read dropped by its token, applied without losing the user's place.

use std::collections::HashSet;
use std::sync::Arc;

use futures::StreamExt as _;
use gpui::{AnyElement, AppContext as _, Context, Pixels, SharedString, Subscription, Task, Window, px};
use supraman_gitman::{FIRST_PAGE, Layout, Repo, Status};
use supraman_ui::hit::SNAP;
use supraman_ui::stall;

use super::live::{self, FOCUS_DELAY, Live, Refresh};
use super::load::{self, First, GraphRow, Kept, Wip};
use super::ref_panel::Walk;
use super::splice::Loaded;
use super::{GitView, Selected, graph};

/// What the tab keeps to follow its repository.
#[derive(Default)]
pub(crate) struct Follow {
    live: Option<Live>,
    starting: bool,
    graph_token: u64,
    wip_token: u64,
    /// Bumped by each graph refresh; a page asked for under an older one is dropped.
    pub(crate) generation: u64,
    activation: Option<Subscription>,
    focus: Option<Task<()>>,
    hidden: bool,
    /// What changed while the tab was hidden, read when it is shown.
    pub(super) pending: Option<Refresh>,
    /// The walk the loaded rows were read with.
    pub(super) walked: Walk,
    not_following: Option<String>,
    pub(super) graph_failed: Option<String>,
    wip_failed: Option<String>,
    #[cfg(test)]
    pub(crate) refreshes: usize,
}

impl Follow {
    #[cfg(test)]
    pub(crate) fn live(&self) -> Option<&Live> {
        self.live.as_ref()
    }
}

#[cfg(test)]
thread_local! {
    /// gpui's test scheduler refuses wake-ups from the watchers' threads unless a test allows parking.
    pub(crate) static FOLLOW: std::cell::Cell<Option<live::Debounces>> = const { std::cell::Cell::new(None) };
}

/// Whether a loaded tab starts following its repository, and with which debounces.
pub(super) fn follows() -> Option<live::Debounces> {
    #[cfg(test)]
    return FOLLOW.get();
    #[cfg(not(test))]
    Some(live::DEBOUNCES)
}

/// The row a refresh keeps in place: the selected commit while it is wholly in view, else the first commit row at or
/// below the top; how far below the top it starts, and where the view was.
struct Anchor {
    sha: SharedString,
    offset: Pixels,
    top: Pixels,
}

/// Two refreshes as one: the graph's covers the WIP's, and the changed files add up.
fn merge(a: Refresh, b: Refresh) -> Refresh {
    match (a, b) {
        (Refresh::Graph, _) | (_, Refresh::Graph) => Refresh::Graph,
        (Refresh::Wip { files: Some(mut a) }, Refresh::Wip { files: Some(b) }) => {
            a.extend(b);
            Refresh::Wip { files: Some(a) }
        }
        _ => Refresh::Wip { files: None },
    }
}

impl GitView {
    /// Starts the watchers of `repo` off the UI thread, once.
    pub(super) fn follow(&mut self, repo: Repo, debounces: live::Debounces, cx: &mut Context<Self>) {
        if self.follow.live.is_some() || self.follow.starting {
            return;
        }
        self.follow.starting = true;
        let started = cx.background_spawn(async move { live::start(&repo, debounces) });
        cx.spawn(async move |this, cx| {
            let mut rx = match started.await {
                Ok((live, rx)) => {
                    if this.update(cx, |v, _| v.follow.live = Some(live)).is_err() {
                        return;
                    }
                    rx
                }
                Err(why) => {
                    log::warn!("git tab: not following the repository: {why}");
                    this.update(cx, |v, cx| {
                        v.follow.not_following = Some(format!("Not following this repository's changes: {why}"));
                        cx.notify();
                    })
                    .ok();
                    return;
                }
            };
            while let Some(mut refresh) = rx.next().await {
                while let Ok(more) = rx.try_recv() {
                    refresh = merge(refresh, more);
                }
                if this.update(cx, |v, cx| v.live_refresh(refresh, cx)).is_err() {
                    return;
                }
            }
        })
        .detach();
    }

    /// A refresh `FOCUS_DELAY` after the window becomes active, from the first frame on, for a tab not following
    /// its repository.
    pub(super) fn watch_activation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.follow.activation.is_some() {
            return;
        }
        self.follow.activation = Some(cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.focus_regained(cx);
            }
        }));
    }

    pub(crate) fn focus_regained(&mut self, cx: &mut Context<Self>) {
        self.follow.focus = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(FOCUS_DELAY).await;
            this.update(cx, |v, cx| {
                if v.follow.live.is_none() {
                    v.live_refresh(Refresh::Graph, cx);
                }
            })
            .ok();
        }));
    }

    /// Whether the tab is the one shown; a hidden tab keeps what changed and reads it when shown.
    pub fn set_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        if self.follow.hidden != shown {
            return;
        }
        self.follow.hidden = !shown;
        if shown {
            super::background::shown(cx.entity_id(), cx);
        }
        if let Some(refresh) = self.follow.pending.take().filter(|_| shown) {
            self.live_refresh(refresh, cx);
        }
    }

    pub(crate) fn live_refresh(&mut self, refresh: Refresh, cx: &mut Context<Self>) {
        if self.follow.hidden {
            self.follow.pending = Some(match self.follow.pending.take() {
                Some(pending) => merge(pending, refresh),
                None => refresh,
            });
            return;
        }
        match refresh {
            Refresh::Graph => self.refresh_graph(None, cx),
            Refresh::Wip { files } => self.refresh_wip(files, cx),
        }
    }

    /// After a write of the tab's own: the watchers refresh once it settles, else this does.
    pub(super) fn refresh_after_write(&mut self, cx: &mut Context<Self>) {
        if self.follow.live.is_none() {
            self.refresh_graph(None, cx);
        }
    }

    /// The refs, the WIP and the first page read again, the commits loaded below it kept while the walk below is
    /// the same; `files` the work dir's changed ones.
    pub(crate) fn refresh_graph(&mut self, files: Option<HashSet<String>>, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else { return };
        self.follow.graph_token += 1;
        self.follow.wip_token += 1;
        let token = self.follow.graph_token;
        self.history = None;
        let walk = self.ref_panel.read(cx).walk().clone();
        let loaded = (self.commits > FIRST_PAGE && walk == self.follow.walked).then(|| self.loaded());
        self.worktree_rows = self.wanted_worktrees(cx);
        let worktrees = self.worktree_rows.clone();
        let read = cx.background_spawn(async move { load::read(repo, &walk, loaded.as_ref(), &worktrees) });
        cx.spawn(async move |this, cx| {
            let read = read.await;
            this.update(cx, |v, cx| v.apply_refresh(token, read, files, cx)).ok();
        })
        .detach();
    }

    fn apply_refresh(&mut self, token: u64, read: Result<First, String>, files: Option<HashSet<String>>, cx: &mut Context<Self>) {
        if token != self.follow.graph_token {
            return;
        }
        let first = match read {
            Ok(first) => first,
            Err(why) => {
                self.follow.graph_failed = Some(format!("Could not read the repository again: {why}"));
                return cx.notify();
            }
        };
        let _stall = stall::section("git tab: refresh");
        self.follow.graph_failed = None;
        self.follow.wip_failed = None;
        let (selected, anchor) = (self.selected(), self.anchor());
        let (mut rows, mut more, mut layout) = (first.page.rows, first.page.more, first.page.layout);
        if let Some((kept, kept_layout, kept_more)) = first.kept.and_then(|k| self.take_kept(k)) {
            rows.extend(kept);
            (more, layout) = (kept_more, kept_layout);
        }
        self.follow.generation += 1;
        self.follow.walked = first.walk;
        self.paging = false;
        self.page_error = None;
        self.commits = rows.iter().filter(|r| r.commit()).count();
        self.rows = rows;
        self.more = more;
        self.marks.reset(self.rows.len(), self.commits, &layout);
        self.layout = Some(layout);
        self.history = first.history;
        self.tips = first.tips.into();
        self.heads = first.heads;
        let before = std::mem::replace(&mut self.wip, first.wip);
        self.ref_panel.update(cx, |p, cx| p.set_built(first.refs, cx));
        self.hand_refs(&first.repo, first.all.clone(), &first.status, cx);
        self.measure_lanes();
        self.seek = None;
        match self.reveal_after.take() {
            Some(sha) => self.reveal_sha(sha, cx),
            None => self.keep_selection(selected, &first.status, cx),
        }
        self.select_conflict(before, cx);
        if let Some(anchor) = anchor {
            self.restore(anchor);
        }
        self.reload_diff(files.as_ref(), cx);
        self.refresh_undo(cx);
        #[cfg(test)]
        {
            self.follow.refreshes += 1;
        }
        cx.notify();
    }

    fn loaded(&self) -> Loaded {
        let commits = self.rows.iter().filter(|r| r.commit()).map(|r| (r.sha.clone(), r.parents.clone())).collect();
        Loaded { tips: self.tips.clone(), commits, more: self.more }
    }

    /// The rows `kept` lays out again, unless they changed since they were asked for.
    fn take_kept(&mut self, kept: Kept) -> Option<(Vec<GraphRow>, Layout, bool)> {
        let shas: Vec<&str> = kept.rows.iter().filter_map(|r| r.sha.as_deref()).collect();
        let loaded: Vec<&GraphRow> = self.rows.iter().filter(|r| r.commit()).skip(kept.from).take(shas.len()).collect();
        if loaded.len() < shas.len() || loaded.iter().zip(&shas).any(|(g, sha)| g.sha.as_ref() != *sha) {
            return None;
        }
        let mut old = std::mem::take(&mut self.rows).into_iter().filter(GraphRow::commit).skip(kept.from);
        let rows = kept
            .rows
            .into_iter()
            .map(|row| match row.sha.is_some().then(|| old.next()).flatten() {
                Some(g) => GraphRow { row, ..g },
                None => load::graph_row(row, None),
            })
            .collect();
        Some((rows, kept.layout, kept.more))
    }

    /// The same commit selected at its new row; the WIP row while there is one; else nothing.
    fn keep_selection(&mut self, selected: Option<Selected>, status: &Arc<Status>, cx: &mut Context<Self>) {
        let row = match &selected {
            None => return,
            Some(Selected::Commit(sha)) => self.rows.iter().position(|r| r.sha == *sha),
            Some(Selected::Stash(stash)) => self.rows.iter().position(|r| r.stash().is_some_and(|s| s.sha == stash.sha)),
            Some(Selected::Wip) => self.rows.first().is_some_and(GraphRow::wip).then_some(0),
            Some(Selected::Worktree(path)) => self.rows.iter().position(|r| r.worktree() == Some(path.as_ref())),
        };
        self.table.select(row);
        match (row, selected) {
            (None, _) => self.selection_moved(cx),
            (Some(_), Some(Selected::Wip)) => {
                let colours = self.ref_colours();
                self.wip_panel.update(cx, |p, cx| {
                    p.set_ref_colours(colours);
                    p.refresh(status.clone(), cx)
                });
            }
            _ => {}
        }
    }

    fn anchor(&self) -> Option<Anchor> {
        let (top, row) = (self.table.scroll_top(), px(graph::ROW));
        let shown = |ix: usize| row * ix as f32 >= top - SNAP && self.table.viewport().is_some_and(|v| row * (ix + 1) as f32 <= top + v + SNAP);
        let selected = self.table.selected().filter(|ix| shown(*ix)).and_then(|ix| self.rows.get(ix).filter(|r| r.commit()).map(|r| (ix, r)));
        let first = (top / row).floor().max(0.) as usize;
        let (ix, r) = selected.or_else(|| self.rows.iter().enumerate().skip(first).find(|(_, r)| r.commit()))?;
        Some(Anchor { sha: r.sha.clone(), offset: row * ix as f32 - top, top })
    }

    /// Rows added above the anchor take the view to the top when it was there or the selected row stays wholly in
    /// view at the top; otherwise the anchor keeps its place.
    fn restore(&self, anchor: Anchor) {
        let Some(ix) = self.rows.iter().position(|r| r.sha == anchor.sha) else { return };
        let row = px(graph::ROW);
        let kept = row * ix as f32 - anchor.offset;
        let fits = |ix: usize| self.table.viewport().is_some_and(|v| row * (ix + 1) as f32 <= v + SNAP);
        let to_top = kept > anchor.top && (anchor.top < SNAP || self.table.selected().is_some_and(fits));
        self.table.set_scroll_top(if to_top { px(0.) } else { kept });
    }

    /// The status read again: the WIP row's counts and panel, or the graph when the row comes or goes.
    pub(crate) fn refresh_wip(&mut self, files: Option<HashSet<String>>, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else { return };
        self.follow.wip_token += 1;
        let token = self.follow.wip_token;
        let read = cx.background_spawn(async move { load::wip(&repo) });
        cx.spawn(async move |this, cx| {
            let read = read.await;
            this.update(cx, |v, cx| v.apply_wip(token, read, files, cx)).ok();
        })
        .detach();
    }

    fn apply_wip(&mut self, token: u64, read: Result<(Option<Wip>, Arc<Status>), String>, files: Option<HashSet<String>>, cx: &mut Context<Self>) {
        if token != self.follow.wip_token {
            return;
        }
        let (wip, status) = match read {
            Ok(read) => read,
            Err(why) => {
                self.follow.wip_failed = Some(format!("Could not read the working directory again: {why}"));
                return cx.notify();
            }
        };
        self.follow.wip_failed = None;
        if wip.is_some() != self.rows.first().is_some_and(GraphRow::wip) {
            return self.refresh_graph(files, cx);
        }
        let before = std::mem::replace(&mut self.wip, wip);
        if self.selected() == Some(Selected::Wip) {
            self.wip_panel.update(cx, |p, cx| p.refresh(status, cx));
        }
        self.select_conflict(before, cx);
        self.reload_diff(files.as_ref(), cx);
        cx.notify();
    }

    /// Why the tab may show a stale repository: its watchers did not start, or its last read failed.
    pub(super) fn follow_notice(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let f = &self.follow;
        let lines: Vec<&str> = [&f.not_following, &f.graph_failed, &f.wip_failed].into_iter().flatten().map(String::as_str).collect();
        if lines.is_empty() {
            return None;
        }
        Some(self.notice_banner("git-follow-notice-dismiss", lines.join("\n"), |this, _| {
            this.follow.not_following = None;
            this.follow.graph_failed = None;
            this.follow.wip_failed = None;
        }, cx))
    }

    /// The open working-tree diff read again when its file is among `files` (`None`: any may have changed).
    fn reload_diff(&mut self, files: Option<&HashSet<String>>, cx: &mut Context<Self>) {
        let (Some((view, _)), Some(repo)) = (&self.diff, self.repo.clone()) else { return };
        view.update(cx, |d, cx| {
            let changed = files.is_none_or(|f| f.contains(&d.target.path) || f.contains(d.target.old_path()));
            if d.target.side_word().is_some() && changed {
                d.reload(repo, cx);
            }
        });
    }
}
