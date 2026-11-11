//! The right column: the selected item, else the run, over Overview | Evidence | Notes | Activity | Brief.

use std::rc::Rc;

use gpui::{ClickEvent, FontWeight, Hsla};
use supraman_core::time::{near, rfc3339_local, rfc3339_ms};
use supraman_ui::details::{self, Section};
use supraman_ui::hit::hit_target;
use supraman_ui::marks::dot;
use supraman_ui::segmented::{Segment, segmented};
use supraman_ui::{banner, button, fonts};
use supraman_workman::{BriefFor, ItemPage, Report, Verification};

use super::item::attach_target;
use super::state::{StateKey, independent};
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum DetailTab {
    #[default]
    Overview,
    Evidence,
    Notes,
    Activity,
    Brief,
}

pub(crate) const TABS: [(DetailTab, &str, &str); 5] = [
    (DetailTab::Overview, "overview", "Overview"),
    (DetailTab::Evidence, "evidence", "Evidence"),
    (DetailTab::Notes, "notes", "Notes"),
    (DetailTab::Activity, "activity", "Activity"),
    (DetailTab::Brief, "brief", "Brief"),
];

pub(crate) fn tab_label(tab: DetailTab, page: &ItemPage) -> String {
    let name = TABS.iter().find(|(t, _, _)| *t == tab).map_or("", |(_, _, n)| *n);
    let n = match tab {
        DetailTab::Evidence => page.reports.len() + page.verifications.len(),
        DetailTab::Notes => page.notes.len(),
        DetailTab::Activity => page.events.len(),
        DetailTab::Overview | DetailTab::Brief => 0,
    };
    if n > 0 { format!("{name} {n}") } else { name.to_string() }
}

fn when(at: &str, now: i64) -> String {
    rfc3339_ms(at).map_or_else(|| rfc3339_local(at), |ms| near(ms, now))
}

pub(crate) fn facts(page: &ItemPage, now: i64) -> Vec<(&'static str, String)> {
    let mut out = vec![("kind", page.item.kind.clone())];
    if let Some(commit) = page.artifacts.iter().filter(|a| a.kind == "commit").max_by(|a, b| a.at.cmp(&b.at)) {
        out.push(("commit", supraman_gitman::short_sha(&commit.value).to_string()));
    }
    if let Some(r) = page.resolution {
        out.push(("resolution", format!("{r:?}").to_lowercase()));
    }
    if let Some(at) = &page.closed_at {
        out.push(("closed", when(at, now)));
    }
    out
}

pub(crate) fn overview(page: &ItemPage) -> Vec<Section> {
    let mut out = Vec::new();
    if let Some(intent) = page.intent.as_deref().filter(|i| !i.trim().is_empty()) {
        out.push(Section::new("intent", "What this is for").text(intent));
    }
    if !page.criteria.is_empty() {
        let checked = page.criteria.iter().filter(|c| c.checked).count();
        let mut s = Section::new("criteria", format!("Done when · {checked} of {}", page.criteria.len()));
        for c in &page.criteria {
            s = s.check(c.checked, c.text.clone(), c.evidence.clone().filter(|e| !e.trim().is_empty()));
        }
        out.push(s);
    }
    if let Some(cmd) = page.verify_cmd.as_deref().filter(|c| !c.trim().is_empty()) {
        out.push(Section::new("verify", "Verified by running").long(cmd));
    }
    if !page.children.is_empty() {
        let mut s = Section::new("inside", "Inside");
        for c in &page.children {
            let label = state::of(state::Input { kind: &c.item.kind, status: c.item.status, resolution: None, claim_expired: false, proof: None, waiting_on: Vec::new() }).label;
            s = s.link(c.item.ref_.clone(), format!("{} · {label}", c.item.title), c.item.id.clone());
        }
        out.push(s);
    }
    out
}

pub(crate) fn notes(page: &ItemPage, now: i64) -> Vec<Section> {
    let mut s = Section::new("notes", "Notes");
    if page.notes.is_empty() {
        s = s.text("No notes.");
    }
    for n in &page.notes {
        s = s.row(format!("{} · {}", n.kind, when(&n.created_at, now)), n.body.clone());
    }
    vec![s]
}

