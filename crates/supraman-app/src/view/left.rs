use std::{
    collections::{BTreeMap, HashSet},
    time::{Duration, Instant},
};

use supraman_claude::{Health, now_ms};
use supraman_core::{Ask, AskState, OutboxMessage, ask_states};
use supraman_supervisor::{NeedsKind, NeedsYou, Remedy, repo_session_name, workspace_session_name};
use gpui::{AnyElement, ClickEvent, Context, Div, FontWeight, Hsla, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, SharedString, Stateful, div, prelude::*, px, relative, rems};
use supraman_ui::banner;
use supraman_ui::button;
use supraman_ui::hit::{HitTargets, hit_target};
use supraman_ui::tabs::DraggedSession;
use theme::ActiveTheme as _;

use super::icon::{Icon, Tooltip};
use super::{Cli, FleetView, MenuTarget, TabKind, INBOX_MAX_SHARE, MAX_WORKING_LIMIT, MESSAGE_LINES, MESSAGES_LISTED, MESSAGES_MAX_SHARE};
use crate::clipped::clipped;
use crate::settings_window::page;
use crate::fleet::{self, Open, RepoNode, SessionRow, WorkspaceNode};
use crate::work_view;

const TREE_MIN_SHARE: f32 = 0.3;
/// Inbox and Messages shrink only once the notices are squeezed to nothing.
const NOTICES_SHRINK: f32 = 1000.;
pub const HOLD_STILL: Duration = Duration::from_secs(2);

/// Only these rows are drawn while the pointer moves, so a row arriving above never shifts what it aims at.
pub(crate) struct Hold {
    keys: HashSet<String>,
    moved: Instant,
}

fn need_key(need: &NeedsYou) -> String {
    format!("need:{}/{}/{}", need.workspace, need.name, needs_label(need.kind))
}

fn work_key(workspace: &str, row: &supraman_workstate::InboxRow) -> String {
    format!("work:{workspace}/{}", row.key)
}

fn message_key(workspace: &str, m: &OutboxMessage) -> String {
    format!("{workspace}/{}", m.file)
}

const SEAM_GAP: f32 = 6.;

pub(crate) use supraman_ui::themes::{DIMMED, QUIET, SECONDARY};
use supraman_ui::themes::{MESSAGE_TINT, MESSAGE_TINT_HOVER};

pub(crate) fn quiet(cx: &gpui::App) -> Hsla {
    cx.theme().colors().text.opacity(QUIET)
}

pub(crate) fn secondary(cx: &gpui::App) -> Hsla {
    cx.theme().colors().text.opacity(SECONDARY)
}

fn seam_color(cx: &gpui::App) -> Hsla {
    cx.theme().colors().text_muted.opacity(0.35)
}

fn needs_color(kind: NeedsKind, cx: &gpui::App) -> Hsla {
    let status = cx.theme().status();
    match kind {
        NeedsKind::Blocked | NeedsKind::TrustNeeded => status.warning,
        NeedsKind::GaveUp | NeedsKind::LaunchFailed | NeedsKind::TokenExpired | NeedsKind::OtherServer | NeedsKind::StopFailed => {
            status.error
        }
        NeedsKind::Duplicate => status.hint,
    }
}

fn needs_label(kind: NeedsKind) -> &'static str {
    match kind {
        NeedsKind::Blocked => "blocked",
        NeedsKind::TrustNeeded => "trust needed",
        NeedsKind::GaveUp => "gave up",
        NeedsKind::LaunchFailed => "launch failed",
        NeedsKind::Duplicate => "duplicate",
        NeedsKind::TokenExpired => "token expired",
        NeedsKind::OtherServer => "other server",
        NeedsKind::StopFailed => "stop failed",
    }
}

fn work_kind_color(kind: &str, cx: &gpui::App) -> Hsla {
    let status = cx.theme().status();
    match kind {
        "blocked" | "blocker_finding" => status.error,
        "question" | "assumption_due" | "done_unverified" => status.warning,
        _ => status.info,
    }
}

fn short_since(at: &str) -> String {
    if at.is_empty() { String::new() } else { format!("since {}", work_view::short_time(at)) }
}

pub fn inbox_key(need: &NeedsYou) -> Option<String> {
    let verb = match need.action.as_ref()? {
        Remedy::Attach { .. } => "attach",
        Remedy::Retry | Remedy::Adopt { .. } => "retry",
        Remedy::RunInTerminal { .. } => "run",
        Remedy::Remint { .. } => "remint",
        Remedy::RestartElevated => "restart-elevated",
    };
    Some(format!("inbox-{verb}-{}", need.name))
}

fn health_color(health: &Health, cx: &gpui::App) -> Hsla {
    let status = cx.theme().status();
    match health {
        Health::Running => status.success,
        Health::Blocked { .. } => status.warning,
        Health::Idle => status.info,
        Health::Stopped | Health::Orphaned { .. } => cx.theme().colors().text_disabled,
        Health::Unknown => status.hint,
    }
}

fn icon_button(hits: &HitTargets, key: &'static str, icon: Icon, tip: impl Into<SharedString>, on: bool, cx: &gpui::App) -> Stateful<Div> {
    let colors = cx.theme().colors();
    tinted_button(hits, key, icon, tip, on, if on { colors.text } else { colors.text_muted }, cx)
}

fn tinted_button(hits: &HitTargets, key: &'static str, icon: Icon, tip: impl Into<SharedString>, on: bool, fg: Hsla, cx: &gpui::App) -> Stateful<Div> {
    button::toolbar(hits, key, icon.svg(16., fg), on, fg, cx).tooltip(Tooltip::text(tip))
}

const WORKSPACE_ROW: f32 = 30.;
pub(super) const REPO_ROW: f32 = 26.;
pub(super) const SESSION_LINE: f32 = 24.;
/// One hover icon's place: always laid out, so a row's icons never shift.
const SLOT: f32 = 20.;

fn slot() -> AnyElement {
    div().flex_none().size(px(SLOT)).into_any_element()
}

/// Every tree row, the search results' too: `indent(level)`, a disclosure slot, an icon slot, the label;
/// a child's icon starts where its parent's label does.
pub(crate) mod grid {
    use gpui::{Pixels, px};

    pub const DISCLOSURE: f32 = 16.;
    pub const ICON: f32 = 16.;
    pub const GAP: f32 = 8.;
    const BASE: f32 = 6.;
    pub const STEP: f32 = DISCLOSURE + GAP;
    /// The label's x from the row's indent.
    pub const LABEL: f32 = DISCLOSURE + GAP + ICON + GAP;

    pub fn indent(level: usize) -> Pixels {
        px(BASE + level as f32 * STEP)
    }

    pub fn gap() -> Pixels {
        px(GAP)
    }
}

pub(super) fn disclosure(open: Option<bool>, color: Hsla) -> Div {
    let slot = div().flex().flex_none().items_center().justify_center().size(px(grid::DISCLOSURE));
    match open {
        Some(open) => slot.child(if open { Icon::ChevronDown } else { Icon::ChevronRight }.svg(14., color)),
        None => slot,
    }
}

/// A row's icon slot, painted under `tree-icon-<key>`.
pub(super) fn icon_slot(hits: &HitTargets, key: &str, icon: Option<AnyElement>) -> Div {
    div()
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(grid::ICON))
        .children(icon)
        .child(hit_target(hits, format!("tree-icon-{key}")))
}

/// A row's label, painted under `tree-label-<key>`.
pub(super) fn tree_label<E: Styled + ParentElement>(hits: &HitTargets, key: &str, label: E) -> E {
    label.relative().child(hit_target(hits, format!("tree-label-{key}")))
}

fn glyph_button(hits: &HitTargets, key: String, icon: Icon, tip: impl Into<SharedString>, cx: &gpui::App) -> Stateful<Div> {
    button::icon(hits, key, icon.svg(14., cx.theme().colors().text_muted), px(20.), cx).tooltip(Tooltip::text(tip))
}

