//! The Overview page, the web's: one sentence, Needs you, Working now, the honesty tiles, and a card per bound project.

use gpui::{ClickEvent, FontWeight, Hsla};
use supraman_core::time::{self, now_ms, rfc3339_ms};
use supraman_ui::hit::hit_target;
use supraman_ui::marks::{Mark, dot};
use supraman_ui::{button, fonts};
use supraman_workman::{Claim, Honesty, InboxRow, OverviewRun, Rollup};

use super::cards::{self, Card};
use super::item::attach_target;
use super::navigator::done_total;
use super::projects::{active, closed_runs};
use super::state::StateKey;
use super::*;

pub const NEEDS_SHOWN: usize = 5;

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

pub fn open_of(r: &Rollup) -> u32 {
    r.open + r.in_progress + r.review + r.blocked
}

pub fn sentence(projects: &[OverviewProject], needs: usize) -> String {
    let tasks: u32 = projects.iter().map(|p| open_of(&p.rollup)).sum();
    let needs = if needs == 1 { "1 thing needs you".to_string() } else { format!("{needs} things need you") };
    format!("{} open across {}. {needs}.", plural(tasks as usize, "task", "tasks"), plural(projects.len(), "project", "projects"))
}

pub fn as_of(at: &str, now: i64) -> String {
    match rfc3339_ms(at) {
        Some(ms) => format!("As of {}", time::near(ms, now)),
        None => String::new(),
    }
}

/// The tiles the read API gives: Workman's honesty counters (every project of the server) and the bound projects' open tasks.
pub fn tiles(honesty: &Honesty, tasks_open: u32) -> Vec<(u32, &'static str, bool)> {
    vec![
        (honesty.done_unverified, "closed without independent proof", false),
        (honesty.forced, "forced closures", honesty.forced > 0),
        (honesty.followups_open, "follow-ups open", false),
        (tasks_open, "tasks open", false),
    ]
}

pub fn lease(expires_at: &str, expired: bool, now: i64) -> String {
    match rfc3339_ms(expires_at) {
        Some(_) if expired => "lease ended".into(),
        Some(at) if at <= now => "lease ended".into(),
        Some(at) => format!("lease ends in {}m", (at - now + 59_999) / 60_000),
        None => String::new(),
    }
}

pub fn claim_facts(claim: &Claim, now: i64) -> Vec<String> {
    let mut out = vec![claim.role.clone(), claim.item.qualified_ref.clone()];
    out.extend(claim.tree.clone().or_else(|| claim.session.as_ref().and_then(|s| s.tree.clone())));
    out.push(lease(&claim.expires_at, claim.expired, now));
    out.retain(|t| !t.is_empty());
    out
}

pub fn working_now<'a>(overview: &'a Overview, projects: &[OverviewProject]) -> Vec<&'a Claim> {
    let runs: HashSet<&str> = projects.iter().flat_map(|p| p.runs.iter().map(|r| r.item.id.as_str())).collect();
    overview.working_now.iter().filter(|c| c.run.as_ref().is_some_and(|r| runs.contains(r.id.as_str()))).collect()
}

pub fn runs_line(project: &OverviewProject) -> String {
    format!("{} open, {} closed", active(project).count(), closed_runs(project).count())
}

pub fn run_open(run: &OverviewRun) -> String {
    format!("{} open", open_of(&run.rollup))
}

pub const LEGEND: [(StateKey, &str); 6] =
    [(StateKey::Proven, "Verified"), (StateKey::Claimed, "Done, not verified"), (StateKey::Doing, "In progress"), (StateKey::Stop, "Blocked"), (StateKey::Wait, "Waiting"), (StateKey::Idle, "Open")];

