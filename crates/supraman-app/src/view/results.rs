use std::{collections::HashSet, ops::Range};

use supraman_claude::now_ms;
use supraman_ui::matcher::{count, highlighted};
use gpui::{AnyElement, ClickEvent, Context, Div, FontWeight, SharedString, Stateful, div, prelude::*, px};
use theme::ActiveTheme as _;

use supraman_ui::icon::Icon;
use super::left::{disclosure, grid, home_text, icon_slot, quiet, repo_label, secondary, tree_label};
use super::{FleetView, hit_target};
use crate::fleet::{Open, Supervised, Tree};
use crate::search::{self, SessionHit};

impl FleetView {
    pub fn search_tree(&self) -> Tree {
        let all: HashSet<String> = self.workspaces.iter().flat_map(|w| w.folders.iter().map(move |r| format!("{}/{}", w.name, r.name))).collect();
        let sup = Supervised {
            desired: Some(&self.supervising.desired),
            hooks: Some(&self.supervising.hooks),
            polled_at: self.polls.polled_at,
            main_repos: Some(&self.main_repos),
            closed: &self.closed,
            starred: Some(self.stars.ids()),
            expanded: Some(&all),
            disabled_folders: true,
            own_prefix: self.own_prefix.as_deref(),
        };
        crate::fleet::build(&self.workspaces, &self.sessions, now_ms(), true, &sup)
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

    pub(super) fn reveal(&mut self, workspace: &str, repo: Option<&str>, cx: &mut Context<Self>) {
        self.sidebar.collapsed.remove(&format!("workspace:{workspace}"));
        if let Some(repo) = repo {
            self.sidebar.collapsed.remove(&format!("repo:{workspace}/{repo}"));
        }
        self.clear_search(cx);
    }

    /// A click opens what needs no new claude; the row's own double-click also resumes.
    fn open_result(&mut self, workspace: &str, repo: &str, hit: &SessionHit, window: &mut gpui::Window, cx: &mut Context<Self>) {
        if matches!(self.open_of(&hit.row), Open::Tab(_) | Open::Attach) {
            self.open_row(workspace, repo, &hit.row, window, cx);
        }
    }

    pub(super) fn render_results(&mut self, mut body: Stateful<Div>, query: &str, cx: &mut Context<Self>) -> Stateful<Div> {
        let colors = cx.theme().colors().clone();
        let results = search::search(&self.search_tree(), query);
        self.sidebar.search_marks = results
            .workspaces
            .iter()
            .flat_map(|w| w.supervised.iter().chain(w.repos.iter().flat_map(|r| &r.sessions)))
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
        let row = |key: String, level: usize, h: f32| {
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
        };
        for w in &results.workspaces {
            let name = w.name.clone();
            let ws_key = format!("result-ws-{}", w.name);
            body = body.child(
                row(ws_key.clone(), 0, 28.)
                    .mt_1()
                    .child(disclosure(None, secondary(cx)))
                    .child(icon_slot(&self.hits, &ws_key, None))
                    .child(tree_label(&self.hits, &ws_key, highlighted(format!("result-ws-name-{}", w.name), w.name.clone(), &w.name_marks, cx).flex_none().font_weight(FontWeight::BOLD)))
                    .child(supraman_ui::clipped::clipped(format!("result-ws-root-{}", w.name), home_text(w.home.as_deref()), true).flex_1().text_xs().text_color(secondary(cx)))
                    .child(count(w.count(), cx))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.reveal(&name, None, cx))),
            );
            for hit in &w.supervised {
                body = body.child(self.render_result_session(&w.name, "supervised", hit, 1, cx));
            }
            for r in &w.repos {
                let label = repo_label(&r.name, &r.path, w.home.as_deref());
                let shift = label.find(r.name.as_str()).unwrap_or(0);
                let marks: Vec<Range<usize>> = r.name_marks.iter().map(|m| m.start + shift..m.end + shift).collect();
                let (wname, rname, disabled, owner, path) = (w.name.clone(), r.name.clone(), r.disabled, r.supervised_in.clone(), r.path.clone());
                let repo_key = format!("result-repo-{}/{}", w.name, r.name);
                let icon = if r.kind == supraman_core::FolderKind::Git { Icon::GitBranch } else { Icon::Folder };
                body = body.child(
                    row(repo_key.clone(), 1, 24.)
                        .text_sm()
                        .when(r.sessions.is_empty(), |d| d.text_color(quiet(cx)))
                        .child(disclosure(None, secondary(cx)))
                        .child(icon_slot(&self.hits, &repo_key, Some(icon.slotted(grid::ICON, secondary(cx)).into_any_element())))
                        .child(tree_label(&self.hits, &repo_key, highlighted(format!("result-repo-name-{}/{}", w.name, r.name), label, &marks, cx).flex_1()))
                        .when(r.disabled, |d| d.child(super::left::tag("disabled", cx).relative().child(hit_target(&self.hits, format!("result-disabled-{}/{}", w.name, r.name)))))
                        .when_some(r.supervised_in.as_ref(), |d, owner| {
                            d.child(super::left::tag(format!("supervised in {owner}"), cx).relative().child(hit_target(&self.hits, format!("result-supervised-in-{}/{}", w.name, r.name))))
                        })
                        .child(count(r.sessions.len(), cx))
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            if let Some(owner) = &owner {
                                this.reveal_folder(owner, &path, cx);
                            } else if disabled {
                                this.clear_search(cx);
                                this.edit_folders(&wname, window, cx);
                            } else {
                                this.reveal(&wname, Some(&rname), cx);
                            }
                        })),
                );
                for hit in &r.sessions {
                    body = body.child(self.render_result_session(&w.name, &r.name, hit, 2, cx));
                }
            }
        }
        body
    }

    fn render_result_session(&self, workspace: &str, repo: &str, hit: &SessionHit, level: usize, cx: &mut Context<Self>) -> AnyElement {
        let (w, r, h) = (workspace.to_string(), repo.to_string(), hit.clone());
        div()
            .id(SharedString::from(format!("result-session-{}{}", hit.row.session.session_id, hit.row.scope(workspace))))
            .flex_none()
            .cursor_pointer()
            .child(self.render_session(workspace, repo, &hit.row, grid::indent(level), cx))
            .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                if e.click_count() == 1 {
                    this.open_result(&w, &r, &h, window, cx);
                }
            }))
            .into_any_element()
    }
}

