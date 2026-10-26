//! The item page (m4/3): a board row's `GET /api/v1/items/{id}` in a pane right of the board, in the
//! same Work tab. What was asked (intent, verify command, criteria), what the agent **said** (reports,
//! muted and marked "claimed") against what was **proven** (verifications, with a pass/fail mark), and
//! the rest of the v1 item; every section is hidden when empty. The claim's session joins the fleet on
//! `claudeSessionId`, so a live one offers Attach. Back steps through the items navigated to; Escape
//! closes the pane.

use std::{cell::RefCell, rc::Rc};

use foreman_workstate::{Claim, ItemPage, ItemResolution, Report, Verification};
use gpui::Stateful;

use super::*;

/// A fleet session Attach can open: `claude agents` row's Claude uuid, 8-hex id and name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveSession {
    pub claude_session_id: String,
    pub id: String,
    pub name: String,
}

/// What a Work tab knows of the fleet: the attachable sessions (refreshed by each poll) and the
/// fleet's own attach, by 8-hex id.
#[derive(Clone)]
pub struct FleetLink {
    pub live: Rc<RefCell<Vec<LiveSession>>>,
    pub attach: Rc<dyn Fn(&str, &mut Window, &mut App)>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ItemState {
    Loading,
    Ready(Box<ItemPage>),
    Failed(String),
}

pub struct OpenItem {
    pub id: String,
    pub state: ItemState,
    pub events_open: bool,
    /// The items navigated away from, newest last: Back returns to them, then to the board.
    pub back: Vec<String>,
}

/// A verification that proves something: passed, and not recorded by the worker itself.
pub fn proven(v: &Verification) -> bool {
    v.passed && v.run_by != "worker"
}

/// `done` with nothing but the agent's word for it.
pub fn done_unverified(page: &ItemPage) -> bool {
    page.item.status == ItemStatus::Done && !page.verifications.iter().any(proven)
}

/// The run the item belongs to (itself, when it is one).
pub fn run_of(page: &ItemPage) -> Option<&str> {
    if page.item.kind == "run" {
        return Some(page.item.id.as_str());
    }
    page.path.iter().rev().find(|p| p.kind == "run").map(|p| p.id.as_str())
}

/// An event re-reads the open item when it names the item or the item's run.
pub fn item_to_reload(event: &StreamEvent, open: &OpenItem) -> bool {
    if event.item_id.as_deref() == Some(open.id.as_str()) {
        return true;
    }
    match (&open.state, event.run_id.as_deref()) {
        (ItemState::Ready(page), Some(run)) => run_of(page) == Some(run),
        _ => false,
    }
}

/// The live fleet session holding the claim, joined on `claudeSessionId`.
pub fn attach_target<'a>(claim: &Claim, live: &'a [LiveSession]) -> Option<&'a LiveSession> {
    let sid = claim.session.as_ref()?.claude_session_id.as_deref().filter(|s| !s.is_empty())?;
    live.iter().find(|s| s.claude_session_id == sid)
}

pub fn resolution_text(r: ItemResolution) -> String {
    format!("{r:?}").to_lowercase()
}

/// The key the selftest reads a criterion's painted state by.
pub fn criterion_key(id: &str, checked: bool) -> String {
    format!("item-criterion-{id}={}", if checked { "checked" } else { "unchecked" })
}

pub fn verification_key(index: usize, passed: bool) -> String {
    format!("item-verification-{index}={}", if passed { "pass" } else { "fail" })
}

impl WorkView {
    /// A board row: the item opens with an empty Back history.
    pub fn show_item(&mut self, id: String, cx: &mut Context<Self>) {
        self.item = Some(OpenItem { id, state: ItemState::Loading, events_open: false, back: Vec::new() });
        self.read_item(cx);
    }

    /// A WorkState row of the app's Inbox: like a board row.
    pub fn open_from_inbox(&mut self, id: String, cx: &mut Context<Self>) {
        self.selected = Some(id.clone());
        self.show_item(id, cx);
        cx.notify();
    }

    /// A path element, child or link: Back returns here.
    pub fn navigate(&mut self, id: String, cx: &mut Context<Self>) {
        let back = match self.item.take() {
            Some(open) if open.id == id => {
                self.item = Some(open);
                return;
            }
            Some(mut open) => {
                open.back.push(open.id.clone());
                open.back
            }
            None => Vec::new(),
        };
        self.selected = Some(id.clone());
        self.item = Some(OpenItem { id, state: ItemState::Loading, events_open: false, back });
        self.read_item(cx);
    }

