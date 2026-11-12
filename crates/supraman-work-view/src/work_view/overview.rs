//! The Overview page, the web's: one sentence, Needs you, Working now, the honesty tiles, and a card per bound project.

use supraman_core::text::count;
use gpui::{ClickEvent, FontWeight, Hsla};
use supraman_core::time::{self, rfc3339_ms};
use supraman_ui::placeholder::placeholder;
use supraman_ui::hit::hit_target;
use supraman_ui::marks::{Mark, dot};
use supraman_ui::clipped::clipped;
use supraman_ui::{button, fonts};
use supraman_workman::{Claim, Honesty, InboxRow, OverviewRun, Rollup};

use super::cards::{self, Card};
use super::item::holder_session;
use super::navigator::done_total;
use super::projects::{active, closed_runs};
use super::state::StateKey;
use super::*;

pub(crate) const NEEDS_SHOWN: usize = 5;
/// Wide enough for the longest word of a tile's label, so a label wraps between words only.
const TILE_MIN: f32 = 120.;

pub(crate) fn open_of(r: &Rollup) -> u32 {
    r.open + r.in_progress + r.review + r.blocked
}

pub(crate) fn sentence(projects: &[OverviewProject], needs: usize) -> String {
    let tasks: u32 = projects.iter().map(|p| open_of(&p.rollup)).sum();
    let needs = count(needs, "thing needs you", "things need you");
    format!("{} open across {}. {needs}.", count(tasks as usize, "task", "tasks"), count(projects.len(), "project", "projects"))
}

pub(crate) fn as_of(at: &str, now: i64) -> String {
    match rfc3339_ms(at) {
        Some(ms) => format!("As of {}", time::near(ms, now)),
        None => String::new(),
    }
}

/// The tiles the read API gives: Workman's honesty counters (every project of the server) and the bound projects' open tasks.
pub(crate) fn tiles(honesty: &Honesty, tasks_open: u32) -> Vec<(u32, &'static str, bool)> {
    vec![
        (honesty.done_unverified, "closed without independent proof", false),
        (honesty.forced, "forced closures", honesty.forced > 0),
        (honesty.followups_open, "follow-ups open", false),
        (tasks_open, "tasks open", false),
    ]
}

/// Within the hour in minutes, as the web says it; later its time here.
pub(crate) fn lease(expires_at: &str, expired: bool, now: i64) -> String {
    match rfc3339_ms(expires_at) {
        Some(at) if expired || at <= now => "lease ended".into(),
        Some(at) if at - now <= 3_600_000 => format!("lease ends in {}m", (at - now + 59_999) / 60_000),
        Some(at) => format!("lease ends at {}", time::near(at, now)),
        None => String::new(),
    }
}

pub(crate) fn claim_facts(claim: &Claim, now: i64) -> Vec<String> {
    let mut out = vec![claim.role.clone(), claim.item.qualified_ref.clone()];
    out.extend(claim.tree.clone().or_else(|| claim.session.as_ref().and_then(|s| s.tree.clone())));
    out.push(lease(&claim.expires_at, claim.expired, now));
    out.retain(|t| !t.is_empty());
    out
}

pub(crate) fn working_now<'a>(overview: &'a Overview, projects: &[OverviewProject]) -> Vec<&'a Claim> {
    let runs: HashSet<&str> = projects.iter().flat_map(|p| p.runs.iter().map(|r| r.item.id.as_str())).collect();
    overview.working_now.iter().filter(|c| c.run.as_ref().is_some_and(|r| runs.contains(r.id.as_str()))).collect()
}

pub(crate) fn runs_line(project: &OverviewProject) -> String {
    format!("{} open, {} closed", active(project).count(), closed_runs(project).count())
}

pub(crate) fn run_open(run: &OverviewRun) -> String {
    format!("{} open", open_of(&run.rollup))
}

pub(crate) const LEGEND: [(StateKey, &str); 6] =
    [(StateKey::Proven, "Verified"), (StateKey::Claimed, "Done, not verified"), (StateKey::Doing, "In progress"), (StateKey::Stop, "Blocked"), (StateKey::Wait, "Waiting"), (StateKey::Idle, "Open")];

impl WorkView {
    /// The Needs you cards of this workspace's Inbox rows, as the sidebar sorts them.
    pub(crate) fn needs_cards(&self, cx: &App) -> Vec<Card> {
        let Some(inbox) = &self.inbox else { return Vec::new() };
        let rows: Vec<(&str, &InboxRow)> = inbox.read(cx).rows().into_iter().filter(|(w, _)| *w == self.workspace).collect();
        let live: Vec<String> = (self.link.live)().into_iter().filter(|s| s.attach.is_some()).map(|s| s.name).collect();
        cards::sections(&rows, &live, (self.now)()).needs_you
    }