fn by(session: Option<&supraman_workman::Session>, fallback: &str) -> String {
    session.map(|s| s.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| fallback.to_string())
}

impl WorkView {
    pub(crate) fn set_detail_tab(&mut self, tab: DetailTab, cx: &mut Context<Self>) {
        self.detail_tab = tab;
        if tab == DetailTab::Brief
            && let Some(id) = self.item.as_ref().map(|o| o.id.clone())
            && !self.briefs.contains_key(&id)
        {
            self.read_brief(id, cx);
        }
        cx.notify();
    }

    fn read_brief(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(client) = self.client(cx) else { return };
        cx.spawn(async move |this, cx| {
            let asked = id.clone();
            let read = cx.background_executor().spawn(async move { client.brief(&asked, BriefFor::Worker).map_err(read_failed) }).await;
            this.update(cx, |this, cx| {
                this.briefs.insert(id, read);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn render_detail(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(open) = &self.item else {
            return placeholder("Nothing selected", ["Pick a run in the navigator."], cx).size_full().into_any_element();
        };
        let page = match &open.state {
            ItemState::Ready(page) => page.clone(),
            ItemState::Loading => return placeholder("Reading the item…", Vec::<SharedString>::new(), cx).size_full().into_any_element(),
            ItemState::Failed(message) => return placeholder("Cannot show this item", [message.clone()], cx).size_full().px_6().into_any_element(),
        };
        let state = state::of_page(&page);
        let sections = match self.detail_tab {
            DetailTab::Overview => overview(&page),
            DetailTab::Notes => notes(&page, (self.now)()),
            DetailTab::Brief => vec![match self.briefs.get(&page.item.id) {
                Some(Ok(text)) => Section::new("brief", "Worker brief").long(text.clone()),
                Some(Err(why)) => Section::new("brief", "Worker brief").text(why.clone()),
                None => Section::new("brief", "Worker brief").text("Reading the brief…"),
            }],
            DetailTab::Evidence | DetailTab::Activity => Vec::new(),
        };
        let top = match self.detail_tab {
            DetailTab::Evidence => Some(self.render_evidence(&page, cx)),
            DetailTab::Activity => Some(self.render_activity(&page, cx)),
            _ => None,
        };
        let on_link: details::OnLink<WorkView> = Rc::new(|this, target, _, cx| this.select(target.to_string(), cx));
        let detail = details::Page {
            key: "work-detail",
            title: page.item.title.clone(),
            pill: Some((state.label.clone(), state.key.color(cx))),
            sections: &sections,
            actions: Vec::new(),
            scroll: &self.detail_scroll,
            on_link: Some(on_link),
            top,
            over: Some(self.render_over(&page, state.key, cx)),
            bar: Some(self.render_bar(&page, state.key, cx)),
        };
        let focused = self.detail_focused;
        let accent = cx.theme().colors().border_focused;
        div()
            .size_full()
            .when(focused, |d| d.border_t_2().border_color(accent))
            .child(details::render(detail, &self.hits, cx))
            .child(hit_target(&self.hits, format!("work-detail={}", if focused { "focused" } else { "idle" })))
            .into_any_element()
    }

    fn render_over(&self, page: &ItemPage, key: StateKey, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let live = (self.link.live)();
        let attach = page.claim.as_ref().and_then(|c| attach_target(c, &live)).map(|session| {
            let name = if session.name.is_empty() { session.id.clone() } else { session.name.clone() };
            let id = session.id.clone();
            button::bordered(&self.hits, format!("work-attach-{}", session.id), format!("Attach to {name}"), cx).on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                let attach = this.link.attach.clone();
                attach(&id, window, cx);
            }))
        });
        div()
            .relative()
            .flex()
            .items_center()
            .gap_1()
            .child(div().font_family(fonts::MONO).text_xs().text_color(colors.text_muted).child(SharedString::from(page.item.qualified_ref.clone())))
            .child(self.ref_actions("work-detail", &page.item, cx))
            .children(attach)
            .child(hit_target(&self.hits, format!("work-pill-{}={}", page.item.qualified_ref, key.as_str())))
            .into_any_element()
    }

    fn render_bar(&self, page: &ItemPage, key: StateKey, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let chips = facts(page, (self.now)()).into_iter().map(|(label, value)| {
            div()
                .flex()
                .flex_none()
                .gap_1()
                .px_1()
                .rounded_sm()
                .bg(colors.element_background)
                .text_xs()
                .child(div().text_color(colors.text_muted).child(label))
                .child(div().font_family(fonts::MONO).child(SharedString::from(value)))
        });
        let prompts = (key == StateKey::Stop && page.item.kind != "run").then(|| self.render_prompts(&page.item.id, "item", cx)).flatten();
        let segments: Vec<Segment> = TABS.iter().map(|(tab, k, _)| Segment::new(*k).label(tab_label(*tab, page))).collect();
        let chosen = TABS.iter().position(|(t, _, _)| *t == self.detail_tab).unwrap_or(0);
        let tabs = segmented(&self.hits, "work-tab", &segments, chosen, |this: &mut WorkView, i, _, cx| this.set_detail_tab(TABS[i].0, cx), cx);
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().flex().flex_wrap().gap_1().children(chips.collect::<Vec<_>>()))
            .children(prompts)
            .child(tabs)
            .into_any_element()
    }

    fn render_evidence(&self, page: &ItemPage, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let status = cx.theme().status().clone();
        let (proven, own): (Vec<(usize, &Verification)>, Vec<(usize, &Verification)>) =
            page.verifications.iter().enumerate().partition(|(_, v)| v.run_by != "worker");
        let head = |title: &str, sub: &str, mark: Hsla| {
            div()
                .flex()
                .flex_col()
                .child(div().flex().items_center().gap_2().text_sm().font_weight(FontWeight::SEMIBOLD).child(dot(supraman_ui::marks::Mark::Filled(mark))).child(SharedString::from(title.to_string())))
                .child(div().text_xs().text_color(colors.text_muted).child(SharedString::from(sub.to_string())))
        };
        let mut reported = div().flex_1().min_w_0().flex().flex_col().gap_2().child(head("What the agent reported", "From work_report. A claim, not proof.", status.warning));
        if page.reports.is_empty() && own.is_empty() {
            reported = reported.child(div().text_sm().text_color(colors.text_muted).child("No reports."));
        }
        for r in &page.reports {
            reported = reported.child(self.render_report(r, cx));
        }
        for (i, v) in own {
            reported = reported.child(self.render_verification(i, v, cx));
        }
        let mut verified = div().flex_1().min_w_0().flex().flex_col().gap_2().child(head("What was verified", "From work_verify, run by someone other than the author.", status.success));
        if proven.is_empty() {
            verified = verified.child(div().text_sm().text_color(colors.text_muted).child("Nothing verified yet."));
        }
        for (i, v) in proven {
            verified = verified.child(self.render_verification(i, v, cx));
        }
        div().flex().gap_4().py_3().child(reported).child(verified).into_any_element()
    }

    fn render_report(&self, r: &Report, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let fields: [(&str, Option<String>); 6] = [
            ("files", (!r.files.is_empty()).then(|| r.files.join(", "))),
            ("verify", r.verify.clone()),
            ("red check", r.red_check.clone()),
            ("deviations", r.deviations.clone()),
            ("risks", r.risks.clone()),
            ("not done", r.not_done.clone()),
        ];
        let lines = fields.into_iter().filter_map(|(label, v)| v.filter(|v| !v.trim().is_empty()).map(|v| div().text_xs().whitespace_normal().child(SharedString::from(format!("{label}: {v}")))));
        div()
            .relative()
            .flex()
            .flex_col()
            .gap(px(2.))
            .p_2()
            .rounded_md()
            .border_1()
            .border_dashed()
            .border_color(colors.border)
            .child(
                div()
                    .text_xs()
                    .text_color(colors.text_muted)
                    .child(SharedString::from(format!("attempt {} · {} · {} · {}", r.attempt, r.status, by(r.session.as_ref(), r.agent.as_deref().unwrap_or("")), when(&r.at, (self.now)())))),
            )
            .children(lines.collect::<Vec<_>>())
            .child(hit_target(&self.hits, format!("work-evidence-report-{}", r.attempt)))
            .into_any_element()
    }

    fn render_verification(&self, index: usize, v: &Verification, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let status = cx.theme().status().clone();
        let color = if !v.passed { status.error } else if independent(v.passed, &v.run_by) { status.success } else { status.warning };
        let pill = div().flex_none().px_2().rounded_full().bg(banner::tint(color)).text_xs().text_color(color).child(if v.passed { "Passed" } else { "Failed" });
        div()
            .relative()
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .rounded_md()
            .border_1()
            .border_color(colors.border_variant)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(pill)
                    .child(div().flex_1().text_xs().text_color(colors.text_muted).child(SharedString::from(format!("run by {} · {}", by(v.session.as_ref(), &v.run_by), when(&v.at, (self.now)()))))),
            )
            .when(!v.result_line.is_empty(), |d| d.child(div().text_sm().whitespace_normal().child(SharedString::from(v.result_line.clone()))))
            .when_some(v.command.as_deref().filter(|c| !c.is_empty()), |d, c| {
                d.child(div().p_1().rounded_sm().bg(colors.element_background).font_family(fonts::MONO).text_xs().whitespace_normal().child(SharedString::from(c.to_string())))
            })
            .child(hit_target(&self.hits, format!("work-evidence-verification-{index}={}", if v.passed { "pass" } else { "fail" })))
            .into_any_element()
    }