    pub fn back(&mut self, cx: &mut Context<Self>) {
        let Some(mut open) = self.item.take() else { return };
        match open.back.pop() {
            Some(id) => {
                self.selected = Some(id.clone());
                self.item = Some(OpenItem { id, state: ItemState::Loading, events_open: false, back: open.back });
                self.read_item(cx);
            }
            None => self.close_item(cx),
        }
    }

    pub fn close_item(&mut self, cx: &mut Context<Self>) {
        self.item = None;
        self.selected = None;
        cx.notify();
    }

    /// Escape: closes the item page; whether it was open.
    pub fn escape(&mut self, cx: &mut Context<Self>) -> bool {
        let open = self.item.is_some();
        if open {
            self.close_item(cx);
        }
        open
    }

    pub fn toggle_events(&mut self, cx: &mut Context<Self>) {
        if let Some(open) = &mut self.item {
            open.events_open = !open.events_open;
            cx.notify();
        }
    }

    pub(super) fn schedule_item(&mut self, cx: &mut Context<Self>) {
        if !self.item_pending {
            self.item_pending = true;
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(DEBOUNCE).await;
                this.update(cx, |this, cx| {
                    this.item_pending = false;
                    this.read_item(cx);
                })
                .ok();
            })
            .detach();
        }
    }

    pub(super) fn read_item(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.item.as_ref().map(|o| o.id.clone()) else { return };
        let Some(client) = self.client(cx) else { return };
        self.item_reads += 1;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let asked = id.clone();
            let read = cx
                .background_executor()
                .spawn(async move { client.item(&asked).map_err(|e| format!("Cannot read this item: {e}")) })
                .await;
            this.update(cx, |this, cx| {
                // The pane moved on to another item meanwhile.
                let Some(open) = this.item.as_mut().filter(|o| o.id == id) else { return };
                match (read, &open.state) {
                    (Ok(page), ItemState::Ready(shown)) if page.seq < shown.seq => {}
                    (Ok(page), _) => open.state = ItemState::Ready(Box::new(page)),
                    // A failed re-read keeps the page on screen; the next event reads it again.
                    (Err(_), ItemState::Ready(_)) => {}
                    (Err(message), _) => open.state = ItemState::Failed(message),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn render_item(&self, open: &OpenItem, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let status = cx.theme().status().clone();
        let back_label = if open.back.is_empty() { "← Board" } else { "← Back" };
        let back = div()
            .id("item-back")
            .relative()
            .flex_none()
            .px_2()
            .rounded_sm()
            .border_1()
            .border_color(colors.border)
            .text_xs()
            .cursor_pointer()
            .hover(|s| s.bg(colors.ghost_element_hover))
            .active(|s| s.bg(colors.ghost_element_active))
            .child(back_label)
            .child(hit_target(&self.hits, "item-back".into()))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.back(cx)));
        let pane = div()
            .id("item-pane")
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .gap_3()
            .px_4()
            .py_2()
            .overflow_y_scroll()
            .border_l_1()
            .border_color(colors.border)
            .child(div().flex().items_center().gap_2().child(back).child(small("Esc closes", colors.text_muted)))
            .child(hit_target(&self.hits, "item-pane".into()));
        let page = match &open.state {
            ItemState::Ready(page) => page,
            ItemState::Loading => return pane.child(div().text_color(colors.text_muted).child("Reading the item…")).into_any_element(),
            ItemState::Failed(message) => {
                return pane.child(div().text_color(status.error).child(SharedString::from(message.clone()))).into_any_element();
            }
        };
        let mut pane = pane.child(self.render_header(page, cx));
        if !page.intent.trim().is_empty() {
            pane = pane.child(section("Intent", None).child(prose(&page.intent, colors.text)).child(hit_target(&self.hits, "item-intent".into())));
        }
        if let Some(cmd) = page.verify_cmd.as_deref().filter(|c| !c.trim().is_empty()) {
            pane = pane.child(section("Verify command", None).child(mono(cmd, colors.text)).child(hit_target(&self.hits, "item-verify-cmd".into())));
        }
        if let Some(claim) = &page.claim {
            pane = pane.child(self.render_claim(claim, cx));
        }
        if !page.criteria.is_empty() {
            let checked = page.criteria.iter().filter(|c| c.checked).count();
            let mut s = section("Criteria", Some(format!("{checked}/{}", page.criteria.len())));
            for c in &page.criteria {
                let (mark, color) = if c.checked { ("☑", status.success) } else { ("☐", colors.text_muted) };
                let mut body = div().flex_1().min_w_0().flex().flex_col().child(prose(&c.text, colors.text));
                if let Some(evidence) = c.evidence.as_deref().filter(|e| !e.is_empty()) {
                    body = body.child(prose(&format!("evidence: {evidence}"), colors.text_muted).text_xs());
                }
                if let Some(at) = &c.checked_at {
                    body = body.child(small(format!("checked {}", short_time(at)), colors.text_muted));
                }
                s = s.child(
                    div()
                        .relative()
                        .flex()
                        .gap_2()
                        .child(div().flex_none().text_color(color).child(mark))
                        .child(div().flex_none().w(px(28.)).text_xs().text_color(colors.text_muted).child(SharedString::from(c.id.clone())))
                        .child(body)
                        .child(hit_target(&self.hits, criterion_key(&c.id, c.checked))),
                );
            }
            pane = pane.child(s);
        }
        if done_unverified(page) {
            pane = pane.child(
                div()
                    .relative()
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .border_1()
                    .border_color(status.warning)
                    .bg(status.warning.opacity(0.12))
                    .text_sm()
                    .text_color(status.warning)
                    .child("⚠ Done, but no orchestrator verification proves it: only the agent's report says so.")
                    .child(hit_target(&self.hits, "item-done-unverified".into())),
            );
        }
        if !page.reports.is_empty() {
            let mut s = section("Reports — what the agent said", Some(page.reports.len().to_string()));
            for r in &page.reports {
                s = s.child(self.render_report(r, cx));
            }
            pane = pane.child(s);
        }
        if !page.verifications.is_empty() {
            let mut s = section("Verifications — what was proven", Some(page.verifications.len().to_string()));
            for (i, v) in page.verifications.iter().enumerate() {
                s = s.child(self.render_verification(i, v, cx));
            }
            pane = pane.child(s);
        }
        if !page.artifacts.is_empty() {
            let mut s = section("Artifacts", None);
            for a in &page.artifacts {
                s = s.child(
                    div()
                        .relative()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(small(a.kind.clone(), colors.text_muted))
                        .child(mono(&a.value, colors.text))
                        .child(small(short_time(&a.at), colors.text_muted))
                        .child(hit_target(&self.hits, format!("item-artifact-{}", a.value))),
                );
            }
            pane = pane.child(s);
        }
        if !page.children.is_empty() {
            let mut s = section("Children", Some(page.children.len().to_string()));
            for c in &page.children {
                let mut link = self.item_link(format!("item-child-{}", c.item.qualified_ref), &c.item, cx);
                if c.criteria.total > 0 {
                    link = link.child(small(format!("{}/{}", c.criteria.checked, c.criteria.total), colors.text_muted));
                }
                s = s.child(link);
            }
            pane = pane.child(s);
        }
        if !page.findings.is_empty() {
            let mut s = section("Findings", Some(page.findings.len().to_string()));
            for f in &page.findings {
                let mut link = self.item_link(format!("item-finding-{}", f.item.qualified_ref), &f.item, cx);
                if let Some(sev) = &f.severity {
                    let color = if sev == "blocker" { status.error } else { status.warning };
                    link = link.child(badge(sev.clone(), color));
                }
                if let Some(source) = &f.source {
                    link = link.child(small(source.clone(), colors.text_muted));
                }
                s = s.child(link);
            }
            pane = pane.child(s);
        }
        if !page.notes.is_empty() {
            let mut s = section("Notes", Some(page.notes.len().to_string()));
            for n in &page.notes {
                let route = match (&n.from, &n.to) {
                    (Some(f), Some(t)) => format!("{} → {}", f.qualified_ref, t.qualified_ref),
                    (Some(f), None) => format!("from {}", f.qualified_ref),
                    (None, Some(t)) => format!("to {}", t.qualified_ref),
                    (None, None) => String::new(),
                };
                let (state, color) = match &n.consumed_at {
                    Some(at) => (format!("consumed {}", short_time(at)), colors.text_muted),
                    None => ("not consumed".to_string(), status.warning),
                };
                s = s.child(
                    div()
                        .relative()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .gap_2()
                                .child(badge(n.kind.clone(), colors.text_muted))
                                .when(!route.is_empty(), |d| d.child(small(route, colors.text_muted)))
                                .when_some(n.session.as_ref(), |d, s| d.child(small(format!("by {}", s.name), colors.text_muted)))
                                .child(small(state, color))
                                .child(small(short_time(&n.created_at), colors.text_muted)),
                        )
                        .child(prose(&n.body, colors.text))
                        .child(hit_target(&self.hits, format!("item-note-{}", n.id))),
                );
            }
            pane = pane.child(s);
        }
        if !page.links.out.is_empty() || !page.links.in_.is_empty() {
            let mut s = section("Links", None);
            for l in &page.links.out {
                s = s.child(
                    self.item_link(format!("item-link-out-{}-{}", l.type_, l.item.qualified_ref), &l.item, cx)
                        .child(small(format!("this {} it", l.type_), colors.text_muted)),
                );
            }
            for l in &page.links.in_ {
                s = s.child(
                    self.item_link(format!("item-link-in-{}-{}", l.type_, l.item.qualified_ref), &l.item, cx)
                        .child(small(format!("it {} this", l.type_), colors.text_muted)),
                );
            }
            pane = pane.child(s);
        }
        if !page.assumptions.is_empty() {
            let mut s = section("Assumptions", Some(page.assumptions.len().to_string()));
            for a in &page.assumptions {
                let color = if a.status == "open" { status.warning } else { colors.text_muted };
                s = s.child(
                    div()
                        .relative()
                        .flex()
                        .gap_2()
                        .child(div().flex_none().w(px(28.)).text_xs().text_color(colors.text_muted).child(SharedString::from(a.ref_.clone())))
                        .child(small(a.status.clone(), color))
                        .child(div().flex_1().min_w_0().child(prose(&a.text, colors.text)))
                        .when_some(a.revisit_when.as_ref(), |d, r| d.child(small(format!("revisit at {}", r.qualified_ref), colors.text_muted)))
                        .child(hit_target(&self.hits, format!("item-assumption-{}", a.ref_))),
                );
            }
            pane = pane.child(s);
        }
        if !page.events.is_empty() {
            pane = pane.child(self.render_events(open, page, cx));
        }
        pane.child(div().h(px(24.)).flex_none()).into_any_element()
    }

    fn render_header(&self, page: &ItemPage, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let item = &page.item;
        let mut crumbs = div().flex().flex_wrap().items_center().gap_1().text_xs();
        for (i, p) in page.path.iter().enumerate() {
            if i > 0 {
                crumbs = crumbs.child(div().text_color(colors.text_muted).child("/"));
            }
            let key = format!("item-path-{}", p.qualified_ref);
            let id = p.id.clone();
            crumbs = crumbs.child(
                div()
                    .id(SharedString::from(key.clone()))
                    .relative()
                    .text_color(colors.text_accent)
                    .cursor_pointer()
                    .hover(|s| s.underline())
                    .child(SharedString::from(p.qualified_ref.clone()))
                    .child(hit_target(&self.hits, key))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.navigate(id.clone(), cx))),
            );
        }
        let color = status_color(item.status, cx);
        let mut when = vec![format!("opened {}", short_time(&page.opened_at))];
        when.extend(page.started_at.as_deref().map(|at| format!("started {}", short_time(at))));
        when.extend(page.closed_at.as_deref().map(|at| format!("closed {}", short_time(at))));
        div()
            .relative()
            .flex()
            .flex_col()
            .gap_1()
            .when(!page.path.is_empty(), |d| d.child(crumbs))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(div().flex_none().text_lg().font_weight(FontWeight::BOLD).child(SharedString::from(item.qualified_ref.clone())))
                    .child(badge(item.kind.clone(), colors.text_muted))
                    .child(
                        div()
                            .relative()
                            .flex_none()
                            .text_sm()
                            .text_color(color)
                            .child(item.status.as_str())
                            .child(hit_target(&self.hits, format!("item-status-{}={}", item.qualified_ref, item.status.as_str()))),
                    )
                    .when_some(page.resolution, |d, r| d.child(badge(resolution_text(r), colors.text_muted)))
                    .when_some(page.severity.as_ref(), |d, s| d.child(badge(s.clone(), cx.theme().status().warning)))
                    .children(page.risk_tags.iter().map(|t| badge(t.clone(), cx.theme().status().warning))),
            )
            .child(div().text_lg().child(SharedString::from(item.title.clone())))
            .child(small(when.join(" · "), colors.text_muted))
            .child(hit_target(&self.hits, format!("item-header-{}", item.qualified_ref)))
            .into_any_element()
    }

    fn render_claim(&self, claim: &Claim, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let status = cx.theme().status().clone();
        let live = self.link.live.borrow();
        let mut row = div().relative().flex().flex_wrap().items_center().gap_2().text_sm();
        match attach_target(claim, &live) {
            Some(session) => {
                let name = if session.name.is_empty() { session.id.clone() } else { session.name.clone() };
                let key = format!("item-attach-{}", session.id);
                let id = session.id.clone();
                row = row
                    .child(div().text_color(status.info).child(SharedString::from(format!("worked on by {name}"))))
                    .child(
                        div()
                            .id(SharedString::from(key.clone()))
                            .relative()
                            .flex_none()
                            .px_2()
                            .rounded_sm()
                            .border_1()
                            .border_color(status.info)
                            .text_xs()
                            .text_color(status.info)
                            .cursor_pointer()
                            .hover(|s| s.bg(colors.ghost_element_hover))
                            .active(|s| s.bg(colors.ghost_element_active))
                            .child("Attach")
                            .child(hit_target(&self.hits, key))
                            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                let attach = this.link.attach.clone();
                                attach(&id, window, cx);
                            })),
                    );
            }
            None => {
                let color = if claim.expired { status.warning } else { status.info };
                row = row.child(div().text_color(color).child(SharedString::from(format!("claimed by {}", claim.holder()))));
            }
        }
        if let Some(tree) = &claim.tree {
            row = row.child(small(format!("@ {}", last_segment(tree)), colors.text_muted));
        }
        if !claim.expires_at.is_empty() {
            let text = if claim.expired { format!("expired {}", short_time(&claim.expires_at)) } else { format!("expires {}", short_time(&claim.expires_at)) };
            row = row.child(small(text, if claim.expired { status.warning } else { colors.text_muted }));
        }
        section("Claim", None).child(row.child(hit_target(&self.hits, "item-claim".into()))).into_any_element()
    }

    fn render_report(&self, r: &Report, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let muted = colors.text_muted;
        let by = r.session.as_ref().map(|s| s.name.as_str()).filter(|n| !n.is_empty()).unwrap_or(r.agent.as_str());
        let mut card = div()
            .relative()
            .flex()
            .flex_col()
            .gap(px(2.))
            .px_2()
            .py_1()
            .rounded_sm()
            .border_1()
            .border_dashed()
            .border_color(colors.border)
            .text_sm()
            .text_color(muted)
            .italic()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(badge("claimed", muted))
                    .child(small(format!("attempt {}", r.attempt), muted))
                    .child(small(format!("status {}", r.status), muted))
                    .when(!by.is_empty(), |d| d.child(small(format!("by {by}"), muted)))
                    .child(small(short_time(&r.at), muted)),
            );
        let fields: [(&str, Option<String>); 6] = [
            ("files", (!r.files.is_empty()).then(|| r.files.join(", "))),
            ("verify", r.verify.clone()),
            ("red check", r.red_check.clone()),
            ("deviations", r.deviations.clone()),
            ("risks", r.risks.clone()),
            ("not done", r.not_done.clone()),
        ];
        for (label, value) in fields {
            if let Some(value) = value.filter(|v| !v.trim().is_empty()) {
                card = card.child(prose(&format!("{label}: {value}"), muted));
            }
        }
        card.child(hit_target(&self.hits, format!("item-report-{}", r.attempt))).into_any_element()
    }

    fn render_verification(&self, index: usize, v: &Verification, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let status = cx.theme().status().clone();
        let (mark, color) = match (v.passed, proven(v)) {
            (true, true) => ("✓ proven", status.success),
            (true, false) => ("✓ passed (worker's own run)", status.warning),
            (false, _) => ("✗ failed", status.error),
        };
        let by = v.session.as_ref().map(|s| format!("{} ({})", v.run_by, s.name)).unwrap_or_else(|| v.run_by.clone());
        div()
            .relative()
            .flex()
            .flex_col()
            .gap(px(2.))
            .px_2()
            .py_1()
            .rounded_sm()
            .border_1()
            .border_color(color)
            .bg(color.opacity(0.06))
            .text_sm()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(div().flex_none().font_weight(FontWeight::BOLD).text_color(color).child(mark))
                    .child(small(format!("run by {by}"), colors.text_muted))
                    .child(small(short_time(&v.at), colors.text_muted)),
            )
            .child(mono(&v.command, colors.text))
            .when(!v.result_line.is_empty(), |d| d.child(prose(&v.result_line, colors.text)))
            .when_some(v.red_check.as_deref().filter(|r| !r.is_empty()), |d, r| d.child(prose(&format!("red check: {r}"), colors.text_muted)))
            .child(hit_target(&self.hits, verification_key(index, v.passed)))
            .into_any_element()
    }

    fn render_events(&self, open: &OpenItem, page: &ItemPage, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let more = if page.events_has_more { "+" } else { "" };
        let toggle = div()
            .id("item-events")
            .relative()
            .flex()
            .items_center()
            .gap_2()
            .text_xs()
            .text_color(colors.text_muted)
            .cursor_pointer()
            .hover(|s| s.bg(colors.ghost_element_hover))
            .active(|s| s.bg(colors.ghost_element_active))
            .child(if open.events_open { "▾" } else { "▸" })
            .child(SharedString::from(format!("RECENT EVENTS {}{more}", page.events.len())))
            .child(hit_target(&self.hits, "item-events".into()))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_events(cx)));
        let mut s = div().flex().flex_col().gap_1().child(toggle);
        if open.events_open {
            if page.events_has_more {
                s = s.child(small("older events are not shown", colors.text_muted));
            }
            for e in &page.events {
                let who = e.session.as_ref().map(|s| s.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| e.actor.clone());
                s = s.child(
                    div()
                        .relative()
                        .flex()
                        .gap_2()
                        .text_xs()
                        .child(div().flex_none().w(px(110.)).text_color(colors.text_muted).child(SharedString::from(short_time(&e.at))))
                        .child(div().flex_none().w(px(80.)).text_color(colors.text_muted).child(SharedString::from(e.kind.clone())))
                        .child(div().flex_none().w(px(110.)).overflow_hidden().whitespace_nowrap().text_color(colors.text_muted).child(SharedString::from(who)))
                        .child(div().flex_1().min_w_0().child(SharedString::from(e.summary.clone())))
                        .child(hit_target(&self.hits, format!("item-event-{}", e.seq))),
                );
            }
        }
        s.into_any_element()
    }

    /// A clickable item: ref, status, title; navigates there.
    fn item_link(&self, key: String, item: &ItemRef, cx: &mut Context<Self>) -> Stateful<gpui::Div> {
        let colors = cx.theme().colors().clone();
        let id = item.id.clone();
        div()
            .id(SharedString::from(key.clone()))
            .relative()
            .flex()
            .items_center()
            .gap_2()
            .text_sm()
            .cursor_pointer()
            .hover(|s| s.bg(colors.ghost_element_hover))
            .active(|s| s.bg(colors.ghost_element_active))
            .child(div().flex_none().text_color(colors.text_accent).child(SharedString::from(item.qualified_ref.clone())))
            .child(small(item.status.as_str(), status_color(item.status, cx)))
            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().child(SharedString::from(item.title.clone())))
            .child(hit_target(&self.hits, key))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.navigate(id.clone(), cx)))
    }
}

