//! Read only (C21): the token stays inside the hub's [`Client`]; no message here may carry it.

mod inbox;
mod item;
mod journal;

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    time::Duration,
};

use supraman_core::time::rfc3339_local;
use supraman_workman::{
    AuthStatus, Board, BoardRow, Client, ItemRef, ItemStatus, OverviewProject, OverviewRun, Resolved, Rollup, Stream,
    StreamEvent, StreamSignal,
};
use gpui::{
    AnyElement, App, AppContext as _, ClickEvent, Context, Entity, EventEmitter, FontWeight, Global, Hsla, Render,
    SharedString, Window, div, prelude::*, px, relative,
};
use theme::ActiveTheme as _;

use crate::clipped::clipped;
use supraman_ui::{banner, button, fonts};
use supraman_ui::hit::{HitTargets, hit_target};
use crate::workstate_setup::WsStatus;

pub use inbox::{DISMISSED_FILE, WorkInbox, dismiss_key, kind_label, row_key};
pub use item::{FleetLink, LiveSession, OpenItem, criterion_key, verification_key};
pub use journal::JournalView;

pub const DEBOUNCE: Duration = Duration::from_millis(300);

fn after_debounce<T: 'static>(cx: &mut Context<T>, then: impl FnOnce(&mut T, &mut Context<T>) + 'static) {
    cx.spawn(async move |this, cx| {
        cx.background_executor().timer(DEBOUNCE).await;
        this.update(cx, then).ok();
    })
    .detach();
}

pub enum Access<'a> {
    Starting,
    Ready(&'a Client),
    Unconfigured(&'a str),
}

enum HubState {
    Starting,
    Ready(Client),
    Unconfigured(String),
}

pub struct WorkHub {
    home: Option<PathBuf>,
    state: HubState,
    _stream: Option<Stream>,
    connected: bool,
    lost: Option<String>,
}

pub enum HubEvent {
    Configured,
    Changed(StreamEvent),
    /// Connected again after a loss: frames may have been missed.
    Resync,
    Connection,
}

impl EventEmitter<HubEvent> for WorkHub {}

struct GlobalHub(Entity<WorkHub>);

impl Global for GlobalHub {}

impl WorkHub {
    /// An unconfigured hub reads the settings again, so a sign-in meanwhile takes effect.
    pub fn shared(home: Option<PathBuf>, cx: &mut App) -> Entity<WorkHub> {
        if let Some(GlobalHub(hub)) = cx.try_global::<GlobalHub>() {
            let hub = hub.clone();
            hub.update(cx, |hub, cx| {
                if matches!(hub.state, HubState::Unconfigured(_)) {
                    hub.configure(cx);
                }
            });
            return hub;
        }
        let hub = cx.new(|cx| {
            let mut hub = WorkHub { home, state: HubState::Starting, _stream: None, connected: false, lost: None };
            hub.configure(cx);
            hub
        });
        cx.set_global(GlobalHub(hub.clone()));
        hub
    }

    pub fn reconfigure(home: Option<PathBuf>, cx: &mut App) {
        if let Some(GlobalHub(hub)) = cx.try_global::<GlobalHub>() {
            hub.clone().update(cx, |hub, cx| {
                hub._stream = None;
                hub.connected = false;
                hub.lost = None;
                hub.configure(cx);
            });
        } else {
            Self::shared(home, cx);
        }
    }

    pub fn access(&self) -> Access<'_> {
        match &self.state {
            HubState::Starting => Access::Starting,
            HubState::Ready(client) => Access::Ready(client),
            HubState::Unconfigured(why) => Access::Unconfigured(why),
        }
    }

    pub fn connection(&self) -> (String, bool) {
        match (&self.state, self.connected, &self.lost) {
            (HubState::Ready(_), true, _) => ("live".into(), true),
            (HubState::Ready(_), false, Some(why)) => (format!("reconnecting ({why})"), false),
            (HubState::Ready(_), false, None) => ("connecting…".into(), false),
            _ => (String::new(), false),
        }
    }

    fn configure(&mut self, cx: &mut Context<Self>) {
        self.state = HubState::Starting;
        let home = self.home.clone();
        cx.spawn(async move |this, cx| {
            let loaded = cx.background_executor().spawn(async move { load_client(home) }).await;
            let rx = this
                .update(cx, |this, cx| {
                    let rx = match loaded {
                        Ok(client) => {
                            let (stream, rx) = Stream::subscribe(&client);
                            this._stream = Some(stream);
                            this.state = HubState::Ready(client);
                            Some(rx)
                        }
                        Err(why) => {
                            this.state = HubState::Unconfigured(why);
                            None
                        }
                    };
                    cx.emit(HubEvent::Configured);
                    cx.notify();
                    rx
                })
                .ok()
                .flatten();
            let Some(rx) = rx else { return };
            while let Ok(signal) = rx.recv().await {
                if this.update(cx, |this, cx| this.on_signal(signal, cx)).is_err() {
                    return;
                }
            }
        })
        .detach();
    }

    fn on_signal(&mut self, signal: StreamSignal, cx: &mut Context<Self>) {
        match signal {
            StreamSignal::Connected => {
                self.connected = true;
                if self.lost.take().is_some() {
                    cx.emit(HubEvent::Resync);
                }
                cx.emit(HubEvent::Connection);
            }
            StreamSignal::Event(event) => cx.emit(HubEvent::Changed(event)),
            StreamSignal::Lost(why) => {
                self.connected = false;
                self.lost = Some(why);
                cx.emit(HubEvent::Connection);
            }
        }
        cx.notify();
    }
}

