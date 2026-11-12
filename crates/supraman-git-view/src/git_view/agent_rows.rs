//! The graph's rows of the live sessions, each stacked on its checkout's row (the tab's own or another worktree's), laid out without a git read, and their panel.

use std::cmp::Reverse;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{AnyElement, App, ClickEvent, Context, Entity, EventEmitter, Hsla, MouseButton, MouseDownEvent, Pixels, Point, ScrollHandle, SharedString, Window, div, prelude::*};
use supraman_gitman::Row;
use supraman_ui::details::{self, Page, Section};
use supraman_ui::hit::{HitTargets, hit_target};
use supraman_ui::marks::Mark;
use supraman_ui::clipped::clipped;
use supraman_ui::session::{self, Opening};
use supraman_ui::tooltip::Tooltip;
use supraman_ui::{button, placeholder};

use super::colours::Palette;
use super::load::{self, GraphRow};
use supraman_core::text::capitalised;
use super::{GitView, Selected, background, cells, graph};
use crate::occupancy::{Live, LiveSession};
use supraman_core::paths::{key_within, path_key};

/// Open asked for this session: the fleet's one open (its tab, an attach or a resume).
pub struct OpenAgent(pub LiveSession);

/// A right-click on this session's row: the fleet's menu of the session, at `position` in the window.
pub struct AgentMenu {
    pub session: LiveSession,
    pub position: Point<Pixels>,
}

/// One of a [`SessionPage`]'s actions: the fleet's menu item `key`, dimmed with why when it cannot run.
pub struct PageAction {
    pub key: String,
    pub label: String,
    pub dimmed: Option<String>,
}

/// The fleet's page of a live session, its Details tab's but the transcript: the facts, Open, then the menu's actions.
pub struct SessionPage {
    pub title: String,
    pub pill: (String, Hsla),
    pub sections: Vec<Section>,
    pub open: Opening,
    pub actions: Vec<PageAction>,
}

/// How an agent row's panel reads its session's page from the fleet, runs one of its actions, and what Open does for a session it has no page of.
#[derive(Clone)]
pub struct SessionPages {
    pub page: Rc<dyn Fn(&LiveSession, &App) -> Option<SessionPage>>,
    pub act: Rc<dyn Fn(&LiveSession, &str, &mut Window, &mut App)>,
    pub opening: Rc<dyn Fn(&LiveSession, &App) -> Opening>,
}

/// The longest the live watch sleeps before it looks whether the tab is still open.
const WAKE_AT_LEAST: Duration = Duration::from_secs(60);

#[derive(Default)]
pub(super) struct Agents {
    live: Option<Live>,
    pages: Option<SessionPages>,
    stacks: Vec<Stack>,
    /// The sessions working in the repo's other worktrees, as last seen.
    elsewhere: Vec<(String, PathBuf)>,
    scroll: ScrollHandle,
}

/// The checkout whose row a stack stands on.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum On {
    /// The tab's own: its WIP row, else HEAD's.
    Here,
    /// Another worktree's row, by [`path_key`].
    Worktree(String),
}

/// The rows from `at` on, a session's each by name, stacked on `base`'s row.
struct Stack {
    on: On,
    sessions: Vec<LiveSession>,
    at: usize,
    base: Row,
}

impl Agents {
    /// The rows from `rows` on were dropped: a stack among them went with them.
    pub(super) fn dropped_from(&mut self, rows: usize) {
        self.stacks.retain(|s| s.at < rows);
    }

    /// The rows were read afresh: nothing is stacked on them.
    pub(super) fn reset(&mut self) {
        self.stacks.clear();
    }

    /// Every session shown as a row, stack by stack.
    pub(super) fn shown(&self) -> impl Iterator<Item = &LiveSession> {
        self.stacks.iter().flat_map(|s| &s.sessions)
    }

    fn ids(&self) -> Vec<(On, Vec<String>)> {
        let mut ids: Vec<_> = self.stacks.iter().map(|s| (s.on.clone(), ids(&s.sessions))).collect();
        ids.sort();
        ids
    }
}

fn ids(sessions: &[LiveSession]) -> Vec<String> {
    sessions.iter().map(|s| s.claude_session_id.clone()).collect()
}

