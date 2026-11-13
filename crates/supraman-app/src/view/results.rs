use std::collections::HashSet;

use supraman_claude::now_ms;
use supraman_ui::matcher::{count, highlighted};
use gpui::{AnyElement, ClickEvent, Context, Div, FontWeight, SharedString, Stateful, div, prelude::*, px};
use theme::ActiveTheme as _;

use supraman_ui::icon::Icon;
use super::left::{disclosure, grid, icon_slot, quiet, secondary, tree_label};
use super::{FleetView, hit_target};
use crate::fleet::{Open, Tree, TreeInputs};
use crate::search::{self, EntryHit, FolderHit, SessionHit};

impl FleetView {
    /// The tree with every closed session and stopped row: what a search looks through.
    pub fn search_tree(&self) -> Tree {
        let all: HashSet<String> = self.tree().folders().map(|(_, f)| f.key.clone()).collect();
        let own_names = self.own_names();
        let sup = TreeInputs {
            hooks: Some(&self.polled.hooks),
            polled_at: self.polls.polled_at,
            main_repos: Some(&self.listing.main_repos),
            disk: Some(&self.listed.disk),
            closed: &self.listing.closed,
            starred: Some(self.stars.ids()),
            expanded: Some(&all),
            own_names: &own_names,
            own_prefix: self.own_prefix.as_deref(),
        };
        crate::fleet::build(&self.listed.entries, &self.sessions, now_ms(), true, &sup)
    }

    pub(super) fn render_search_box(&self) -> AnyElement {
        div()
            .id("search-box")
            .relative()
            .flex_none()
            .px_2()
            .py(px(4.))
            .child(self.search.clone())
            .child(hit_target(&self.hits, "search-box".into()))
            .into_any_element()
    }

    /// Out of the search, with the group and the folder `key` open.
    pub(super) fn reveal(&mut self, group: Option<&str>, key: Option<&str>, cx: &mut Context<Self>) {
        if let Some(group) = group {
            self.sidebar.collapsed.remove(&format!("group:{group}"));
        }
        if let Some(key) = key {
            self.sidebar.collapsed.remove(&format!("repo:{key}"));
        }
        self.clear_search(cx);
    }

    /// A click opens what needs no new claude; the row's own double-click also resumes.
    fn open_result(&mut self, place: &str, hit: &SessionHit, cx: &mut Context<Self>) {
        if matches!(self.open_of(&hit.row), Open::Tab(_) | Open::Attach) {
            self.open_row(&place.replace('/', " · "), &hit.row, cx);
        }
    }

    pub(super) fn render_results(&mut self, mut body: Stateful<Div>, query: &str, cx: &mut Context<Self>) -> Stateful<Div> {
        let results = search::search(&self.search_tree(), query);
        self.sidebar.search_marks = results
            .entries
            .iter()
            .flat_map(|w| w.runs.iter().chain(w.folders.iter().flat_map(|r| &r.sessions)))
            .map(|h| (h.row.session.session_id.clone(), h.marks.clone()))
            .collect();
        let (key, line) = if results.is_empty() { ("search-none", format!("No results for \u{201c}{query}\u{201d}")) } else { ("search-summary", results.summary()) };
        body = body.child(
            div()
                .relative()
                .flex_none()
                .px_3()
                .pt_2()
                .pb_1()
                .text_xs()
                .text_color(secondary(cx))
                .child(SharedString::from(line))
                .child(hit_target(&self.hits, key.into())),
        );
        for entry in &results.entries {
            body = self.render_result_entry(body, entry, cx);
        }
        body
    }

    fn result_row(&self, key: String, level: usize, h: f32, cx: &mut Context<Self>) -> Stateful<Div> {
        let colors = cx.theme().colors().clone();
        div()
            .id(SharedString::from(key.clone()))
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(grid::gap())
            .h(px(h))
            .pl(grid::indent(level))
            .pr_2()
            .cursor_pointer()
            .hover(|s| s.bg(colors.ghost_element_hover))
            .active(|s| s.bg(colors.ghost_element_active))
            .child(hit_target(&self.hits, key))
    }

    fn render_result_entry(&self, mut body: Stateful<Div>, w: &EntryHit, cx: &mut Context<Self>) -> Stateful<Div> {
        let group = w.group.then(|| w.name.clone());
        if w.group {
            let (name, key) = (w.name.clone(), format!("result-group-{}", w.name));
            body = body.child(
                self.result_row(key.clone(), 0, 28., cx)
                    .mt_1()
                    .child(disclosure(None, secondary(cx)))
                    .child(icon_slot(&self.hits, &key, None))
                    .child(tree_label(&self.hits, &key, highlighted(format!("result-group-name-{}", w.name), w.name.clone(), &w.name_marks, cx).flex_1().font_weight(FontWeight::BOLD)))
                    .child(count(w.count(), cx))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.reveal(Some(&name), None, cx))),
            );
        }
        for hit in &w.runs {
            body = body.child(self.render_result_session(&w.name, "scheduled", hit, 1, cx));
        }
        for r in &w.folders {
            body = body.child(self.render_result_folder(group.clone(), r, usize::from(w.group), cx));
            for hit in &r.sessions {
                body = body.child(self.render_result_session(&w.name, &r.key, hit, usize::from(w.group) + 1, cx));
            }
        }
        body
    }

    fn render_result_folder(&self, group: Option<String>, r: &FolderHit, level: usize, cx: &mut Context<Self>) -> Stateful<Div> {
        let key = format!("result-repo-{}", r.key);
        let icon = if r.kind == supraman_core::FolderKind::Git { Icon::GitBranch } else { Icon::Folder };
        let folder = r.key.clone();
        self.result_row(key.clone(), level, 24., cx)
            .text_sm()
            .when(r.sessions.is_empty(), |d| d.text_color(quiet(cx)))
            .child(disclosure(None, secondary(cx)))
            .child(icon_slot(&self.hits, &key, Some(icon.slotted(grid::ICON, secondary(cx)).into_any_element())))
            .child(tree_label(&self.hits, &key, highlighted(format!("result-repo-name-{}", r.key), r.name.clone(), &r.name_marks, cx).flex_1()))
            .child(count(r.sessions.len(), cx))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.reveal(group.as_deref(), Some(&folder), cx)))
    }

    fn render_result_session(&self, entry: &str, place: &str, hit: &SessionHit, level: usize, cx: &mut Context<Self>) -> AnyElement {
        let (p, h) = (place.to_string(), hit.clone());
        div()
            .id(SharedString::from(format!("result-session-{}{}", hit.row.session.session_id, hit.row.scope)))
            .flex_none()
            .cursor_pointer()
            .child(self.render_session(entry, place, &hit.row, grid::indent(level), cx))
            .on_click(cx.listener(move |this, e: &ClickEvent, _, cx| {
                if e.click_count() == 1 {
                    this.open_result(&p, &h, cx);
                }
            }))
            .into_any_element()
    }
}

