use std::rc::Rc;
use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

use supraman_claude::{Health, now_ms};
use gpui::{AnyElement, ClickEvent, Context, Div, FontWeight, Hsla, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, SharedString, Stateful, div, prelude::*, px, relative, rems};
use supraman_ui::surfaces::banner;
use supraman_ui::controls::button;
use supraman_ui::hit::{HitTargets, hit_target};
use supraman_ui::dnd;
use supraman_ui::tabs::DraggedSession;
use theme::ActiveTheme as _;

use supraman_ui::icon::Icon;
use supraman_ui::marks;
use supraman_ui::controls::tooltip::Tooltip;
use super::{Cli, FleetView, MenuTarget, TabKind, INBOX_MAX_SHARE};
use supraman_ui::clipped::clipped;
use crate::settings_window::page;
use crate::updates::Stage;
use crate::fleet::{self, EntryNode, FolderNode, Open, SessionRow};
use supraman_ui::label::{Label, LabelMark};

mod action_bar;
mod drag;
mod excluded;
pub(super) mod keys;
pub(super) mod fold;
pub(super) mod group_field;
pub(super) mod unlisted;
pub(super) mod other_schedules;
mod work_cards;

const REVEAL_WITHIN: Duration = Duration::from_secs(60);

/// A sidebar row to scroll into view once it exists: a row key of [`FleetView::set_left_rows`], or a tab's claude session.
pub(super) enum PendingReveal {
    Row(String),
    TabSession(u64),
}

#[derive(Clone, Copy)]
pub(super) enum LeftRow {
    Group(usize),
    Folder(usize, usize),
    /// A folder's Schedules, under the first row of its path.
    Schedules(usize, usize),
    Session(usize, usize, usize),
    More(usize, usize),
    /// Other sessions' row, after the entries; its sessions are `Session(e, 0, i)` with `e` past them ([`fleet::Tree::node`]).
    Unlisted,
    /// Other schedules: the schedules with no row of their folder, then Task Scheduler's tasks in no folder.
    Others,
    /// Add Group's new row, its name typed in place.
    NewGroup,
    /// Below the tree: a dragged folder dropped here goes to the top level's end.
    DropEnd,
}

const TREE_MIN_SHARE: f32 = 0.3;
/// The Inbox shrinks only once the notices are squeezed to nothing.
const NOTICES_SHRINK: f32 = 1000.;
pub const HOLD_STILL: Duration = Duration::from_secs(2);

/// Only these rows are drawn while the pointer moves, so a row arriving above never shifts what it aims at.
pub(crate) struct Hold {
    keys: HashSet<String>,
    moved: Instant,
}

impl Hold {
    fn new(keys: HashSet<String>, now: Instant) -> Self {
        Self { keys, moved: now }
    }

    fn shows(&self, key: &str) -> bool {
        self.keys.contains(key)
    }

    /// How long the pointer has been still, while short of [`HOLD_STILL`]; `None` releases the hold.
    fn still_for(&self, now: Instant) -> Option<Duration> {
        Some(now.duration_since(self.moved)).filter(|still| *still < HOLD_STILL)
    }
}

fn work_key(row: &crate::work_view::cards::Row) -> String {
    format!("work:{}/{}", row.project, row.row.key)
}

const SEAM_GAP: f32 = 6.;

pub(crate) use supraman_ui::themes::{DIMMED, QUIET};

pub(crate) fn quiet(cx: &gpui::App) -> Hsla {
    cx.theme().colors().text.opacity(QUIET)
}

pub(crate) fn secondary(cx: &gpui::App) -> Hsla {
    supraman_ui::themes::secondary(cx.theme().colors())
}

fn seam_color(cx: &gpui::App) -> Hsla {
    cx.theme().colors().text_muted.opacity(0.35)
}

fn icon_button(hits: &HitTargets, key: &'static str, icon: Icon, tip: impl Into<SharedString>, on: bool, cx: &gpui::App) -> button::Button {
    let colors = cx.theme().colors();
    tinted_button(hits, key, icon, tip, on, if on { colors.text } else { colors.text_muted })
}

fn tinted_button(hits: &HitTargets, key: &'static str, icon: Icon, tip: impl Into<SharedString>, on: bool, fg: Hsla) -> button::Button {
    button::Button::toolbar(hits, key).icon(icon.svg(16., fg)).on(on).text_color(fg).tooltip(Tooltip::text(tip))
}

/// Every top-level row's height: a group's, a top-level folder's, Other sessions'; no margin between them.
pub(crate) const TOP_ROW: f32 = 30.;
pub(super) const REPO_ROW: f32 = 26.;
pub(super) const SESSION_LINE: f32 = 24.;
/// One hover icon's place: always laid out, so a row's icons never shift.
const SLOT: f32 = 20.;
/// The widest an Other sessions row's folder name gets before it is cut.
const FOLDER_MAX: f32 = 120.;

fn slot() -> AnyElement {
    div().flex_none().size(px(SLOT)).into_any_element()
}

/// A child's icon starts where its parent's label does.
pub(crate) mod grid {
    use gpui::{Pixels, px};

    pub const DISCLOSURE: f32 = 16.;
    pub const ICON: f32 = 16.;
    pub const GAP: f32 = 8.;
    const BASE: f32 = 6.;
    pub const STEP: f32 = DISCLOSURE + GAP;
    pub const LABEL: f32 = DISCLOSURE + GAP + ICON + GAP;

    pub fn indent(level: usize) -> Pixels {
        px(BASE + level as f32 * STEP)
    }

    pub fn gap() -> Pixels {
        px(GAP)
    }
}

