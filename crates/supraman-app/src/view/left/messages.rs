//! The Messages section: the controllers' messages to the human, each with Reply, and the reply box.

use super::*;

pub(super) fn message_key(workspace: &str, m: &OutboxMessage) -> String {
    format!("{workspace}/{}", m.file)
}

impl FleetView {
    fn listed_messages(&self) -> Vec<(&String, &OutboxMessage)> {
        self.messages
            .iter()
            .flat_map(|(w, msgs)| msgs.iter().map(move |m| (w, m)))
            .filter(|(w, m)| !self.dismissed_messages.hides(w, &m.file))
            .collect()
    }

    #[cfg(feature = "selftest")]
    pub fn messages_count(&self) -> usize {
        self.listed_messages().len()
    }

    pub(super) fn render_messages(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
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
                                            .child(SharedString::from(supraman_core::time::ago(now - m.at))),
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