impl EventEmitter<OpenAgent> for GitView {}
impl EventEmitter<AgentMenu> for GitView {}

impl GitView {
    /// [`GitView::open_with`] of `live`, whose sessions working in the worktree the graph shows as agent rows.
    pub fn open_following(folder: PathBuf, home: Option<PathBuf>, hits: HitTargets, live: Live, cx: &mut App) -> Entity<GitView> {
        let view = GitView::open_with(folder, home, hits, Arc::new(live.clone()), cx);
        view.update(cx, |v, cx| v.follow_live(live, cx));
        view
    }

    pub fn set_session_pages(&mut self, pages: SessionPages, cx: &mut Context<Self>) {
        self.agents.pages = Some(pages);
        cx.notify();
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
        self.other_wips(cx).iter().flat_map(|w| live.working_in(w.path()).into_iter().map(|s| (s.claude_session_id, w.path().to_path_buf()))).collect()
    }

    /// Each checkout's live sessions by name, the tab's own first, a session on the deepest checkout holding its cwd; only those whose row is loaded.
    fn agents_by_checkout(&self, cx: &Context<Self>) -> Vec<(On, Vec<LiveSession>)> {
        let top = self.repo.as_ref().and_then(|r| r.known_dirs()).map(|d| d.top.clone());
        let (Some(live), Some(top)) = (&self.agents.live, top) else { return Vec::new() };
        let others = self.other_wips(cx).into_iter().map(|w| (On::Worktree(path_key(w.path())), path_key(w.path())));
        let checkouts: Vec<(On, String)> = std::iter::once((On::Here, path_key(&top))).chain(others).collect();
        let mut sessions = live.sessions();
        sessions.sort_by(|a, b| a.name.cmp(&b.name));
        let mut grouped: Vec<(On, Vec<LiveSession>)> = checkouts.iter().map(|(on, _)| (on.clone(), Vec::new())).collect();
        for s in sessions {
            let cwd = path_key(&s.cwd);
            let deepest = checkouts.iter().enumerate().filter(|(_, (_, key))| key_within(&cwd, key)).max_by_key(|(_, (_, key))| key.len());
            if let Some((i, _)) = deepest {
                grouped[i].1.push(s);
            }
        }
        let rows = &self.pages.read(cx).rows;
        let head = self.heads.head.as_deref();
        grouped.retain(|(on, sessions)| !sessions.is_empty() && base_of(on, rows, head).is_some());
        grouped
    }

    /// The agent rows laid out again for the sessions working now; the selection kept on its row.
    pub(super) fn relay_agents(&mut self, cx: &mut Context<Self>) {
        let wanted = self.agents_by_checkout(cx);
        let mut wanted_ids: Vec<_> = wanted.iter().map(|(on, s)| (on.clone(), ids(s))).collect();
        wanted_ids.sort();
        if wanted_ids == self.agents.ids() {
            for (on, sessions) in wanted {
                if let Some(stack) = self.agents.stacks.iter_mut().find(|s| s.on == on) {
                    stack.sessions = sessions;
                }
            }
            return;
        }
        let selected = self.selected(cx);
        let ix = self.unstack_agents(self.table.selected(), cx);
        let ix = self.stack_agents(wanted, ix, cx);
        let ix = match &selected {
            Some(Selected::Agent(id)) => self.agent_row(id, cx),
            _ => ix,
        };
        self.table.select(ix);
        if self.selected(cx) != selected {
            self.selection_moved(cx);
        }
    }

    /// The rows read afresh with the live sessions stacked on them.
    pub(super) fn stack_fresh(&mut self, cx: &mut Context<Self>) {
        self.agents.reset();
        let wanted = self.agents_by_checkout(cx);
        self.stack_agents(wanted, None, cx);
    }

    pub(super) fn agent_row(&self, id: &str, cx: &App) -> Option<usize> {
        self.pages.read(cx).rows.iter().position(|r| r.agent.as_deref() == Some(id))
    }

