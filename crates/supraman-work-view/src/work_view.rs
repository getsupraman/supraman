//! Read only (C21): the token stays inside the hub's [`Client`]; no message here may carry it.

mod actions;
mod activity;
pub mod cards;
mod detail;
pub(crate) mod find;
mod inbox;
pub mod inbox_card;
mod item;
mod layout;
mod navigator;
pub(crate) mod overview;
mod projects;
mod prompts;
mod run_list;
pub(crate) mod state;
#[cfg(test)]
mod tests;

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    time::Duration,
};

use supraman_workman::{AuthStatus, Board, Client, ItemRef, Overview, OverviewProject, Stream, StreamEvent, StreamSignal};
use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, Global, KeyDownEvent, Render, ScrollHandle,
    SharedString, Subscription, Window, div, prelude::*, px,
};
use theme::ActiveTheme as _;

use supraman_ui::hit::{HitTargets, hit_target};
use supraman_ui::menu::ContextMenu;
use supraman_ui::placeholder::placeholder;
use supraman_ui::toast::ToastSlot;
use supraman_ui::open::Link;
use supraman_workman::status::WorkmanStatus;

pub(crate) use actions::RowAction;
pub(crate) use detail::DetailTab;
pub use inbox::{Binding, DISMISSED_FILE, HOUSEKEEPING_KEY, WorkInbox, dismiss_key_of, row_key_of};
pub use item::FleetLink;
pub(crate) use item::{ItemState, OpenItem};
pub use projects::RepoOf;
pub(crate) use state::Filter;

pub(crate) const DEBOUNCE: Duration = Duration::from_millis(300);

fn after_debounce<T: 'static>(cx: &mut Context<T>, then: impl FnOnce(&mut T, &mut Context<T>) + 'static) {
    cx.spawn(async move |this, cx| {
        cx.background_executor().timer(DEBOUNCE).await;
        this.update(cx, then).ok();
    })
    .detach();
}

pub(crate) enum Access<'a> {
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
    lost: Option<String>,
}

pub(crate) enum HubEvent {
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
            let mut hub = WorkHub { home, state: HubState::Starting, _stream: None, lost: None };
            hub.configure(cx);
            hub
        });
        cx.set_global(GlobalHub(hub.clone()));
        hub
    }

    /// A hub reading through `client` at once, with no stream: a test's.
    #[cfg(test)]
    pub(crate) fn ready(client: Client, cx: &mut App) -> Entity<WorkHub> {
        cx.new(|_| WorkHub { home: None, state: HubState::Ready(client), _stream: None, lost: None })
    }

    pub fn reconfigure(home: Option<PathBuf>, cx: &mut App) {
        if let Some(GlobalHub(hub)) = cx.try_global::<GlobalHub>() {
            hub.clone().update(cx, |hub, cx| {
                hub._stream = None;
                hub.lost = None;
                hub.configure(cx);
            });
        } else {
            Self::shared(home, cx);
        }
    }

    pub(crate) fn access(&self) -> Access<'_> {
        match &self.state {
            HubState::Starting => Access::Starting,
            HubState::Ready(client) => Access::Ready(client),
            HubState::Unconfigured(why) => Access::Unconfigured(why),
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
                if self.lost.take().is_some() {
                    cx.emit(HubEvent::Resync);
                }
                cx.emit(HubEvent::Connection);
            }
            StreamSignal::Event(event) => cx.emit(HubEvent::Changed(event)),
            StreamSignal::Lost(why) => {
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
    let auth = supraman_workman::Auth::shared(&home);
    let url = auth.configured_origin().map_err(|e| WorkmanStatus::NotSetUp { why: e.to_string() }.message().unwrap_or_default())?;
    match auth.status(&url) {
        AuthStatus::SignedIn { .. } => Ok(Client::with_source(url.as_str(), auth.bearer_for(&url))),
        AuthStatus::NotSignedIn => Err(WorkmanStatus::NotSignedIn { url }.message().unwrap_or_default()),
        AuthStatus::SignInAgain(why) => Err(WorkmanStatus::SignInAgain { url, why }.message().unwrap_or_default()),
    }
}

