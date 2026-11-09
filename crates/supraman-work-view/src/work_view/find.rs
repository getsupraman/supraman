//! Ctrl+P: the shared palette over the items this tab has loaded, then Workman's `/api/v1/search` hits in the bound projects.

use std::ops::Range;
use std::rc::Rc;

use gpui::{FontWeight, Hsla};
use supraman_ui::fonts;
use supraman_ui::matcher::highlighted;
use supraman_ui::palette::{Entry, Palette, PaletteEvent, marks_in};
use supraman_workman::{ItemStatus, SearchHit};

use super::*;

pub(crate) const LIMIT: u32 = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Found {
    pub(crate) item: ItemRef,
    pub(crate) project: String,
    pub(crate) snippet: Option<String>,
}

impl Entry for Found {
    fn key(&self) -> SharedString {
        self.item.id.clone().into()
    }

    fn text(&self) -> String {
        format!("{} {}", self.item.qualified_ref, self.item.title)
    }
}

pub(crate) fn loaded(projects: &[OverviewProject], boards: &HashMap<String, BoardState>) -> Vec<Found> {
    let mut out: Vec<Found> = Vec::new();
    let mut push = |item: &ItemRef, project: &str| {
        if !out.iter().any(|f| f.item.id == item.id) {
            out.push(Found { item: item.clone(), project: project.to_string(), snippet: None });
        }
    };
    for p in projects {
        push(&p.item, &p.item.title);
        for r in &p.runs {
            push(&r.item, &p.item.title);
        }
    }
    let mut runs: Vec<&Board> = boards.values().filter_map(|b| if let BoardState::Ready(b) = b { Some(b) } else { None }).collect();
    runs.sort_by(|a, b| a.run.qualified_ref.cmp(&b.run.qualified_ref));
    for board in runs {
        let project = board.project.as_ref().map(|p| p.title.clone()).unwrap_or_default();
        for row in &board.rows {
            push(&row.item, &project);
        }
    }
    out
}

pub(crate) fn remote(hits: Vec<SearchHit>, projects: &[OverviewProject]) -> Vec<Found> {
    hits.into_iter()
        .filter_map(|h| {
            let project = projects.iter().find(|p| h.project.as_ref().is_some_and(|hp| hp.id == p.item.id))?;
            Some(Found { item: h.item, project: project.item.title.clone(), snippet: h.snippet })
        })
        .collect()
}

fn status_color(status: ItemStatus, cx: &App) -> Hsla {
    let s = cx.theme().status();
    match status {
        ItemStatus::Done => s.success,
        ItemStatus::InProgress | ItemStatus::Review => s.info,
        ItemStatus::Blocked => s.error,
        _ => cx.theme().colors().text_muted,
    }
}

fn paint(found: &Found, marks: &[Range<usize>], on: bool, cx: &mut App) -> AnyElement {
    let colors = cx.theme().colors().clone();
    let qref = found.item.qualified_ref.clone();
    let split = qref.len() + 1;
    let ref_marks = marks_in(marks, 0..qref.len());
    let title_marks = marks_in(marks, split..split + found.item.title.len());
    let status = found.item.status;
    let pill = status_color(status, cx);
    div()
        .flex()
        .flex_col()
        .gap(px(1.))
        .child(highlighted(SharedString::from(format!("work-find-ref-{qref}")), qref.clone(), &ref_marks, cx).font_family(fonts::MONO).text_xs().text_color(if on { colors.text_accent } else { colors.text_muted }))
        .child(highlighted(SharedString::from(format!("work-find-title-{qref}")), found.item.title.clone(), &title_marks, cx).text_sm().font_weight(FontWeight::MEDIUM))
        .children(found.snippet.clone().map(|s| div().text_xs().text_color(colors.text_muted).overflow_hidden().whitespace_nowrap().text_ellipsis().child(SharedString::from(s))))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_xs()
                .text_color(colors.text_muted)
                .child(SharedString::from(found.item.kind.clone()))
                .child(div().px_1().rounded_sm().bg(pill.opacity(0.15)).text_color(pill).child(SharedString::from(status.as_str().replace('_', " "))))
                .child(SharedString::from(found.project.clone())),
        )
        .into_any_element()
}

impl WorkView {
    pub fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(palette) = &self.palette {
            palette.update(cx, |p, cx| p.focus(window, cx));
            return;
        }
        let entries = loaded(&self.projects, &self.boards);
        let hits = self.hits.clone();
        let palette = cx.new(|cx| Palette::new("work-find", hits, entries, Rc::new(paint), window, cx));
        self.palette_sub = Some(cx.subscribe_in(&palette, window, Self::on_palette));
        palette.update(cx, |p, cx| p.focus(window, cx));
        self.palette = Some(palette);
        cx.notify();
    }

    pub fn close_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.take().is_some() {
            self.palette_sub = None;
            window.focus(&self.focus, cx);
            cx.notify();
        }
    }

    fn on_palette(&mut self, palette: &Entity<Palette<Found>>, event: &PaletteEvent<Found>, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            PaletteEvent::Query(q) => self.search(q.clone(), palette.downgrade(), cx),
            PaletteEvent::Close => self.close_palette(window, cx),
            PaletteEvent::Open(found) => {
                let item = found.item.clone();
                self.close_palette(window, cx);
                match item.kind.as_str() {
                    "run" => self.open_run(item.id, cx),
                    "project" => self.show_page(Page::Overview, cx),
                    _ => self.open_from_inbox(item.id, cx),
                }
            }
        }
    }

    fn search(&mut self, q: String, palette: gpui::WeakEntity<Palette<Found>>, cx: &mut Context<Self>) {
        if q.trim().is_empty() {
            return;
        }
        let Some(client) = self.client(cx) else { return };
        self.search_reads += 1;
        cx.spawn(async move |this, cx| {
            let asked = q.clone();
            let read = cx.background_executor().spawn(async move { client.search(&asked, Some(LIMIT)) }).await;
            let Ok(hits) = read else { return };
            let found = this.update(cx, |this, _| remote(hits.hits, &this.projects)).unwrap_or_default();
            palette.update(cx, |p, cx| p.set_remote(&q, found, cx)).ok();
        })
        .detach();
    }

    pub(super) fn render_palette(&self) -> Option<AnyElement> {
        let palette = self.palette.clone()?;
        Some(div().absolute().top_0().left_0().right_0().flex().justify_center().pt(px(48.)).px_4().child(palette).into_any_element())
    }
}

#[cfg(test)]
mod tests {
    use supraman_workman::ProjectRepo;

    use super::*;

    fn iref(id: &str, qref: &str, kind: &str) -> ItemRef {
        ItemRef { id: id.into(), qualified_ref: qref.into(), kind: kind.into(), title: format!("T {qref}"), ..Default::default() }
    }

    #[test]
    fn workmans_hits_outside_the_bound_projects_are_left_out() {
        let bound = OverviewProject { item: ItemRef { title: "Alpha".into(), ..iref("pa", "pa", "project") }, repos: vec![ProjectRepo::default()], ..Default::default() };
        let hit = |id: &str, project: Option<&str>| SearchHit { item: iref(id, id, "task"), project: project.map(|p| iref(p, p, "project")), snippet: Some("s".into()), ..Default::default() };
        let found = remote(vec![hit("a1", Some("pa")), hit("c1", Some("pc")), hit("x", None)], &[bound]);
        assert_eq!(found, [Found { item: iref("a1", "a1", "task"), project: "Alpha".into(), snippet: Some("s".into()) }]);
        assert_eq!(found[0].text(), "a1 T a1");
    }
}
