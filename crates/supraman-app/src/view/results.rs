use std::collections::HashSet;

use supraman_claude::now_ms;
use supraman_ui::tokens;
use supraman_ui::controls::badge::count;
use supraman_ui::matcher::highlighted;
use gpui::{AnyElement, ClickEvent, Context, Div, Entity, Focusable as _, FontWeight, SharedString, Stateful, Window, div, prelude::*, px};
use supraman_ui::list::{Rows, Selection, Step};
use supraman_ui::text::input::{self, InputEvent, TextInput};

use supraman_ui::icon::Icon;
use supraman_ui::scroll::ScrollArea;
use super::left::{disclosure, fold, grid, icon_slot, quiet, secondary, tree_label};
use super::{FleetView, hit_target};
use crate::fleet::{Open, Tree, TreeInputs};
use crate::search::{self, EntryHit, FolderHit, SessionHit};

/// The search results' keyboard selection: the rows the last frame showed, in order, and the one selected, by key.
#[derive(Default)]
pub(super) struct Picks {
    rows: Vec<(SharedString, Pick)>,
    selected: Option<SharedString>,
    /// The results show in the tree's place.
    pub shown: bool,
    /// The search box holds the keys: the selection is the active fill.
    pub held: bool,
}

/// What a result row does when picked.
#[derive(Clone)]
enum Pick {
    Group(String),
    Folder(Option<String>, String),
    Session(String, SessionHit),
}

impl Picks {
    pub fn is_selected(&self, key: &str) -> bool {
        self.selected.as_deref() == Some(key)
    }

    pub fn select(&mut self, key: SharedString) {
        self.selected = Some(key);
    }

    fn add(&mut self, key: &str, pick: Pick) {
        self.rows.push((key.to_string().into(), pick));
    }

    /// The selection moved `by` rows, from none the first; whether it moved.
    fn step(&mut self, by: isize) -> bool {
        let mut selection = Selection::default();
        selection.select(self.selected.as_ref().and_then(|k| self.rows.iter().position(|(key, _)| key == k)));
        let Some(to) = selection.go(Step::By(by), &Rows::new(self.rows.len())) else { return false };
        self.selected = Some(self.rows[to].0.clone());
        true
    }

    fn picked(&self) -> Option<Pick> {
        let key = self.selected.as_ref()?;
        self.rows.iter().find(|(k, _)| k == key).map(|(_, pick)| pick.clone())
    }
}

impl FleetView {
    /// The search box's events, and whether it holds the keys.
    pub(super) fn watch_search(search: &Entity<TextInput>, window: &mut Window, cx: &mut Context<Self>) {
        cx.subscribe_in(search, window, Self::on_search_event).detach();
        let focus = search.read(cx).focus_handle(cx);
        cx.on_focus_in(&focus, window, |this, _, cx| this.search_holds(true, cx)).detach();
        cx.on_focus_out(&focus, window, |this, _, _, cx| this.search_holds(false, cx)).detach();
    }

    fn search_holds(&mut self, held: bool, cx: &mut Context<Self>) {
        self.sidebar.results.held = held;
        cx.notify();
    }

    pub fn search_query(&self, cx: &gpui::App) -> String {
        self.search.read(cx).text().trim().to_string()
    }