impl FleetView {
    fn render_action_bar(&self, hidden_stopped: usize, cx: &mut Context<Self>) -> Stateful<Div> {
        let colors = cx.theme().colors().clone();
        let status = cx.theme().status().clone();
        let hits = &self.hits;
        let separator = || div().w(px(1.)).h(px(16.)).mx_1().bg(colors.border_variant);
        let stopped_tip: SharedString = if self.show_stopped {
            "Hide stopped sessions".into()
        } else {
            format!("Show stopped sessions ({hidden_stopped} hidden)").into()
        };
        let n = self.max_working;
        let busy_tip: SharedString = format!(
            "At most {n} session(s) of this Supraman working at once, across its workspaces; launches wait while it is reached. 0 pauses them all. Another Supraman's sessions are not counted."
        )
        .into();
        let warning = match &self.cli {
            Cli::Supported(version) => version.untested(),
            _ => None,
        };
        div()
            .id("action-bar")
            .flex()
            .flex_none()
            .items_center()
            .gap(px(2.))
            .px_2()
            .py(px(3.))
            .border_b_1()
            .border_color(colors.border)
            .child(
                icon_button(hits, "add-workspace", Icon::FolderAdd, "Add workspace…", false, cx)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.open_add_menu(cx))),
            )
            .child(separator())
            .child(
                icon_button(hits, "expand-all", Icon::ExpandAll, "Expand all", false, cx)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.collapse_all(false, cx))),
            )
            .child(
                icon_button(hits, "collapse-all", Icon::CollapseAll, "Collapse all", false, cx)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.collapse_all(true, cx))),
            )
            .child(
                icon_button(
                    hits,
                    "toggle-stopped",
                    if self.show_stopped { Icon::Eye } else { Icon::EyeOff },
                    stopped_tip,
                    self.show_stopped,
                    cx,
                )
                .when(!self.show_stopped && hidden_stopped > 0, |d| d.child(SharedString::from(hidden_stopped.to_string())))
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.show_stopped = !this.show_stopped;
                    cx.notify();
                })),
            )
            .child(
                button::toolbar_turning(hits, "refresh", Icon::Refresh.svg(16., colors.text_muted), false, self.refreshing(), colors.text_muted, cx)
                    .tooltip(Tooltip::text("Refresh: read the running sessions and the closed ones' transcripts now (F5 outside a terminal)"))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.refresh_now(cx))),
            )
            .child(div().flex_1())
            .children(warning.map(|w| {
                tinted_button(hits, "cli-warning", Icon::Warning, w, false, status.warning, cx)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.open_settings_at(Some(page::ABOUT), cx)))
            }))
            .child(
                div()
                    .id("busy-limit")
                    .flex()
                    .flex_none()
                    .items_center()
                    .h(px(26.))
                    .rounded_md()
                    .border_1()
                    .border_color(colors.border_variant)
                    .child(
                        icon_button(hits, "busy-dec", Icon::Minus, "One fewer busy session", false, cx)
                            .h(px(24.))
                            .min_w(px(22.))
                            .px(px(3.))
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.set_max_working(this.max_working.saturating_sub(1), cx))),
                    )
                    .child(
                        div()
                            .id("busy-value")
                            .px_1()
                            .text_xs()
                            .text_color(colors.text_muted)
                            .child(SharedString::from(format!("busy ≤ {n}")))
                            .tooltip(Tooltip::text(busy_tip)),
                    )
                    .child(
                        icon_button(hits, "busy-inc", Icon::Plus, "One more busy session", false, cx)
                            .h(px(24.))
                            .min_w(px(22.))
                            .px(px(3.))
                            .when(n >= MAX_WORKING_LIMIT, |d| d.opacity(0.4))
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.set_max_working(this.max_working + 1, cx))),
                    ),
            )
            .child(
                icon_button(hits, "bar-settings", Icon::Settings, "Settings (Ctrl+,)", false, cx)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.open_settings(cx))),
            )
    }

    fn pointer_on_left(&mut self, cx: &mut Context<Self>) {
        if let Some(hold) = &mut self.left_hold {
            hold.moved = Instant::now();
            return;
        }
        let mut keys: HashSet<String> = self.needs.iter().map(need_key).collect();
        keys.extend(self.messages.iter().flat_map(|(w, msgs)| msgs.iter().map(move |m| message_key(w, m))));
        keys.extend(self.work_inbox.read(cx).rows().into_iter().map(|(w, r)| work_key(w, r)));
        self.left_hold = Some(Hold { keys, moved: Instant::now() });
        self.left_holds += 1;
        self.left_hold_timer = Some(cx.spawn(async move |this, cx| {
            let mut wait = HOLD_STILL;
            loop {
                cx.background_executor().timer(wait).await;
                let still_for = this.update(cx, |this, cx| {
                    let still = this.left_hold.as_ref().map(|h| h.moved.elapsed()).filter(|still| *still < HOLD_STILL);
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

    fn release_left(&mut self, cx: &mut Context<Self>) {
        if self.left_hold.take().is_some() {
            cx.notify();
        }
        self.left_hold_timer = None;
    }

    pub fn left_held(&self) -> bool {
        self.left_hold.is_some()
    }

    fn shown_now(&self, key: &str) -> bool {
        self.left_hold.as_ref().is_none_or(|h| h.keys.contains(key))
    }

    pub fn collapse_all(&mut self, collapse: bool, cx: &mut Context<Self>) {
        if collapse {
            self.collapsed.extend(self.workspaces.iter().map(|w| format!("workspace:{}", w.name)));
        } else {
            self.collapsed.clear();
        }
        cx.notify();
    }

    pub(super) fn render_left(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let status = cx.theme().status().clone();
        let tree = self.tree();

        let header = self.render_action_bar(tree.hidden_stopped, cx);

        let mut notices: Vec<AnyElement> = Vec::new();
        let notice = |text: String, banner: Div| banner.mx_3().my_2().child(SharedString::from(text)).into_any_element();
        if let Cli::Failed(error) = &self.cli {
            notices.push(notice(
                format!("Cannot use the claude CLI, so no sessions are shown: {error}"),
                banner::tinted(status.error),
            ));
        }
        if let Some(error) = &self.registry_error {
            notices.push(notice(
                format!("Cannot read the workspace registry: {error}"),
                banner::tinted(status.error),
            ));
        }
        for skipped in &self.registry_skipped {
            notices.push(notice(
                format!("A workspace is left out, the others run: {skipped}"),
                banner::tinted(status.error),
            ));
        }
        if let Some(error) = &self.sessions_error {
            notices.push(notice(
                format!("Session list is stale, the last read failed (nothing is started or stopped meanwhile): {error}"),
                banner::tinted(status.warning),
            ));
        }
        let plain = supraman_ui::notices::render(
            self.plain_notices(cx),
            status.warning,
            |key, cx| glyph_button(&self.hits, key, Icon::Close, "Dismiss", cx),
            |this: &mut Self, kind, text, cx| this.dismiss_notice(kind, text, cx),
            cx,
        );
        notices.extend(plain.into_iter().map(|n| n.mx_3().my_2().into_any_element()));
        if let Some(problem) = self.workstate_notice() {
            notices.push(self.render_workstate_notice(problem, cx));
        }
        notices.extend(self.render_update_notice(cx));
        if let Some(warning) = &self.start_warning {
            notices.push(notice(warning.clone(), banner::tinted(status.warning)));
        }
        for (name, r) in &self.removing {
            let waiting = r.wanted.len();
            notices.push(notice(
                format!("Removing {name}: waiting for its {waiting} supervised session(s) to stop…"),
                banner::plain(cx),
            ));
        }
        if let Some(e) = &self.other_scan_error {
            notices.push(notice(e.clone(), banner::tinted(status.warning)));
        }
        notices.extend(self.render_confirm(cx));
        notices.extend(self.render_add(cx));
        notices.extend(self.render_edit_folders(cx));
        notices.extend(self.render_schedule_edit(cx));
        if self.registry_error.is_none() && self.registry_skipped.is_empty() && self.refreshes > 0 && tree.workspaces.is_empty() && self.adding.is_none() {
            notices.push(notice(
                "No workspaces registered. Add one with the folder button above, or:  sm workspace add <dir>".into(),
                banner::plain(cx),
            ));
        }

        let query = self.search_query(cx);
        let mut body = div()
            .id(if query.is_empty() { "fleet" } else { "search-results" })
            .relative()
            .child(hit_target(&self.hits, "tree-area".into()))
            .flex()
            .flex_col()
            .flex_1()
            .min_h(relative(TREE_MIN_SHARE))
            .overflow_y_scroll()
            .pb_4();
        self.search_marks.clear();
        for workspace in tree.workspaces.iter().filter(|_| query.is_empty()) {
            let pkey = format!("workspace:{}", workspace.name);
            let open = !self.collapsed.contains(&pkey);
            body = body.child(self.render_workspace_row(workspace, open, cx));
            if !open {
                continue;
            }
            for row in &workspace.supervised {
                body = body.child(self.render_session(&workspace.name, "supervised", row, grid::indent(1), cx));
            }
            body = body.children(self.render_schedules(workspace, cx));
            for repo in &workspace.repos {
                let rkey = format!("repo:{}/{}", workspace.name, repo.name);
                let ropen = !self.collapsed.contains(&rkey);
                body = body.child(self.render_folder_row(workspace, repo, ropen, cx));
                if !ropen {
                    continue;
                }
                for row in &repo.sessions {
                    body = body.child(self.render_session(&workspace.name, &repo.name, row, grid::indent(2), cx));
                }
                if repo.more > 0 {
                    let key = format!("{}/{}", workspace.name, repo.name);
                    let hit = format!("more-{key}");
                    body = body.child(
                        div()
                            .id(SharedString::from(hit.clone()))
                            .relative()
                            .flex()
                            .flex_none()
                            .items_center()
                            .h(px(SESSION_LINE))
                            .pl(grid::indent(2) + px(grid::LABEL))
                            .text_xs()
                            .text_color(secondary(cx))
                            .cursor_pointer()
                            .hover(|s| s.bg(colors.ghost_element_hover).text_color(colors.text))
                            .child(SharedString::from(format!("{} more closed…", repo.more)))
                            .child(hit_target(&self.hits, hit))
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.closed_expanded.insert(key.clone());
                                cx.notify();
                            })),
                    );
                }
            }
        }
        if query.is_empty() {
            body = body.children(self.render_unowned_others(cx));
        } else {
            body = self.render_results(body, &query, cx);
        }

        let toast = self.toast.render(&self.hits, "toast", cx);

        div()
            .id("left-panel")
            .relative()
            .flex()
            .flex_col()
            .w(self.left_width)
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
            .children(self.render_messages(cx))
            .when(!notices.is_empty(), |d| {
                d.child(
                    div()
                        .id("notices")
                        .relative()
                        .child(hit_target(&self.hits, "notices-area".into()))
                        .flex()
                        .flex_col()
                        .flex_shrink(NOTICES_SHRINK)
                        .min_h_0()
                        .mb(px(SEAM_GAP))
                        .border_b_1()
                        .border_color(seam_color(cx))
                        .overflow_y_scroll()
                        .children(notices),
                )
            })
            .child(self.render_search_box())
            .child(body)
            .children(toast)
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
        let all_work: Vec<(String, supraman_workstate::InboxRow)> =
            self.work_inbox.read(cx).rows().into_iter().map(|(w, r)| (w.to_string(), r.clone())).collect();
        let total = self.needs.len() + all_work.len();
        let work_rows: Vec<_> = all_work.into_iter().filter(|(w, r)| self.shown_now(&work_key(w, r))).collect();
        let mut needs: Vec<&NeedsYou> = self.needs.iter().filter(|n| self.shown_now(&need_key(n))).collect();
        let scheduled = self.render_schedule_inbox(cx);
        if needs.is_empty() && work_rows.is_empty() && scheduled.is_empty() {
            return None;
        }
        let waiting = total - needs.len() - work_rows.len();
        let total = total + scheduled.len();
        let colors = cx.theme().colors().clone();
        let now = now_ms();
        needs.sort_by_key(|n| (n.since, n.kind));
        let mut inbox = div()
            .id("inbox")
            .relative()
            .child(hit_target(&self.hits, "inbox-area".into()))
            .flex()
            .flex_col()
            .min_h_0()
            .max_h(relative(INBOX_MAX_SHARE))
            .overflow_y_scroll()
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
        for need in needs {
            let color = needs_color(need.kind, cx);
            let remedy = match (&need.action, inbox_key(need)) {
                (Some(action), Some(key)) => {
                    let (label, action, workspace) = match action {
                        Remedy::Attach { .. } => ("Attach", action.clone(), need.workspace.clone()),
                        Remedy::Retry => ("Retry", action.clone(), need.workspace.clone()),
                        Remedy::RunInTerminal { .. } => ("Run in terminal", action.clone(), need.workspace.clone()),
                        Remedy::Remint { .. } => ("Re-mint", action.clone(), need.workspace.clone()),
                        Remedy::RestartElevated => ("Restart as administrator", action.clone(), need.workspace.clone()),
                        Remedy::Adopt { .. } => ("Retry", action.clone(), need.workspace.clone()),
                    };
                    let name = need.name.clone();
                    let fresh = matches!(action, Remedy::Adopt { .. }).then(|| {
                        let (workspace, name) = (workspace.clone(), name.clone());
                        button::bordered(&self.hits, format!("inbox-fresh-{name}"), "Start fresh", cx)
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.start_fresh(&workspace, &name, cx)))
                    });
                    let primary = button::bordered(&self.hits, key, label, cx).on_click(cx.listener(
                        move |this, _: &ClickEvent, window, cx| match &action {
                            Remedy::Attach { id } => {
                                this.attach_by_id(&workspace, id, window, cx);
                            }
                            Remedy::Retry | Remedy::Remint { .. } | Remedy::Adopt { .. } => this.retry(&workspace, &name, cx),
                            Remedy::RunInTerminal { cwd, command } => {
                                this.run_in_terminal(cwd.clone(), command.clone(), window, cx);
                            }
                            Remedy::RestartElevated => this.restart_elevated(window, cx),
                        },
                    ));
                    Some(div().flex().flex_none().gap_1().child(primary).children(fresh))
                }
                _ => None,
            };
            inbox = inbox.child(
                div()
                    .flex()
                    .items_start()
                    .gap_2()
                    .p_2()
                    .rounded_md()
                    .border_l_2()
                    .border_color(color)
                    .bg(color.opacity(0.10))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap(px(2.))
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .text_sm()
                                    .child(div().font_weight(FontWeight::BOLD).text_color(color).child(needs_label(need.kind)))
                                    .child(SharedString::from(format!("{} · {}", need.workspace, need.name)))
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(colors.text_muted)
                                            .child(SharedString::from(fleet::ago(now - need.since))),
                                    ),
                            )
                            .child(div().text_xs().text_color(colors.text).child(SharedString::from(need.detail.clone()))),
                    )
                    .children(remedy),
            );
        }
        for (workspace, row) in &work_rows {
            inbox = inbox.child(self.render_work_row(workspace, row, cx));
        }
        inbox = inbox.children(scheduled);
        Some(inbox.into_any_element())
    }

    fn render_workstate_notice(&self, problem: String, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let status = cx.theme().status().clone();
        let color = if self.workstate.unreachable() { status.error } else { status.warning };
        let mut actions = div().flex().flex_wrap().gap_2().pt_1();
        if self.workstate.signed_out() {
            actions = match self.signing_in {
                Some(_) => actions.child(button::bordered(&self.hits, "ws-setup-cancel", "Cancel sign-in", cx).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        this.cancel_sign_in();
                    },
                ))),
                None => actions.child(button::bordered(&self.hits, "ws-setup-signin", "Sign in", cx).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        this.sign_in(cx);
                    },
                ))),
            };
        }
        if self.workstate.unreachable() {
            actions = actions.child(button::bordered(&self.hits, "ws-setup-retry", "Retry", cx).on_click(cx.listener(
                |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.recheck_workstate();
                },
            )));
        }
        actions = actions.child(button::bordered(&self.hits, "ws-setup-settings", "Settings…", cx).on_click(cx.listener(
            |this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.open_settings_at(Some(page::WORKSTATE), cx);
            },
        )));
        let dismiss = glyph_button(&self.hits, "ws-setup-dismiss".into(), Icon::Close, "Hide until the problem changes", cx)
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.dismiss_workstate_notice(cx);
            }));
        banner::tinted(color)
            .id("ws-setup-notice")
            .relative()
            .mx_3()
            .my_2()
            .child(banner::dismissable(
                div().flex().items_start().gap_2().child(crate::brand::workman(rems(1.), color).mt(px(2.))).child(SharedString::from(problem)),
                dismiss,
            ))
            .children(self.workstate.fix().map(|fix| div().text_xs().text_color(colors.text).child(SharedString::from(fix))))
            .children(self.signing_in.as_ref().map(|url| {
                div().text_xs().text_color(colors.text).child(SharedString::from(format!("Waiting for the sign-in to {url} in your browser…")))
            }))
            .children(self.sign_in_error.clone().map(|e| div().text_xs().text_color(status.error).child(SharedString::from(e))))
            .child(actions)
            .child(hit_target(&self.hits, "ws-setup-notice".into()))
            .into_any_element()
    }

    fn render_update_notice(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let version = self.updates.offered()?.version().to_string();
        let color = cx.theme().status().info;
        let dismiss = glyph_button(&self.hits, "update-dismiss".into(), Icon::Close, "Hide until a newer release", cx)
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.dismiss_update(cx);
            }));
        let actions = div()
            .flex()
            .flex_wrap()
            .gap_2()
            .pt_2()
            .child(button::bordered(&self.hits, "update-notes", "What's new", cx).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.update_notes(cx);
            })))
            .child(button::bordered(&self.hits, "update-install", "Update", cx).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.run_update(cx);
            })));
        Some(
            banner::tinted(color)
                .id("update-notice")
                .relative()
                .mx_3()
                .my_2()
                .child(banner::dismissable(SharedString::from(format!("Supraman {version} is available")), dismiss))
                .child(actions)
                .child(hit_target(&self.hits, "update-notice".into()))
                .into_any_element(),
        )
    }

    /// The MCP call that would resolve it is shown, never run (C21).
    fn render_work_row(&self, workspace: &str, row: &supraman_workstate::InboxRow, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let color = work_kind_color(&row.kind, cx);
        let key = work_view::row_key(row);
        let dismiss_key = work_view::dismiss_key(row);
        let (open_ws, item) = (workspace.to_string(), row.item.id.clone());
        let (dismiss_ws, row_id) = (workspace.to_string(), row.key.clone());
        let inbox = self.work_inbox.clone();
        let dismiss = glyph_button(&self.hits, dismiss_key, Icon::Close, "Dismiss here (Workman is not changed)", cx)
            .on_click(move |_: &ClickEvent, _, cx| {
                cx.stop_propagation();
                inbox.update(cx, |inbox, cx| inbox.dismiss(&dismiss_ws, &row_id, cx));
            });
        div()
            .id(SharedString::from(key.clone()))
            .relative()
            .flex()
            .items_start()
            .gap_2()
            .p_2()
            .rounded_md()
            .border_l_2()
            .border_dashed()
            .border_color(color)
            .bg(color.opacity(0.06))
            .cursor_pointer()
            .hover(|s| s.bg(color.opacity(0.14)))
            .child(crate::brand::workman(rems(1.), color).mt(px(2.)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_2()
                            .text_sm()
                            .child(div().font_weight(FontWeight::BOLD).text_color(color).child(SharedString::from(format!(
                                "Workman · {}",
                                work_view::kind_label(&row.kind)
                            ))))
                            .child(SharedString::from(format!("{workspace} · {}", row.item.qualified_ref)))
                            .child(div().text_xs().text_color(colors.text_muted).child(SharedString::from(short_since(&row.since)))),
                    )
                    .child(div().text_xs().text_color(colors.text).child(SharedString::from(row.what.clone())))
                    .when(!row.call.is_empty(), |d| {
                        d.child(
                            clipped(format!("work-call-{workspace}/{}", row.item.qualified_ref), row.call.clone(), false)
                                .font_family(supraman_ui::fonts::MONO)
                                .text_xs()
                                .text_color(colors.text_muted),
                        )
                    }),
            )
            .child(dismiss)
            .child(hit_target(&self.hits, key))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.open_work_item(&open_ws, &item, cx);
            }))
            .into_any_element()
    }

    fn listed_messages(&self) -> Vec<(&String, &OutboxMessage)> {
        self.messages
            .iter()
            .flat_map(|(w, msgs)| msgs.iter().map(move |m| (w, m)))
            .filter(|(w, m)| !self.dismissed_messages.hides(w, &m.file))
            .collect()
    }

    pub fn messages_count(&self) -> usize {
        self.listed_messages().len()
    }

    fn render_messages(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let listed = self.listed_messages();
        let total = listed.len();
        let mut all: Vec<(&String, &OutboxMessage)> = listed.into_iter().filter(|(w, m)| self.shown_now(&message_key(w, m))).collect();
        if all.is_empty() {
            return None;
        }
        all.sort_by_key(|(_, m)| std::cmp::Reverse((m.at, m.file.clone())));
        let waiting = total - all.len();
        all.truncate(MESSAGES_LISTED);
        let colors = cx.theme().colors().clone();
        let status = cx.theme().status().clone();
        let now = now_ms();
        let no_answers = BTreeMap::new();
        let asks: BTreeMap<&String, BTreeMap<String, AskState>> = self
            .messages
            .iter()
            .map(|(w, msgs)| (w, ask_states(msgs, &workspace_session_name(w), self.answered.get(w).unwrap_or(&no_answers))))
            .collect();
        let mut section = div()
            .id("messages")
            .relative()
            .child(hit_target(&self.hits, "messages-area".into()))
            .flex()
            .flex_col()
            .min_h_0()
            .max_h(relative(MESSAGES_MAX_SHARE))
            .overflow_y_scroll()
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
                        0 => format!("MESSAGES · {total}"),
                        n => format!("MESSAGES · {total} · {n} new, shown when the pointer leaves"),
                    })),
            );
        for (workspace, m) in all {
            let key = message_key(workspace, m);
            let color = if m.is_notice() || m.error.is_some() { status.warning } else { status.info };
            let sender = match (&m.error, m.from_name.as_str()) {
                (Some(_), _) => format!("unreadable {}", m.file),
                (None, "") => "?".to_string(),
                (None, name) => name.to_string(),
            };
            let lines: Vec<&str> = m.body.lines().collect();
            let open = self.expanded.contains(&key);
            let shown = if open { lines.len() } else { lines.len().min(MESSAGE_LINES) };
            let mut body = div().flex().flex_col().min_w_0().text_xs().text_color(colors.text);
            for (n, segment) in segments(&lines[..shown]).into_iter().enumerate() {
                body = body.child(match segment {
                    Segment::Text(text) => div().child(SharedString::from(text)),
                    Segment::Table(rows) => div().flex().flex_col().py_1().overflow_hidden().font_family(supraman_ui::fonts::MONO).children(
                        rows.into_iter().enumerate().map(|(i, row)| {
                            div()
                                .relative()
                                .whitespace_nowrap()
                                .child(SharedString::from(row))
                                .child(hit_target(&self.hits, format!("msg-table-{key}#{n}.{i}")))
                        }),
                    ),
                });
            }
            if shown < lines.len() {
                body = body.child(SharedString::from(format!("… {} more line(s), click to show", lines.len() - shown)));
            }
            // every row, not the tree's: a sender in the workspace root but outside its repos has no tree row
            let target = self
                .sessions
                .iter()
                .find(|s| !m.from_name.is_empty() && s.name == m.from_name && fleet::attachable(s, &s.health(now)))
                .map(|s| s.id.clone());
            let attach = target.map(|id| {
                let workspace = workspace.clone();
                button::bordered(&self.hits, format!("msg-attach-{key}"), "Attach", cx).on_click(cx.listener(
                    move |this, _: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        this.attach_by_id(&workspace, &id, window, cx);
                    },
                ))
            });
            let answer = asks.get(workspace).and_then(|a| a.get(&m.file)).map(|state| {
                let row = div().flex().items_center().gap_2().text_xs();
                let reply = |cx: &mut Context<Self>| {
                    let (w, file) = (workspace.clone(), m.file.clone());
                    button::bordered(&self.hits, format!("msg-reply-{key}"), "Reply…", cx).on_click(cx.listener(
                        move |this, _: &ClickEvent, window, cx| {
                            cx.stop_propagation();
                            this.open_compose(&w, file.clone(), window, cx);
                        },
                    ))
                };
                match state {
                    AskState::Open(Ask::Approve) => {
                        let (w, file) = (workspace.clone(), m.file.clone());
                        row.child(button::bordered(&self.hits, format!("msg-approve-{key}"), "Approve", cx).on_click(cx.listener(
                            move |this, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                this.approve(&w, &file, cx);
                            },
                        )))
                        .child(reply(cx))
                    }
                    AskState::Open(Ask::Decide) => row.child(reply(cx)),
                    AskState::Answered(line) => row.child(
                        div()
                            .relative()
                            .text_color(status.success)
                            .child(SharedString::from(match self.answered_route.get(&key) {
                                Some(route) => format!("answered: {line} · {route}"),
                                None => format!("answered: {line}"),
                            }))
                            .child(hit_target(&self.hits, format!("msg-answered-{key}"))),
                    ),
                    AskState::Superseded => {
                        row.child(div().text_color(colors.text_muted).child("superseded by a later APPROVE?"))
                    }
                }
            });
            let dismiss = {
                let (w, file) = (workspace.clone(), m.file.clone());
                glyph_button(&self.hits, format!("msg-dismiss-{key}"), Icon::Close, "Dismiss here", cx).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        this.dismiss_message(&w, &file, cx);
                    },
                ))
            };
            let replying = self
                .compose
                .as_ref()
                .is_some_and(|c| &c.workspace == workspace && c.reply_to == m.file);
            section = section.child(
                div()
                    .id(SharedString::from(format!("msg-{key}")))
                    .flex()
                    .items_start()
                    .gap_2()
                    .p_2()
                    .rounded_md()
                    .border_l_2()
                    .border_color(color)
                    .bg(color.opacity(MESSAGE_TINT))
                    .cursor_pointer()
                    .hover(|s| s.bg(color.opacity(MESSAGE_TINT_HOVER)))
                    .active(|s| s.bg(color.opacity(0.22)))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap(px(2.))
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .text_sm()
                                    .child(div().font_weight(FontWeight::BOLD).text_color(colors.text).child(SharedString::from(sender)))
                                    .child(div().text_color(quiet(cx)).child(SharedString::from(workspace.clone())))
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(quiet(cx))
                                            .child(SharedString::from(fleet::ago(now - m.at))),
                                    ),
                            )
                            .child(body)
                            .children(answer),
                    )
                    .children(attach)
                    .child(dismiss)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        if !this.expanded.remove(&key) {
                            this.expanded.insert(key.clone());
                        }
                        cx.notify();
                    })),
            );
            if replying {
                section = section.children(self.render_compose(cx));
            }
        }
        Some(section.into_any_element())
    }

    fn render_compose(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let c = self.compose.as_ref()?;
        let colors = cx.theme().colors().clone();
        let status = cx.theme().status().clone();
        let ctrl = workspace_session_name(&c.workspace);
        let title = format!("Reply to {ctrl}");
        let refusal = self.relay_refusal(&c.workspace);
        let line = |text: String, color: Hsla| div().text_xs().text_color(color).child(SharedString::from(text));
        Some(
            div()
                .id("compose")
                .relative()
                .flex()
                .flex_col()
                .flex_none()
                .gap_1()
                .mx_3()
                .my_1()
                .p_2()
                .rounded_md()
                .border_1()
                .border_color(colors.border)
                .bg(colors.editor_background)
                .child(hit_target(&self.hits, "compose-box".into()))
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .text_xs()
                        .text_color(colors.text_muted)
                        .child(SharedString::from(title))
                        .child("Ctrl+Enter sends · Esc cancels"),
                )
                .child(div().relative().child(c.input.clone()).child(hit_target(&self.hits, "compose-text".into())))
                .children(refusal.map(|why| line(format!("Cannot send now: {why}."), status.warning)))
                .children(c.error.clone().map(|e| line(e, status.error)))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .justify_end()
                        .child(button::bordered(&self.hits, "compose-send", "Send", cx).on_click(cx.listener(
                            |this, _: &ClickEvent, window, cx| {
                                cx.stop_propagation();
                                this.send_compose(cx);
                                // the press gave the keys to the window: an open box takes them back
                                if let Some(c) = &this.compose {
                                    c.input.update(cx, |input, cx| input.focus(window, cx));
                                }
                            },
                        )))
                        .child(button::bordered(&self.hits, "compose-cancel", "Cancel", cx).on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                this.compose = None;
                                cx.notify();
                            },
                        ))),
                )
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    if let Some(c) = &this.compose {
                        c.input.update(cx, |input, cx| input.focus(window, cx));
                    }
                }))
                .into_any_element(),
        )
    }

    pub(super) fn hover_icon(&self, key: String, icon: Icon, tip: &'static str, group: &SharedString, shown: bool, cx: &gpui::App) -> Stateful<Div> {
        button::revealed(&self.hits, key, icon.svg(14., cx.theme().colors().text_muted), px(SLOT), group.clone(), shown, cx).tooltip(Tooltip::text(tip))
    }

    pub(super) fn open_of(&self, row: &SessionRow) -> Open {
        Open::of(row, self.session_tab(&row.session), self.active, self.resumable(row))
    }

    /// The row's one open action, as its slot runs it; a refusal shows why.
    pub(super) fn open_row(&mut self, workspace: &str, repo: &str, row: &SessionRow, window: &mut gpui::Window, cx: &mut Context<Self>) {
        let s = &row.session;
        let open = self.open_of(row);
        match open {
            Open::Tab(id) => self.activate_tab(id, cx),
            Open::Attach => {
                self.attach(workspace, repo, s, window, cx);
            }
            Open::Resume => {
                self.resume(workspace, repo, s, window, cx);
            }
            Open::Elsewhere | Open::Supervised => {
                let text = format!("{}: {}", fleet::display_name(s), open.hint().unwrap_or_default());
                self.toast.show_why(text, |this: &mut Self| &mut this.toast, cx);
            }
            Open::Shown | Open::Foreign | Open::Nothing => {}
        }
    }

    fn workspace_selected(&self, workspace: &WorkspaceNode, cx: &gpui::App) -> bool {
        let Some(tab) = self.active.and_then(|id| self.tab(id)) else { return false };
        match &tab.kind {
            TabKind::Work(view) => view.read(cx).workspace == workspace.name,
            _ => workspace.own.as_ref().is_some_and(|own| Self::tab_runs(tab, &own.session)),
        }
    }

    /// The workspace row stands for the workspace session: its health, its `waiting for`, its controls.
    fn render_workspace_row(&self, workspace: &WorkspaceNode, open: bool, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let status = cx.theme().status().clone();
        let w = workspace.name.clone();
        let pkey = format!("workspace:{w}");
        let group = SharedString::from(format!("ws-group-{w}"));
        let shown = self.workspace_selected(workspace, cx);
        let started = workspace.started == Some(true);
        let own = workspace.own.as_ref();
        let live = own.filter(|r| fleet::is_alive(&r.health));
        let blocked = live.is_some_and(|r| matches!(r.health, Health::Blocked { .. }));
        let waiting = live.and_then(|r| match &r.health {
            Health::Blocked { waiting_for: Some(why) } => Some(format!("waiting for: {why}")),
            _ => None,
        });
        let dot = match live {
            Some(r) => div().size(px(8.)).rounded_full().bg(health_color(&r.health, cx)),
            None => div().size(px(8.)).rounded_full().border_1().border_color(colors.text_disabled),
        };
        let relay = self.relays.get(&w).and_then(|r| r.label(now_ms()));
        let relay_glyph = relay.and_then(|(text, _)| {
            let (icon, color) = match relay_word(&text) {
                "direct" => return None,
                "ready" => (Icon::RelayReady, status.success),
                "starting" => (Icon::RelayStarting, status.warning),
                _ => (Icon::Warning, status.warning),
            };
            let key = format!("relay-{w}");
            Some(
                div()
                    .id(SharedString::from(key.clone()))
                    .relative()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size_full()
                    .child(icon.svg(14., color))
                    .child(hit_target(&self.hits, key))
                    .tooltip(Tooltip::text(text)),
            )
        });

        let work = {
            let w = w.clone();
            self.hover_icon(format!("work-{w}"), Icon::Work, "Work: the workspace's Workman board", &group, shown, cx).on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.open_work(&w, cx);
                },
            ))
        };
        let task = {
            let w = w.clone();
            self.hover_icon(format!("task-{w}"), Icon::Task, "New task…: start a session with a first prompt", &group, shown, cx).on_click(cx.listener(
                move |this, _: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_start(&w, None, false, window, cx);
                },
            ))
        };
        let session = own.and_then(|row| {
            let (w, session) = (w.clone(), row.session.clone());
            if row.attachable {
                Some(self.hover_icon(format!("attach-ws-{w}"), Icon::Attach, "Attach the workspace session", &group, shown, cx).on_click(cx.listener(
                    move |this, _: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        this.attach(&w, "workspace", &session, window, cx);
                    },
                )))
            } else if self.resumable(row) {
                Some(self.hover_icon(format!("resume-ws-{w}"), Icon::Resume, "Resume the workspace session", &group, shown, cx).on_click(cx.listener(
                    move |this, _: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        this.resume(&w, "workspace", &session, window, cx);
                    },
                )))
            } else {
                None
            }
        });
        let start_stop = {
            let w = w.clone();
            let (key, icon, tip) = if started {
                (format!("stop-{w}"), Icon::Square, "Stop: its supervised sessions stop (stop, never rm)")
            } else {
                (format!("start-{w}"), Icon::Play, "Start: supervise the workspace, its session first")
            };
            self.hover_icon(key, icon, tip, &group, shown, cx).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                if started {
                    this.stop_workspace(&w, cx);
                } else {
                    this.start_workspace(&w, cx);
                }
            }))
        };

        let line = div()
            .flex()
            .items_center()
            .gap(grid::gap())
            .h(px(WORKSPACE_ROW))
            .child(disclosure(Some(open), colors.text_muted))
            .child(icon_slot(&self.hits, &format!("ws-{w}"), Some(dot.into_any_element())))
            .child(tree_label(&self.hits, &format!("ws-{w}"), div().flex_none().font_weight(FontWeight::BOLD).whitespace_nowrap().child(SharedString::from(w.clone()))))
            .child(clipped(format!("ws-root-{w}"), home_text(workspace.home.as_deref()), true).flex_1().text_xs().text_color(secondary(cx)))
            .child(
                div()
                    .flex_none()
                    .whitespace_nowrap()
                    .text_xs()
                    .text_color(secondary(cx))
                    .child(SharedString::from(workspace.session_count().to_string())),
            )
            .child(div().flex_none().size(px(16.)).children(relay_glyph))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(2.))
                    .child(work)
                    .child(task)
                    .child(session.map_or_else(slot, IntoElement::into_any_element))
                    .child(start_stop),
            );
        div()
            .id(SharedString::from(pkey.clone()))
            .group(group)
            .relative()
            .flex()
            .flex_col()
            .flex_none()
            .pl(grid::indent(0))
            .pr_2()
            .mt_2()
            .cursor_pointer()
            .hover(|s| s.bg(colors.ghost_element_hover))
            .active(|s| s.bg(colors.ghost_element_active))
            .when(blocked, |d| d.bg(status.warning.opacity(0.12)).hover(|s| s.bg(status.warning.opacity(0.18))))
            .child(line)
            .when_some(waiting, |d, why| {
                d.child(clipped(format!("ws-waiting-{w}"), why, false).pl(px(grid::LABEL)).pb(px(4.)).text_xs().text_color(status.warning))
            })
            .child(hit_target(&self.hits, format!("ws-row-{w}")))
            .on_mouse_down(MouseButton::Right, {
                let w = w.clone();
                cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.open_menu(MenuTarget::Workspace(w.clone()), e.position, cx);
                })
            })
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle(pkey.clone(), cx)))
            .into_any_element()
    }

    fn render_folder_row(&self, workspace: &WorkspaceNode, repo: &RepoNode, open: bool, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let w = &workspace.name;
        let rkey = format!("repo:{w}/{}", repo.name);
        let tree_key = format!("repo-{w}/{}", repo.name);
        let group = SharedString::from(format!("repo-group-{w}/{}", repo.name));
        let empty = repo.sessions.is_empty();
        let git = repo.kind == supraman_core::FolderKind::Git;
        let branch = repo.branch.clone().filter(|_| git).unwrap_or_default();
        let offers_new = repo.enabled && repo.supervised_in.is_none();
        let parked = repo.listed && !repo.enabled && repo.supervised_in.is_none();
        let new_claude = offers_new.then(|| {
            let (cwd, title) = (repo.path.clone(), format!("{w} · {} · claude", repo.name));
            self.hover_icon(format!("new-claude-{w}/{}", repo.name), Icon::Plus, "New claude session in this folder", &group, false, cx).on_click(cx.listener(
                move |this, _: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    this.new_claude(cwd.clone(), title.clone(), window, cx);
                },
            ))
        });
        let terminal = {
            let (cwd, title) = (repo.path.clone(), format!("{w} · {} · pwsh", repo.name));
            self.hover_icon(format!("terminal-{w}/{}", repo.name), Icon::Terminal, "Open a terminal here", &group, false, cx).on_click(cx.listener(
                move |this, _: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_shell_in(cwd.clone(), title.clone(), window, cx);
                },
            ))
        };
        div()
            .id(SharedString::from(rkey.clone()))
            .group(group)
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(grid::gap())
            .h(px(REPO_ROW))
            .pl(grid::indent(1))
            .pr_2()
            .text_sm()
            .cursor_pointer()
            .hover(|s| s.bg(colors.ghost_element_hover))
            .active(|s| s.bg(colors.ghost_element_active))
            .when(empty, |d| d.text_color(quiet(cx)))
            .when(parked, |d| d.opacity(DIMMED))
            .child(disclosure((!empty).then_some(open), secondary(cx)))
            .child(icon_slot(&self.hits, &tree_key, Some(if git { Icon::GitBranch } else { Icon::Folder }.slotted(grid::ICON, secondary(cx)).into_any_element())))
            .child(tree_label(&self.hits, &tree_key, div().flex_none().whitespace_nowrap().child(SharedString::from(folder_label(&repo.name, &repo.path, workspace.home.as_deref())))))
            .child(clipped(format!("branch-{w}/{}", repo.name), branch, false).text_xs().text_color(secondary(cx)))
            .when(!empty, |d| {
                d.child(div().flex_none().text_xs().text_color(secondary(cx)).child(SharedString::from(repo.sessions.len().to_string())))
            })
            .when(parked, |d| d.child(tag("disabled", cx)))
            .children(self.folder_tags(w, repo, cx))
            .child(div().flex_1())
            .child(div().flex().flex_none().items_center().gap(px(2.)).child(new_claude.map_or_else(slot, IntoElement::into_any_element)).child(terminal))
            .child(hit_target(&self.hits, format!("repo-row-{w}/{}", repo.name)))
            .on_mouse_down(MouseButton::Right, {
                let (w, r) = (w.clone(), repo.name.clone());
                cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.open_menu(MenuTarget::Repo { workspace: w.clone(), repo: r.clone() }, e.position, cx);
                })
            })
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle(rkey.clone(), cx)))
            .into_any_element()
    }

    pub(super) fn render_session(
        &self,
        workspace: &str,
        repo: &str,
        row: &SessionRow,
        indent: Pixels,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let status = cx.theme().status().clone();
        let s = &row.session;
        let at = row.scope(workspace);
        let foreign = row.foreign;
        let dot = if foreign { colors.text_muted } else { health_color(&row.health, cx) };
        let state = if foreign || matches!(row.health, Health::Stopped | Health::Orphaned { .. }) { secondary(cx) } else { dot };
        let blocked = !foreign && matches!(row.health, Health::Blocked { .. });
        let dimmed = foreign || matches!(row.health, Health::Orphaned { .. } | Health::Stopped);
        let waiting_for = match &row.health {
            Health::Blocked { waiting_for: Some(w) } => Some(format!("waiting for: {w}")),
            _ => None,
        };
        let name = if s.name.is_empty() { "(unnamed)".to_string() } else { s.name.clone() };
        let marks = self.search_marks.get(&s.session_id);
        let hook = row
            .hook
            .as_ref()
            .filter(|_| s.name.starts_with(supraman_core::FM_PREFIX))
            .map(|h| format!("hook: {}", h.label(now_ms())));
        let shown = self.is_marked(s);
        let group = SharedString::from(format!("session-group-{}{at}", s.session_id));
        let tree_key = format!("session-{}{at}", s.session_id);

        let star = {
            let session_id = s.session_id.clone();
            let (icon, fg, tip) = if row.starred {
                (Icon::StarFilled, status.warning, "Unstar: shown only while it runs or stopped sessions are shown")
            } else {
                (Icon::Star, colors.text_muted, "Star: always shown, stopped or closed, however old")
            };
            button::revealed(&self.hits, format!("star-{}{at}", s.session_id), icon.svg(13., fg), px(SLOT), group.clone(), shown || row.starred, cx)
                .tooltip(Tooltip::text(tip))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.toggle_star(&session_id, cx);
                }))
        };
        let open = self.open_of(row);
        let action = {
            let (workspace, repo, session) = (workspace.to_string(), repo.to_string(), s.clone());
            let tip = open.hint().unwrap_or_default();
            match open {
                // open in a tab not shown now: a state, so it shows without a hover
                Open::Tab(tab) => Some(self.hover_icon(format!("tab-link-{}{at}", s.id), Icon::Tab, tip, &group, true, cx).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        this.activate_tab(tab, cx);
                    },
                ))),
                Open::Attach => Some(self.hover_icon(format!("attach-{}{at}", s.id), Icon::Attach, tip, &group, shown, cx).on_click(cx.listener(
                    move |this, _: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        this.attach(&workspace, &repo, &session, window, cx);
                    },
                ))),
                Open::Resume => Some(self.hover_icon(format!("resume-{}{at}", s.id), Icon::Resume, tip, &group, shown, cx).on_click(cx.listener(
                    move |this, _: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        this.resume(&workspace, &repo, &session, window, cx);
                    },
                ))),
                _ => None,
            }
        };
        let state_word = if row.closed { "closed".to_string() } else { fleet::health_label(&row.health) };
        let name_el = match marks {
            Some(m) if !s.name.is_empty() => supraman_ui::matcher::highlighted(format!("session-name-{}{at}", s.id), name, &m.name, cx),
            _ => clipped(format!("session-name-{}{at}", s.id), name, false),
        };
        let name_el = if foreign {
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
        };

        let line = div()
            .flex()
            .items_center()
            .gap(grid::gap())
            .h(px(SESSION_LINE))
            .child(disclosure(None, dot))
            .child(icon_slot(&self.hits, &tree_key, Some(Icon::Claude.slotted(grid::ICON, dot).into_any_element())))
            .child(tree_label(&self.hits, &tree_key, name_el))
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
            .child(
                div()
                    .flex()
                    .flex_none()
                    .whitespace_nowrap()
                    .text_xs()
                    .text_color(secondary(cx))
                    .child(SharedString::from(format!("{} · ", s.kind.as_str())))
                    .child(div().text_color(state).child(SharedString::from(state_word))),
            )
            .when_some(row.also_in.clone(), |d, owner| {
                d.child(
                    div()
                        .id(SharedString::from(format!("also-in-{}{at}", s.session_id)))
                        .flex_none()
                        .whitespace_nowrap()
                        .text_xs()
                        .text_color(quiet(cx))
                        .child(SharedString::from(format!("also in {owner}"))),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(2.))
                    .child(star)
                    .child(action.map_or_else(slot, IntoElement::into_any_element)),
            );

        let (w, r, clicked) = (workspace.to_string(), repo.to_string(), row.clone());
        let target = MenuTarget::Session { workspace: workspace.to_string(), repo: repo.to_string(), session_id: s.session_id.clone() };
        div()
            .id(SharedString::from(format!("session-{}{at}", s.id)))
            .group(group)
            .relative()
            .flex()
            .flex_none()
            .flex_col()
            .pl(indent)
            .pr_2()
            .py(px(1.))
            .text_sm()
            .hover(|s| s.bg(colors.ghost_element_hover))
            .when(shown, |d| d.bg(colors.ghost_element_selected).border_l_2().border_color(colors.border_focused).pl(indent - px(2.)))
            .when(blocked, |d| d.bg(status.warning.opacity(0.12)).hover(|s| s.bg(status.warning.opacity(0.18))))
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
            .child(hit_target(&self.hits, format!("session-row-{}{at}", s.session_id)))
            .on_drag(DraggedSession { session_id: s.session_id.clone(), title: fleet::display_name(s).to_string().into() }, |row, _, _, cx| {
                cx.new(|_| row.clone())
            })
            .when(shown, |d| d.child(hit_target(&self.hits, format!("marked-{}{at}", s.session_id))))
            .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                if e.click_count() == 2 {
                    this.open_row(&w, &r, &clicked, window, cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.open_menu(target.clone(), e.position, cx);
                }),
            )
            .into_any_element()
    }

    /// `supervised in <ws>` (a click reveals it there), `also in <ws>, …`.
    fn folder_tags(&self, workspace: &str, repo: &RepoNode, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out = Vec::new();
        if let Some(owner) = &repo.supervised_in {
            let key = format!("supervised-in-{workspace}/{}", repo.name);
            let (owner, path) = (owner.clone(), repo.path.clone());
            out.push(
                tag(format!("supervised in {owner}"), cx)
                    .id(SharedString::from(key.clone()))
                    .relative()
                    .cursor_pointer()
                    .hover(|s| s.text_color(cx.theme().colors().text))
                    .child(hit_target(&self.hits, key))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        this.reveal_folder(&owner, &path, cx);
                    }))
                    .into_any_element(),
            );
        }
        let adopting = repo_session_name(workspace, &repo.name)
            .and_then(|name| self.needs.iter().find(|n| n.workspace == workspace && n.name == name && matches!(n.action, Some(Remedy::Adopt { .. }))));
        if let Some(need) = adopting {
            let key = format!("adopt-failed-{workspace}/{}", repo.name);
            let why = format!("{}: {}. Retry or Start fresh in the Inbox.", need.name, need.detail);
            out.push(
                tag("adopt failed", cx)
                    .id(SharedString::from(key.clone()))
                    .relative()
                    .text_color(cx.theme().status().warning)
                    .child(hit_target(&self.hits, key))
                    .tooltip(Tooltip::text(why))
                    .into_any_element(),
            );
        }
        let also: Vec<&str> = repo.also_in.iter().map(String::as_str).filter(|w| repo.supervised_in.as_deref() != Some(*w)).collect();
        if !also.is_empty() {
            out.push(
                div()
                    .flex_none()
                    .whitespace_nowrap()
                    .text_xs()
                    .text_color(secondary(cx))
                    .child(SharedString::from(format!("also in {}", also.join(", "))))
                    .into_any_element(),
            );
        }
        out
    }

    pub(super) fn reveal_folder(&mut self, workspace: &str, path: &std::path::Path, cx: &mut Context<Self>) {
        let folder = self.workspaces.iter().find(|w| w.name == workspace).and_then(|w| w.folder_at(path)).map(|f| f.name.clone());
        self.reveal(workspace, folder.as_deref(), cx);
    }

    fn render_confirm(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let confirm = self.confirm.as_ref()?;
        Some(confirm.render(&self.hits, "confirm", |this: &mut Self, yes, _, cx| this.answer_confirm(yes, cx), cx).mx_3().my_2().into_any_element())
    }

    fn render_add(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let draft = self.adding.as_ref()?;
        let colors = cx.theme().colors().clone();
        let status = cx.theme().status().clone();
        let mut repos = div().flex().flex_col().gap(px(2.));
        if draft.scanning {
            repos = repos.child(div().text_xs().text_color(colors.text_muted).child("scanning…"));
        }
        for (i, (repo, on)) in draft.repos.iter().enumerate() {
            let key = format!("add-repo-{}", repo.name);
            let branch = repo.branch.as_deref().map(|b| format!("({b})")).unwrap_or_default();
            repos = repos.child(
                div()
                    .id(SharedString::from(key.clone()))
                    .relative()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_1()
                    .rounded_sm()
                    .text_sm()
                    .cursor_pointer()
                    .hover(|s| s.bg(colors.ghost_element_hover))
                    .when(!*on, |d| d.text_color(colors.text_muted))
                    .child(if *on { "☑" } else { "☐" })
                    .child(SharedString::from(repo_label(&repo.name, &repo.path, Some(&draft.root))))
                    .child(div().text_xs().text_color(colors.text_muted).child(SharedString::from(branch)))
                    .child(hit_target(&self.hits, key))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        this.toggle_add_repo(i, cx);
                    })),
            );
        }
        let ticked = draft.repos.iter().filter(|(_, on)| *on).count();
        let summary = if draft.empty {
            "No folders yet: Save opens Edit folders… to add them.".to_string()
        } else {
            format!("{ticked} of {} repos enabled; unticked ones are registered disabled", draft.repos.len())
        };
        Some(
            div()
                .id("add-review")
                .relative()
                .flex()
                .flex_col()
                .gap_2()
                .mx_3()
                .my_2()
                .p_2()
                .rounded_md()
                .border_1()
                .border_color(colors.border_focused)
                .bg(colors.editor_background)
                .child(hit_target(&self.hits, "add-review".into()))
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .text_xs()
                        .text_color(colors.text_muted)
                        .child(if draft.empty { "ADD EMPTY WORKSPACE" } else { "ADD WORKSPACE" })
                        .child("Enter saves · Esc cancels"),
                )
                .when(!draft.empty, |d| {
                    d.child(
                        div()
                            .text_xs()
                            .text_color(colors.text_muted)
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .child(SharedString::from(draft.root.display().to_string())),
                    )
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_sm()
                        .child("name")
                        .child(
                            div()
                                .id("add-name")
                                .relative()
                                .flex_1()
                                .min_w_0()
                                .child(draft.name.clone())
                                .child(hit_target(&self.hits, "add-name".into()))
                                .child(div().absolute().top_0().bottom_0().left(supraman_ui::text_input::TEXT_LEFT).right(supraman_ui::text_input::TEXT_LEFT).child(hit_target(&self.hits, "add-name-text".into()))),
                        ),
                )
                .children(draft.hint.clone().map(|h| div().text_xs().text_color(status.warning).child(SharedString::from(h))))
                .child(div().text_xs().text_color(colors.text_muted).child(SharedString::from(summary)))
                .when(!draft.empty, |d| d.child(repos))
                .child(
                    div()
                        .flex()
                        .items_start()
                        .gap_2()
                        .justify_end()
                        .children(draft.error.clone().map(|e| {
                            div()
                                .relative()
                                .flex_1()
                                .min_w_0()
                                .px_1()
                                .rounded_sm()
                                .bg(status.error.opacity(0.14))
                                .text_xs()
                                .text_color(status.error)
                                .child(SharedString::from(e))
                                .child(hit_target(&self.hits, "add-error".into()))
                        }))
                        .child(button::bordered(&self.hits, "add-save", "Save", cx).on_click(cx.listener(
                            |this, _: &ClickEvent, window, cx| {
                                cx.stop_propagation();
                                this.save_add(window, cx);
                            },
                        )))
                        .child(button::bordered(&self.hits, "add-cancel", "Cancel", cx).on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                this.cancel_add(cx);
                            },
                        ))),
                )
                .into_any_element(),
        )
    }
}

