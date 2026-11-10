//! The graph's rows of the live sessions working in the tab's worktree, laid out without a git read, and their panel.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{AnyElement, App, ClickEvent, Context, Entity, EventEmitter, ScrollHandle, SharedString, div, prelude::*};
use supraman_gitman::Row;
use supraman_ui::details::{self, Page, Section};
use supraman_ui::hit::{HitTargets, hit_target};
use supraman_ui::marks::{self, Activity, Mark};
use supraman_ui::tooltip::Tooltip;
use supraman_ui::{button, placeholder};

use super::colours::Palette;
use super::load::{self, GraphRow};
use super::write_gate::capitalised;
use super::{GitView, Selected, background, cells, graph};
use crate::occupancy::{Live, LiveSession};

/// Attach asked for this session, in a terminal tab of the fleet's.
pub struct AttachAgent(pub LiveSession);

/// Why Attach is dimmed.
pub(crate) const NOT_ATTACHABLE: &str = "Only a running background session can be attached";

/// The longest the live watch sleeps before it looks whether the tab is still open.
const WAKE_AT_LEAST: Duration = Duration::from_secs(60);

#[derive(Default)]
pub(super) struct Agents {
    live: Option<Live>,
    /// The sessions working in the worktree, by name; the rows from `at` while `base` holds the row under them.
    shown: Vec<LiveSession>,
    at: usize,
    stacked: usize,
    base: Option<Row>,
    /// The sessions working in the repo's other worktrees, as last seen.
    elsewhere: Vec<(String, PathBuf)>,
    scroll: ScrollHandle,
}

impl Agents {
    /// The rows from `rows` on were dropped: a stack among them went with them.
    pub(super) fn dropped_from(&mut self, rows: usize) {
        if self.base.is_some() && self.at >= rows {
            self.base = None;
        }
    }

    /// The rows were read afresh: nothing is stacked on them.
    pub(super) fn reset(&mut self) {
        self.base = None;
    }
}

impl EventEmitter<AttachAgent> for GitView {}

impl GitView {
    /// [`GitView::open_with`] of `live`, whose sessions working in the worktree the graph shows as agent rows.
    pub fn open_following(folder: PathBuf, home: Option<PathBuf>, hits: HitTargets, live: Live, cx: &mut App) -> Entity<GitView> {
        let view = GitView::open_with(folder, home, hits, Arc::new(live.clone()), cx);
        view.update(cx, |v, cx| v.follow_live(live, cx));
        view
    }

    fn follow_live(&mut self, live: Live, cx: &mut Context<Self>) {
        let mut woken = live.watch();
        self.agents.live = Some(live);
        cx.spawn(async move |this, cx| {
            let executor = cx.background_executor().clone();
            loop {
                let changed = woken.wait(&executor, WAKE_AT_LEAST).await;
                let open = this.update(cx, |v, cx| {
                    if changed {
                        v.live_changed(cx);
                    }
                });
                if open.is_err() {
                    return;
                }
            }
        })
        .detach();
    }

    fn live_changed(&mut self, cx: &mut Context<Self>) {
        self.relay_agents(cx);
        let elsewhere = self.sessions_elsewhere(cx);
        if elsewhere != self.agents.elsewhere {
            self.agents.elsewhere = elsewhere;
            background::refresh_wips(cx.entity_id(), cx);
        }
        cx.notify();
    }

    fn sessions_elsewhere(&self, cx: &Context<Self>) -> Vec<(String, PathBuf)> {
        let Some(live) = &self.agents.live else { return Vec::new() };
        self.other_wips(cx).iter().flat_map(|w| live.working_in(w.path()).into_iter().map(|s| (s.name, w.path().to_path_buf()))).collect()
    }

    fn agents_here(&self) -> Vec<LiveSession> {
        let top = self.repo.as_ref().and_then(|r| r.known_dirs()).map(|d| d.top.as_path());
        match (&self.agents.live, top) {
            (Some(live), Some(top)) => live.working_in(top),
            _ => Vec::new(),
        }
    }

    /// The agent rows laid out again for the sessions working here now; the selection kept on its row.
    pub(super) fn relay_agents(&mut self, cx: &mut Context<Self>) {
        let sessions = self.agents_here();
        let names = |s: &[LiveSession]| s.iter().map(|s| s.name.clone()).collect::<Vec<_>>();
        let unchanged = if self.agents.base.is_some() { names(&sessions) == names(&self.agents.shown) } else { sessions.is_empty() };
        if unchanged {
            self.agents.shown = sessions;
            return;
        }
        let selected = self.selected(cx);
        let ix = self.unstack_agents(self.table.selected(), cx);
        let ix = self.stack_agents(sessions, ix, cx);
        let ix = match &selected {
            Some(Selected::Agent(name)) => self.agent_row(name, cx),
            _ => ix,
        };
        self.table.select(ix);
        if self.selected(cx) != selected {
            self.selection_moved(cx);
        }
    }

    /// The rows read afresh with the sessions working here stacked on them.
    pub(super) fn stack_fresh(&mut self, cx: &mut App) {
        self.agents.reset();
        let sessions = self.agents_here();
        self.stack_agents(sessions, None, cx);
    }

    pub(super) fn agent_row(&self, name: &str, cx: &App) -> Option<usize> {
        self.pages.read(cx).rows.iter().position(|r| r.agent.as_deref() == Some(name))
    }