    /// Takes the stacks away; where the row `ix` is then.
    fn unstack_agents(&mut self, mut ix: Option<usize>, cx: &mut App) -> Option<usize> {
        let mut stacks = std::mem::take(&mut self.agents.stacks);
        stacks.sort_by_key(|s| Reverse(s.at));
        for Stack { sessions, at, base, .. } in stacks {
            let n = sessions.len();
            self.pages.update(cx, |p, _| {
                let rows = Rc::make_mut(&mut p.rows);
                rows.drain(at..at + n);
                rows[at].row = base;
                p.marks.shift(at, -(n as isize));
            });
            ix = ix.and_then(|i| if i < at { Some(i) } else if i < at + n { None } else { Some(i - n) });
        }
        ix
    }

    /// Stacks a row per session on its checkout's row; where the row `ix` is then.
    fn stack_agents(&mut self, wanted: Vec<(On, Vec<LiveSession>)>, mut ix: Option<usize>, cx: &mut App) -> Option<usize> {
        let head = self.heads.head.clone();
        let mut placed: Vec<(usize, On, Vec<LiveSession>)> = {
            let rows = &self.pages.read(cx).rows;
            wanted.into_iter().filter_map(|(on, sessions)| Some((base_of(&on, rows, head.as_deref())?, on, sessions))).collect()
        };
        placed.sort_by_key(|(at, ..)| Reverse(*at));
        let mut stacks = Vec::new();
        for (at, on, sessions) in placed {
            let n = sessions.len();
            let base = self.pages.update(cx, |p, _| {
                let rows = Rc::make_mut(&mut p.rows);
                let (above, base) = rows[at].row.stacked_above(n);
                let base = std::mem::replace(&mut rows[at].row, base);
                let added: Vec<GraphRow> = above.into_iter().zip(&sessions).map(|(row, s)| GraphRow { agent: Some(s.claude_session_id.clone().into()), ..load::graph_row(row, None) }).collect();
                rows.splice(at..at, added);
                p.marks.shift(at, n as isize);
                base
            });
            ix = ix.map(|i| if i >= at { i + n } else { i });
            stacks.push(Stack { on, sessions, at, base });
        }
        let laid: Vec<(usize, usize)> = stacks.iter().map(|s| (s.at, s.sessions.len())).collect();
        for stack in &mut stacks {
            stack.at += laid.iter().filter(|(at, _)| *at < stack.at).map(|(_, n)| n).sum::<usize>();
        }
        stacks.reverse();
        self.agents.stacks = stacks;
        ix
    }

    fn shown_agent(&self, id: &str) -> Option<&LiveSession> {
        self.agents.shown().find(|s| s.claude_session_id == id)
    }

    /// Row `ix`'s cell of column `col` for an agent row: the bot node, then the session's lead with its state's word.
    pub(super) fn agent_cell(&mut self, ix: usize, col: usize, palette: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let rows = self.pages.read(cx).rows.clone();
        let row = &rows[ix];
        let Some(id) = row.agent.clone() else { return div().into_any_element() };
        let session = self.shown_agent(&id).cloned();
        let cell = match (col, &session) {
            (1, _) => graph::cell(ix, row, false, palette, &self.hits),
            (2, Some(s)) => cells::wip_message(row, session_lead(&format!("git-agent-{ix}"), &format!("git-agent-state-{ix}"), s, palette, &self.hits, cx), String::new(), palette),
            _ => div().into_any_element(),
        };
        let menu_id = id.clone();
        div()
            .id(SharedString::from(format!("git-agent-{ix}-{col}")))
            .size_full()
            .on_click(cx.listener(move |this, e: &ClickEvent, _, cx| {
                if e.click_count() == 2 {
                    this.open_agent(&id, cx);
                }
            }))
            .on_mouse_down(MouseButton::Right, cx.listener(move |this, e: &MouseDownEvent, _, cx| this.agent_menu(&menu_id, e.position, cx)))
            .child(cell)
            .into_any_element()
    }

    fn agent_menu(&mut self, id: &str, position: Point<Pixels>, cx: &mut Context<Self>) {
        if let Some(session) = self.shown_agent(id).cloned() {
            cx.stop_propagation();
            cx.emit(AgentMenu { session, position });
        }
    }

    fn open_agent(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(session) = self.shown_agent(id).cloned() {
            cx.emit(OpenAgent(session));
        }
    }