fn section(title: &str, count: Option<String>) -> gpui::Div {
    let heading = match count {
        Some(count) => format!("{} · {count}", title.to_uppercase()),
        None => title.to_uppercase(),
    };
    div()
        .relative()
        .flex()
        .flex_col()
        .gap_1()
        .child(div().text_xs().font_weight(FontWeight::BOLD).opacity(0.7).child(SharedString::from(heading)))
}

fn prose(text: &str, color: Hsla) -> gpui::Div {
    div().text_color(color).whitespace_normal().child(SharedString::from(text.to_string()))
}

fn mono(text: &str, color: Hsla) -> gpui::Div {
    div().font_family("Cascadia Mono").text_xs().text_color(color).whitespace_normal().child(SharedString::from(text.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use foreman_workstate::Session;

    fn iref(id: &str, kind: &str) -> ItemRef {
        ItemRef { id: id.into(), ref_: id.into(), qualified_ref: id.into(), kind: kind.into(), ..Default::default() }
    }

    fn verification(passed: bool, run_by: &str) -> Verification {
        Verification { passed, run_by: run_by.into(), ..Default::default() }
    }

    fn page(status: ItemStatus, verifications: Vec<Verification>) -> ItemPage {
        let mut item = iref("t", "task");
        item.status = status;
        ItemPage { item, path: vec![iref("p", "project"), iref("r", "run"), iref("ph", "phase")], verifications, ..Default::default() }
    }

    #[test]
    fn done_needs_a_passed_verification_the_worker_did_not_record() {
        assert!(done_unverified(&page(ItemStatus::Done, vec![])));
        assert!(done_unverified(&page(ItemStatus::Done, vec![verification(true, "worker")])));
        assert!(done_unverified(&page(ItemStatus::Done, vec![verification(false, "orchestrator")])));
        assert!(!done_unverified(&page(ItemStatus::Done, vec![verification(false, "worker"), verification(true, "orchestrator")])));
        assert!(!done_unverified(&page(ItemStatus::Done, vec![verification(true, "controller")])));
        assert!(!done_unverified(&page(ItemStatus::InProgress, vec![])));
    }

    #[test]
    fn an_event_reloads_the_open_item_when_it_names_the_item_or_its_run() {
        let open = |state| OpenItem { id: "t".into(), state, events_open: false, back: vec![] };
        let event = |run: Option<&str>, item: Option<&str>| StreamEvent {
            seq: 1,
            run_id: run.map(str::to_string),
            item_id: item.map(str::to_string),
            kind: "transition".into(),
        };
        let ready = open(ItemState::Ready(Box::new(page(ItemStatus::Open, vec![]))));
        assert!(item_to_reload(&event(None, Some("t")), &ready));
        assert!(item_to_reload(&event(Some("r"), Some("other")), &ready));
        assert!(!item_to_reload(&event(Some("r2"), Some("other")), &ready));
        assert!(!item_to_reload(&event(None, None), &ready));
        let loading = open(ItemState::Loading);
        assert!(item_to_reload(&event(None, Some("t")), &loading));
        assert!(!item_to_reload(&event(Some("r"), None), &loading));
    }

    #[test]
    fn a_run_is_its_own_run() {
        let mut run = page(ItemStatus::Open, vec![]);
        run.item = iref("r9", "run");
        assert_eq!(run_of(&run), Some("r9"));
        assert_eq!(run_of(&page(ItemStatus::Open, vec![])), Some("r"));
        assert_eq!(run_of(&ItemPage::default()), None);
    }

    #[test]
    fn a_claim_joins_a_live_session_on_its_claude_session_id() {
        let live = vec![
            LiveSession { claude_session_id: "uuid-a".into(), id: "aaaaaaaa".into(), name: "aw-a".into() },
            LiveSession { claude_session_id: "uuid-b".into(), id: "bbbbbbbb".into(), name: "aw-b".into() },
        ];
        let claim = |sid: Option<&str>| Claim {
            session: Some(Session { name: "aw-b".into(), claude_session_id: sid.map(str::to_string), ..Default::default() }),
            ..Default::default()
        };
        assert_eq!(attach_target(&claim(Some("uuid-b")), &live).map(|s| s.id.as_str()), Some("bbbbbbbb"));
        // The name alone is not a join: only the Claude session id is.
        assert_eq!(attach_target(&claim(Some("uuid-gone")), &live), None);
        assert_eq!(attach_target(&claim(None), &live), None);
        assert_eq!(attach_target(&claim(Some("")), &[LiveSession { claude_session_id: String::new(), ..live[0].clone() }]), None);
        assert_eq!(attach_target(&Claim::default(), &live), None);
    }

    #[test]
    fn keys_and_labels() {
        assert_eq!(criterion_key("c1", true), "item-criterion-c1=checked");
        assert_eq!(criterion_key("c2", false), "item-criterion-c2=unchecked");
        assert_eq!(verification_key(0, false), "item-verification-0=fail");
        assert_eq!(resolution_text(ItemResolution::Wontfix), "wontfix");
    }
}