    /// Takes the stack away; where the row `ix` is then.
    fn unstack_agents(&mut self, ix: Option<usize>, cx: &mut App) -> Option<usize> {
        let Some(base) = self.agents.base.take() else { return ix };
        let (at, n) = (self.agents.at, self.agents.stacked);
        self.pages.update(cx, |p, _| {
            let rows = Rc::make_mut(&mut p.rows);
            rows.drain(at..at + n);
            rows[at].row = base;
            p.marks.shift(at, -(n as isize));
        });
        ix.and_then(|i| if i < at { Some(i) } else if i < at + n { None } else { Some(i - n) })
    }

    /// Stacks a row per session on the WIP row, else on HEAD's row once it is loaded; where the row `ix` is then.
    fn stack_agents(&mut self, sessions: Vec<LiveSession>, ix: Option<usize>, cx: &mut App) -> Option<usize> {
        self.agents.shown = sessions;
        let n = self.agents.shown.len();
        let (shown, head) = (&self.agents.shown, self.heads.head.as_ref());
        let stacked = self.pages.update(cx, |p, _| {
            let wip = p.rows.first().filter(|r| r.wip()).map(|_| 0);
            let head = || head.and_then(|head| p.rows.iter().position(|r| r.commit() && r.sha == *head));
            let at = wip.or_else(head).filter(|_| n > 0)?;
            let rows = Rc::make_mut(&mut p.rows);
            let (above, base) = rows[at].row.stacked_above(n);
            let base = std::mem::replace(&mut rows[at].row, base);
            let added: Vec<GraphRow> = above.into_iter().zip(shown).map(|(row, s)| GraphRow { agent: Some(s.name.clone().into()), ..load::graph_row(row, None) }).collect();
            rows.splice(at..at, added);
            p.marks.shift(at, n as isize);
            Some((at, base))
        });
        let Some((at, base)) = stacked else { return ix };
        self.agents.base = Some(base);
        (self.agents.at, self.agents.stacked) = (at, n);
        ix.map(|i| if i >= at { i + n } else { i })
    }

    fn shown_agent(&self, name: &str) -> Option<&LiveSession> {
        self.agents.shown.iter().find(|s| s.name == name)
    }

    /// The live session `name` working in `path`, for its state's dot.
    pub(super) fn activity_in(&self, path: &Path, name: &str) -> Option<Activity> {
        self.agents.live.as_ref()?.working_in(path).into_iter().find(|s| s.name == name).map(|s| s.activity)
    }

    /// Row `ix`'s cell of column `col` for an agent row: the bot node, its dot and `agent <name> is working here`.
    pub(super) fn agent_cell(&mut self, ix: usize, col: usize, palette: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let rows = self.pages.read(cx).rows.clone();
        let row = &rows[ix];
        let Some(name) = row.agent.clone() else { return div().into_any_element() };
        let cell = match col {
            1 => graph::cell(ix, row, false, palette, &self.hits),
            2 => {
                let activity = self.shown_agent(&name).map(|s| s.activity);
                let lead = div()
                    .relative()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .min_w_0()
                    .whitespace_nowrap()
                    .text_color(palette.text)
                    .children(activity.map(|a| state_dot(a, cx)))
                    .child(SharedString::from(format!("agent {name} is working here")))
                    .child(hit_target(&self.hits, format!("git-agent-{ix}")));
                cells::wip_message(row, lead, activity.map(Activity::word).unwrap_or_default().to_string(), palette)
            }
            _ => div().into_any_element(),
        };
        div()
            .id(SharedString::from(format!("git-agent-{ix}-{col}")))
            .size_full()
            .on_click(cx.listener(move |this, e: &ClickEvent, _, cx| {
                if e.click_count() == 2 {
                    this.attach_agent(&name, cx);
                }
            }))
            .child(cell)
            .into_any_element()
    }

    fn attach_agent(&mut self, name: &str, cx: &mut Context<Self>) {
        if let Some(session) = self.shown_agent(name).filter(|s| s.attach.is_some()).cloned() {
            cx.emit(AttachAgent(session));
        }
    }

    /// The selected agent row's session: its name, state and folder, and Attach, dimmed with why when it cannot be.
    pub(super) fn render_agent_panel(&mut self, name: &str, cx: &mut Context<Self>) -> AnyElement {
        let Some(session) = self.shown_agent(name).cloned() else {
            return placeholder::placeholder(name.to_string(), ["The session ended."], cx).size_full().into_any_element();
        };
        let colour = session.activity.color(cx);
        let sections = [Section::new("session", "Session").row("Folder", session.cwd.display().to_string()).row("State", session.activity.word())];
        let attach = button::bordered(&self.hits, "git-agent-attach", "Attach", cx);
        let attach = match session.attach {
            Some(_) => {
                let name = name.to_string();
                attach.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.attach_agent(&name, cx))).into_any_element()
            }
            None => button::dimmed(attach).tooltip(Tooltip::text(NOT_ATTACHABLE)).into_any_element(),
        };
        let scroll = self.agents.scroll.clone();
        let page = Page { key: "git-agent", title: name.to_string(), pill: Some((capitalised(session.activity.word()), colour)), sections: &sections, actions: vec![attach], scroll: &scroll, on_link: None, top: None, over: None, bar: None };
        details::render(page, &self.hits.clone(), cx)
    }
}

/// A session's state as its dot: the sidebar's status colours.
pub(super) fn state_dot(activity: Activity, cx: &App) -> gpui::Div {
    marks::dot(Mark::Filled(activity.color(cx)), 8.)
}

#[cfg(test)]
impl GitView {
    pub(crate) fn agent_rows(&self, cx: &App) -> Vec<(usize, String)> {
        self.pages.read(cx).rows.iter().enumerate().filter_map(|(i, r)| Some((i, r.agent.as_ref()?.to_string()))).collect()
    }
}

#[cfg(test)]
mod tests;