    /// The selected agent row's session: the fleet's page of it, else its folder and state with Open, dimmed with why when it cannot run.
    pub(super) fn render_agent_panel(&mut self, id: &str, cx: &mut Context<Self>) -> AnyElement {
        let Some(session) = self.shown_agent(id).cloned() else {
            return placeholder::placeholder("The session ended.", Vec::<String>::new(), cx).size_full().into_any_element();
        };
        let scroll = self.agents.scroll.clone();
        let hits = self.hits.clone();
        let pages = self.agents.pages.clone();
        if let Some(page) = pages.as_ref().and_then(|p| (p.page)(&session, cx)) {
            let open = self.open_control(id, &page.open, cx);
            let actions = std::iter::once(open).chain(page.actions.into_iter().map(|a| self.page_action(&session, a, cx))).collect();
            let page = Page { key: "git-agent", title: page.title, pill: Some(page.pill), sections: &page.sections, actions, scroll: &scroll, on_link: None, top: None, over: None, bar: None };
            return details::render(page, &hits, cx);
        }
        let sections = [Section::new("session", "Session").path("Working directory", &session.cwd).row("State", session.activity.word())];
        let actions = pages.map(|p| (p.opening)(&session, cx)).map(|opening| self.open_control(id, &opening, cx)).into_iter().collect();
        let pill = (capitalised(session.activity.word()), session.activity.color(cx));
        let page = Page { key: "git-agent", title: session.name.clone(), pill: Some(pill), sections: &sections, actions, scroll: &scroll, on_link: None, top: None, over: None, bar: None };
        details::render(page, &hits, cx)
    }

    fn open_control(&self, id: &str, opening: &Opening, cx: &mut Context<Self>) -> AnyElement {
        let (view, id) = (cx.weak_entity(), id.to_string());
        let on_open = move |_: &mut Window, cx: &mut App| {
            view.update(cx, |this, cx| this.open_agent(&id, cx)).ok();
        };
        session::open(&self.hits, "git-agent-open", opening, button::Fill::Outline, on_open, cx).into_any_element()
    }

    fn page_action(&self, session: &LiveSession, action: PageAction, cx: &mut Context<Self>) -> AnyElement {
        let button = button::action(&self.hits, format!("git-agent-{}", action.key), action.label, button::Tone::Neutral, button::Fill::Outline, cx);
        if let Some(why) = action.dimmed {
            return button::dimmed(button).tooltip(Tooltip::text(why)).into_any_element();
        }
        let (session, key) = (session.clone(), action.key);
        button
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                if let Some(pages) = this.agents.pages.clone() {
                    (pages.act)(&session, &key, window, cx);
                }
            }))
            .into_any_element()
    }
}

/// The row `on`'s stack stands on in `rows`: the WIP row, else HEAD's; another worktree's row.
fn base_of(on: &On, rows: &[GraphRow], head: Option<&str>) -> Option<usize> {
    match on {
        On::Here => rows.iter().position(|r| r.agent.is_none()).filter(|&i| rows[i].wip()).or_else(|| head.and_then(|head| rows.iter().position(|r| r.commit() && r.sha == *head))),
        On::Worktree(key) => rows.iter().position(|r| r.worktree().is_some_and(|p| path_key(p) == *key)),
    }
}

/// A session's [`session::lead`] in a graph row, painted under `key`: its state's dot, its name, then its state's word under `state_key`.
pub(super) fn session_lead(key: &str, state_key: &str, s: &LiveSession, palette: &Palette, hits: &HitTargets, cx: &App) -> gpui::Div {
    let state = clipped(state_key.to_string(), s.activity.word(), false).flex_shrink(4.).text_color(palette.muted).child(hit_target(hits, state_key.to_string())).into_any_element();
    session::lead(key, Mark::Filled(s.activity.color(cx)), &s.name, Some(state)).relative().text_color(palette.text).child(hit_target(hits, key.to_string()))
}

#[cfg(test)]
impl GitView {
    pub(crate) fn agent_rows(&self, cx: &App) -> Vec<(usize, String)> {
        self.pages.read(cx).rows.iter().enumerate().filter_map(|(i, r)| Some((i, r.agent.as_ref()?.to_string()))).collect()
    }
}

#[cfg(test)]
mod tests;