    fn render_activity(&self, page: &ItemPage, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let mut out = div().flex().flex_col().py_3();
        if page.events.is_empty() {
            out = out.child(div().text_sm().text_color(colors.text_muted).child("No activity yet."));
        }
        for e in page.events.iter().rev() {
            out = out.child(
                div()
                    .relative()
                    .flex()
                    .items_start()
                    .gap_3()
                    .py_1()
                    .border_b_1()
                    .border_color(colors.border_variant)
                    .text_sm()
                    .child(fonts::figures(div().flex_none().w(px(96.)).text_xs().text_color(colors.text_muted)).child(SharedString::from(when(&e.at, (self.now)()))))
                    .child(div().flex_none().w(px(84.)).text_xs().child(SharedString::from(e.kind.clone())))
                    .child(div().flex_1().min_w_0().whitespace_normal().child(SharedString::from(e.summary.clone().unwrap_or_default())))
                    .child(div().flex_none().text_xs().text_color(colors.text_muted).child(SharedString::from(by(e.session.as_ref(), &e.actor))))
                    .child(hit_target(&self.hits, format!("work-event-{}", e.seq))),
            );
        }
        if page.events_has_more {
            out = out.child(div().pt_2().text_xs().text_color(colors.text_muted).child("Older events are on the item's web page."));
        }
        out.into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use supraman_workman::{Artifact, Criterion, ItemRef, ItemResolution, Note};

    use super::*;

    #[test]
    fn facts_tabs_and_sections_say_what_the_page_holds() {
        let page = ItemPage {
            item: ItemRef { kind: "task".into(), ..Default::default() },
            artifacts: vec![
                Artifact { kind: "commit".into(), value: "0123456789abcdef".into(), at: "2026-11-06T10:00:00Z".into() },
                Artifact { kind: "commit".into(), value: "62efdbe000000000".into(), at: "2026-11-06T11:00:00Z".into() },
            ],
            resolution: Some(ItemResolution::Accepted),
            intent: Some("Why".into()),
            criteria: vec![Criterion { id: "c1".into(), text: "one".into(), checked: true, evidence: Some("merged".into()), checked_at: None }],
            verify_cmd: Some("cargo test".into()),
            notes: vec![Note { kind: "decision".into(), body: "b".into(), created_at: "x".into(), ..Default::default() }],
            verifications: vec![Verification::default()],
            ..Default::default()
        };
        let facts = facts(&page, 0);
        assert_eq!(facts[..3], [("kind", "task".to_string()), ("commit", supraman_gitman::short_sha("62efdbe000000000").to_string()), ("resolution", "accepted".into())]);
        assert_eq!(tab_label(DetailTab::Evidence, &page), "Evidence 1");
        assert_eq!(tab_label(DetailTab::Notes, &page), "Notes 1");
        assert_eq!(tab_label(DetailTab::Activity, &page), "Activity", "no count without events");
        let text = details::text("t", &overview(&page));
        assert!(text.contains("What this is for\nWhy") && text.contains("Done when · 1 of 1\n[x] one\n    merged") && text.contains("Verified by running\ncargo test"), "{text}");
    }
}