pub(crate) fn repo_label(name: &str, path: &std::path::Path, home: Option<&std::path::Path>) -> String {
    if home.is_some_and(|h| supraman_core::same_path(path, h)) { format!("{name} (home)") } else { format!("./{name}") }
}

fn folder_label(name: &str, path: &std::path::Path, home: Option<&std::path::Path>) -> String {
    if home.is_some_and(|h| supraman_core::same_path(path, h)) { format!("{name} (home)") } else { name.to_string() }
}

pub(crate) fn home_text(home: Option<&std::path::Path>) -> String {
    home.map(|h| h.display().to_string()).unwrap_or_default()
}

/// The relay glyph's state for a [`crate::supervise::RelayStatus::label`].
fn relay_word(label: &str) -> &str {
    match label {
        "relay ready" => "ready",
        "relay starting" => "starting",
        "direct" => "direct",
        _ => "expired",
    }
}

pub(crate) fn tag(text: impl Into<SharedString>, cx: &gpui::App) -> Div {
    div()
        .flex_none()
        .px_1()
        .rounded_sm()
        .border_1()
        .border_color(cx.theme().colors().border_variant)
        .text_xs()
        .text_color(secondary(cx))
        .whitespace_nowrap()
        .child(text.into())
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Segment {
    Text(String),
    Table(Vec<String>),
}

fn columnar(line: &str) -> bool {
    let body = line.trim();
    body.contains('\t') || body.contains("  ") || body.contains(['│', '─', '┼', '├', '└'])
}

fn expand_tabs(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    for c in line.chars() {
        if c == '\t' {
            let n = 8 - out.chars().count() % 8;
            out.extend(std::iter::repeat_n(' ', n));
        } else {
            out.push(c);
        }
    }
    out
}

pub(crate) fn segments(lines: &[&str]) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let run = lines[i..].iter().take_while(|l| columnar(l)).count();
        if run >= 2 {
            out.push(Segment::Table(lines[i..i + run].iter().map(|l| expand_tabs(l.trim_end())).collect()));
            i += run;
            continue;
        }
        let start = i;
        i += run.max(1);
        while i < lines.len() && !(columnar(lines[i]) && lines.get(i + 1).is_some_and(|l| columnar(l))) {
            i += 1;
        }
        let text = lines[start..i].join("\n");
        match out.last_mut() {
            Some(Segment::Text(prev)) => {
                prev.push('\n');
                prev.push_str(&text);
            }
            _ => out.push(Segment::Text(text)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_childs_icon_starts_under_its_parents_label() {
        for level in 0..3 {
            assert_eq!(grid::indent(level + 1) + px(grid::DISCLOSURE + grid::GAP), grid::indent(level) + px(grid::LABEL));
        }
    }

    #[test]
    fn folder_labels_name_the_home_and_relay_labels_shorten_to_a_state_word() {
        use std::path::Path;
        assert_eq!(repo_label("Gaia", Path::new(r"P:\x\Gaia"), Some(Path::new(r"p:/x/gaia/"))), "Gaia (home)");
        assert_eq!(repo_label("child", Path::new(r"P:\x\Gaia\child"), Some(Path::new(r"P:\x\Gaia"))), "./child");
        assert_eq!(repo_label("child", Path::new(r"P:\x\Gaia\child"), None), "./child");
        assert_eq!(folder_label("Gaia", Path::new(r"P:\x\Gaia"), Some(Path::new(r"p:/x/gaia/"))), "Gaia (home)");
        assert_eq!(folder_label("child", Path::new(r"P:\x\Gaia\child"), Some(Path::new(r"P:\x\Gaia"))), "child");
        assert_eq!(relay_word("relay watcher expired, not re-armed (armed 40 min ago)"), "expired");
        assert_eq!(relay_word("relay ready"), "ready");
    }

    #[test]
    fn work_tree_rows_are_one_table_between_prose() {
        let body = "APPROVE? Add --shout\nRuns: two\n\ne2e4\tproj\topen\te2e4\n  e2e4-shout-textkit   run   open   shout\n    e2e4-shout-textkit-fn  task  open  ✗0/3\nReply approve.";
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(
            segments(&lines),
            [
                Segment::Text("APPROVE? Add --shout\nRuns: two\n".into()),
                Segment::Table(vec![
                    "e2e4    proj    open    e2e4".into(),
                    "  e2e4-shout-textkit   run   open   shout".into(),
                    "    e2e4-shout-textkit-fn  task  open  ✗0/3".into(),
                ]),
                Segment::Text("Reply approve.".into()),
            ]
        );
    }

    #[test]
    fn one_gappy_line_or_an_indent_is_prose() {
        let lines = ["Done.  Next is m5/2.", "  indented only", "plain"];
        assert_eq!(segments(&lines), [Segment::Text(lines.join("\n"))]);
        assert_eq!(segments(&[]), []);
        assert_eq!(expand_tabs("ab\tc\t"), "ab      c       ");
    }
}
