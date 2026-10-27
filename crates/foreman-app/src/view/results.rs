//! The search box over the tree and its results in place of the tree (m5/30): a summary line, then the
//! workspaces › repos › sessions found ([`crate::search`]), each with its count and the matched text
//! highlighted. A session row is the tree's own ([`FleetView::render_session`]), so it keeps its star,
//! attach / resume and right-click menu; a click elsewhere on it opens it: its attach tab, else an attach
//! when it can take one. A workspace or repo row, clicked, shows it in the tree: the box is cleared and
//! the row unfolded.

use std::{collections::HashSet, ops::Range};

use foreman_claude::now_ms;
use gpui::{AnyElement, ClickEvent, Context, Div, FontWeight, HighlightStyle, SharedString, Stateful, StyledText, div, prelude::*, px};
use theme::ActiveTheme as _;

use super::left::{quiet, repo_label, secondary};
use super::{FleetView, TabKind, hit_target};
use crate::fleet::{Supervised, Tree};
use crate::search::{self, Results, SessionHit};

/// `text` on one line, cut with an ellipsis, `marks` (byte ranges) in the search match colours.
pub(crate) fn marked(key: impl Into<SharedString>, text: impl Into<SharedString>, marks: &[Range<usize>], cx: &gpui::App) -> Stateful<Div> {
    let style = HighlightStyle {
        background_color: Some(cx.theme().colors().search_match_background),
        font_weight: Some(FontWeight::BOLD),
        ..Default::default()
    };
    div()
        .id(key.into())
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .child(StyledText::new(text.into()).with_highlights(marks.iter().map(|r| (r.clone(), style))))
}

fn count(n: usize, cx: &gpui::App) -> Div {
    let colors = cx.theme().colors();
    div()
        .flex_none()
        .min_w(px(18.))
        .px_1()
        .rounded_full()
        .bg(colors.element_background)
        .text_xs()
        .text_center()
        .text_color(colors.text)
        .child(SharedString::from(n.to_string()))
}

impl FleetView {
    /// The whole tree for the search: stopped sessions shown, every closed one listed (none behind `more…`).
    pub fn search_tree(&self) -> Tree {
        let all: HashSet<String> = self.workspaces.iter().flat_map(|w| w.repos.iter().map(move |r| format!("{}/{}", w.name, r.name))).collect();
        let sup = Supervised {
            desired: Some(&self.desired),
            hooks: Some(&self.hooks),
            polled_at: self.polled_at,
            main_repos: Some(&self.main_repos),
            closed: &self.closed,
            starred: Some(self.stars.ids()),
            expanded: Some(&all),
        };
        crate::fleet::build(&self.workspaces, &self.sessions, now_ms(), true, &sup)
    }

    pub fn search_results(&self, cx: &gpui::App) -> Results {
        search::search(&self.search_tree(), &self.search_query(cx))
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

    /// The box cleared, the workspace (and the repo) unfolded in the tree.
    fn reveal(&mut self, workspace: &str, repo: Option<&str>, cx: &mut Context<Self>) {
        self.collapsed.remove(&format!("workspace:{workspace}"));
        if let Some(repo) = repo {
            self.collapsed.remove(&format!("repo:{workspace}/{repo}"));
        }
        self.clear_search(cx);
    }

    /// A click on a session result: its attach tab comes forward, or one opens when it can attach. Its
    /// own attach pill ran first, so the tab it opened is the one brought forward.
    fn open_result(&mut self, workspace: &str, repo: &str, hit: &SessionHit, window: &mut gpui::Window, cx: &mut Context<Self>) {
        let s = &hit.row.session;
        let open = self.tabs.iter().find(|t| matches!(&t.kind, TabKind::Attach { session_id } if *session_id == s.id)).map(|t| t.id);
        match open {
            Some(id) => self.activate_tab(id, cx),
            None if hit.row.attachable => {
                self.attach(workspace, repo, s, window, cx);
            }
            None => {}
        }
    }

    /// The results in place of the tree, into `body`.
    pub(super) fn render_results(&mut self, mut body: Stateful<Div>, query: &str, cx: &mut Context<Self>) -> Stateful<Div> {
        let colors = cx.theme().colors().clone();
        let results = search::search(&self.search_tree(), query);
        self.search_marks = results
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
                .py_1()
                .text_xs()
                .text_color(secondary(cx))
                .child(SharedString::from(line))
                .child(hit_target(&self.hits, key.into())),
        );
        let row = |key: String, indent: f32, h: f32| {
            div()
                .id(SharedString::from(key.clone()))
                .relative()
                .flex()
                .flex_none()
                .items_center()
                .gap_2()
                .h(px(h))
                .pl(px(indent))
                .pr_2()
                .cursor_pointer()
                .hover(|s| s.bg(colors.ghost_element_hover))
                .active(|s| s.bg(colors.ghost_element_active))
                .child(hit_target(&self.hits, key))
        };
        for w in &results.workspaces {
            let name = w.name.clone();
            body = body.child(
                row(format!("result-ws-{}", w.name), 8., 28.)
                    .mt_1()
                    .child(marked(format!("result-ws-name-{}", w.name), w.name.clone(), &w.name_marks, cx).flex_none().font_weight(FontWeight::BOLD))
                    .child(crate::clipped::clipped(format!("result-ws-root-{}", w.name), w.root.display().to_string(), true).flex_1().text_xs().text_color(secondary(cx)))
                    .child(count(w.count(), cx))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.reveal(&name, None, cx))),
            );
            for hit in &w.supervised {
                body = body.child(self.render_result_session(&w.name, "supervised", hit, 26., cx));
            }
            for r in &w.repos {
                let label = repo_label(&r.name, &r.path, &w.root);
                // the name sits after `./` in the label, or first in `<name> (root)`
                let shift = label.find(r.name.as_str()).unwrap_or(0);
                let marks: Vec<Range<usize>> = r.name_marks.iter().map(|m| m.start + shift..m.end + shift).collect();
                let (wname, rname) = (w.name.clone(), r.name.clone());
                body = body.child(
                    row(format!("result-repo-{}/{}", w.name, r.name), 26., 24.)
                        .text_sm()
                        .when(r.sessions.is_empty(), |d| d.text_color(quiet(cx)))
                        .child(marked(format!("result-repo-name-{}/{}", w.name, r.name), label, &marks, cx).flex_1())
                        .child(count(r.sessions.len(), cx))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.reveal(&wname, Some(&rname), cx))),
                );
                for hit in &r.sessions {
                    body = body.child(self.render_result_session(&w.name, &r.name, hit, 46., cx));
                }
            }
        }
        body
    }

    fn render_result_session(&self, workspace: &str, repo: &str, hit: &SessionHit, indent: f32, cx: &mut Context<Self>) -> AnyElement {
        let (w, r, h) = (workspace.to_string(), repo.to_string(), hit.clone());
        div()
            .id(SharedString::from(format!("result-session-{}", hit.row.session.session_id)))
            .flex_none()
            .cursor_pointer()
            .child(self.render_session(workspace, repo, &hit.row, px(indent), cx))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| this.open_result(&w, &r, &h, window, cx)))
            .into_any_element()
    }
}