    fn on_search_event(&mut self, _: &Entity<TextInput>, event: &InputEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            InputEvent::Edited => cx.notify(),
            InputEvent::Confirm => self.open_picked(cx),
            InputEvent::Cancel if self.dismiss_popup() => cx.notify(),
            InputEvent::Cancel if !self.search.read(cx).text().is_empty() => self.clear_search(cx),
            InputEvent::Cancel => self.focus.focus(window, cx),
            InputEvent::ConfirmBack | InputEvent::Refused(_) | InputEvent::Submit | InputEvent::Pressed => {}
        }
    }

    pub fn clear_search(&mut self, cx: &mut Context<Self>) {
        self.search.update(cx, |s, cx| s.set_text("", cx));
        self.sidebar.results.selected = None;
        cx.notify();
    }

    fn step_result(&mut self, by: isize, cx: &mut Context<Self>) {
        if self.sidebar.results.shown && self.sidebar.results.step(by) {
            cx.notify();
        }
    }

    /// Enter in the search box: the selected result as a click on it.
    fn open_picked(&mut self, cx: &mut Context<Self>) {
        match self.sidebar.results.picked() {
            Some(Pick::Group(name)) => self.reveal(Some(&name), None, cx),
            Some(Pick::Folder(group, key)) => self.reveal(group.as_deref(), Some(&key), cx),
            Some(Pick::Session(place, hit)) => self.open_result(&place, &hit, cx),
            None => {}
        }
    }

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

    pub(super) fn render_search_box(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("search-box")
            .on_action(cx.listener(|this, _: &input::Up, _, cx| this.step_result(-1, cx)))
            .on_action(cx.listener(|this, _: &input::Down, _, cx| this.step_result(1, cx)))
            .relative()
            .flex_none()
            .px_2()
            .py(tokens::space::S)
            .child(self.search.clone())
            .child(hit_target(&self.hits, "search-box".into()))
            .into_any_element()
    }

    /// Out of the search, with the group and the folder `key` open.
    pub(super) fn reveal(&mut self, group: Option<&str>, key: Option<&str>, cx: &mut Context<Self>) {
        if let Some(group) = group {
            self.sidebar.collapsed.remove(&fold::group(group));
        }
        if let Some(key) = key {
            self.sidebar.collapsed.remove(&fold::folder(key));
        }
        self.clear_search(cx);
    }

    /// A click opens what needs no new claude; the row's own double-click also resumes.
    fn open_result(&mut self, place: &str, hit: &SessionHit, cx: &mut Context<Self>) {
        if matches!(self.open_of(&hit.row), Open::Tab(_) | Open::Attach) {
            self.open_row(&place.replace('/', " · "), &hit.row, cx);
        }
    }

    pub(super) fn render_results(&mut self, mut body: ScrollArea, query: &str, cx: &mut Context<Self>) -> ScrollArea {
        let results = search::search(&self.search_tree(), query);
        self.sidebar.results.rows.clear();
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
        let picks = &mut self.sidebar.results;
        if picks.selected.as_ref().is_some_and(|k| !picks.rows.iter().any(|(key, _)| key == k)) {
            picks.selected = None;
        }
        body
    }

    fn result_row(&self, key: String, level: usize, h: f32, cx: &mut Context<Self>) -> Stateful<Div> {
        self.sidebar_row(supraman_ui::controls::button::RowKeys::one(key.clone().into()), Some(&key), cx)
            .flex()
            .flex_none()
            .items_center()
            .gap(grid::gap())
            .h(px(h))
            .pl(grid::indent(level))
            .pr_2()
    }

    fn render_result_entry(&mut self, mut body: ScrollArea, w: &EntryHit, cx: &mut Context<Self>) -> ScrollArea {
        let group = w.group.then(|| w.name.clone());
        if w.group {
            let (name, key) = (w.name.clone(), format!("result-group-{}", w.name));
            self.sidebar.results.add(&key, Pick::Group(name.clone()));
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
                body = body.child(self.render_result_session(&w.name, &crate::fleet::row_place(&r.key, &hit.row), hit, usize::from(w.group) + 1, cx));
            }
        }
        body
    }

    fn render_result_folder(&mut self, group: Option<String>, r: &FolderHit, level: usize, cx: &mut Context<Self>) -> Stateful<Div> {
        let key = format!("result-repo-{}", r.key);
        self.sidebar.results.add(&key, Pick::Folder(group.clone(), r.key.clone()));
        let icon = match r.kind {
            _ if r.key == crate::fleet::UNLISTED => None,
            supraman_core::FolderKind::Git => Some(Icon::GitFolder),
            _ => Some(Icon::Folder),
        };
        let folder = r.key.clone();
        self.result_row(key.clone(), level, 24., cx)
            .text_sm()
            .when(r.sessions.is_empty(), |d| d.text_color(quiet(cx)))
            .child(disclosure(None, secondary(cx)))
            .child(icon_slot(&self.hits, &key, icon.map(|i| i.slotted(grid::ICON, secondary(cx)).into_any_element())))
            .child(tree_label(&self.hits, &key, highlighted(format!("result-repo-name-{}", r.key), r.name.clone(), &r.name_marks, cx).flex_1()))
            .child(count(r.sessions.len(), cx))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.reveal(group.as_deref(), Some(&folder), cx)))
    }

    fn render_result_session(&mut self, entry: &str, place: &str, hit: &SessionHit, level: usize, cx: &mut Context<Self>) -> AnyElement {
        let (p, h) = (place.to_string(), hit.clone());
        let key = format!("result-session-{}{}", hit.row.session.session_id, hit.row.scope);
        self.sidebar.results.add(&key, Pick::Session(p.clone(), h.clone()));
        div()
            .id(SharedString::from(key.clone()))
            .flex_none()
            .cursor_pointer()
            .child(self.render_session(entry, place, &hit.row, Some(&key), grid::indent(level), cx))
            .on_click(cx.listener(move |this, e: &ClickEvent, _, cx| {
                if e.click_count() == 1 {
                    this.open_result(&p, &h, cx);
                }
            }))
            .into_any_element()
    }
}


#[cfg(feature = "selftest")]
impl FleetView {
    /// The result rows in order and the selected one, and whether the search box holds the keys.
    pub fn result_picks(&self) -> (Vec<String>, Option<String>, bool) {
        let picks = &self.sidebar.results;
        (picks.rows.iter().map(|(k, _)| k.to_string()).collect(), picks.selected.as_ref().map(|k| k.to_string()), picks.held)
    }
}

#[cfg(test)]
mod tests;
