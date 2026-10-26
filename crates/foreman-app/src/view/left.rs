//! The left panel: the header, the Inbox, the Messages, notices and the workspace tree.

use std::collections::BTreeMap;

use foreman_claude::{Health, now_ms};
use foreman_core::{Ask, AskState, OutboxMessage, ask_states};
use foreman_supervisor::{NeedsKind, NeedsYou, Remedy, workspace_session_name};
use gpui::{AnyElement, ClickEvent, Context, FontWeight, Hsla, Pixels, SharedString, div, prelude::*, px, relative};
use theme::ActiveTheme as _;

use super::{Cli, FleetView, INBOX_MAX_SHARE, MESSAGE_LINES, MESSAGES_LISTED, MESSAGES_MAX_SHARE, REFRESH_EVERY, button, hit_target};
use crate::fleet::{self, SessionRow};
use crate::work_view;


fn needs_color(kind: NeedsKind, cx: &gpui::App) -> Hsla {
    let status = cx.theme().status();
    match kind {
        NeedsKind::Blocked | NeedsKind::TrustNeeded => status.warning,
        NeedsKind::GaveUp | NeedsKind::LaunchFailed => status.error,
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

/// `2026-10-26T09:30:00+00:00` → `since 2026-10-26 09:30`.
fn short_since(at: &str) -> String {
    if at.is_empty() { String::new() } else { format!("since {}", at.get(..16).unwrap_or(at).replace('T', " ")) }
}

/// The Inbox button's hit-target key for one need.
pub fn inbox_key(need: &NeedsYou) -> Option<String> {
    let verb = match need.action.as_ref()? {
        Remedy::Attach { .. } => "attach",
        Remedy::Retry => "retry",
        Remedy::RunInTerminal { .. } => "run",
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

impl FleetView {
    pub(super) fn render_left(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let status = cx.theme().status().clone();
        let tree = self.tree();

        let cli_line: SharedString = match &self.cli {
            Cli::Checking => "checking the claude CLI…".into(),
            Cli::Supported(v) => format!("claude {}", v.raw).into(),
            Cli::Failed(_) => "claude CLI unavailable".into(),
        };
        let refresh_line: SharedString = match self.last_refresh_took {
            Some(took) => format!("refresh every {} s · last {} ms", REFRESH_EVERY.as_secs(), took.as_millis()).into(),
            None => "loading…".into(),
        };
        let stopped_label: SharedString = if self.show_stopped {
            "hide stopped".into()
        } else {
            format!("show stopped ({})", tree.hidden_stopped).into()
        };
        let busy: SharedString = format!("busy ≤ {}", self.max_working).into();

        let header = div()
            .flex()
            .flex_col()
            .gap_1()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(colors.border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .gap_2()
                            .text_xs()
                            .text_color(colors.text_muted)
                            .child(cli_line)
                            .child("·")
                            .child(refresh_line),
                    )
                    .child(
                        div()
                            .id("busy-limit")
                            .flex_none()
                            .px_2()
                            .rounded_sm()
                            .text_xs()
                            .text_color(colors.text_muted)
                            .cursor_pointer()
                            .hover(|s| s.bg(colors.ghost_element_hover))
                            .child(busy)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.open_settings(cx))),
                    )
                    .child(
                        div()
                            .id("toggle-stopped")
                            .flex_none()
                            .px_2()
                            .rounded_sm()
                            .text_xs()
                            .text_color(colors.text_muted)
                            .cursor_pointer()
                            .hover(|s| s.bg(colors.ghost_element_hover))
                            .child(stopped_label)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.show_stopped = !this.show_stopped;
                                cx.notify();
                            })),
                    ),
            );

        let mut notices: Vec<AnyElement> = Vec::new();
        // A tint of the text colour, never the theme's `*_background` fills (unreadable in m1).
        let notice = |text: String, fg: Hsla, bg: Hsla| {
            div()
                .mx_3()
                .my_2()
                .p_2()
                .rounded_md()
                .bg(bg)
                .text_sm()
                .text_color(fg)
                .child(SharedString::from(text))
                .into_any_element()
        };
        if let Cli::Failed(error) = &self.cli {
            notices.push(notice(
                format!("Cannot use the claude CLI, so no sessions are shown: {error}"),
                status.error,
                status.error.opacity(0.12),
            ));
        }
        if let Cli::Supported(version) = &self.cli
            && let Some(warning) = version.untested()
        {
            notices.push(notice(warning, status.warning, status.warning.opacity(0.12)));
        }
        if let Some(error) = &self.registry_error {
            notices.push(notice(
                format!("Cannot read the workspace registry: {error}"),
                status.error,
                status.error.opacity(0.12),
            ));
        }
        if let Some(error) = &self.sessions_error {
            notices.push(notice(
                format!("Session list is stale, the last read failed (nothing is started or stopped meanwhile): {error}"),
                status.warning,
                status.warning.opacity(0.12),
            ));
        }
        for text in self.setup_notices.iter().chain(&self.supervise_errors).chain(&self.action_error) {
            notices.push(notice(text.clone(), status.warning, status.warning.opacity(0.12)));
        }
        if self.registry_error.is_none() && self.refreshes > 0 && tree.workspaces.is_empty() {
            notices.push(notice(
                "No workspaces registered. Add one with:  fm workspace add <dir>".into(),
                colors.text,
                colors.element_background,
            ));
        }

        let mut body = div()
            .id("fleet")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .pb_4();
        for workspace in &tree.workspaces {
            let pkey = format!("workspace:{}", workspace.name);
            let open = !self.collapsed.contains(&pkey);
            let started = workspace.started == Some(true);
            let (state_text, state_color) = match workspace.started {
                Some(true) => ("started", status.success),
                Some(false) => ("stopped", colors.text_muted),
                None => ("", colors.text_muted),
            };
            let toggle_key = format!("{}-{}", if started { "stop" } else { "start" }, workspace.name);
            let pname = workspace.name.clone();
            let start_stop = button(&self.hits, toggle_key, if started { "Stop" } else { "Start" }, cx).on_click(
                cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    if started {
                        this.stop_workspace(&pname, cx);
                    } else {
                        this.start_workspace(&pname, cx);
                    }
                }),
            );
            // only a workspace whose relay can take it now; otherwise the relay label beside it says why (and
            // a long label plus this button would push Stop past the panel's edge)
            let task = (started && self.relay_refusal(&workspace.name).is_none()).then(|| {
                let w = workspace.name.clone();
                button(&self.hits, format!("task-{}", workspace.name), "Task…", cx).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        this.open_compose(&w, None, cx);
                    },
                ))
            });
            let work_name = workspace.name.clone();
            let work = button(&self.hits, format!("work-{}", workspace.name), "Work", cx).on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.open_work(&work_name, cx);
                },
            ));
            body = body.child(
                div()
                    .id(SharedString::from(pkey.clone()))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_1()
                    .mt_2()
                    .cursor_pointer()
                    .hover(|s| s.bg(colors.ghost_element_hover))
                    .child(if open { "▾" } else { "▸" })
                    .child(
                        div()
                            .font_weight(FontWeight::BOLD)
                            .child(SharedString::from(workspace.name.clone())),
                    )
                    .child(work)
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .text_xs()
                            .text_color(colors.text_muted)
                            .whitespace_nowrap()
                            .child(SharedString::from(workspace.root.display().to_string())),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(colors.text_muted)
                            .child(SharedString::from(format!("{} sessions", workspace.session_count()))),
                    )
                    .child(div().text_xs().text_color(state_color).child(state_text))
                    .children(self.relays.get(&workspace.name).and_then(|r| r.label(now_ms())).map(|(text, ok)| {
                        div()
                            .id(SharedString::from(format!("relay-{}", workspace.name)))
                            // truncates rather than push the buttons after it past the panel's edge
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_xs()
                            .text_color(if ok { status.success } else { status.warning })
                            .child(SharedString::from(text))
                    }))
                    .children(task)
                    .child(start_stop)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle(pkey.clone(), cx))),
            );
            if self.compose.as_ref().is_some_and(|c| c.workspace == workspace.name && c.reply_to.is_none()) {
                body = body.children(self.render_compose(cx));
            }
            if !open {
                continue;
            }
            for row in &workspace.supervised {
                body = body.child(self.render_session(&workspace.name, "supervised", row, px(28.), cx));
            }
            for repo in &workspace.repos {
                let rkey = format!("repo:{}/{}", workspace.name, repo.name);
                let ropen = !self.collapsed.contains(&rkey);
                let empty = repo.sessions.is_empty();
                let branch = repo.branch.as_deref().map(|b| format!("({b})")).unwrap_or_default();
                let new_key = format!("new-claude-{}/{}", workspace.name, repo.name);
                let (new_cwd, new_title) = (repo.path.clone(), format!("{} · {} · claude", workspace.name, repo.name));
                let new_claude = div()
                    .id(SharedString::from(new_key.clone()))
                    .relative()
                    .flex_none()
                    .px_1()
                    .rounded_sm()
                    .text_color(colors.text_muted)
                    .hover(|s| s.bg(colors.element_hover).text_color(colors.text))
                    .child("+")
                    .child(hit_target(&self.hits, new_key))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        this.new_claude(new_cwd.clone(), new_title.clone(), window, cx);
                    }));
                body = body.child(
                    div()
                        .id(SharedString::from(rkey.clone()))
                        .flex()
                        .items_center()
                        .gap_2()
                        .pl(px(28.))
                        .pr_3()
                        .py(px(2.))
                        .text_sm()
                        .cursor_pointer()
                        .hover(|s| s.bg(colors.ghost_element_hover))
                        .when(empty, |d| d.text_color(colors.text_muted))
                        .child(if empty { " " } else if ropen { "▾" } else { "▸" })
                        .child(SharedString::from(format!("./{}", repo.name)))
                        .child(
                            div()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_xs()
                                .text_color(colors.text_muted)
                                .child(SharedString::from(branch)),
                        )
                        .when(!empty, |d| {
                            d.child(
                                div()
                                    .text_xs()
                                    .text_color(colors.text_muted)
                                    .child(SharedString::from(format!("{}", repo.sessions.len()))),
                            )
                        })
                        .child(div().flex_1())
                        .child(new_claude)
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle(rkey.clone(), cx))),
                );
                if !ropen {
                    continue;
                }
                for row in &repo.sessions {
                    body = body.child(self.render_session(&workspace.name, &repo.name, row, px(48.), cx));
                }
            }
        }

        div()
            .flex()
            .flex_col()
            .w(self.left_width)
            .flex_none()
            .h_full()
            .bg(colors.panel_background)
            .child(header)
            .children(self.render_inbox(cx))
            .children(self.render_messages(cx))
            .children(notices)
            .child(body)
            .into_any_element()
    }

    /// One row per [`NeedsYou`] across workspaces, oldest first, then each workspace's WorkState rows
    /// not dismissed ([`WorkInbox`]); nothing when both are empty.
    fn render_inbox(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let work_rows: Vec<(String, foreman_workstate::InboxRow)> =
            self.work_inbox.read(cx).rows().into_iter().map(|(w, r)| (w.to_string(), r.clone())).collect();
        if self.needs.is_empty() && work_rows.is_empty() {
            return None;
        }
        let colors = cx.theme().colors().clone();
        let now = now_ms();
        let mut needs: Vec<&NeedsYou> = self.needs.iter().collect();
        needs.sort_by_key(|n| (n.since, n.kind));
        let mut inbox = div()
            .id("inbox")
            .flex()
            .flex_col()
            .flex_shrink_0()
            .max_h(relative(INBOX_MAX_SHARE))
            .overflow_y_scroll()
            .gap_1()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(colors.border)
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::BOLD)
                    .text_color(colors.text_muted)
                    .child(SharedString::from(format!("INBOX · {}", needs.len() + work_rows.len()))),
            );
        for need in needs {
            let color = needs_color(need.kind, cx);
            let remedy = match (&need.action, inbox_key(need)) {
                (Some(action), Some(key)) => {
                    let (label, action, workspace) = match action {
                        Remedy::Attach { .. } => ("Attach", action.clone(), need.workspace.clone()),
                        Remedy::Retry => ("Retry", action.clone(), need.workspace.clone()),
                        Remedy::RunInTerminal { .. } => ("Run in terminal", action.clone(), need.workspace.clone()),
                    };
                    let name = need.name.clone();
                    Some(button(&self.hits, key, label, cx).on_click(cx.listener(
                        move |this, _: &ClickEvent, window, cx| match &action {
                            Remedy::Attach { id } => {
                                this.attach_by_id(&workspace, id, window, cx);
                            }
                            Remedy::Retry => this.retry(&workspace, &name, cx),
                            Remedy::RunInTerminal { cwd, command } => {
                                this.run_in_terminal(cwd.clone(), command.clone(), window, cx);
                            }
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
        let dismiss = div()
            .id(SharedString::from(dismiss_key.clone()))
            .relative()
            .flex_none()
            .px_1()
            .rounded_sm()
            .text_xs()
            .text_color(colors.text_muted)
            .cursor_pointer()
            .hover(|s| s.bg(colors.element_hover).text_color(colors.text))
            .child("×")
            .child(hit_target(&self.hits, dismiss_key))
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
                            div()
                                .font_family("Cascadia Mono")
                                .text_xs()
                                .text_color(colors.text_muted)
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(SharedString::from(row.call.clone())),
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

    /// What the workspaces' relays received, newest first, across workspaces; nothing when empty. A body
    /// shows its first lines until clicked; Attach opens the sender's session when it is attachable.
    fn render_messages(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let mut all: Vec<(&String, &OutboxMessage)> =
            self.messages.iter().flat_map(|(w, msgs)| msgs.iter().map(move |m| (w, m))).collect();
        if all.is_empty() {
            return None;
        }
        all.sort_by_key(|(_, m)| std::cmp::Reverse((m.at, m.file.clone())));
        let total = all.len();
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
            .flex()
            .flex_col()
            .flex_shrink_0()
            .max_h(relative(MESSAGES_MAX_SHARE))
            .overflow_y_scroll()
            .gap_1()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(colors.border)
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::BOLD)
                    .text_color(colors.text_muted)
                    .child(SharedString::from(format!("MESSAGES · {total}"))),
            );
        for (workspace, m) in all {
            let key = format!("{workspace}/{}", m.file);
            let color = if m.is_notice() || m.error.is_some() { status.warning } else { status.info };
            let sender = match (&m.error, m.from_name.as_str()) {
                (Some(_), _) => format!("unreadable {}", m.file),
                (None, "") => "?".to_string(),
                (None, name) => name.to_string(),
            };
            let lines: Vec<&str> = m.body.lines().collect();
            let open = self.expanded.contains(&key);
            let shown = if open { lines.len() } else { lines.len().min(MESSAGE_LINES) };
            let mut body = lines[..shown].join("\n");
            if shown < lines.len() {
                body.push_str(&format!("\n… {} more line(s), click to show", lines.len() - shown));
            }
            // every row, not the tree's: a sender in the workspace root but outside its repos has no tree row
            let target = self
                .sessions
                .iter()
                .find(|s| !m.from_name.is_empty() && s.name == m.from_name && fleet::attachable(s, &s.health(now)))
                .map(|s| s.id.clone());
            let attach = target.map(|id| {
                let workspace = workspace.clone();
                button(&self.hits, format!("msg-attach-{key}"), "Attach", cx).on_click(cx.listener(
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
                    button(&self.hits, format!("msg-reply-{key}"), "Reply…", cx).on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            cx.stop_propagation();
                            this.open_compose(&w, Some(file.clone()), cx);
                        },
                    ))
                };
                match state {
                    AskState::Open(Ask::Approve) => {
                        let (w, file) = (workspace.clone(), m.file.clone());
                        row.child(button(&self.hits, format!("msg-approve-{key}"), "Approve", cx).on_click(cx.listener(
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
                    .bg(color.opacity(0.10))
                    .cursor_pointer()
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
                                    .child(div().font_weight(FontWeight::BOLD).text_color(color).child(SharedString::from(sender)))
                                    .child(div().text_color(colors.text_muted).child(SharedString::from(workspace.clone())))
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(colors.text_muted)
                                            .child(SharedString::from(fleet::ago(now - m.at))),
                                    ),
                            )
                            .child(div().text_xs().text_color(colors.text).child(SharedString::from(body)))
                            .children(answer),
                    )
                    .children(attach)
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

    /// The open [`Compose`]: its text with a caret while focused, what Ctrl+Enter does, why the relay
    /// cannot take it now, the last error or the last task sent; Send and Cancel.
    fn render_compose(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let c = self.compose.as_ref()?;
        let colors = cx.theme().colors().clone();
        let status = cx.theme().status().clone();
        let ctrl = workspace_session_name(&c.workspace);
        let title = match &c.reply_to {
            Some(_) => format!("Reply to {ctrl}"),
            None => format!("Task for {ctrl}"),
        };
        let caret = if c.focused { "▏" } else { "" };
        let (before, after) = c.text.split_at(c.cursor());
        let text = if c.text.is_empty() {
            div()
                .text_color(colors.text_muted)
                .child(SharedString::from(format!("{caret}Type here; Enter adds a line.")))
        } else {
            div().text_color(colors.text).child(SharedString::from(format!("{before}{caret}{after}")))
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
                .border_color(if c.focused { colors.border_focused } else { colors.border })
                .bg(colors.editor_background)
                .cursor_text()
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
                .child(div().min_h(px(48.)).text_sm().child(text))
                .children(refusal.map(|why| line(format!("Cannot send now: {why}."), status.warning)))
                .children(c.error.clone().map(|e| line(e, status.error)))
                .children(c.sent.clone().map(|s| line(format!("{}: {s}", c.route.as_deref().unwrap_or("sent")), status.success)))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .justify_end()
                        .child(button(&self.hits, "compose-send".into(), "Send", cx).on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                this.send_compose(cx);
                            },
                        )))
                        .child(button(&self.hits, "compose-cancel".into(), "Cancel", cx).on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                this.compose = None;
                                cx.notify();
                            },
                        ))),
                )
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    if let Some(c) = &mut this.compose {
                        c.focused = true;
                    }
                    cx.notify();
                }))
                .into_any_element(),
        )
    }

    fn render_session(
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
        let dot = health_color(&row.health, cx);
        let blocked = matches!(row.health, Health::Blocked { .. });
        let dimmed = matches!(row.health, Health::Orphaned { .. } | Health::Stopped);
        let waiting_for = match &row.health {
            Health::Blocked { waiting_for: Some(w) } => Some(format!("waiting for: {w}")),
            _ => None,
        };
        let name = if s.name.is_empty() { "(unnamed)".to_string() } else { s.name.clone() };
        let hook = row
            .hook
            .as_ref()
            .filter(|_| s.name.starts_with("fm-"))
            .map(|h| format!("hook: {}", h.label(now_ms())));

        let line = div()
            .flex()
            .items_center()
            .gap_2()
            .child(div().size(px(8.)).flex_none().rounded_full().bg(dot))
            .child(
                div()
                    .w(px(78.))
                    .flex_none()
                    .text_xs()
                    .text_color(dot)
                    .child(SharedString::from(fleet::health_label(&row.health))),
            )
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(SharedString::from(name)),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(colors.text_muted)
                    .child(SharedString::from(format!("{} · {}", s.kind.as_str(), fleet::short_id(s)))),
            )
            .when(!row.attachable && self.resumable(row), |d| {
                let key = format!("resume-{}", s.id);
                let (workspace, repo, session) = (workspace.to_string(), repo.to_string(), s.clone());
                d.child(
                    div()
                        .id(SharedString::from(key.clone()))
                        .relative()
                        .px_2()
                        .rounded_sm()
                        .border_1()
                        .border_color(colors.border)
                        .text_xs()
                        .cursor_pointer()
                        .hover(|s| s.bg(colors.element_hover))
                        .child("resume")
                        .child(hit_target(&self.hits, key))
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.resume(&workspace, &repo, &session, window, cx);
                        })),
                )
            })
            .when(row.attachable, |d| {
                let key = format!("attach-{}", s.id);
                let (workspace, repo, session) = (workspace.to_string(), repo.to_string(), s.clone());
                d.child(
                    div()
                        .id(SharedString::from(key.clone()))
                        .relative()
                        .px_2()
                        .rounded_sm()
                        .border_1()
                        .border_color(colors.border)
                        .text_xs()
                        .cursor_pointer()
                        .hover(|s| s.bg(colors.element_hover))
                        .child("attach")
                        .child(hit_target(&self.hits, key))
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.attach(&workspace, &repo, &session, window, cx);
                        })),
                )
            });

        div()
            .flex()
            .flex_col()
            .pl(indent)
            .pr_3()
            .py(px(3.))
            .text_sm()
            .when(blocked, |d| d.bg(status.warning.opacity(0.12)))
            .when(dimmed, |d| d.opacity(0.5))
            .child(line)
            .when_some(waiting_for, |d, w| {
                d.child(
                    div()
                        .pl(px(16.))
                        .text_xs()
                        .text_color(status.warning)
                        .child(SharedString::from(w)),
                )
            })
            .when_some(hook, |d, h| {
                d.child(
                    div()
                        .pl(px(16.))
                        .text_xs()
                        .text_color(colors.text_muted)
                        .child(SharedString::from(h)),
                )
            })
            .into_any_element()
    }
}
