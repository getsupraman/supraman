//! The left panel: the header, the Inbox, the Messages, notices and the workspace tree.

use std::{
    collections::{BTreeMap, HashSet},
    time::{Duration, Instant},
};

use foreman_claude::{Health, now_ms};
use foreman_core::{Ask, AskState, OutboxMessage, ask_states};
use foreman_supervisor::{NeedsKind, NeedsYou, Remedy, workspace_session_name};
use gpui::{AnyElement, ClickEvent, Context, Div, FontWeight, Hsla, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, SharedString, Stateful, div, prelude::*, px, relative};
use foreman_ui::banner;
use foreman_ui::button;
use foreman_ui::hit::{HitTargets, hit_target};
use theme::ActiveTheme as _;

use super::icon::{Icon, Tooltip};
use super::notices;
use super::menu::MenuAction;
use super::{Cli, FleetView, MenuTarget, INBOX_MAX_SHARE, MAX_WORKING_LIMIT, MESSAGE_LINES, MESSAGES_LISTED, MESSAGES_MAX_SHARE};
use crate::clipped::clipped;
use crate::settings_window::Page;
use crate::fleet::{self, SessionRow};
use crate::work_view;

/// The tree keeps at least this share of the panel's height, however many notices there are.
const TREE_MIN_SHARE: f32 = 0.3;
/// A [`Hold`] ends this long after the pointer last moved, or when it leaves the panel.
pub const HOLD_STILL: Duration = Duration::from_secs(2);

/// The Inbox and Messages rows there were when the pointer came over the left panel (m5/18). Only these
/// are drawn while it moves there, so a row arriving above it never shifts what it is aiming at; a row
/// that goes away still goes at once. The rest appear when the pointer leaves or rests for [`HOLD_STILL`].
pub(crate) struct Hold {
    keys: HashSet<String>,
    moved: Instant,
}

fn need_key(need: &NeedsYou) -> String {
    format!("need:{}/{}/{}", need.workspace, need.name, needs_label(need.kind))
}

fn work_key(workspace: &str, row: &foreman_workstate::InboxRow) -> String {
    format!("work:{workspace}/{}", row.key)
}

fn message_key(workspace: &str, m: &OutboxMessage) -> String {
    format!("{workspace}/{}", m.file)
}

/// Below each capped, scrolling section (Inbox, Messages, notices): a border that shows against the panel,
/// then this much panel background before the next, so a row cut by one section's clip never reads as
/// running into the row cut by the next (m5/16).
const SEAM_GAP: f32 = 6.;

/// The tree's text blends and their contrast floors live with the themes (m5/26, m5/31).
pub(crate) use foreman_ui::themes::{DIMMED, QUIET, SECONDARY};
use foreman_ui::themes::{MESSAGE_TINT, MESSAGE_TINT_HOVER};

/// A name that is a step back: a repo without sessions.
pub(crate) fn quiet(cx: &gpui::App) -> Hsla {
    cx.theme().colors().text.opacity(QUIET)
}

/// Branches, counts, ids, states, paths.
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

/// `2026-10-26T09:30:00+00:00` → `since 2026-10-26 10:30`, local time ([`work_view::short_time`]).
fn short_since(at: &str) -> String {
    if at.is_empty() { String::new() } else { format!("since {}", work_view::short_time(at)) }
}