/// A row's chevron, open or shut, in its slot on the name's letters (none: the slot kept empty).
pub(super) fn disclosure(open: Option<bool>, color: Hsla) -> Label {
    let chevron = open.map(|open| LabelMark::Svg(if open { Icon::ChevronDown } else { Icon::ChevronRight }.svg(14., color)));
    Label::none().mark_if(chevron).slot(px(grid::DISCLOSURE)).flex_none()
}

/// A row's icon slot on its name's letters, painted under `tree-icon-<key>`.
pub(super) fn icon_slot(hits: &HitTargets, key: &str, icon: Option<AnyElement>) -> Label {
    let slot = div().relative().flex().flex_none().items_center().justify_center().size(px(grid::ICON)).children(icon).child(hit_target(hits, format!("tree-icon-{key}")));
    Label::none().mark(LabelMark::Element(slot.into_any_element())).flex_none()
}

/// A row's label, painted under `tree-label-<key>`.
/// Every top-level row's text size: a group's, a top-level folder's, Other sessions'.
pub(super) fn top_text<E: Styled>(row: E) -> E {
    row.text_base()
}

pub(super) fn tree_label<E: Styled + ParentElement>(hits: &HitTargets, key: &str, label: E) -> E {
    label.relative().child(hit_target(hits, format!("tree-label-{key}")))
}

fn glyph_button(hits: &HitTargets, key: String, icon: Icon, tip: impl Into<SharedString>, cx: &gpui::App) -> button::Button {
    button::Button::new(hits, key).icon(icon.svg(14., cx.theme().colors().text_muted)).icon_only(px(20.)).tooltip(Tooltip::text(tip))
}

impl FleetView {
    fn pointer_on_left(&mut self, cx: &mut Context<Self>) {
        if let Some(hold) = &mut self.sidebar.left_hold {
            hold.moved = Instant::now();
            return;
        }
        let keys: HashSet<String> = self.work_inbox.read(cx).rows().iter().map(work_key).collect();
        self.sidebar.left_hold = Some(Hold::new(keys, Instant::now()));
        self.sidebar.left_holds += 1;
        self.sidebar.left_hold_timer = Some(cx.spawn(async move |this, cx| {
            let mut wait = HOLD_STILL;
            loop {
                cx.background_executor().timer(wait).await;
                let still_for = this.update(cx, |this, cx| {
                    let still = this.sidebar.left_hold.as_ref().and_then(|h| h.still_for(Instant::now()));
                    if still.is_none() {
                        this.release_left(cx);
                    }
                    still
                });
                match still_for {
                    Ok(Some(still)) => wait = HOLD_STILL - still,
                    _ => break,
                }
            }
        }));
    }

    pub(super) fn release_left(&mut self, cx: &mut Context<Self>) {
        if self.sidebar.left_hold.take().is_some() {
            cx.notify();
        }
        self.sidebar.left_hold_timer = None;
    }

    #[cfg(feature = "selftest")]
    pub fn left_held(&self) -> bool {
        self.sidebar.left_hold.is_some()
    }

    fn shown_now(&self, key: &str) -> bool {
        self.sidebar.left_hold.as_ref().is_none_or(|h| h.shows(key))
    }

    pub fn collapse_all(&mut self, collapse: bool, cx: &mut Context<Self>) {
        if collapse {
            self.sidebar.collapsed.extend(fold::all(&self.tree()));
        } else {
            self.sidebar.collapsed = fold::expanded();
        }
        cx.notify();
    }