impl WorkView {
    /// The Needs you cards of this workspace's Inbox rows, as the sidebar sorts them.
    pub fn needs_cards(&self, cx: &App) -> Vec<Card> {
        let Some(inbox) = &self.inbox else { return Vec::new() };
        let rows: Vec<(&str, &InboxRow)> = inbox.read(cx).rows().into_iter().filter(|(w, _)| *w == self.workspace).collect();
        let live: Vec<String> = self.link.live.borrow().iter().map(|s| s.name.clone()).collect();
        cards::sections(&rows, &live, now_ms()).needs_you
    }

    pub(super) fn render_overview(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let now = now_ms();
        let needs = self.needs_cards(cx);
        let said = sentence(&self.projects, needs.len());
        let head = div()
            .flex()
            .items_end()
            .gap_3()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_xl().font_weight(FontWeight::SEMIBOLD).child("Overview"))
                    .child(div().relative().text_sm().text_color(colors.text_muted).child(SharedString::from(said.clone())).child(hit_target(&self.hits, format!("work-overview-sentence={said}")))),
            )
            .children(self.overview.as_ref().map(|o| div().flex_none().text_xs().text_color(colors.text_muted).child(SharedString::from(as_of(&o.as_of, now)))));
        let side = div().flex().flex_col().gap_3().flex_1().min_w(px(260.)).child(self.render_working(now, cx)).children(self.render_tiles(cx));
        let mut root = div()
            .id("work-overview")
            .size_full()
            .flex()
            .flex_col()
            .gap_4()
            .px_5()
            .pt_4()
            .pb_6()
            .overflow_y_scroll()
            .track_scroll(&self.list_scroll)
            .child(head)
            .child(div().flex().flex_wrap().gap_3().items_start().child(self.render_needs(&needs, now, cx)).child(side));
        for project in self.projects.clone() {
            root = root.child(self.render_project_card(&project, cx));
        }
        root.child(self.render_legend(cx)).into_any_element()
    }

    fn card(&self, title: &str, count: Option<usize>, cx: &App) -> gpui::Div {
        let colors = cx.theme().colors();
        div().flex().flex_col().rounded_md().border_1().border_color(colors.border_variant).bg(colors.panel_background).child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .py_2()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child(SharedString::from(title.to_string()))
                .children(count.map(|n| div().text_xs().font_weight(FontWeight::NORMAL).text_color(colors.text_muted).child(SharedString::from(n.to_string())))),
        )
    }

    fn render_needs(&self, needs: &[Card], now: i64, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let error = cx.theme().status().error;
        let mut card = self.card("Needs you", Some(needs.len()), cx).flex_1().min_w(px(320.));
        if needs.is_empty() {
            card = card.child(div().px_3().pb_3().text_sm().text_color(colors.text_muted).child("Nothing needs you."));
        }
        for c in needs.iter().take(NEEDS_SHOWN) {
            let id = c.item_id.clone();
            card = card.child(
                button::ghost(&self.hits, format!("work-overview-needs-{}", c.ref_), cx)
                    .flex()
                    .items_start()
                    .gap_2()
                    .px_3()
                    .py(px(5.))
                    .border_t_1()
                    .border_color(colors.border_variant)
                    .child(div().pt(px(6.)).child(dot(Mark::Filled(error), 7.)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_sm().whitespace_normal().child(SharedString::from(c.title.clone())))
                            .child(div().text_xs().text_color(colors.text_muted).child(SharedString::from(format!("{} · {}", c.badge, c.ref_)))),
                    )
                    .child(div().flex_none().text_xs().text_color(colors.text_muted).child(SharedString::from(c.since.map(|at| time::ago(now - at)).unwrap_or_default())))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.open_from_inbox(id.clone(), cx))),
            );
        }
        if needs.len() > NEEDS_SHOWN {
            let more = format!("+{} more in the inbox", needs.len() - NEEDS_SHOWN);
            card = card.child(
                button::ghost(&self.hits, "work-overview-more", cx)
                    .px_3()
                    .py_2()
                    .border_t_1()
                    .border_color(colors.border_variant)
                    .text_xs()
                    .text_color(colors.text_accent)
                    .child(SharedString::from(more.clone()))
                    .child(hit_target(&self.hits, format!("work-overview-more={more}")))
                    .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(WorkEvent::ShowInbox))),
            );
        }
        card.into_any_element()
    }

    fn render_working(&self, now: i64, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let info = cx.theme().status().info;
        let claims: Vec<Claim> = self.overview.as_ref().map(|o| working_now(o, &self.projects).into_iter().cloned().collect()).unwrap_or_default();
        let live = self.link.live.borrow().clone();
        let mut card = self.card("Working now", Some(claims.len()), cx);
        if claims.is_empty() {
            card = card.child(div().px_3().pb_3().text_sm().text_color(colors.text_muted).child("No one holds a claim."));
        }
        for claim in claims {
            let qref = claim.item.qualified_ref.clone();
            let id = claim.item.id.clone();
            let holder = claim.holder().to_string();
            let session = match attach_target(&claim, &live) {
                Some(target) => {
                    let target = target.id.clone();
                    button::ghost(&self.hits, format!("work-working-session-{qref}"), cx)
                        .text_color(colors.text_accent)
                        .child(SharedString::from(holder))
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            cx.stop_propagation();
                            let attach = this.link.attach.clone();
                            attach(&target, window, cx);
                        }))
                        .into_any_element()
                }
                None => div().child(SharedString::from(holder)).into_any_element(),
            };
            card = card.child(
                div()
                    .flex()
                    .items_start()
                    .gap_2()
                    .px_3()
                    .py(px(5.))
                    .border_t_1()
                    .border_color(colors.border_variant)
                    .child(div().pt(px(6.)).child(dot(Mark::Filled(info), 7.)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                button::ghost(&self.hits, format!("work-working-{qref}"), cx)
                                    .text_sm()
                                    .whitespace_normal()
                                    .child(SharedString::from(claim.item.title.clone()))
                                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.open_from_inbox(id.clone(), cx))),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_1()
                                    .text_xs()
                                    .text_color(colors.text_muted)
                                    .child(session)
                                    .children(claim_facts(&claim, now).into_iter().map(|t| SharedString::from(format!("· {t}")))),
                            ),
                    ),
            );
        }
        card.into_any_element()
    }

    fn render_tiles(&self, cx: &App) -> Option<AnyElement> {
        let honesty = self.overview.as_ref()?.honesty;
        let colors = cx.theme().colors();
        let error = cx.theme().status().error;
        let tasks: u32 = self.projects.iter().map(|p| open_of(&p.rollup)).sum();
        let mut row = div().flex().rounded_md().border_1().border_color(colors.border_variant).bg(colors.panel_background);
        for (n, label, alarm) in tiles(&honesty, tasks) {
            let fg: Hsla = if alarm { error } else { colors.text };
            row = row.child(
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .px_3()
                    .py_2()
                    .child(fonts::figures(div().text_xl().font_weight(FontWeight::SEMIBOLD).text_color(fg)).child(SharedString::from(n.to_string())))
                    .child(div().text_xs().whitespace_normal().text_color(colors.text_muted).child(label))
                    .child(hit_target(&self.hits, format!("work-tile-{label}={n}"))),
            );
        }
        Some(row.into_any_element())
    }

    fn render_project_card(&self, project: &OverviewProject, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let qref = project.item.qualified_ref.clone();
        let head = div()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .pt_2()
            .child(div().text_base().font_weight(FontWeight::SEMIBOLD).child(SharedString::from(project.item.title.clone())))
            .child(div().font_family(fonts::MONO).text_xs().text_color(colors.text_muted).child(SharedString::from(qref.clone())))
            .child(div().flex_1())
            .child(div().text_xs().text_color(colors.text_muted).child(SharedString::from(runs_line(project))));
        let mut card = div()
            .relative()
            .flex()
            .flex_col()
            .gap_2()
            .rounded_md()
            .border_1()
            .border_color(colors.border_variant)
            .bg(colors.panel_background)
            .pb_1()
            .child(hit_target(&self.hits, format!("work-overview-project-{qref}")))
            .child(head)
            .children(self.render_prompts(&project.item.id, &format!("project-{qref}"), cx).map(|chips| div().px_3().child(chips)));
        for run in active(project) {
            let id = run.item.id.clone();
            card = card.child(
                button::ghost(&self.hits, format!("work-overview-run-{}", run.item.qualified_ref), cx)
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .py(px(5.))
                    .border_t_1()
                    .border_color(colors.border_variant)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_sm().whitespace_normal().child(SharedString::from(run.item.title.clone())))
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .text_xs()
                                    .text_color(colors.text_muted)
                                    .child(div().font_family(fonts::MONO).child(SharedString::from(run.item.ref_.clone())))
                                    .child(SharedString::from(run_open(run))),
                            ),
                    )
                    .child(fonts::figures(div().flex_none().text_base().font_weight(FontWeight::SEMIBOLD)).child(SharedString::from(done_total(&run.rollup))))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.open_run(id.clone(), cx))),
            );
        }
        card.into_any_element()
    }

    fn render_legend(&self, cx: &App) -> AnyElement {
        let colors = cx.theme().colors();
        let mut line = div().relative().flex().flex_wrap().gap_3().text_xs().text_color(colors.text_muted).child(hit_target(&self.hits, "work-legend".into()));
        for (key, label) in LEGEND {
            line = line.child(div().flex().items_center().gap_1().child(dot(key.mark(cx), 8.)).child(label));
        }
        line.into_any_element()
    }

    pub(super) fn read_project_prompts(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<String> = self.projects.iter().map(|p| p.item.id.clone()).filter(|id| !self.prompts.by_scope.contains_key(id)).collect();
        for id in ids {
            self.read_prompts(id, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use supraman_workman::{ItemRef, Session};

    use super::*;

    fn project(open: u32, blocked: u32) -> OverviewProject {
        OverviewProject { rollup: Rollup { open, blocked, done: 9, ..Default::default() }, ..Default::default() }
    }

    #[test]
    fn the_sentence_counts_open_tasks_projects_and_what_needs_you() {
        assert_eq!(sentence(&[project(3, 1), project(1, 0)], 2), "5 tasks open across 2 projects. 2 things need you.");
        assert_eq!(sentence(&[project(1, 0)], 1), "1 task open across 1 project. 1 thing needs you.");
    }

    #[test]
    fn forced_closures_alone_are_in_the_error_colour() {
        let t = tiles(&Honesty { done_unverified: 0, forced: 4, ungated: 9, followups_open: 316 }, 457);
        assert_eq!(t, [(0, "closed without independent proof", false), (4, "forced closures", true), (316, "follow-ups open", false), (457, "tasks open", false)]);
        assert!(!tiles(&Honesty::default(), 0)[1].2);
    }

    #[test]
    fn a_claim_says_role_ref_tree_and_when_its_lease_ends() {
        let now = rfc3339_ms("2026-11-07T00:15:00Z").unwrap();
        let claim = Claim {
            item: ItemRef { qualified_ref: "w1/1".into(), ..Default::default() },
            session: Some(Session { name: "supraman".into(), tree: Some("p:/wt/w1".into()), ..Default::default() }),
            role: "orchestrator".into(),
            expires_at: "2026-11-07T00:43:30Z".into(),
            ..Default::default()
        };
        assert_eq!(claim_facts(&claim, now), ["orchestrator", "w1/1", "p:/wt/w1", "lease ends in 29m"]);
        assert_eq!(lease("2026-11-07T00:10:00Z", false, now), "lease ended");
        assert_eq!(lease("2026-11-07T00:43:30Z", true, now), "lease ended");
    }
}