    pub(super) fn render_overview(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let now = (self.now)();
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
        let mut card = self.card("Needs you", Some(needs.len()), cx).flex_1().min_w(px(320.));
        if needs.is_empty() {
            card = card.child(placeholder("Nothing needs you.", Vec::<SharedString>::new(), cx).px_3().pb_3());
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
                    .child(div().pt(px(6.)).child(dot(Mark::Filled(supraman_ui::inbox_card::kind_color(&c.kind, cx)))))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_sm().whitespace_normal().child(SharedString::from(c.title.clone())))
                            .child(div().text_xs().text_color(colors.text_muted).child(SharedString::from(format!("{} · {}", c.badge, c.ref_)))),
                    )
                    .children(c.since.and_then(|at| supraman_ui::inbox_card::when(format!("work-overview-needs-when-{}", c.ref_), at, now, cx)).map(|w| w.text_xs()))
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
        let claims: Vec<Claim> = self.overview.as_ref().map(|o| working_now(o, &self.projects).into_iter().cloned().collect()).unwrap_or_default();
        let live = (self.link.live)();
        let mut card = self.card("Working now", Some(claims.len()), cx);
        if claims.is_empty() {
            card = card.child(placeholder("No one holds a claim.", Vec::<SharedString>::new(), cx).px_3().pb_3());
        }
        for claim in claims {
            let qref = claim.item.qualified_ref.clone();
            let id = claim.item.id.clone();
            let holder = claim.holder().to_string();
            let facts = claim_facts(&claim, now);
            let found = holder_session(&claim, &live);
            let mark = Mark::Filled(found.map_or(cx.theme().status().info, |s| s.activity.color(cx)));
            let open = found.map(|s| self.open_control(format!("work-working-open-{qref}"), &s.claude_session_id, cx));
            card = card.child(
                div()
                    .flex()
                    .items_start()
                    .gap_2()
                    .px_3()
                    .py(px(5.))
                    .border_t_1()
                    .border_color(colors.border_variant)
                    .child(div().pt(px(6.)).child(dot(mark)))
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
                                    .relative()
                                    .flex()
                                    .flex_wrap()
                                    .items_center()
                                    .gap_1()
                                    .text_xs()
                                    .text_color(colors.text_muted)
                                    .child(SharedString::from(holder))
                                    .children(open)
                                    .children(facts.iter().map(|t| SharedString::from(format!("· {t}"))))
                                    .child(hit_target(&self.hits, format!("work-working-facts-{qref}={}", facts.join(" · ")))),
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
        let mut row = div().flex().flex_wrap().rounded_md().border_1().border_color(colors.border_variant).bg(colors.panel_background);
        for (n, label, alarm) in tiles(&honesty, tasks) {
            let fg: Hsla = if alarm { error } else { colors.text };
            let words = label.split(' ').enumerate().map(|(i, word)| {
                div().relative().flex_none().whitespace_nowrap().child(word).child(hit_target(&self.hits, format!("work-tile-word-{label}-{i}")))
            });
            row = row.child(
                div()
                    .relative()
                    .flex_grow(1.)
                    .flex_basis(px(TILE_MIN))
                    .min_w(px(TILE_MIN))
                    .px_3()
                    .py_2()
                    .child(fonts::figures(div().text_xl().font_weight(FontWeight::SEMIBOLD).text_color(fg)).child(SharedString::from(n.to_string())))
                    .child(div().flex().flex_wrap().gap_x(px(3.)).text_xs().text_color(colors.text_muted).children(words))
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
            .child(clipped(format!("work-overview-project-title-{qref}"), project.item.title.clone(), false).flex_shrink(1.).text_base().font_weight(FontWeight::SEMIBOLD))
            .child(div().flex_none().font_family(fonts::MONO).text_xs().text_color(colors.text_muted).child(SharedString::from(qref.clone())))
            .child(div().flex_1())
            .child(
                div()
                    .relative()
                    .flex_none()
                    .text_xs()
                    .text_color(colors.text_muted)
                    .child(SharedString::from(runs_line(project)))
                    .child(hit_target(&self.hits, format!("work-overview-runs-{qref}"))),
            );
        let mut card = div()
            .relative()
            .flex()
            .flex_col()
            .gap_2()
            .rounded_md()
            .border_1()
            .border_color(colors.border_variant)
            .bg(colors.panel_background)
            .min_w_0()
            .overflow_hidden()
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
            line = line.child(div().flex().items_center().gap_1().child(dot(key.mark(cx))).child(label));
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
        assert_eq!(lease("2026-11-07T00:41:00Z", false, now), "lease ends in 26m");
        assert_eq!(lease("2026-11-07T00:10:00Z", false, now), "lease ended");
        assert_eq!(lease("2026-11-07T00:43:30Z", true, now), "lease ended");
        let later = rfc3339_ms("2026-11-07T03:00:00Z").unwrap();
        assert_eq!(lease("2026-11-07T03:00:00Z", false, now), format!("lease ends at {}", time::near(later, now)), "past the hour: its time, never a count of minutes");
    }
}