    pub(super) fn render_left(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let _stall = supraman_runtime::stall::section("render: left");
        let colors = cx.theme().colors().clone();
        let tree = self.tree();

        let header = self.render_action_bar(tree.hidden_stopped, cx);

        let notices = self.left_notices(&tree, cx);

        let body = self.left_body(&tree, cx);

        let toast = self.toast.render(&self.hits, "toast", cx);

        div()
            .id("left-panel")
            .relative()
            .flex()
            .flex_col()
            .w(self.sidebar.left_width)
            .flex_none()
            .h_full()
            .bg(colors.panel_background)
            .on_mouse_move(cx.listener(|this, _: &MouseMoveEvent, _, cx| this.pointer_on_left(cx)))
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                if !*hovered {
                    this.release_left(cx);
                }
            }))
            .child(header)
            .children(self.render_loading(cx))
            .children(self.render_inbox(cx))
            .when(!notices.is_empty(), |d| {
                d.child(
                    supraman_ui::scroll::area("notices")
                        .relative()
                        .child(hit_target(&self.hits, "notices-area".into()))
                        .flex()
                        .flex_col()
                        .flex_shrink(NOTICES_SHRINK)
                        .min_h_0()
                        .mb(px(SEAM_GAP))
                        .border_b_1()
                        .border_color(seam_color(cx))
                        .children(notices),
                )
            })
            .child(self.render_search_box())
            .child(body)
            .children(toast)
            .track_focus(&self.sidebar.focus)
            .key_context(group_field::KEY_CONTEXT)
            .on_action(cx.listener(Self::rename_selected))
            .on_key_down(cx.listener(Self::sidebar_key))
            .child(supraman_ui::focus::Pane::new("sidebar", &self.sidebar.focus).paint(&self.hits))
            .into_any_element()
    }

    fn left_notices(&self, tree: &fleet::Tree, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let status = cx.theme().status().clone();
        let mut notices: Vec<AnyElement> = Vec::new();
        let notice = |text: String, banner: Div| banner.mx_3().my_2().child(SharedString::from(text)).into_any_element();
        if let Cli::Failed(error) = &self.cli {
            notices.push(notice(
                format!("Cannot use the claude CLI, so no sessions are shown: {error}"),
                banner::tinted(status.error),
            ));
        }
        if let Some(error) = &self.listed.error {
            notices.push(notice(format!("Cannot read the folders: {error}"), banner::tinted(status.error)));
        }

        if let Some(error) = &self.listing.sessions_error {
            notices.push(notice(
                format!("Session list is stale, the last read failed (nothing is started or stopped meanwhile): {error}"),
                banner::tinted(status.warning),
            ));
        }
        let plain = supraman_ui::surfaces::notices::render(
            self.plain_notices(cx),
            status.warning,
            |key, cx| glyph_button(&self.hits, key, Icon::Close, "Dismiss", cx),
            |this: &mut Self, kind, text, cx| this.dismiss_notice(kind, text, cx),
            cx,
        );
        notices.extend(plain.into_iter().map(|n| n.mx_3().my_2().into_any_element()));
        if let Some(problem) = self.workman_notice() {
            notices.push(self.render_workman_notice(problem, cx));
        }
        notices.extend(self.render_update_notice(cx));
        if let Some(e) = &self.scheduling.other_scan_error {
            notices.push(notice(e.clone(), banner::tinted(status.warning)));
        }
        if self.listed.error.is_none() && self.polls.refreshes > 0 && tree.entries.is_empty() {
            notices.push(notice("No folders yet. Add one with the folder button above, or:  sm folder add <dir>".into(), banner::plain(cx)));
        }
        notices
    }

    fn left_body(&mut self, tree: &Rc<fleet::Tree>, cx: &mut Context<Self>) -> AnyElement {
        let query = self.search_query(cx);
        self.sidebar.search_marks.clear();
        if query.is_empty() {
            self.set_left_rows(tree, cx);
            let list = gpui::list(self.sidebar.left_list.state(), cx.processor(|this: &mut Self, ix: usize, window, cx| {
                this.sidebar.keys_held = this.sidebar.focus.contains_focused(window, cx);
                div().w_full().child(this.render_left_row(ix, cx)).into_any_element()
            })).size_full().pb_4();
            let area = div().id("fleet").relative().child(hit_target(&self.hits, "tree-area".into())).flex().flex_col().flex_1().min_h(relative(TREE_MIN_SHARE)).child(list).child(self.sidebar.left_list.scrollbar("fleet-bar"));
            self.tree_room(area, cx).into_any_element()
        } else {
            let body = supraman_ui::scroll::area("search-results")
                .relative()
                .child(hit_target(&self.hits, "tree-area".into()))
                .flex()
                .flex_col()
                .flex_1()
                .min_h(relative(TREE_MIN_SHARE))
                .pb_4();
            self.render_results(body, &query, cx).into_any_element()
        }
    }

    /// The sidebar's rows in order: a key each and the tokens its hit keys contain (supraman_ui::list::keyed_list).
    fn set_left_rows(&mut self, tree: &Rc<fleet::Tree>, cx: &gpui::App) {
        let mut rows = Vec::new();
        for (e, entry) in tree.entries.iter().enumerate() {
            let name = &entry.name;
            if entry.group {
                rows.push((LeftRow::Group(e), fold::group(name), vec![name.clone()]));
                if self.sidebar.collapsed.contains(&fold::group(name)) {
                    continue;
                }
            }
            for (f, folder) in entry.folders.iter().enumerate() {
                let key = &folder.key;
                if folder.parent.as_ref().is_some_and(|p| self.sidebar.collapsed.contains(&fold::folder(p))) {
                    continue;
                }
                rows.push((LeftRow::Folder(e, f), fold::folder(key), vec![key.clone()]));
                if self.sidebar.collapsed.contains(&fold::folder(key)) {
                    continue;
                }
                let fkey = supraman_core::schedule::folder_key(&folder.path);
                if tree.schedule_row(&fkey) == Some((e, f)) {
                    let tokens = ["schedules-row-", "sched-row-", "sched-state-", "sched-error-", "sched-approve-", "sched-run-", "others-row-"].iter().map(|p| format!("{p}{fkey}")).collect();
                    rows.push((LeftRow::Schedules(e, f), format!("sched:{fkey}"), tokens));
                }
                session_rows(&mut rows, e, f, folder);
            }
        }
        if self.adding_group(cx) {
            rows.push((LeftRow::NewGroup, group_field::NEW_ROW.into(), vec![crate::view::folder_edits::FIELD.to_string()]));
        }
        if let Some(node) = &tree.unlisted {
            rows.push((LeftRow::Unlisted, unlisted::KEY.into(), unlisted::tokens()));
            if !self.sidebar.collapsed.contains(unlisted::KEY) {
                session_rows(&mut rows, tree.entries.len(), 0, &node.folders[0]);
            }
        }
        rows.push((LeftRow::Others, other_schedules::KEY.into(), self.other_schedules_tokens(tree)));
        rows.push((LeftRow::DropEnd, "drop-end".into(), vec![drag::END.to_string()]));
        self.sidebar.left_list.set_rows(rows.iter().map(|(_, key, tokens)| (SharedString::from(key.clone()), tokens.clone())).collect());
        self.sidebar.left_rows = Rc::new((tree.clone(), rows.into_iter().map(|(row, ..)| row).collect()));
        if let Some((pending, since)) = &self.sidebar.reveal_pending {
            let shown = match pending {
                PendingReveal::Row(key) => self.sidebar.left_list.reveal_where(|k| k == key),
                PendingReveal::TabSession(tab) => match self.tab(*tab).and_then(|t| t.claude.clone()) {
                    Some(sid) => self.sidebar.left_list.reveal_where(|k| k.ends_with(&format!("/{sid}"))),
                    None => self.tab(*tab).is_none(),
                },
            };
            if shown || since.elapsed() > REVEAL_WITHIN {
                self.sidebar.reveal_pending = None;
            }
        }
    }

    /// The row of what the user just made, scrolled into view once the tree holds it.
    pub(super) fn reveal_new(&mut self, pending: PendingReveal) {
        self.sidebar.reveal_pending = Some((pending, Instant::now()));
    }

    #[cfg(feature = "selftest")]
    pub fn reveal_hit(&self, hit: &str) -> bool {
        self.sidebar.left_list.reveal(hit)
    }

    /// A selftest's look at a sidebar key: painted, or false after the row that claims it was scrolled to.
    #[cfg(feature = "selftest")]
    pub fn shown(&self, key: &str) -> bool {
        if self.hits.borrow().contains_key(key) {
            return true;
        }
        if self.reveal_hit(key) {
            crate::selftest_seams::ask_frame();
        }
        false
    }

    #[cfg(feature = "selftest")]
    pub fn unlisted_open(&self) -> bool {
        !self.sidebar.collapsed.contains(unlisted::KEY)
    }

    /// How many folder rows of the entry `name` the sidebar holds, in view or not.
    #[cfg(feature = "selftest")]
    pub fn folder_rows(&self, name: &str) -> usize {
        let (tree, rows) = &*self.sidebar.left_rows;
        rows.iter().filter(|r| matches!(r, LeftRow::Folder(e, _) if tree.entries[*e].name == name)).count()
    }

    #[cfg(feature = "selftest")]
    pub fn left_list(&self) -> &supraman_ui::list::keyed_list::KeyedList {
        &self.sidebar.left_list
    }

    fn render_left_row(&mut self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let rows = self.sidebar.left_rows.clone();
        let (tree, rows) = (&rows.0, &rows.1);
        let Some(row) = rows.get(ix) else { return div().into_any_element() };
        match *row {
            LeftRow::Group(e) => {
                let entry = &tree.entries[e];
                let open = !self.sidebar.collapsed.contains(&fold::group(&entry.name));
                self.render_group_row(entry, open, cx)
            }
            LeftRow::Schedules(e, f) => {
                let entry = &tree.entries[e];
                div().flex().flex_col().children(self.render_schedules(entry, &entry.folders[f], folder_level(entry, &entry.folders[f]) + 1, cx)).into_any_element()
            }
            LeftRow::Folder(e, f) => {
                let (entry, folder) = (&tree.entries[e], &tree.entries[e].folders[f]);
                if folder.excluded {
                    return self.render_excluded_row(entry, folder, cx);
                }
                let open = !self.sidebar.collapsed.contains(&fold::folder(&folder.key));
                self.render_folder_row(entry, folder, open, cx)
            }
            LeftRow::Session(e, f, i) => {
                let (entry, folder) = (tree.node(e), &tree.node(e).folders[f]);
                let row = &folder.sessions[i];
                let key = session_key(folder, row);
                self.render_session(&entry.name, &folder.row_place(row), row, Some(&key), grid::indent(folder_level(entry, folder) + 1), cx)
            }
            LeftRow::More(e, f) => self.render_more_closed(tree.node(e), &tree.node(e).folders[f], cx),
            LeftRow::Unlisted => self.render_unlisted_row(!self.sidebar.collapsed.contains(unlisted::KEY), cx),
            LeftRow::Others => div().flex().flex_col().children(self.render_other_schedules(tree, cx)).into_any_element(),
            LeftRow::NewGroup => self.render_new_group_row(cx),
            LeftRow::DropEnd => self.render_drop_end(cx),
        }
    }

    fn render_more_closed(&self, entry: &EntryNode, repo: &FolderNode, cx: &mut Context<Self>) -> AnyElement {
        let key = repo.key.clone();
        let hit = format!("more-{key}");
        self.sidebar_row(button::RowKeys::one(hit.clone().into()), Some(&format!("more:{key}")), cx)
            .flex()
            .flex_none()
            .items_center()
            .h(px(SESSION_LINE))
            .pl(grid::indent(folder_level(entry, repo) + 1) + px(grid::LABEL))
            .text_xs()
            .text_color(secondary(cx))
            .child(SharedString::from(format!("{} more closed…", repo.more)))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.sidebar.closed_expanded.insert(key.clone());
                this.invalidate_tree();
                cx.notify();
            }))
            .into_any_element()
    }

    fn render_loading(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let pending = self.loading(cx);
        if pending.is_empty() {
            return None;
        }
        Some(
            div()
                .relative()
                .flex_none()
                .px_3()
                .py(px(2.))
                .border_b_1()
                .border_color(cx.theme().colors().border)
                .text_xs()
                .text_color(secondary(cx))
                .child(hit_target(&self.hits, "loading-sessions".into()))
                .child(SharedString::from(format!("Loading sessions… {}", pending.join(", "))))
                .into_any_element(),
        )
    }

    fn render_inbox(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let all_work = self.work_inbox.read(cx).rows();
        let total = all_work.len();
        let work_rows: Vec<_> = all_work.into_iter().filter(|r| self.shown_now(&work_key(r))).collect();
        let scheduled = self.render_schedule_inbox(cx);
        if work_rows.is_empty() && scheduled.is_empty() {
            return None;
        }
        let waiting = total - work_rows.len();
        let total = total + scheduled.len();
        let colors = cx.theme().colors().clone();
        let (scroll, flashing) = { let w = self.work_inbox.read(cx); (w.scroll.clone(), w.flashing()) };
        let mut inbox = supraman_ui::scroll::area("inbox")
            .track(&scroll)
            .relative()
            .when(flashing, |d| d.bg(colors.element_selected))
            .child(hit_target(&self.hits, if flashing { "inbox-area=flash".into() } else { "inbox-area".into() }))
            .flex()
            .flex_col()
            .min_h_0()
            .max_h(relative(INBOX_MAX_SHARE))
            .gap_1()
            .px_3()
            .py_2()
            .mb(px(SEAM_GAP))
            .border_b_1()
            .border_color(seam_color(cx))
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::BOLD)
                    .text_color(colors.text_muted)
                    .child(SharedString::from(match waiting {
                        0 => format!("INBOX · {total}"),
                        n => format!("INBOX · {total} · {n} new, shown when the pointer leaves"),
                    })),
            );
        let (work_needs, housekeeping) = self.render_work_cards(&work_rows, cx);
        inbox = inbox.children(work_needs).children(scheduled).children(housekeeping);
        Some(inbox.into_any_element())
    }

    fn render_workman_notice(&self, problem: String, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let status = cx.theme().status().clone();
        let color = if self.workman.unreachable() { status.error } else { status.warning };
        let mut actions = div().flex().flex_wrap().gap_2().pt_1();
        if self.workman.signed_out() {
            actions = match self.workman_state.signing_in {
                Some(_) => actions.child(button::Button::new(&self.hits, "ws-setup-cancel").label("Cancel sign-in").on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        this.cancel_sign_in();
                    },
                ))),
                None => actions.child(button::Button::new(&self.hits, "ws-setup-signin").label("Sign in").on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        this.sign_in(cx);
                    },
                ))),
            };
        }
        if self.workman.unreachable() {
            actions = actions.child(button::Button::new(&self.hits, "ws-setup-retry").label("Retry").on_click(cx.listener(
                |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.recheck_workman();
                },
            )));
        }
        actions = actions.child(button::Button::new(&self.hits, "ws-setup-settings").label("Settings…").on_click(cx.listener(
            |this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.open_settings_at(Some(page::WORKMAN), cx);
            },
        )));
        let dismiss = glyph_button(&self.hits, "ws-setup-dismiss".into(), Icon::Close, "Hide until the problem changes", cx)
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.dismiss_workman_notice(cx);
            }));
        banner::tinted(color)
            .id("ws-setup-notice")
            .relative()
            .mx_3()
            .my_2()
            .child(banner::dismissable(
                banner::marked(crate::brand::workman(rems(1.), color), SharedString::from(problem)),
                dismiss,
            ))
            .children(self.workman.fix().map(|fix| div().text_xs().text_color(colors.text).child(SharedString::from(fix))))
            .children(self.workman_state.signing_in.as_ref().map(|url| {
                div().text_xs().text_color(colors.text).child(SharedString::from(format!("Waiting for the sign-in to {url} in your browser…")))
            }))
            .children(self.workman_state.sign_in_error.clone().map(|e| div().text_xs().text_color(status.error).child(SharedString::from(e))))
            .child(actions)
            .child(hit_target(&self.hits, "ws-setup-notice".into()))
            .into_any_element()
    }

    fn render_update_notice(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let version = self.updates.offered()?.version().to_string();
        let status = cx.theme().status().clone();
        let stage = self.updates.stage.clone();
        let failed = matches!(stage, Stage::Failed(_));
        let dismiss = glyph_button(&self.hits, "update-dismiss".into(), Icon::Close, "Hide until a newer release", cx)
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.dismiss_update(cx);
            }));
        let mut actions = div().flex().flex_wrap().gap_2().pt_2();
        match stage {
            Stage::Downloading { .. } => {
                actions = actions.child(button::Button::new(&self.hits, "update-cancel").label("Cancel").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.cancel_update(cx);
                })));
            }
            Stage::Applying => {}
            Stage::Idle | Stage::Failed(_) => {
                actions = actions
                    .child(button::Button::new(&self.hits, "update-notes").label("What's new").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        this.update_notes(cx);
                    })))
                    .child(button::Button::new(&self.hits, "update-install").label("Update").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        this.run_update(cx);
                    })));
            }
        }
        let line = self.updates.stage_line().map(|line| {
            div()
                .id("update-stage")
                .relative()
                .pt_1()
                .text_xs()
                .when(failed, |d| d.text_color(status.error))
                .child(SharedString::from(line))
                .child(hit_target(&self.hits, "update-stage".into()))
        });
        Some(
            banner::tinted(if failed { status.error } else { status.info })
                .id("update-notice")
                .relative()
                .mx_3()
                .my_2()
                .child(banner::dismissable(SharedString::from(format!("Supraman {version} is available")), dismiss))
                .children(line)
                .child(actions)
                .child(hit_target(&self.hits, "update-notice".into()))
                .into_any_element(),
        )
    }

    pub(super) fn hover_icon(&self, key: String, icon: Icon, tip: &'static str, group: &SharedString, shown: bool, cx: &gpui::App) -> button::Button {
        button::Button::new(&self.hits, key).icon(icon.svg(14., cx.theme().colors().text_muted)).icon_only(px(SLOT)).reveal(group.clone(), shown).tooltip(Tooltip::text(tip))
    }

    fn entry_selected(&self, entry: &EntryNode, cx: &gpui::App) -> bool {
        let Some(tab) = self.active.and_then(|id| self.tab(id)) else { return false };
        match &tab.kind {
            TabKind::Work(view) => super::work_tab::entry_target(entry).as_ref() == Some(&view.read(cx).target),
            _ => false,
        }
    }

    fn render_group_row(&self, entry: &EntryNode, open: bool, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let name = entry.name.clone();
        let pkey = fold::group(&name);
        let group = SharedString::from(format!("group-hover-{name}"));
        let shown = self.entry_selected(entry, cx);
        let editing = self.group_name_field(Some(&name), cx);
        let add = {
            let name = name.clone();
            self.hover_icon(format!("group-add-folder-{name}"), Icon::FolderAdd, "Add Folder… to this group", &group, false, cx).on_click(cx.listener(
                move |this, _: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    this.add_folder(Some(name.clone()), window, cx);
                },
            ))
        };
        let buttons = div().flex().flex_none().items_center().gap(px(2.)).child(add).children(self.work_button(entry, &group, shown, cx)).child(self.task_button(entry, &group, shown, cx));
        let editing_now = editing.is_some();
        let label = editing.unwrap_or_else(|| {
            tree_label(&self.hits, &format!("group-{name}"), div().flex_none().font_weight(FontWeight::BOLD).whitespace_nowrap().child(SharedString::from(name.clone()))).into_any_element()
        });
        let line = div()
            .flex()
            .items_center()
            .gap(grid::gap())
            .h(px(TOP_ROW))
            .child(disclosure(Some(open), colors.text_muted))
            .child(icon_slot(&self.hits, &format!("group-{name}"), Some(Icon::Group.slotted(grid::ICON, colors.text_muted).into_any_element())))
            .child(label)
            .when(!editing_now, |d| d.child(div().flex_1()).child(buttons));
        let keys = button::RowKeys { id: pkey.clone().into(), group, hit: format!("group-row-{name}").into() };
        let row = self
            .sidebar_row(keys, Some(&pkey), cx)
            .flex()
            .flex_col()
            .flex_none()
            .pl(grid::indent(0))
            .pr_2()
            .map(top_text)
            .child(line);
        if editing_now {
            return row.into_any_element();
        }
        let selected = SharedString::from(pkey.clone());
        self.group_drag(row, &name, cx)
            .on_mouse_down(MouseButton::Right, {
                let name = name.clone();
                cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.sidebar.left_list.select_key(Some(selected.clone()));
                    this.open_menu(MenuTarget::Group(name.clone()), Some(format!("group-row-{name}")), e.position, cx);
                })
            })
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle(pkey.clone(), cx)))
            .into_any_element()
    }

    /// An entry's Work, on its group's row or its top-level folder's; a folder's dimmed with why while it binds no project.
    fn work_button(&self, entry: &EntryNode, group: &SharedString, shown: bool, cx: &mut Context<Self>) -> Option<button::Button> {
        let target = super::work_tab::entry_target(entry)?;
        let why = target.folder().and_then(|f| self.work_why_not(f, cx));
        let key = format!("work-{}", entry.name);
        let Some(why) = why else {
            let tip = if entry.group { "Work: the Workman board of its folders' projects" } else { "Work: the Workman board of its project" };
            return Some(self.hover_icon(key, Icon::Work, tip, group, shown, cx).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.open_work(target.clone(), cx);
            })));
        };
        let icon = Icon::Work.svg(14., cx.theme().colors().text_disabled);
        Some(button::Button::new(&self.hits, key).icon(icon).icon_only(px(SLOT)).reveal(group.clone(), shown).cursor_default().tooltip(Tooltip::text(why)).on_click(|_: &ClickEvent, _, cx| cx.stop_propagation()))
    }

    /// A group's New task…, its folder picked on the Start page.
    fn task_button(&self, entry: &EntryNode, group: &SharedString, shown: bool, cx: &mut Context<Self>) -> button::Button {
        let name = entry.name.clone();
        self.hover_icon(format!("task-{name}"), Icon::Task, "New task…: start a session with a first prompt", group, shown, cx).on_click(cx.listener(
            move |this, _: &ClickEvent, window, cx| {
                cx.stop_propagation();
                this.open_start(&name, None, false, window, cx);
            },
        ))
    }

    fn render_folder_row(&self, entry: &EntryNode, repo: &FolderNode, open: bool, cx: &mut Context<Self>) -> AnyElement {
        let key = &repo.key;
        let rkey = fold::folder(key);
        let tree_key = format!("repo-{key}");
        let group = SharedString::from(format!("repo-group-{key}"));
        let children = entry.folders.iter().any(|f| f.parent.as_ref() == Some(key));
        let empty = repo.sessions.is_empty() && !children;
        let git = repo.kind == supraman_core::FolderKind::Git;
        let branch = repo.branch.clone().filter(|_| git).unwrap_or_default();
        let place = repo.place();
        let new_claude = {
            let (cwd, title) = (repo.path.clone(), format!("{place} · claude"));
            self.hover_icon(format!("new-claude-{key}"), Icon::Plus, "New claude session in this folder", &group, false, cx).on_click(cx.listener(
                move |this, _: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    this.new_claude(cwd.clone(), title.clone(), window, cx);
                },
            ))
        };
        let terminal = {
            let (cwd, title) = (repo.path.clone(), format!("{place} · pwsh"));
            self.hover_icon(format!("terminal-{key}"), Icon::Terminal, "Open a terminal here", &group, false, cx).on_click(cx.listener(
                move |this, _: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_shell_in(cwd.clone(), title.clone(), window, cx);
                },
            ))
        };
        let history = git.then(|| {
            let (folder, title) = (repo.path.clone(), format!("Git · {}", repo.name));
            self.hover_icon(format!("git-graph-{key}"), Icon::GitGraph, "Show this folder's commit graph", &group, false, cx).on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.open_git(folder.clone(), title.clone(), cx);
                },
            ))
        });
        let top = !entry.group && repo.listed();
        let work = top.then(|| self.work_button(entry, &group, self.entry_selected(entry, cx), cx)).flatten();
        let keys = button::RowKeys { id: rkey.clone().into(), group, hit: format!("repo-row-{key}").into() };
        let row = self
            .sidebar_row(keys, Some(&rkey), cx)
            .flex()
            .flex_none()
            .items_center()
            .gap(grid::gap())
            .h(px(if top { TOP_ROW } else { REPO_ROW }))
            .pl(grid::indent(folder_level(entry, repo)))
            .pr_2()
            .map(|d| if top { top_text(d) } else { d.text_sm() });
        self.folder_drag(row, repo, cx)
            .when(empty, |d| d.text_color(quiet(cx)))
            .child(disclosure((!empty).then_some(open), secondary(cx)))
            .child(icon_slot(&self.hits, &tree_key, Some(if git { Icon::GitFolder } else { Icon::Folder }.slotted(grid::ICON, secondary(cx)).into_any_element())))
            .child(tree_label(&self.hits, &tree_key, div().flex_none().whitespace_nowrap().when(top, |d| d.font_weight(FontWeight::BOLD)).child(SharedString::from(repo.name.clone()))))
            .child(clipped(format!("branch-{key}"), branch, false).text_xs().text_color(secondary(cx)).child(hit_target(&self.hits, format!("branch-{key}"))))
            .child(div().relative().flex_1().child(hit_target(&self.hits, format!("repo-room-{key}"))))
            .child(div().flex().flex_none().items_center().gap(px(2.)).children(history).child(new_claude).child(terminal).children(work))
            .on_mouse_down(MouseButton::Right, {
                let key = key.clone();
                cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.open_menu(MenuTarget::Folder(key.clone()), Some(format!("repo-row-{key}")), e.position, cx);
                })
            })
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle(rkey.clone(), cx)))
            .into_any_element()
    }

    pub(super) fn render_session(
        &self,
        entry: &str,
        place: &str,
        row: &SessionRow,
        row_key: Option<&str>,
        indent: Pixels,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let status = cx.theme().status().clone();
        let s = &row.session;
        let at = row.scope.clone();
        let foreign = row.foreign;
        let blocked = !foreign && matches!(row.health, Health::Blocked { .. });
        let dimmed = foreign || matches!(row.health, Health::Orphaned { .. } | Health::Stopped);
        let waiting_for = match &row.health {
            Health::Blocked { waiting_for: Some(w) } => Some(format!("waiting for: {w}")),
            _ => None,
        };
        let hook = row
            .hook
            .as_ref()
            .filter(|_| s.name.starts_with(supraman_core::FM_PREFIX))
            .map(|h| format!("hook: {}", h.label(now_ms())));
        let shown = self.is_marked(s);
        let group = SharedString::from(format!("session-group-{}{at}", s.session_id));
        let line = self.session_line(place, (entry == fleet::UNLISTED).then_some(place), row, cx);

        let (title, clicked) = (place.replace('/', " · "), row.clone());
        let target = MenuTarget::Session { entry: entry.to_string(), place: place.to_string(), session_id: s.session_id.clone() };
        let tint = status.warning.opacity(0.12);
        let anchor = format!("session-row-{}{at}", s.session_id);
        let keys = button::RowKeys {
            id: format!("session-{}{at}", s.id).into(),
            group,
            hit: format!("session-row-{}{at}", s.session_id).into(),
        };
        let band = if blocked { Some(tint) } else { shown.then_some(colors.ghost_element_selected) };
        let chrome = supraman_ui::list::Chrome::new().band(band).hover_fill(if blocked && !shown { status.warning.opacity(0.18) } else { colors.ghost_element_hover });
        self.sidebar_row_with(keys, row_key, chrome, cx)
            .flex()
            .flex_none()
            .flex_col()
            .pl(indent)
            .pr_2()
            .py(px(1.))
            .text_sm()
            .when(shown, |d| d.border_l_2().border_color(colors.border_focused).pl(indent - px(2.)))
            .when(dimmed, |d| d.opacity(DIMMED))
            .child(line)
            .when_some(waiting_for, |d, w| {
                d.child(
                    clipped(format!("waiting-{}{at}", s.id), w, false)
                        .pl(px(grid::LABEL))
                        .text_xs()
                        .text_color(if foreign { secondary(cx) } else { status.warning }),
                )
            })
            .when_some(hook, |d, h| {
                d.child(clipped(format!("hook-{}{at}", s.id), h, false).pl(px(grid::LABEL)).text_xs().text_color(secondary(cx)))
            })
            .map(|d| dnd::source(d, format!("session-row-{}{at}", s.session_id), DraggedSession { session_id: s.session_id.clone(), title: fleet::display_name(s).to_string().into() }, cx))
            .when(shown, |d| d.child(hit_target(&self.hits, format!("marked-{}{at}", s.session_id))))
            .on_click(cx.listener(move |this, e: &ClickEvent, _, cx| {
                if e.click_count() == 2 {
                    this.open_row(&title, &clicked, cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.open_menu(target.clone(), Some(anchor.clone()), e.position, cx);
                }),
            )
            .into_any_element()
    }

    /// `folder`: the row's own folder's name, shown where a listed row's folder is its parent row.
    /// A row's own folder after its name, muted and cut (Other sessions' and Other schedules' rows); `path` its tooltip.
    pub(super) fn folder_tag(&self, key: String, name: String, path: Option<String>, cx: &mut Context<Self>) -> Stateful<Div> {
        let tag = match path {
            Some(path) => supraman_ui::clipped::clipped_with(key.clone(), name, false, false).tooltip(Tooltip::text(path)),
            None => clipped(key.clone(), name, false),
        };
        tag.min_w_0().max_w(px(FOLDER_MAX)).text_xs().text_color(secondary(cx)).child(hit_target(&self.hits, key))
    }

    fn session_line(&self, place: &str, folder: Option<&str>, row: &SessionRow, cx: &mut Context<Self>) -> Div {
        let colors = cx.theme().colors().clone();
        let s = &row.session;
        let at = row.scope.clone();
        let state = fleet::State::of_row(row);
        let mark = if row.foreign { state.mark(cx).in_color(colors.text_muted) } else { state.mark(cx) };
        let word_color = if row.foreign || !matches!(state, fleet::State::Live(_)) { secondary(cx) } else { mark.color() };
        let marks = self.sidebar.search_marks.get(&s.session_id);
        let tree_key = format!("session-{}{at}", s.session_id);
        let star = self.session_star(row, &at, cx);
        let action = self.session_open(place, row, cx);
        div()
            .flex()
            .items_center()
            .gap(grid::gap())
            .h(px(SESSION_LINE))
            .child(disclosure(None, colors.text_muted))
            .child(icon_slot(&self.hits, &tree_key, Some(marks::dot(mark).into_any_element())))
            .child(tree_label(&self.hits, &tree_key, self.session_name(row, &at, marks, cx)))
            .when_some(marks.and_then(|m| m.id.clone()), |d, id| {
                let key = format!("session-id-{}{at}", s.id);
                d.child(
                    supraman_ui::matcher::highlighted(key.clone(), fleet::short_id(s).to_string(), &[id], cx)
                        .relative()
                        .flex_none()
                        .text_xs()
                        .text_color(secondary(cx))
                        .child(hit_target(&self.hits, key)),
                )
            })
            .when_some(folder, |d, folder| d.child(self.folder_tag(format!("session-folder-{}{at}", s.session_id), folder.to_string(), None, cx)))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .whitespace_nowrap()
                    .text_xs()
                    .text_color(secondary(cx))
                    .child(SharedString::from(format!("{} · ", s.kind.as_str())))
                    .child(div().text_color(word_color).child(SharedString::from(state.word()))),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(2.))
                    .child(star)
                    .child(action.map_or_else(slot, IntoElement::into_any_element)),
            )
    }

    fn session_name(&self, row: &SessionRow, at: &str, marks: Option<&crate::search::Marks>, cx: &mut Context<Self>) -> Stateful<Div> {
        let s = &row.session;
        let name = fleet::display_name(s).to_string();
        let name_el = match marks {
            Some(m) if !s.name.is_empty() => supraman_ui::matcher::highlighted(format!("session-name-{}{at}", s.id), name, &m.name, cx),
            _ => clipped(format!("session-name-{}{at}", s.id), name, false),
        };
        if row.foreign {
            let key = format!("foreign-{}{at}", s.session_id);
            div()
                .id(SharedString::from(key.clone()))
                .relative()
                .flex()
                .flex_1()
                .min_w_0()
                .child(name_el)
                .child(hit_target(&self.hits, key))
                .tooltip(Tooltip::text("another Supraman session"))
        } else {
            name_el.flex_1()
        }
    }

    fn session_star(&self, row: &SessionRow, at: &str, cx: &mut Context<Self>) -> button::Button {
        let (colors, status) = (cx.theme().colors().clone(), cx.theme().status().clone());
        let s = &row.session;
        let group = SharedString::from(format!("session-group-{}{at}", s.session_id));
        let session_id = s.session_id.clone();
        let (icon, fg, tip) = if row.starred {
            (Icon::StarFilled, status.warning, "Unstar: shown only while it runs or stopped sessions are shown")
        } else {
            (Icon::Star, colors.text_muted, "Star: always shown, stopped or closed, however old")
        };
        button::Button::new(&self.hits, format!("star-{}{at}", s.session_id)).icon(icon.svg(13., fg)).icon_only(px(SLOT)).reveal(group, self.is_marked(s) || row.starred)
            .tooltip(Tooltip::text(tip))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.toggle_star(&session_id, cx);
            }))
    }

    fn session_open(&self, place: &str, row: &SessionRow, cx: &mut Context<Self>) -> Option<button::Button> {
        let s = &row.session;
        let at = row.scope.clone();
        let group = SharedString::from(format!("session-group-{}{at}", s.session_id));
        let open = self.open_of(row);
        let key = match open {
            Open::Tab(_) => "tab-link",
            Open::Attach => "attach",
            Open::Resume => "resume",
            _ => return None,
        };
        // open in a tab not shown now: a state, so it shows without a hover
        let shown = matches!(open, Open::Tab(_)) || self.is_marked(s);
        let (title, row) = (place.replace('/', " · "), row.clone());
        Some(self.hover_icon(format!("{key}-{}{at}", s.id), open.icon(), open.hint(), &group, shown, cx).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            cx.stop_propagation();
            this.open_row(&title, &row, cx);
        })))
    }
}