/// The Inbox button's hit-target key for one need.
pub fn inbox_key(need: &NeedsYou) -> Option<String> {
    let verb = match need.action.as_ref()? {
        Remedy::Attach { .. } => "attach",
        Remedy::Retry => "retry",
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

/// One square, borderless icon button of the action bar: muted, the text colour on hover, a deeper fill
/// pressed and while `on` (a toggle that is set).
fn icon_button(hits: &HitTargets, key: &'static str, icon: Icon, tip: impl Into<SharedString>, on: bool, cx: &gpui::App) -> Stateful<Div> {
    let colors = cx.theme().colors();
    tinted_button(hits, key, icon, tip, on, if on { colors.text } else { colors.text_muted }, cx)
}

fn tinted_button(hits: &HitTargets, key: &'static str, icon: Icon, tip: impl Into<SharedString>, on: bool, fg: Hsla, cx: &gpui::App) -> Stateful<Div> {
    button::toolbar(hits, key, icon.svg(16., fg), on, fg, cx).tooltip(Tooltip::text(tip))
}

/// The tree's row heights (m5/14): one per row kind, whatever the row holds.
const WORKSPACE_ROW: f32 = 30.;
const REPO_ROW: f32 = 26.;
const SESSION_LINE: f32 = 24.;

/// The chevron of a foldable row in a fixed box, so names line up whether a row folds (`Some`) or not.
fn disclosure(open: Option<bool>, color: Hsla) -> Div {
    let slot = div().flex().flex_none().items_center().justify_center().size(px(16.));
    match open {
        Some(open) => slot.child(if open { Icon::ChevronDown } else { Icon::ChevronRight }.svg(14., color)),
        None => slot,
    }
}

/// A square glyph button inside a row ([+], ×).
fn glyph_button(hits: &HitTargets, key: String, icon: Icon, tip: impl Into<SharedString>, cx: &gpui::App) -> Stateful<Div> {
    button::icon(hits, key, icon.svg(14., cx.theme().colors().text_muted), px(20.), cx).tooltip(Tooltip::text(tip))
}

/// A session row's star (m5/27): filled in the warning colour while starred, else a muted outline.
fn star_button(hits: &HitTargets, key: String, starred: bool, cx: &gpui::App) -> Stateful<Div> {
    let (icon, fg) = if starred { (Icon::StarFilled, cx.theme().status().warning) } else { (Icon::Star, cx.theme().colors().text_muted) };
    let tip = if starred { "Unstar: shown only while it runs or stopped sessions are shown" } else { "Star: always shown, stopped or closed, however old" };
    button::ghost(hits, key, cx)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(20.))
        .rounded_sm()
        .child(icon.svg(13., fg))
        .tooltip(Tooltip::text(tip))
}

impl FleetView {
    /// The left panel's top row (m5/13): add workspace, expand / collapse all, show stopped, refresh, then the busy
    /// limit, a CLI warning when there is one, and Settings. The claude version and the poll timing live in
    /// Settings → About.
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
            "At most {n} session(s) of this Foreman working at once, across its workspaces; launches wait while it is reached. 0 pauses them all. Another Foreman's sessions are not counted."
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
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.add_workspace(window, cx))),
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
                icon_button(hits, "refresh", Icon::Refresh, "Refresh: read the running sessions and the closed ones' transcripts now (F5 outside a terminal)", false, cx)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.refresh_now(cx))),
            )
            .child(div().flex_1())
            .children(warning.map(|w| {
                tinted_button(hits, "cli-warning", Icon::Warning, w, false, status.warning, cx)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.open_settings_at(Some(Page::About), cx)))
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

    /// The pointer moved over the left panel: start a [`Hold`] on what is there now, or keep it.
    fn pointer_on_left(&mut self, cx: &mut Context<Self>) {
        if let Some(hold) = &mut self.left_hold {
            hold.moved = Instant::now();
            return;
        }
        let mut keys: HashSet<String> = self.needs.iter().map(need_key).collect();
        keys.extend(self.messages.iter().flat_map(|(w, msgs)| msgs.iter().map(move |m| message_key(w, m))));
        keys.extend(self.work_inbox.read(cx).rows().into_iter().map(|(w, r)| work_key(w, r)));
        self.left_hold = Some(Hold { keys, moved: Instant::now() });
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

    /// Whether a [`Hold`] is keeping new Inbox or Messages rows back.
    pub fn left_held(&self) -> bool {
        self.left_hold.is_some()
    }

    fn shown_now(&self, key: &str) -> bool {
        self.left_hold.as_ref().is_none_or(|h| h.keys.contains(key))
    }

    /// Every workspace folded (`true`), or everything unfolded.
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
        let mut shown: BTreeMap<notices::Notice, usize> = BTreeMap::new();
        for (kind, text) in self.plain_notices(cx) {
            let index = shown.entry(kind).or_default();
            let key = notices::dismiss_key(kind, *index);
            *index += 1;
            let label = SharedString::from(text.clone());
            let dismiss = glyph_button(&self.hits, key, Icon::Close, "Dismiss", cx).on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.dismiss_notice(kind, &text, cx);
                },
            ));
            notices.push(banner::tinted(status.warning).mx_3().my_2().child(banner::dismissable(label, dismiss)).into_any_element());
        }
        if let Some(problem) = self.workstate_notice() {
            notices.push(self.render_workstate_notice(problem, cx));
        }
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
        notices.extend(self.render_confirm(cx));
        notices.extend(self.render_add(cx));
        if self.registry_error.is_none() && self.registry_skipped.is_empty() && self.refreshes > 0 && tree.workspaces.is_empty() && self.adding.is_none() {
            notices.push(notice(
                "No workspaces registered. Add one with the folder button above, or:  fm workspace add <dir>".into(),
                banner::plain(cx),
            ));
        }

        // while the search box holds text, its results stand in for the tree (m5/30, `results.rs`), in a
        // scroll of their own that starts at the top
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
            let started = workspace.started == Some(true);
            let (state_text, state_color) = match workspace.started {
                Some(true) => ("started", status.success),
                Some(false) => ("stopped", secondary(cx)),
                None => ("", secondary(cx)),
            };
            let toggle_key = format!("{}-{}", if started { "stop" } else { "start" }, workspace.name);
            let pname = workspace.name.clone();
            let start_stop = button::bordered(&self.hits, toggle_key, if started { "Stop" } else { "Start" }, cx).on_click(
                cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    if started {
                        this.stop_workspace(&pname, cx);
                    } else {
                        this.start_workspace(&pname, cx);
                    }
                }),
            );
            // only a started workspace: a message goes direct, or waits in the inbox for the relay it starts
            // (m5/32); the path label beside it says which (a long label plus this button would push Stop past
            // the panel's edge)
            let task = (started && self.relay_refusal(&workspace.name).is_none()).then(|| {
                let w = workspace.name.clone();
                button::bordered(&self.hits, format!("task-{}", workspace.name), "Task…", cx).on_click(cx.listener(
                    move |this, _: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        this.open_compose(&w, None, window, cx);
                    },
                ))
            });
            let work_name = workspace.name.clone();
            let work = button::bordered(&self.hits, format!("work-{}", workspace.name), "Work", cx).on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.open_work(&work_name, cx);
                },
            ));
            body = body.child(
                div()
                    .id(SharedString::from(pkey.clone()))
                    .relative()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_2()
                    .h(px(WORKSPACE_ROW))
                    .pl(px(6.))
                    .pr_2()
                    .mt_2()
                    .cursor_pointer()
                    .hover(|s| s.bg(colors.ghost_element_hover))
                    .active(|s| s.bg(colors.ghost_element_active))
                    .child(disclosure(Some(open), colors.text_muted))
                    .child(
                        div()
                            .flex_none()
                            .font_weight(FontWeight::BOLD)
                            .whitespace_nowrap()
                            .child(SharedString::from(workspace.name.clone())),
                    )
                    .child(work)
                    .child(
                        clipped(format!("ws-root-{}", workspace.name), workspace.root.display().to_string(), true)
                            .flex_1()
                            .text_xs()
                            .text_color(secondary(cx)),
                    )
                    .child(
                        div()
                            .flex_none()
                            .whitespace_nowrap()
                            .text_xs()
                            .text_color(secondary(cx))
                            .child(SharedString::from(format!("{} sessions", workspace.session_count()))),
                    )
                    .child(div().flex_none().text_xs().text_color(state_color).child(state_text))
                    .children(self.relays.get(&workspace.name).and_then(|r| r.label(now_ms())).map(|(text, ok)| {
                        // truncates rather than push the buttons after it past the panel's edge
                        clipped(format!("relay-{}", workspace.name), text, false)
                            .text_xs()
                            .text_color(if ok { status.success } else { status.warning })
                    }))
                    .children(task)
                    .child(start_stop)
                    .child(hit_target(&self.hits, format!("ws-row-{}", workspace.name)))
                    .on_mouse_down(MouseButton::Right, {
                        let w = workspace.name.clone();
                        cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            this.open_menu(MenuTarget::Workspace(w.clone()), e.position, cx);
                        })
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle(pkey.clone(), cx))),
            );
            if self.compose.as_ref().is_some_and(|c| c.workspace == workspace.name && c.reply_to.is_none()) {
                body = body.children(self.render_compose(cx));
            }
            if !open {
                continue;
            }
            for row in &workspace.supervised {
                body = body.child(self.render_session(&workspace.name, "supervised", row, px(26.), cx));
            }
            for repo in &workspace.repos {
                let rkey = format!("repo:{}/{}", workspace.name, repo.name);
                let ropen = !self.collapsed.contains(&rkey);
                let empty = repo.sessions.is_empty();
                let branch = repo.branch.as_deref().map(|b| format!("({b})")).unwrap_or_default();
                let new_key = format!("new-claude-{}/{}", workspace.name, repo.name);
                let (new_cwd, new_title) = (repo.path.clone(), format!("{} · {} · claude", workspace.name, repo.name));
                let new_claude = glyph_button(&self.hits, new_key, Icon::Plus, "New claude session in this repo", cx)
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        this.new_claude(new_cwd.clone(), new_title.clone(), window, cx);
                    }));
                body = body.child(
                    div()
                        .id(SharedString::from(rkey.clone()))
                        .relative()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap_2()
                        .h(px(REPO_ROW))
                        .pl(px(26.))
                        .pr_2()
                        .text_sm()
                        .cursor_pointer()
                        .hover(|s| s.bg(colors.ghost_element_hover))
                        .active(|s| s.bg(colors.ghost_element_active))
                        .when(empty, |d| d.text_color(quiet(cx)))
                        .when(!repo.enabled, |d| d.opacity(DIMMED))
                        .child(disclosure((!empty).then_some(ropen), secondary(cx)))
                        .child(div().flex_none().whitespace_nowrap().child(SharedString::from(repo_label(&repo.name, &repo.path, &workspace.root))))
                        .child(
                            clipped(format!("branch-{}/{}", workspace.name, repo.name), branch, false)
                                .text_xs()
                                .text_color(secondary(cx)),
                        )
                        .when(!empty, |d| {
                            d.child(
                                div()
                                    .flex_none()
                                    .text_xs()
                                    .text_color(secondary(cx))
                                    .child(SharedString::from(format!("{}", repo.sessions.len()))),
                            )
                        })
                        .when(!repo.enabled, |d| {
                            d.child(
                                div()
                                    .flex_none()
                                    .px_1()
                                    .rounded_sm()
                                    .border_1()
                                    .border_color(colors.border_variant)
                                    .text_xs()
                                    .text_color(secondary(cx))
                                    .child("disabled"),
                            )
                        })
                        .child(div().flex_1())
                        .when(repo.enabled, |d| d.child(new_claude))
                        .child(hit_target(&self.hits, format!("repo-row-{}/{}", workspace.name, repo.name)))
                        .on_mouse_down(MouseButton::Right, {
                            let (w, r) = (workspace.name.clone(), repo.name.clone());
                            cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                                cx.stop_propagation();
                                this.open_menu(MenuTarget::Repo { workspace: w.clone(), repo: r.clone() }, e.position, cx);
                            })
                        })
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle(rkey.clone(), cx))),
                );
                if !ropen {
                    continue;
                }
                for row in &repo.sessions {
                    body = body.child(self.render_session(&workspace.name, &repo.name, row, px(46.), cx));
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
                            .pl(px(46. + 24.))
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
        if !query.is_empty() {
            body = self.render_results(body, &query, cx);
        }

        let toast = self.toast.as_ref().map(|t| {
            div()
                .id("toast")
                .absolute()
                .bottom_3()
                .left_3()
                .right_3()
                .flex()
                .justify_center()
                .child(
                    div()
                        .relative()
                        .max_w_full()
                        .px_3()
                        .py_1()
                        .rounded_md()
                        .border_1()
                        .border_color(status.success.opacity(0.5))
                        .bg(colors.elevated_surface_background)
                        .shadow_md()
                        .text_sm()
                        .text_color(colors.text)
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(SharedString::from(format!("✓ {}", t.text)))
                        .child(hit_target(&self.hits, "toast".into())),
                )
        });

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
            .children(self.render_inbox(cx))
            .children(self.render_messages(cx))
            // notices scroll rather than squeeze the tree below its share
            .when(!notices.is_empty(), |d| {
                d.child(
                    div()
                        .id("notices")
                        .flex()
                        .flex_col()
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

    /// One row per [`NeedsYou`] across workspaces, oldest first, then each workspace's WorkState rows
    /// not dismissed ([`WorkInbox`]); nothing when both are empty.
    fn render_inbox(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let all_work: Vec<(String, foreman_workstate::InboxRow)> =
            self.work_inbox.read(cx).rows().into_iter().map(|(w, r)| (w.to_string(), r.clone())).collect();
        let total = self.needs.len() + all_work.len();
        let work_rows: Vec<_> = all_work.into_iter().filter(|(w, r)| self.shown_now(&work_key(w, r))).collect();
        let mut needs: Vec<&NeedsYou> = self.needs.iter().filter(|n| self.shown_now(&need_key(n))).collect();
        if needs.is_empty() && work_rows.is_empty() {
            return None;
        }
        let waiting = total - needs.len() - work_rows.len();
        let colors = cx.theme().colors().clone();
        let now = now_ms();
        needs.sort_by_key(|n| (n.since, n.kind));
        let mut inbox = div()
            .id("inbox")
            .relative()
            .child(hit_target(&self.hits, "inbox-area".into()))
            .flex()
            .flex_col()
            .flex_shrink_0()
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
                    };
                    let name = need.name.clone();
                    Some(button::bordered(&self.hits, key, label, cx).on_click(cx.listener(
                        move |this, _: &ClickEvent, window, cx| match &action {
                            Remedy::Attach { id } => {
                                this.attach_by_id(&workspace, id, window, cx);
                            }
                            Remedy::Retry | Remedy::Remint { .. } => this.retry(&workspace, &name, cx),
                            Remedy::RunInTerminal { cwd, command } => {
                                this.run_in_terminal(cwd.clone(), command.clone(), window, cx);
                            }
                            Remedy::RestartElevated => this.restart_elevated(window, cx),
                        },
                    )))
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
        Some(inbox.into_any_element())
    }

    /// What WorkState lacks and how to fix it (m5/3, m5/43): Sign in when not signed in (Cancel while the
    /// browser page waits), Retry when the server is down, the Settings page, × for this session.
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
                this.open_settings_at(Some(Page::WorkState), cx);
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
            .child(banner::dismissable(SharedString::from(problem), dismiss))
            .children(self.workstate.fix().map(|fix| div().text_xs().text_color(colors.text).child(SharedString::from(fix))))
            .children(self.signing_in.as_ref().map(|url| {
                div().text_xs().text_color(colors.text).child(SharedString::from(format!("Waiting for the sign-in to {url} in your browser…")))
            }))
            .children(self.sign_in_error.clone().map(|e| div().text_xs().text_color(status.error).child(SharedString::from(e))))
            .child(actions)
            .child(hit_target(&self.hits, "ws-setup-notice".into()))
            .into_any_element()
    }

    /// A WorkState inbox row: kind, workspace and item ref, since, what, and the MCP call that would
    /// resolve it (shown, never run: C21). A click opens the item page; × dismisses it here only.
    fn render_work_row(&self, workspace: &str, row: &foreman_workstate::InboxRow, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let color = work_kind_color(&row.kind, cx);
        let key = work_view::row_key(row);
        let dismiss_key = work_view::dismiss_key(row);
        let (open_ws, item) = (workspace.to_string(), row.item.id.clone());
        let (dismiss_ws, row_id) = (workspace.to_string(), row.key.clone());
        let inbox = self.work_inbox.clone();
        let dismiss = glyph_button(&self.hits, dismiss_key, Icon::Close, "Dismiss here (WorkState is not changed)", cx)
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
                                "WorkState · {}",
                                work_view::kind_label(&row.kind)
                            ))))
                            .child(SharedString::from(format!("{workspace} · {}", row.item.qualified_ref)))
                            .child(div().text_xs().text_color(colors.text_muted).child(SharedString::from(short_since(&row.since)))),
                    )
                    .child(div().text_xs().text_color(colors.text).child(SharedString::from(row.what.clone())))
                    .when(!row.call.is_empty(), |d| {
                        d.child(
                            clipped(format!("work-call-{workspace}/{}", row.item.qualified_ref), row.call.clone(), false)
                                .font_family(foreman_ui::fonts::MONO)
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

    /// Every message not dismissed, by workspace, oldest first.
    fn listed_messages(&self) -> Vec<(&String, &OutboxMessage)> {
        self.messages
            .iter()
            .flat_map(|(w, msgs)| msgs.iter().map(move |m| (w, m)))
            .filter(|(w, m)| !self.dismissed_messages.hides(w, &m.file))
            .collect()
    }

    /// The count the MESSAGES header shows.
    pub fn messages_count(&self) -> usize {
        self.listed_messages().len()
    }

    /// What the workspaces' relays received, newest first, across workspaces, less the dismissed ones;
    /// nothing when empty. A body shows its first lines until clicked; Attach opens the sender's session
    /// when it is attachable; × hides it here.
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
            .flex_shrink_0()
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
                    // unwrapped and clipped at the panel's edge: a wrapped column is unreadable
                    Segment::Table(rows) => div().flex().flex_col().py_1().overflow_hidden().font_family(foreman_ui::fonts::MONO).children(
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
                            this.open_compose(&w, Some(file.clone()), window, cx);
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
                .is_some_and(|c| &c.workspace == workspace && c.reply_to.as_deref() == Some(m.file.as_str()));
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
                                    // the tint and the edge carry the kind; the name reads as a name (m5/36)
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

    /// The open [`Compose`]: its text input, what Ctrl+Enter does, why the relay cannot take it now, the
    /// last error or the last task sent; Send and Cancel.
    fn render_compose(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let c = self.compose.as_ref()?;
        let colors = cx.theme().colors().clone();
        let status = cx.theme().status().clone();
        let ctrl = workspace_session_name(&c.workspace);
        let title = match &c.reply_to {
            Some(_) => format!("Reply to {ctrl}"),
            None => format!("Task for {ctrl}"),
        };
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
                .children(c.sent.clone().map(|s| line(format!("{}: {s}", c.route.as_deref().unwrap_or("sent")), status.success)))
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
        // another Foreman's row (m5/10): its prompt is not this user's, so nothing about it asks for attention
        let foreign = row.foreign;
        let dot = if foreign { colors.text_muted } else { health_color(&row.health, cx) };
        // a gone or stopped session's state reads as secondary text, not in its grey dot's colour
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
            .filter(|_| s.name.starts_with(foreman_core::FM_PREFIX))
            .map(|h| format!("hook: {}", h.label(now_ms())));

        let line = div()
            .flex()
            .items_center()
            .gap_2()
            .h(px(SESSION_LINE))
            .child(div().flex().flex_none().items_center().justify_center().size(px(16.)).child(div().size(px(8.)).rounded_full().bg(dot)))
            .child(
                div()
                    .w(px(78.))
                    .flex_none()
                    .text_xs()
                    .text_color(state)
                    .child(SharedString::from(if row.closed { "closed".to_string() } else { fleet::health_label(&row.health) })),
            )
            .child(match marks {
                Some(m) if !s.name.is_empty() => super::results::marked(format!("session-name-{}", s.id), name, &m.name, cx).flex_1(),
                _ => clipped(format!("session-name-{}", s.id), name, false).flex_1(),
            })
            .child(
                div()
                    .flex_none()
                    .whitespace_nowrap()
                    .text_xs()
                    .text_color(secondary(cx))
                    .child(match marks.and_then(|m| m.id.clone()) {
                        // a search result's id prefix is marked (m5/30)
                        Some(id) => {
                            let kind = format!("{} · ", s.kind.as_str());
                            let at = id.start + kind.len()..id.end + kind.len();
                            super::results::marked(format!("session-id-{}", s.id), format!("{kind}{}", fleet::short_id(s)), &[at], cx).into_any_element()
                        }
                        None => SharedString::from(format!("{} · {}", s.kind.as_str(), fleet::short_id(s))).into_any_element(),
                    }),
            )
            .child({
                let session_id = s.session_id.clone();
                star_button(&self.hits, format!("star-{}", s.session_id), row.starred, cx).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        this.toggle_star(&session_id, cx);
                    },
                ))
            })
            .when(!row.attachable && self.resumable(row), |d| {
                let key = format!("resume-{}", s.id);
                let (workspace, repo, session) = (workspace.to_string(), repo.to_string(), s.clone());
                d.child(
                    button::pill(&self.hits, key, "resume", secondary(cx), cx)
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.resume(&workspace, &repo, &session, window, cx);
                        })),
                )
            })
            .when_some(self.session_tab(s).filter(|&id| self.active != Some(id)), |d, tab| {
                d.child(
                    button::pill(&self.hits, format!("tab-link-{}", s.id), "tab", secondary(cx), cx)
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.activate_tab(tab, cx))),
                )
            })
            .when(row.attachable, |d| {
                let key = format!("attach-{}", s.id);
                let (workspace, repo, session) = (workspace.to_string(), repo.to_string(), s.clone());
                d.child(
                    button::pill(&self.hits, key, "attach", secondary(cx), cx)
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.attach(&workspace, &repo, &session, window, cx);
                        })),
                )
            });

        let target = MenuTarget::Session { workspace: workspace.to_string(), repo: repo.to_string(), session_id: s.session_id.clone() };
        // the row the active tab runs (m5/33)
        let shown = self.is_marked(s);
        div()
            .id(SharedString::from(format!("session-{}", s.id)))
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
            .when(foreign, |d| {
                let key = format!("foreign-{}", s.session_id);
                d.child(
                    div()
                        .id(SharedString::from(key.clone()))
                        .relative()
                        .self_start()
                        .mb(px(2.))
                        .ml(px(24.))
                        .px_1()
                        .rounded_sm()
                        .border_1()
                        .border_color(colors.border_variant)
                        .text_xs()
                        .text_color(secondary(cx))
                        .child("another Foreman")
                        .child(hit_target(&self.hits, key))
                        .tooltip(Tooltip::text(
                            "An fm- session this Foreman does not supervise (another FOREMAN_HOME, e.g. a selftest): \
                             it never raises an Inbox row here. Leave its prompts to whoever runs it.",
                        )),
                )
            })
            .when_some(waiting_for, |d, w| {
                d.child(
                    clipped(format!("waiting-{}", s.id), w, false)
                        .pl(px(24.))
                        .text_xs()
                        .text_color(if foreign { secondary(cx) } else { status.warning }),
                )
            })
            .when_some(hook, |d, h| {
                d.child(clipped(format!("hook-{}", s.id), h, false).pl(px(24.)).text_xs().text_color(secondary(cx)))
            })
            .child(hit_target(&self.hits, format!("session-row-{}", s.session_id)))
            .when(shown, |d| d.child(hit_target(&self.hits, format!("marked-{}", s.session_id))))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.open_menu(target.clone(), e.position, cx);
                }),
            )
            .into_any_element()
    }

    /// The open question (Remove…, a Stop that asks): its text, the action's button and Cancel.
    fn render_confirm(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let confirm = self.confirm.as_ref()?;
        let status = cx.theme().status().clone();
        Some(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .mx_3()
                .my_2()
                .p_2()
                .rounded_md()
                .border_1()
                .border_color(status.warning)
                .bg(banner::tint(status.warning))
                .text_sm()
                .child(SharedString::from(confirm.text.clone()))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .justify_end()
                        .child(button::bordered(&self.hits, "confirm-yes", confirm.yes.clone(), cx).on_click(cx.listener(
                            |this, _: &ClickEvent, window, cx| {
                                cx.stop_propagation();
                                this.answer_confirm(true, window, cx);
                            },
                        )))
                        // a report (m5/38's bulk delete) has only its OK
                        .when(confirm.action != MenuAction::Dismiss, |d| {
                            d.child(button::bordered(&self.hits, "confirm-cancel", "Cancel", cx).on_click(cx.listener(
                                |this, _: &ClickEvent, window, cx| {
                                    cx.stop_propagation();
                                    this.answer_confirm(false, window, cx);
                                },
                            )))
                        }),
                )
                .into_any_element(),
        )
    }

    /// The Add workspace review: folder, name (click to edit), the repos found with a tick each, the
    /// reason it cannot be saved yet, Save and Cancel.
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
                    .child(SharedString::from(repo_label(&repo.name, &repo.path, &draft.root)))
                    .child(div().text_xs().text_color(colors.text_muted).child(SharedString::from(branch)))
                    .child(hit_target(&self.hits, key))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        this.toggle_add_repo(i, cx);
                    })),
            );
        }
        let ticked = draft.repos.iter().filter(|(_, on)| *on).count();
        let summary = format!("{ticked} of {} repos enabled; unticked ones are registered disabled", draft.repos.len());
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
                        .child("ADD WORKSPACE")
                        .child("Enter saves · Esc cancels"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.text_muted)
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .child(SharedString::from(draft.root.display().to_string())),
                )
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
                                // the text starts 5 px in (the input's border and padding)
                                .child(hit_target(&self.hits, "add-name".into()))
                                .child(div().absolute().top_0().bottom_0().left(px(5.)).right(px(5.)).child(hit_target(&self.hits, "add-name-text".into()))),
                        ),
                )
                .children(draft.hint.clone().map(|h| div().text_xs().text_color(status.warning).child(SharedString::from(h))))
                .child(div().text_xs().text_color(colors.text_muted).child(SharedString::from(summary)))
                .child(repos)
                .child(
                    // why it cannot be saved sits next to Save, tinted, where the click lands (m5/26)
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
                            |this, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                this.save_add(cx);
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

/// How a repo row names it: `./<name>` below the workspace root, `<name> (root)` for a root that is
/// itself the repo (m5/26).
pub(crate) fn repo_label(name: &str, path: &std::path::Path, root: &std::path::Path) -> String {
    if foreman_core::same_root(path, root) { format!("{name} (root)") } else { format!("./{name}") }
}

/// A message body cut into prose and aligned-column runs (m5/18): a run is two or more lines in a row that
/// each hold a tab or a gap of two or more spaces after their first word, or a box-drawing rule, the
/// shape of a `work_tree` table. Tabs become spaces to the next multiple of 8.
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

    /// WCAG 2 contrast of `fg` (its alpha blended over `bg`) against the opaque `bg`.
    fn contrast(fg: Hsla, bg: Hsla) -> f32 {
        let (f, b) = (gpui::Rgba::from(fg), gpui::Rgba::from(bg));
        let blend = |f: f32, b: f32| f * fg.a + b * (1. - fg.a);
        let lin = |c: f32| if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
        let lum = |r: f32, g: f32, b: f32| 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
        let (lf, lb) = (lum(blend(f.r, b.r), blend(f.g, b.g), blend(f.b, b.b)), lum(b.r, b.g, b.b));
        (lf.max(lb) + 0.05) / (lf.min(lb) + 0.05)
    }

    /// m5/26: names >= 4.5:1 and secondary text >= 3:1 against the panel, also in a dimmed row, and the
    /// hierarchy kept: name > quiet > secondary.
    #[test]
    fn the_tree_text_is_readable_against_the_panel() {
        let registry = theme::ThemeRegistry::default();
        let name = registry.list().into_iter().next().expect("fallback theme").name;
        let theme = registry.get(&name).expect("fallback theme");
        let (text, bg) = (theme.colors().text, theme.colors().panel_background);
        let ratio = |alpha: f32| contrast(text.opacity(alpha), bg);
        let (name, quiet, secondary) = (ratio(1.), ratio(QUIET), ratio(SECONDARY));
        let (dim_quiet, dim_secondary) = (ratio(QUIET * DIMMED), ratio(SECONDARY * DIMMED));
        assert!(name > quiet && quiet > secondary, "{name} > {quiet} > {secondary}");
        assert!(quiet >= 4.5 && dim_quiet >= 4.5, "names: {quiet}, dimmed {dim_quiet}");
        assert!(secondary >= 3. && dim_secondary >= 3., "secondary: {secondary}, dimmed {dim_secondary}");
        assert!(contrast(theme.colors().text_muted, bg) < 3.5, "text_muted ({}) was the too-dim colour", contrast(theme.colors().text_muted, bg));
    }

    #[test]
    fn repo_labels_name_a_root_repo_as_the_root() {
        use std::path::Path;
        assert_eq!(repo_label("Gaia", Path::new(r"P:\x\Gaia"), Path::new(r"p:/x/gaia/")), "Gaia (root)");
        assert_eq!(repo_label("child", Path::new(r"P:\x\Gaia\child"), Path::new(r"P:\x\Gaia")), "./child");
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