/// No network call: the bearer is asked for at each read.
fn load_client(home: Option<PathBuf>) -> Result<Client, String> {
    let home = home.ok_or_else(|| "No Supraman home (set SUPRAMAN_HOME or APPDATA), so no settings.toml.".to_string())?;
    let auth = crate::signin::auth(&home);
    let url = auth.configured_origin();
    match auth.status(&url) {
        AuthStatus::SignedIn { .. } => Ok(Client::with_source(url.as_str(), auth.bearer_for(&url))),
        AuthStatus::NotSignedIn => Err(WsStatus::NotSignedIn { url }.message().unwrap_or_default()),
        AuthStatus::SignInAgain(why) => Err(WsStatus::SignInAgain { url, why }.message().unwrap_or_default()),
    }
}

fn read_failed(e: supraman_workman::Error) -> String {
    match e {
        supraman_workman::Error::Transport { base, reason } => WsStatus::Unreachable { url: base, error: reason }.message().unwrap_or_default(),
        supraman_workman::Error::Auth(why) => format!("{why}. Sign in again in Settings → Workman."),
        e => format!("Cannot read Workman: {e}"),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shown {
    Loading,
    Message(String),
    Runs(OverviewProject),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardState {
    Loading,
    Ready(Board),
    Failed(String),
}

pub enum WorkEvent {
    ItemSelected(String),
}

impl EventEmitter<WorkEvent> for WorkView {}

pub struct WorkView {
    pub workspace: String,
    hub: Entity<WorkHub>,
    hits: HitTargets,
    project: Option<ItemRef>,
    pub shown: Shown,
    /// The `seq` of the overview shown: an older answer arriving late is dropped.
    overview_seq: i64,
    pub boards: HashMap<String, BoardState>,
    pub expanded: HashSet<String>,
    /// Runs the tab has seen once; only a run's first sight decides whether it starts expanded.
    seen_runs: HashSet<String>,
    pub selected: Option<String>,
    overview_pending: bool,
    boards_pending: HashSet<String>,
    pub overview_reads: u64,
    pub board_reads: u64,
    link: FleetLink,
    pub item: Option<OpenItem>,
    item_pending: bool,
    pub item_reads: u64,
    pub journal: JournalView,
}

impl WorkView {
    pub fn open<T: 'static>(workspace: &str, home: Option<PathBuf>, hits: HitTargets, link: FleetLink, cx: &mut Context<T>) -> Entity<WorkView> {
        let hub = WorkHub::shared(home, cx);
        cx.new(|cx| WorkView::new(workspace.to_string(), hub, hits, link, cx))
    }

    /// The Workman project the open view read, once it has.
    pub fn project(&self) -> Option<&ItemRef> {
        self.project.as_ref()
    }

    fn new(workspace: String, hub: Entity<WorkHub>, hits: HitTargets, link: FleetLink, cx: &mut Context<Self>) -> Self {
        cx.subscribe(&hub, Self::on_hub).detach();
        let mut view = WorkView {
            workspace,
            hub,
            hits,
            project: None,
            shown: Shown::Loading,
            overview_seq: i64::MIN,
            boards: HashMap::new(),
            expanded: HashSet::new(),
            seen_runs: HashSet::new(),
            selected: None,
            overview_pending: false,
            boards_pending: HashSet::new(),
            overview_reads: 0,
            board_reads: 0,
            link,
            item: None,
            item_pending: false,
            item_reads: 0,
            journal: JournalView::default(),
        };
        view.read_overview(cx);
        view
    }

    fn on_hub(&mut self, _: Entity<WorkHub>, event: &HubEvent, cx: &mut Context<Self>) {
        match event {
            HubEvent::Configured => {
                self.read_overview(cx);
                self.read_item(cx);
            }
            HubEvent::Changed(event) => {
                self.schedule_overview(cx);
                self.schedule_journal(cx);
                if let Some(run) = board_to_reload(event, &self.expanded) {
                    self.schedule_board(run.to_string(), cx);
                }
                if self.item.as_ref().is_some_and(|open| item::item_to_reload(event, open)) {
                    self.schedule_item(cx);
                }
            }
            HubEvent::Resync => {
                self.read_overview(cx);
                for run in self.expanded.clone() {
                    self.read_board(run, cx);
                }
                self.read_item(cx);
                if self.journal.feed.is_some() {
                    self.read_journal(cx);
                }
            }
            HubEvent::Connection => cx.notify(),
        }
    }

    pub fn on_select_item(&mut self, id: &str, cx: &mut Context<Self>) {
        cx.emit(WorkEvent::ItemSelected(id.to_string()));
        self.show_item(id.to_string(), cx);
    }

    fn select(&mut self, id: String, cx: &mut Context<Self>) {
        self.on_select_item(&id, cx);
        self.selected = Some(id);
        cx.notify();
    }

    pub fn toggle_run(&mut self, run_id: &str, cx: &mut Context<Self>) {
        if !self.expanded.remove(run_id) {
            self.expanded.insert(run_id.to_string());
            if !matches!(self.boards.get(run_id), Some(BoardState::Ready(_))) {
                self.read_board(run_id.to_string(), cx);
            }
        }
        cx.notify();
    }

    fn client(&mut self, cx: &mut Context<Self>) -> Option<Client> {
        match self.hub.read(cx).access() {
            Access::Starting => None,
            Access::Ready(client) => Some(client.clone()),
            Access::Unconfigured(why) => {
                self.shown = Shown::Message(why.to_string());
                cx.notify();
                None
            }
        }
    }

    fn schedule_overview(&mut self, cx: &mut Context<Self>) {
        if !self.overview_pending {
            self.overview_pending = true;
            after_debounce(cx, |this, cx| {
                this.overview_pending = false;
                this.read_overview(cx);
            });
        }
    }

    fn schedule_board(&mut self, run: String, cx: &mut Context<Self>) {
        if self.boards_pending.insert(run.clone()) {
            after_debounce(cx, move |this, cx| {
                this.boards_pending.remove(&run);
                this.read_board(run, cx);
            });
        }
    }

    fn read_overview(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.client(cx) else { return };
        let (workspace, project) = (self.workspace.clone(), self.project.clone());
        self.overview_reads += 1;
        cx.spawn(async move |this, cx| {
            let read = cx.background_executor().spawn(async move { read_overview(&client, &workspace, project) }).await;
            this.update(cx, |this, cx| this.apply_overview(read, cx)).ok();
        })
        .detach();
    }

    fn apply_overview(&mut self, read: Result<(ItemRef, i64, OverviewProject), String>, cx: &mut Context<Self>) {
        match read {
            Ok((_, seq, _)) if seq < self.overview_seq => return,
            Ok((project, seq, overview)) => {
                for run in &overview.runs {
                    if self.seen_runs.insert(run.item.id.clone()) && starts_expanded(run.item.status) {
                        self.expanded.insert(run.item.id.clone());
                        self.read_board(run.item.id.clone(), cx);
                    }
                }
                self.project = Some(project);
                self.overview_seq = seq;
                self.shown = Shown::Runs(overview);
                self.journal_project_known(cx);
            }
            Err(message) => {
                self.project = None;
                self.overview_seq = i64::MIN;
                self.shown = Shown::Message(message);
            }
        }
        cx.notify();
    }

    fn read_board(&mut self, run: String, cx: &mut Context<Self>) {
        let Some(client) = self.client(cx) else { return };
        self.boards.entry(run.clone()).or_insert(BoardState::Loading);
        self.board_reads += 1;
        cx.spawn(async move |this, cx| {
            let id = run.clone();
            let read = cx
                .background_executor()
                .spawn(async move { client.board(&id).map_err(|e| format!("Cannot read this run's board: {e}")) })
                .await;
            this.update(cx, |this, cx| {
                match (read, this.boards.get(&run)) {
                    (Ok(board), Some(BoardState::Ready(shown))) if board.seq < shown.seq => {}
                    (Ok(board), _) => {
                        this.boards.insert(run, BoardState::Ready(board));
                    }
                    // A failed re-read keeps the board on screen; the next event reads it again.
                    (Err(_), Some(BoardState::Ready(_))) => {}
                    (Err(message), _) => {
                        this.boards.insert(run, BoardState::Failed(message));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

fn read_overview(client: &Client, workspace: &str, project: Option<ItemRef>) -> Result<(ItemRef, i64, OverviewProject), String> {
    let failed = read_failed;
    let project = match project {
        Some(project) => project,
        None => pick_project(client.resolve(workspace).map_err(failed)?, workspace)?,
    };
    let overview = client.overview().map_err(failed)?;
    let seq = overview.seq;
    let found = overview.projects.into_iter().find(|p| p.item.id == project.id);
    let found = found.ok_or_else(|| no_project(workspace))?;
    Ok((project, seq, found))
}

fn no_project(workspace: &str) -> String {
    format!("Workman has no project with ref '{workspace}', so there is no work to show for this workspace.")
}

pub fn pick_project(resolved: Resolved, workspace: &str) -> Result<ItemRef, String> {
    let mut found: Vec<ItemRef> = resolved.matches.into_iter().filter(|m| m.kind == "project").collect();
    match found.len() {
        1 => Ok(found.remove(0)),
        0 => Err(no_project(workspace)),
        n => Err(format!("'{workspace}' names {n} Workman projects; Supraman cannot tell which is this workspace's.")),
    }
}

pub fn starts_expanded(status: ItemStatus) -> bool {
    !matches!(status, ItemStatus::Done | ItemStatus::Cancelled | ItemStatus::Superseded)
}

pub fn board_to_reload<'a>(event: &'a StreamEvent, expanded: &HashSet<String>) -> Option<&'a str> {
    event.run_id.as_deref().filter(|run| expanded.contains(*run))
}

/// The server lists the run first; the run is the section's header.
pub fn item_rows(board: &Board) -> impl Iterator<Item = &BoardRow> {
    board.rows.iter().filter(move |r| r.item.id != board.run.id)
}

pub fn done_text(r: &Rollup) -> String {
    format!("{}/{} done", r.done, r.counted())
}

fn status_color(status: ItemStatus, cx: &App) -> Hsla {
    let s = cx.theme().status();
    match status {
        ItemStatus::Done => s.success,
        ItemStatus::InProgress => s.info,
        ItemStatus::Blocked => s.error,
        ItemStatus::Review => s.warning,
        ItemStatus::Open | ItemStatus::Ready => cx.theme().colors().text_muted,
        ItemStatus::Cancelled | ItemStatus::Superseded | ItemStatus::Unknown => cx.theme().colors().text_disabled,
    }
}

fn last_segment(path: &str) -> &str {
    path.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or(path)
}

/// The key the selftest reads a row's painted status by.
pub fn status_key(qualified_ref: &str, status: ItemStatus) -> String {
    format!("work-status-{qualified_ref}={}", status.as_str())
}

fn small(text: impl Into<SharedString>, color: Hsla) -> gpui::Div {
    div().flex_none().text_xs().text_color(color).child(text.into())
}

fn badge(text: impl Into<SharedString>, color: Hsla) -> gpui::Div {
    div()
        .flex_none()
        .px_1()
        .rounded_sm()
        .border_1()
        .border_color(color.opacity(0.6))
        .bg(color.opacity(0.10))
        .text_xs()
        .text_color(color)
        .child(text.into())
}

fn rollup_bar(r: &Rollup, cx: &App) -> gpui::Div {
    let colors = cx.theme().colors();
    let share = if r.counted() == 0 { 0. } else { r.done as f32 / r.counted() as f32 };
    div()
        .flex_none()
        .w(px(72.))
        .h(px(6.))
        .rounded_full()
        .bg(colors.border)
        .child(div().h_full().rounded_full().w(relative(share.clamp(0., 1.))).bg(cx.theme().status().success))
}

impl WorkView {
    fn render_run(&self, run: &OverviewRun, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let status = cx.theme().status().clone();
        let open = self.expanded.contains(&run.item.id);
        let key = format!("work-run-{}", run.item.qualified_ref);
        let id = run.item.id.clone();
        let header = div()
            .id(SharedString::from(key.clone()))
            .relative()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .px_3()
            .py_1()
            .border_t_1()
            .border_color(colors.border)
            .cursor_pointer()
            .hover(|s| s.bg(colors.ghost_element_hover))
            .active(|s| s.bg(colors.ghost_element_active))
            .child(div().flex_none().w(px(10.)).child(if open { "▾" } else { "▸" }))
            .child(div().flex_none().font_weight(FontWeight::BOLD).child(SharedString::from(run.item.qualified_ref.clone())))
            .child(clipped(format!("run-title-{}", run.item.qualified_ref), run.item.title.clone(), false).flex_1().min_w(px(120.)))
            .child(rollup_bar(&run.rollup, cx))
            .child(small(done_text(&run.rollup), colors.text_muted))
            .child(small(run.item.status.as_str(), status_color(run.item.status, cx)))
            .when(run.active_claims > 0, |d| d.child(small(format!("claims {}", run.active_claims), status.info)))
            .when(run.inbox_count > 0, |d| d.child(small(format!("inbox {}", run.inbox_count), status.warning)))
            .when_some(run.last_verification.as_ref(), |d, v| {
                let (text, color) = if v.passed { ("✓ verified", status.success) } else { ("✗ failed", status.error) };
                d.child(small(format!("{text} {}", rfc3339_local(&v.at)), color))
            })
            .child(hit_target(&self.hits, key))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle_run(&id, cx)));
        let mut section = div().flex().flex_col().child(header);
        if open {
            section = match self.boards.get(&run.item.id) {
                Some(BoardState::Ready(board)) => {
                    let mut rows = div().flex().flex_col().pb_1();
                    let mut any = false;
                    for row in item_rows(board) {
                        any = true;
                        rows = rows.child(self.render_row(row, cx));
                    }
                    if !any {
                        rows = rows.child(div().pl(px(34.)).py_1().text_xs().text_color(colors.text_muted).child("no items"));
                    }
                    section.child(rows)
                }
                Some(BoardState::Failed(message)) => section.child(
                    div().pl(px(34.)).py_1().text_xs().text_color(status.error).child(SharedString::from(message.clone())),
                ),
                _ => section.child(div().pl(px(34.)).py_1().text_xs().text_color(colors.text_muted).child("loading…")),
            };
        }
        section.into_any_element()
    }

    fn render_row(&self, row: &BoardRow, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let status = cx.theme().status().clone();
        let item = &row.item;
        let key = format!("work-row-{}", item.qualified_ref);
        let selected = self.selected.as_deref() == Some(item.id.as_str());
        let indent = px(34. + 18. * row.depth.saturating_sub(1) as f32);
        let color = status_color(item.status, cx);

        let line = div()
            .flex()
            .items_center()
            .gap_2()
            .child(div().flex_none().w(px(64.)).text_xs().text_color(colors.text_muted).child(SharedString::from(item.qualified_ref.clone())))
            .child(
                div()
                    .relative()
                    .flex_none()
                    .w(px(78.))
                    .text_xs()
                    .text_color(color)
                    .child(item.status.as_str())
                    .child(hit_target(&self.hits, status_key(&item.qualified_ref, item.status))),
            )
            .child(clipped(format!("item-title-{}", item.qualified_ref), item.title.clone(), false).flex_1())
            .when(row.criteria.total > 0, |d| {
                let done = row.criteria.checked == row.criteria.total;
                d.child(small(
                    format!("{}/{}", row.criteria.checked, row.criteria.total),
                    if done { status.success } else { colors.text_muted },
                ))
            })
            .when_some(row.rollup.as_ref(), |d, r| d.child(small(done_text(r), colors.text_muted)));

        let mut meta: Vec<AnyElement> = Vec::new();
        if let Some(claim) = &row.claim {
            let tree = claim.tree.as_deref().map(|t| format!(" @ {}", last_segment(t))).unwrap_or_default();
            let (text, c) = if claim.expired {
                (format!("claim {}{tree} (expired)", claim.holder()), status.warning)
            } else {
                (format!("claim {}{tree}", claim.holder()), status.info)
            };
            meta.push(small(text, c).into_any_element());
        }
        if let Some(v) = &row.verification {
            let (mark, c) = if v.passed { ("✓ pass", status.success) } else { ("✗ fail", status.error) };
            meta.push(small(format!("{mark} by {}", v.run_by), c).into_any_element());
        }
        if let Some(commit) = &row.commit {
            meta.push(small(commit.chars().take(8).collect::<String>(), colors.text_muted).into_any_element());
        }
        if !row.blocked_by.is_empty() {
            let by: Vec<&str> = row.blocked_by.iter().map(|i| i.qualified_ref.as_str()).collect();
            meta.push(small(format!("blocked by {}", by.join(", ")), status.error).into_any_element());
        }
        for flag in &row.flags {
            meta.push(badge(flag.clone(), status.warning).into_any_element());
        }

        let id = item.id.clone();
        div()
            .id(SharedString::from(key.clone()))
            .relative()
            .flex()
            .flex_col()
            .gap(px(2.))
            .pl(indent)
            .pr_3()
            .py(px(3.))
            .text_sm()
            .cursor_pointer()
            .when(selected, |d| d.bg(colors.element_selected))
            .when(!selected, |d| d.hover(|s| s.bg(colors.ghost_element_hover)))
            .active(|s| s.bg(colors.ghost_element_active))
            .child(line)
            .when(!meta.is_empty(), |d| d.child(div().flex().flex_wrap().items_center().gap_2().pl(px(72.)).children(meta)))
            .child(hit_target(&self.hits, key))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.select(id.clone(), cx)))
            .into_any_element()
    }
}

impl Render for WorkView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors().clone();
        let board = self.render_board(cx);
        let root = div().size_full().flex().bg(colors.editor_background).text_color(colors.text);
        match &self.item {
            // The board keeps 42% of the tab, so another row is one click away.
            Some(open) => root
                .child(div().flex_none().w(relative(0.42)).min_w(px(380.)).h_full().child(board))
                .child(self.render_item(open, cx)),
            None => root.child(board),
        }
    }
}

impl WorkView {
    fn journal_toggle(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let open = self.journal.open;
        let border = if open { colors.border_focused } else { colors.border };
        button::outline(&self.hits, "work-journal", if open { "Runs" } else { "Journal" }, border, cx)
            .when(open, |d| d.bg(colors.element_selected))
            .child(hit_target(&self.hits, format!("work-journal={}", if open { "open" } else { "closed" })))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_journal(cx)))
            .into_any_element()
    }

    fn render_board(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let status = cx.theme().status().clone();
        let root = div()
            .id(SharedString::from(format!("work-{}", self.workspace)))
            .size_full()
            .flex()
            .flex_col()
            .overflow_y_scroll();
        let overview = match &self.shown {
            Shown::Runs(overview) => overview,
            Shown::Loading => {
                return root.child(div().p_4().text_color(colors.text_muted).child("Reading Workman…")).into_any_element();
            }
            Shown::Message(message) => {
                return root.child(div().p_4().text_color(colors.text_muted).child(SharedString::from(message.clone()))).into_any_element();
            }
        };
        let (connection, live) = self.hub.read(cx).connection();
        let header = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .child(div().font_weight(FontWeight::BOLD).child(SharedString::from(overview.item.qualified_ref.clone())))
            .child(clipped(format!("overview-title-{}", overview.item.qualified_ref), overview.item.title.clone(), false).flex_1())
            .child(rollup_bar(&overview.rollup, cx))
            .child(small(done_text(&overview.rollup), colors.text_muted))
            .child(small(connection, if live { status.success } else { status.warning }))
            .child(self.journal_toggle(cx));
        if self.journal.open {
            return div().size_full().flex().flex_col().child(header).child(self.render_journal(cx)).into_any_element();
        }
        let mut body = root.child(header);
        if overview.runs.is_empty() {
            body = body.child(div().px_3().py_2().text_color(colors.text_muted).child("This project has no runs."));
        }
        for run in &overview.runs {
            body = body.child(self.render_run(run, cx));
        }
        body.child(div().h(px(24.)).flex_none()).into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, kind: &str, qref: &str) -> ItemRef {
        ItemRef { id: id.into(), ref_: qref.into(), qualified_ref: qref.into(), kind: kind.into(), ..Default::default() }
    }

    #[test]
    fn the_workspace_is_the_one_project_among_the_matches() {
        let resolved = |m: Vec<ItemRef>| Resolved { matches: m, ..Default::default() };
        let p = pick_project(resolved(vec![item("r", "run", "w"), item("p", "project", "w")]), "w").unwrap();
        assert_eq!(p.id, "p");
        let none = pick_project(resolved(vec![item("r", "run", "w")]), "w").unwrap_err();
        assert!(none.contains("no project with ref 'w'"), "{none}");
        let two = pick_project(resolved(vec![item("a", "project", "w"), item("b", "project", "w")]), "w").unwrap_err();
        assert!(two.contains("names 2 Workman projects"), "{two}");
    }

    #[test]
    fn the_runs_own_row_is_the_header_not_a_child() {
        let row = |id: &str, depth: u32| BoardRow { item: item(id, "task", id), depth, ..Default::default() };
        let board = Board { run: item("run", "run", "13"), rows: vec![row("run", 0), row("t1", 1), row("t2", 2)], ..Default::default() };
        let ids: Vec<&str> = item_rows(&board).map(|r| r.item.id.as_str()).collect();
        assert_eq!(ids, ["t1", "t2"]);
    }

    #[test]
    fn an_event_reloads_a_board_only_when_it_is_shown() {
        let expanded: HashSet<String> = ["r1".to_string()].into();
        let event = |run: Option<&str>| StreamEvent { seq: 1, run_id: run.map(str::to_string), item_id: None, kind: "transition".into() };
        assert_eq!(board_to_reload(&event(Some("r1")), &expanded), Some("r1"));
        assert_eq!(board_to_reload(&event(Some("r2")), &expanded), None);
        assert_eq!(board_to_reload(&event(None), &expanded), None);
    }

    #[test]
    fn closed_runs_start_collapsed() {
        assert!(starts_expanded(ItemStatus::InProgress) && starts_expanded(ItemStatus::Open) && starts_expanded(ItemStatus::Blocked));
        assert!(!starts_expanded(ItemStatus::Done) && !starts_expanded(ItemStatus::Cancelled) && !starts_expanded(ItemStatus::Superseded));
    }

    #[test]
    fn small_formatters() {
        let r = Rollup { total: 14, done: 10, cancelled: 1, ..Default::default() };
        assert_eq!(done_text(&r), "10/13 done");
        assert_eq!(last_segment(r"P:\w\m4-1\"), "m4-1");
        assert_eq!(last_segment("P:/w/m4-1"), "m4-1");
        assert_eq!(status_key("m4/2", ItemStatus::InProgress), "work-status-m4/2=in_progress");
    }
}