/// A session row's key in the sidebar's rows.
fn session_key(folder: &FolderNode, row: &SessionRow) -> String {
    format!("s:{}/{}{}", folder.key, row.session.session_id, row.scope)
}

/// A folder's session rows and its More, keyed by its key.
fn session_rows(rows: &mut Vec<(LeftRow, String, Vec<String>)>, e: usize, f: usize, folder: &FolderNode) {
    let key = &folder.key;
    for (i, row) in folder.sessions.iter().enumerate() {
        rows.push((LeftRow::Session(e, f, i), session_key(folder, row), vec![row.session.session_id.clone(), row.session.id.clone()]));
    }
    if folder.more > 0 {
        rows.push((LeftRow::More(e, f), format!("more:{key}"), vec![format!("more-{key}")]));
    }
}

/// A row's level under the tree's root: a group's folders are one in.
pub(super) fn level(entry: &EntryNode) -> usize {
    usize::from(entry.group)
}

/// A folder row's level: a shown child is one in from its parent.
pub(super) fn folder_level(entry: &EntryNode, folder: &FolderNode) -> usize {
    level(entry) + usize::from(folder.parent.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hold_shows_only_the_cards_it_began_with_until_the_pointer_is_still() {
        let start = Instant::now();
        let hold = Hold::new(HashSet::from(["work:demo/a".to_string()]), start);
        assert!(hold.shows("work:demo/a") && !hold.shows("work:demo/b"), "a card arriving under the pointer waits");
        assert_eq!(hold.still_for(start + Duration::from_millis(500)), Some(Duration::from_millis(500)));
        assert_eq!(hold.still_for(start + HOLD_STILL), None, "still for HOLD_STILL releases it");
    }

    #[test]
    fn a_childs_icon_starts_under_its_parents_label() {
        for level in 0..3 {
            assert_eq!(grid::indent(level + 1) + px(grid::DISCLOSURE + grid::GAP), grid::indent(level) + px(grid::LABEL));
        }
    }
}
