//! The tab's refreshes as the repository changes ([`live`]) or the window comes back to the front: read off
//! the UI thread, a late read dropped by its token, applied without losing the user's place.

use std::collections::HashSet;

use futures::StreamExt as _;
use gpui::{AppContext as _, Context, Pixels, SharedString, Subscription, Task, Window, px};
use supraman_gitman::{FIRST_PAGE, Repo};
use supraman_ui::stall;

use super::live::{self, FOCUS_DELAY, Live, Refresh};
use super::load::{self, First, GraphRow, Wip};
use super::{GitView, Selected, graph};

/// What the tab keeps to follow its repository.
#[derive(Default)]
pub struct Follow {
    live: Option<Live>,
    starting: bool,
    graph_token: u64,
    wip_token: u64,
    /// Bumped by each graph refresh; a page asked for under an older one is dropped.
    pub generation: u64,
    activation: Option<Subscription>,
    focus: Option<Task<()>>,
    #[cfg(test)]
    pub refreshes: usize,
}

impl Follow {
    #[cfg(test)]
    pub fn live(&self) -> Option<&Live> {
        self.live.as_ref()
    }
}

#[cfg(test)]
thread_local! {
    /// gpui's test scheduler refuses wake-ups from the watchers' threads unless a test allows parking.
    pub static FOLLOW: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether a loaded tab starts following its repository.
pub(super) fn follows() -> bool {
    #[cfg(test)]
    return FOLLOW.get();
    #[cfg(not(test))]
    true
}

/// The first commit row at or below the top of the view, and how far below the top it starts.
struct Anchor {
    sha: SharedString,
    offset: Pixels,
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
    pub(super) fn follow(&mut self, repo: Repo, cx: &mut Context<Self>) {
        if self.follow.live.is_some() || self.follow.starting {
            return;
        }
        self.follow.starting = true;
        let started = cx.background_spawn(async move { live::start(&repo) });
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

    /// One full refresh `FOCUS_DELAY` after the window becomes active, from the first frame on.
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

    pub fn focus_regained(&mut self, cx: &mut Context<Self>) {
        self.follow.focus = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(FOCUS_DELAY).await;
            this.update(cx, |v, cx| v.refresh_graph(None, cx)).ok();
        }));
    }

    pub fn live_refresh(&mut self, refresh: Refresh, cx: &mut Context<Self>) {
        match refresh {
            Refresh::Graph => self.refresh_graph(None, cx),
            Refresh::Wip { files } => self.refresh_wip(files, cx),
        }
    }

    /// The refs, the WIP and as many commits as are loaded, read again; `files` the work dir's changed ones.
    pub fn refresh_graph(&mut self, files: Option<HashSet<String>>, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else { return };
        self.follow.graph_token += 1;
        self.follow.wip_token += 1;
        let token = self.follow.graph_token;
        let count = self.commits.max(FIRST_PAGE);
        let read = cx.background_spawn(async move { load::read(repo, count) });
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
        let Ok(first) = read else { return };
        let _stall = stall::section("git tab: refresh");
        let (selected, anchor) = (self.selected(), self.anchor());
        self.follow.generation += 1;
        self.paging = false;
        self.page_error = None;
        self.commits = first.page.rows.iter().filter(|r| !r.wip()).count();
        self.rows = first.page.rows;
        self.more = first.page.more;
        self.layout = Some(first.page.layout);
        self.tips = first.tips.into();
        self.wip = first.wip;
        self.keep_selection(selected, cx);
        if let Some(anchor) = anchor {
            self.restore(anchor);
        }
        self.reload_diff(files.as_ref(), cx);
        #[cfg(test)]
        {
            self.follow.refreshes += 1;
        }
        cx.notify();
    }

    /// The same commit selected at its new row; the WIP row while there is one; else nothing.
    fn keep_selection(&mut self, selected: Option<Selected>, cx: &mut Context<Self>) {
        let row = match &selected {
            None => return,
            Some(Selected::Commit(sha)) => self.rows.iter().position(|r| r.sha == *sha),
            Some(Selected::Wip) => self.rows.first().is_some_and(GraphRow::wip).then_some(0),
        };
        self.table.select(row);
        match (row, selected) {
            (None, _) => self.selection_moved(cx),
            (Some(_), Some(Selected::Wip)) => {
                self.wip_panel.update(cx, |p, cx| p.refresh(cx));
            }
            _ => {}
        }
    }

    fn anchor(&self) -> Option<Anchor> {
        let (top, row) = (self.table.scroll_top(), px(graph::ROW));
        let first = (top / row).floor().max(0.) as usize;
        let (ix, r) = self.rows.iter().enumerate().skip(first).find(|(_, r)| !r.wip())?;
        Some(Anchor { sha: r.sha.clone(), offset: row * ix as f32 - top })
    }

    fn restore(&self, anchor: Anchor) {
        if let Some(ix) = self.rows.iter().position(|r| r.sha == anchor.sha) {
            self.table.set_scroll_top(px(graph::ROW) * ix as f32 - anchor.offset);
        }
    }

    /// The status read again: the WIP row's counts and panel, or the graph when the row comes or goes.
    pub fn refresh_wip(&mut self, files: Option<HashSet<String>>, cx: &mut Context<Self>) {
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

    fn apply_wip(&mut self, token: u64, read: Result<Option<Wip>, String>, files: Option<HashSet<String>>, cx: &mut Context<Self>) {
        if token != self.follow.wip_token {
            return;
        }
        let Ok(wip) = read else { return };
        if wip.is_some() != self.rows.first().is_some_and(GraphRow::wip) {
            return self.refresh_graph(files, cx);
        }
        self.wip = wip;
        if self.selected() == Some(Selected::Wip) {
            self.wip_panel.update(cx, |p, cx| p.refresh(cx));
        }
        self.reload_diff(files.as_ref(), cx);
        cx.notify();
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