fn read_failed(e: supraman_workman::Error) -> String {
    match e {
        supraman_workman::Error::Transport { base, reason } => WorkmanStatus::Unreachable { url: base, error: reason }.message().unwrap_or_default(),
        supraman_workman::Error::Auth(why) => format!("{why}. Sign in again in Settings → Workman."),
        e => format!("Cannot read Workman: {e}"),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Shown {
    Loading,
    Message(String),
    Ready,
}

/// What the tab shows right of the navigator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Page {
    Overview,
    Activity,
    Run(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardState {
    Loading,
    Ready(Board),
    Failed(String),
}

pub enum WorkEvent {
    ItemSelected(String),
    /// Needs you was clicked: the Inbox panel is where it is answered.
    ShowInbox,
}

impl EventEmitter<WorkEvent> for WorkView {}

pub struct WorkView {
    pub workspace: String,
    folders: Vec<PathBuf>,
    repo_of: RepoOf,
    hub: Entity<WorkHub>,
    hits: HitTargets,
    pub(crate) shown: Shown,
    keys: Option<Vec<(&'static str, String)>>,
    overview: Option<Overview>,
    pub projects: Vec<OverviewProject>,
    pub(crate) page: Page,
    page_chosen: bool,
    /// [`WorkView::show_project`]'s project, landed on once the projects are bound.
    wanted: Option<String>,
    pub boards: HashMap<String, BoardState>,
    closed_open: HashSet<String>,
    pub selected: Option<String>,
    pub(crate) filter: Filter,
    pub item: Option<OpenItem>,
    pub(crate) detail_tab: DetailTab,
    pub(crate) prompts: prompts::Prompts,
    briefs: HashMap<String, Result<String, String>>,
    pub(crate) toast: ToastSlot,
    menu: Option<ContextMenu<RowAction>>,
    focus: FocusHandle,
    pub(crate) detail_focused: bool,
    pub(crate) layout: layout::Layout,
    link: FleetLink,
    pub(crate) open_link_with: fn(&Link, &mut App),
    inbox: Option<Entity<WorkInbox>>,
    overview_pending: bool,
    boards_pending: HashSet<String>,
    item_pending: bool,
    pub overview_reads: u64,
    pub board_reads: u64,
    pub item_reads: u64,
    list_scroll: ScrollHandle,
    detail_scroll: ScrollHandle,
    pub(crate) activity: activity::Activity,
    pub(crate) palette: Option<Entity<supraman_ui::palette::Palette<find::Found>>>,
    palette_sub: Option<Subscription>,
    pub(crate) search_reads: u64,
    /// What every relative time here is measured from: `now_ms`, or a fixture's fixed time.
    pub now: fn() -> i64,
}

pub struct Opening {
    pub workspace: String,
    pub folders: Vec<PathBuf>,
    pub home: Option<PathBuf>,
    pub hits: HitTargets,
    pub link: FleetLink,
    pub open_link_with: fn(&Link, &mut App),
    /// A folder's repo: `supraman_supervisor::git_repo`, or a fixture's.
    pub repo_of: RepoOf,
}

impl WorkView {
    pub fn open<T: 'static>(opening: Opening, cx: &mut Context<T>) -> Entity<WorkView> {
        let hub = WorkHub::shared(opening.home.clone(), cx);
        let inbox = WorkInbox::shared(opening.home.clone(), cx);
        cx.new(|cx| {
            let mut view = WorkView::new(opening, hub, cx);
            cx.observe(&inbox, |_, _, cx| cx.notify()).detach();
            view.inbox = Some(inbox);
            view
        })
    }

    fn new(opening: Opening, hub: Entity<WorkHub>, cx: &mut Context<Self>) -> Self {
        cx.subscribe(&hub, Self::on_hub).detach();
        let Opening { workspace, folders, home, hits, link, open_link_with, repo_of } = opening;
        let mut view = WorkView {
            workspace,
            folders,
            repo_of,
            hub,
            hits,
            shown: Shown::Loading,
            keys: None,
            overview: None,
            projects: Vec::new(),
            page: Page::Overview,
            page_chosen: false,
            wanted: None,
            boards: HashMap::new(),
            closed_open: HashSet::new(),
            selected: None,
            filter: Filter::Open,
            item: None,
            detail_tab: DetailTab::Overview,
            prompts: prompts::Prompts::default(),
            briefs: HashMap::new(),
            toast: ToastSlot::default(),
            menu: None,
            focus: cx.focus_handle(),
            detail_focused: false,
            layout: layout::Layout::default(),
            link,
            open_link_with,
            inbox: None,
            overview_pending: false,
            boards_pending: HashSet::new(),
            item_pending: false,
            overview_reads: 0,
            board_reads: 0,
            item_reads: 0,
            list_scroll: ScrollHandle::new(),
            detail_scroll: ScrollHandle::new(),
            activity: activity::Activity::default(),
            palette: None,
            palette_sub: None,
            search_reads: 0,
            now: supraman_core::time::now_ms,
        };
        view.layout.load(home, cx);
        view.read_keys(cx);
        view.read_overview(cx);
        view
    }

    /// A view over `hub`: a test's.
    #[cfg(test)]
    pub(crate) fn for_test(opening: Opening, hub: Entity<WorkHub>, cx: &mut Context<Self>) -> Self {
        Self::new(opening, hub, cx)
    }

    fn on_hub(&mut self, _: Entity<WorkHub>, event: &HubEvent, cx: &mut Context<Self>) {
        match event {
            HubEvent::Configured => {
                self.read_overview(cx);
                self.reload_shown(cx);
            }
            HubEvent::Changed(event) => {
                self.schedule_overview(cx);
                self.schedule_activity(cx);
                if let Some(run) = self.shown_run().filter(|run| event.run_id.as_deref() == Some(*run)).map(str::to_string) {
                    self.schedule_board(run.clone(), cx);
                    self.schedule_prompts(run, cx);
                }
                if self.item.as_ref().is_some_and(|open| item::item_to_reload(event, open)) {
                    self.schedule_item(cx);
                }
            }
            HubEvent::Resync => {
                self.read_overview(cx);
                self.reload_shown(cx);
                self.schedule_activity(cx);
            }
            HubEvent::Connection => cx.notify(),
        }
    }

    fn reload_shown(&mut self, cx: &mut Context<Self>) {
        if let Some(run) = self.shown_run().map(str::to_string) {
            self.read_board(run.clone(), cx);
            self.read_prompts(run, cx);
        }
        self.read_item(cx);
    }

    pub(crate) fn shown_run(&self) -> Option<&str> {
        match &self.page {
            Page::Run(id) => Some(id.as_str()),
            _ => None,
        }
    }

    pub(crate) fn board(&self) -> Option<&Board> {
        match self.boards.get(self.shown_run()?) {
            Some(BoardState::Ready(board)) => Some(board),
            _ => None,
        }
    }

    pub(crate) fn client(&mut self, cx: &mut Context<Self>) -> Option<Client> {
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

    pub(crate) fn web_base(&self, cx: &App) -> Option<String> {
        match self.hub.read(cx).access() {
            Access::Ready(client) => Some(supraman_core::url::origin(client.base())),
            _ => None,
        }
    }

    fn read_keys(&mut self, cx: &mut Context<Self>) {
        let (folders, repo_of) = (self.folders.clone(), self.repo_of);
        cx.spawn(async move |this, cx| {
            let keys = cx.background_executor().spawn(async move { projects::folder_keys(&folders, repo_of) }).await;
            this.update(cx, |this, cx| {
                this.keys = Some(keys);
                this.bind(cx);
            })
            .ok();
        })
        .detach();
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
        self.overview_reads += 1;
        cx.spawn(async move |this, cx| {
            let read = cx.background_executor().spawn(async move { client.overview().map_err(read_failed) }).await;
            this.update(cx, |this, cx| this.apply_overview(read, cx)).ok();
        })
        .detach();
    }

    fn apply_overview(&mut self, read: Result<Overview, String>, cx: &mut Context<Self>) {
        match read {
            Ok(overview) if self.overview.as_ref().is_some_and(|shown| overview.seq < shown.seq) => return,
            Ok(overview) => self.overview = Some(overview),
            Err(message) => {
                self.overview = None;
                self.shown = Shown::Message(message);
                cx.notify();
                return;
            }
        }
        self.bind(cx);
    }

    /// The projects bound to the folders' repos, once both the overview and the keys are read.
    fn bind(&mut self, cx: &mut Context<Self>) {
        let (Some(overview), Some(keys)) = (&self.overview, &self.keys) else { return };
        self.projects = projects::bound(overview, keys);
        self.shown = if self.projects.is_empty() { Shown::Message(projects::no_project_message(&self.workspace, keys)) } else { Shown::Ready };
        match self.wanted.take() {
            Some(project) => self.land_on(&project, cx),
            None => {
                let first = self.projects.iter().flat_map(projects::active).next().map(|r| r.item.id.clone());
                if let Some(run) = first.filter(|_| !self.page_chosen) {
                    self.open_run(run, cx);
                }
            }
        }
        self.read_page(cx);
        cx.notify();
    }

    fn read_board(&mut self, run: String, cx: &mut Context<Self>) {
        let Some(client) = self.client(cx) else { return };
        self.boards.entry(run.clone()).or_insert(BoardState::Loading);
        self.board_reads += 1;
        cx.spawn(async move |this, cx| {
            let id = run.clone();
            let read = cx.background_executor().spawn(async move { client.board(&id).map_err(|e| format!("Cannot read this run's board: {e}")) }).await;
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

    /// `project`'s first active run, else the Overview with its card; once the projects are bound.
    pub fn show_project(&mut self, project: String, cx: &mut Context<Self>) {
        self.wanted = Some(project);
        self.bind(cx);
    }

    fn land_on(&mut self, project: &str, cx: &mut Context<Self>) {
        let run = self.projects.iter().find(|p| p.item.id == project).and_then(|p| projects::active(p).next()).map(|r| r.item.id.clone());
        match run {
            Some(run) => self.open_run(run, cx),
            None => {
                self.page_chosen = true;
                self.page = Page::Overview;
                self.selected = None;
                self.item = None;
            }
        }
    }

    pub fn show_page(&mut self, page: Page, cx: &mut Context<Self>) {
        match page {
            Page::Run(run) => self.open_run(run, cx),
            page => {
                self.page_chosen = true;
                self.page = page;
                self.selected = None;
                self.item = None;
                self.list_scroll.set_offset(gpui::point(px(0.), px(0.)));
                self.read_page(cx);
                cx.notify();
            }
        }
    }

    fn read_page(&mut self, cx: &mut Context<Self>) {
        match self.page {
            Page::Overview => self.read_project_prompts(cx),
            Page::Activity if !self.projects.is_empty() => self.read_activity(cx),
            _ => {}
        }
    }

    pub(crate) fn open_run(&mut self, run: String, cx: &mut Context<Self>) {
        self.page_chosen = true;
        if self.shown_run() != Some(run.as_str()) {
            self.page = Page::Run(run.clone());
            self.filter = Filter::Open;
            self.selected = None;
            self.list_scroll.set_offset(gpui::point(px(0.), px(0.)));
            if !matches!(self.boards.get(&run), Some(BoardState::Ready(_))) {
                self.read_board(run.clone(), cx);
            }
            self.read_prompts(run.clone(), cx);
        }
        self.show_detail(run, cx);
    }

    pub(crate) fn toggle_closed(&mut self, project: &str, cx: &mut Context<Self>) {
        if !self.closed_open.remove(project) {
            self.closed_open.insert(project.to_string());
        }
        cx.notify();
    }

    pub(crate) fn set_filter(&mut self, filter: Filter, cx: &mut Context<Self>) {
        self.filter = filter;
        if self.selected.as_ref().is_some_and(|id| !self.listed().iter().any(|r| &r.item.id == id)) {
            self.clear_selection(cx);
        }
        cx.notify();
    }

    pub(crate) fn listed(&self) -> Vec<&supraman_workman::BoardRow> {
        self.board().map(|b| state::listed(b, self.filter)).unwrap_or_default()
    }

    pub fn select(&mut self, id: String, cx: &mut Context<Self>) {
        cx.emit(WorkEvent::ItemSelected(id.clone()));
        self.selected = Some(id.clone());
        self.show_detail(id, cx);
        cx.notify();
    }

    pub(crate) fn clear_selection(&mut self, cx: &mut Context<Self>) {
        self.selected = None;
        self.detail_focused = false;
        if let Some(run) = self.shown_run().map(str::to_string) {
            self.show_detail(run, cx);
        }
        cx.notify();
    }

    /// Esc: the list again from the detail, else no selection; whether it did anything.
    pub fn escape(&mut self, cx: &mut Context<Self>) -> bool {
        if self.menu.take().is_some() || self.prompts.collapse() {
            cx.notify();
            return true;
        }
        if self.detail_focused {
            self.detail_focused = false;
            cx.notify();
            return true;
        }
        if self.selected.is_some() {
            self.clear_selection(cx);
            return true;
        }
        false
    }

    /// Up/Down move in the list, Enter/Right give the detail the keys, Left takes them back.
    fn on_key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let m = &e.keystroke.modifiers;
        if m.control && !m.alt && !m.shift && e.keystroke.key == "p" && self.shown == Shown::Ready {
            self.open_palette(window, cx);
            cx.stop_propagation();
            return;
        }
        if m.modified() || self.palette.is_some() {
            return;
        }
        let handled = match e.keystroke.key.as_str() {
            "up" | "down" if !self.detail_focused => self.step(if e.keystroke.key == "up" { -1 } else { 1 }, cx),
            "up" | "down" => {
                let by = if e.keystroke.key == "up" { -40. } else { 40. };
                let offset = self.detail_scroll.offset();
                self.detail_scroll.set_offset(gpui::point(offset.x, (offset.y - px(by)).min(px(0.))));
                cx.notify();
                true
            }
            "enter" | "right" if self.selected.is_some() && !self.detail_focused => {
                self.detail_focused = true;
                cx.notify();
                true
            }
            "left" if self.detail_focused => {
                self.detail_focused = false;
                cx.notify();
                true
            }
            "escape" => self.escape(cx),
            _ => false,
        };
        if handled {
            cx.stop_propagation();
        }
    }

    fn step(&mut self, by: isize, cx: &mut Context<Self>) -> bool {
        let ids: Vec<String> = self.listed().iter().map(|r| r.item.id.clone()).collect();
        if ids.is_empty() {
            return false;
        }
        let at = self.selected.as_ref().and_then(|s| ids.iter().position(|id| id == s));
        let next = match at {
            Some(i) => (i as isize + by).clamp(0, ids.len() as isize - 1) as usize,
            None if by > 0 => 0,
            None => ids.len() - 1,
        };
        if at != Some(next) {
            self.select(ids[next].clone(), cx);
        }
        true
    }

    pub(crate) fn needs_you(&self, cx: &App) -> usize {
        self.inbox.as_ref().map_or(0, |inbox| inbox.read(cx).rows().iter().filter(|(w, _)| *w == self.workspace).count())
    }
}

impl Focusable for WorkView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for WorkView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors().clone();
        let body = match &self.shown {
            Shown::Loading => placeholder("Reading Workman…", Vec::<SharedString>::new(), cx).size_full().into_any_element(),
            Shown::Message(message) => placeholder("No work to show", [message.clone()], cx).size_full().px_6().into_any_element(),
            Shown::Ready => self.render_columns(cx),
        };
        let menu = self.menu.as_ref().map(|menu| {
            menu.render(
                &self.hits,
                |this: &mut WorkView, action, window, cx| {
                    this.menu = None;
                    this.run_action(action, window, cx);
                },
                |this: &mut WorkView, cx| {
                    this.menu = None;
                    cx.notify();
                },
                cx,
            )
        });
        div()
            .id(SharedString::from(format!("work-{}", self.workspace)))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .relative()
            .size_full()
            .flex()
            .bg(colors.editor_background)
            .text_color(colors.text)
            .child(body)
            .children(self.render_palette())
            .children(self.toast.render(&self.hits, "work-toast", cx))
            .children(menu)
    }
}

impl WorkView {
    /// A run page is list | detail; Overview and Activity take both columns.
    fn render_main(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Page::Run(run) = self.page.clone() else {
            let page = if self.page == Page::Activity { self.render_activity_page(cx) } else { self.render_overview(cx) };
            return div().relative().size_full().child(page).child(hit_target(&self.hits, "work-page".into())).into_any_element();
        };
        let list = div().flex_grow(1.).flex_basis(px(layout::LIST_MIN)).min_w_0().h_full().child(self.render_run_page(&run, cx));
        let detail = div().flex_shrink(1.).min_w_0().w(self.layout.detail()).h_full().relative().border_l_1().border_color(cx.theme().colors().border).child(self.render_detail(cx)).child(hit_target(&self.hits, "work-detail-column".into()));
        layout::detail_split()
            .render_second(list, detail, |this: &mut WorkView, width, cx| {
                this.layout.set_detail(width);
                cx.notify();
            }, |this: &mut WorkView, cx| this.layout.keep(cx), cx)
            .size_full()
            .into_any_element()
    }

    fn render_columns(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let main = self.render_main(cx);
        if self.layout.nav_shut() {
            let strip = div().flex_none().h_full().border_r_1().border_color(cx.theme().colors().border).child(self.render_nav_strip(cx));
            return div().flex().size_full().child(strip).child(div().flex_1().min_w_0().h_full().child(main)).into_any_element();
        }
        let nav = div().flex_shrink(0.5).min_w_0().overflow_hidden().w(self.layout.nav()).h_full().child(self.render_navigator(cx));
        let rest = div().flex_grow(1.).flex_basis(px(layout::LIST_MIN + layout::DETAIL_MIN + 1.)).min_w_0().h_full().child(main);
        layout::nav_split()
            .render(nav, rest, |this: &mut WorkView, width, cx| {
                this.layout.set_nav(width);
                cx.notify();
            }, |this: &mut WorkView, cx| this.layout.keep(cx), cx)
            .size_full()
            .into_any_element()
    }
}
