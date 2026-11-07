//! The WIP panel's parts: the header with Discard all and the branch (the conflict header while an operation is
//! under way), the sort and Path | Tree toolbar, the collapsible lists with their buttons, and the commit area.

use std::rc::Rc;

use gpui::{AnyElement, ClickEvent, Context, Div, Render, SharedString, Stateful, Window, div, prelude::*, px};
use supraman_gitman::{ChangeKind, FileChange};
use supraman_ui::banner;
use supraman_ui::button;
use supraman_ui::hit::HitTargets;
use supraman_ui::icon::Icon;
use supraman_ui::stall;
use supraman_ui::tooltip::Tooltip;
use theme::ActiveTheme as _;

use super::{Ask, Branch, Section, Shown, State, WipPanel, file_changes};
use crate::git_view::GitView;
use crate::git_view::commit::files::{FileClick, Listed};
use crate::git_view::cells::pill_box;
use crate::git_view::colours::{Palette, blend};
use crate::git_view::details::{FileView, PANEL_STEP, PanelEvent, files, placeholder};
use crate::git_view::keys::{StageSelected, UnstageSelected, WIP_LIST_CONTEXT};

/// `button` live while no write waits or runs, else dimmed with why.
pub(super) fn live(button: Stateful<Div>, busy: Option<&'static str>, cx: &mut Context<WipPanel>, run: impl Fn(&mut WipPanel, &mut Context<WipPanel>) + 'static) -> Stateful<Div> {
    match busy {
        Some(why) => button::dimmed(button).tooltip(Tooltip::text(why)),
        None => button.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            cx.stop_propagation();
            run(this, cx);
        })),
    }
}

impl WipPanel {
    /// The red trash button: Discard all changes, asked first.
    fn discard_all_button(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let error = cx.theme().status().error;
        let trash = button::icon(&self.hits, "wip-discard-all", Icon::Trash.svg(14., error), px(24.), cx).border_1().border_color(error).bg(banner::tint(error));
        let busy = self.busy(cx);
        let trash = if busy.is_none() { trash.tooltip(Tooltip::text("Discard all changes")) } else { trash };
        live(trash, busy, cx, |this, cx| this.ask(Ask::DiscardAll, cx))
    }

    fn header(&self, shown: &Shown, cx: &mut Context<Self>) -> Div {
        let palette = Palette::of(cx.theme());
        let name = match &shown.branch {
            Branch::Named(name) => Some(name.clone()),
            Branch::Detached(sha) => sha.clone(),
        };
        let pill = name.map(|n| pill_box(self.colour, &palette).id("wip-branch").child(SharedString::from(n)));
        let words = div().flex_1().flex().justify_center().items_center().gap_1p5().min_w_0().child(div().id("wip-header").child(Self::header_words(shown))).children(pill);
        div().flex().flex_none().items_center().py_1p5().px_2().text_color(palette.text).child(self.discard_all_button(cx)).child(words).child(div().w(px(24.)))
    }

    /// A row's buttons on hover: stash, discard and Stage File on an unstaged file, stash and Unstage File (Mark
    /// Conflicted while an operation is under way) on a staged one; a conflicted file's are the conflict panel's.
    fn row_tail(section: Section) -> files::Tail<WipPanel> {
        Rc::new(move |this: &mut WipPanel, path: &str, kind: ChangeKind, group: SharedString, cx: &mut Context<WipPanel>| {
            if section == Section::Conflicted {
                return this.conflicted_tail(path, group, cx).into_any_element();
            }
            let (hits, busy, file) = (this.hits.clone(), this.busy(cx), this.shown().and_then(|s| s.files(section).iter().find(|f| f.path == path).cloned()));
            let file = file.unwrap_or_else(|| FileChange { kind, path: path.to_string(), old_path: None, similarity: None });
            let muted = cx.theme().colors().text_muted;
            let stash = button::icon(&hits, format!("wip-stash-{path}"), Icon::Stash.svg(12., muted), px(20.), cx).tooltip(Tooltip::text("Stash file"));
            let stashed = file.clone();
            let stash = live(stash, busy, cx, move |_, cx| cx.emit(PanelEvent::StashFiles(vec![stashed.clone()])));
            let row = div().flex().items_center().gap_1().child(button::reveal(stash, group.clone(), false));
            match section {
                Section::Unstaged => {
                    let discard = button::icon(&hits, format!("wip-discard-{path}"), Icon::Trash.svg(12., muted), px(20.), cx).tooltip(Tooltip::text("Discard changes"));
                    let (gone, staged) = (file.clone(), vec![file]);
                    let discard = live(discard, busy, cx, move |this, cx| this.ask(Ask::Discard(vec![gone.clone()]), cx));
                    let stage = live(button::outline(&hits, format!("wip-stage-{path}"), "Stage File", cx.theme().status().created, cx), busy, cx, move |this, cx| this.stage(staged.clone(), cx));
                    row.child(button::reveal(discard, group.clone(), false)).child(button::reveal(stage, group, false))
                }
                Section::Conflicted => row,
                Section::Staged => {
                    if let Some(unresolve) = this.resolved_tail(path, group.clone(), cx) {
                        return row.child(unresolve).into_any_element();
                    }
                    let unstaged = vec![file];
                    let unstage = live(button::outline(&hits, format!("wip-unstage-{path}"), "Unstage File", cx.theme().status().error, cx), busy, cx, move |this, cx| this.unstage(unstaged.clone(), cx));
                    row.child(button::reveal(unstage, group, false))
                }
            }
            .into_any_element()
        })
    }

    /// A row's right-click: the tab's file menu of `section`'s file or folder.
    fn row_menu(section: Section) -> files::OnMenu<WipPanel> {
        Rc::new(move |_: &mut WipPanel, path: &str, folder: bool, position, cx: &mut Context<WipPanel>| {
            cx.emit(PanelEvent::FileMenu(FileClick { from: Listed::Wip(section), path: path.to_string(), folder, position }));
        })
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> Div {
        let sort = files::sort_button(&self.hits, "wip", self.order, |this: &mut WipPanel, order, cx| this.set_order(order, cx), cx);
        let toggle = files::view_toggle(&self.hits, "wip", self.view, |this: &mut WipPanel, view, cx| this.set_view(view, cx), cx);
        div().flex().flex_none().items_center().h(px(28.)).px_2().child(sort).child(div().flex_1().flex().justify_center().child(toggle)).child(div().w(px(24.)))
    }

    fn file_list(&self, section: Section, shown: &Rc<Shown>, cx: &mut Context<Self>) -> AnyElement {
        let id = section.list();
        let picked = move |this: &mut WipPanel, path: String, cx: &mut Context<WipPanel>| this.file_clicked(section, &path, cx);
        let list = self.list(section);
        let element = match self.view {
            FileView::Path => {
                let rows = shown.clone();
                let file = move |ix: usize| rows.files(section).get(ix).map(|c| (c.path.clone(), c.kind));
                files::path_list(id, &list.paths, &self.hits, shown.files(section).len(), file, picked, Some(Self::row_tail(section)), Some(Self::row_menu(section)), cx)
            }
            FileView::Tree => files::tree_list(id, &list.tree, &self.hits, move |this: &mut WipPanel, path, cx| picked(this, path.to_string(), cx), Some(Self::row_tail(section)), Some(Self::row_menu(section)), cx),
        };
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .key_context(WIP_LIST_CONTEXT)
            .on_action(cx.listener(|this, _: &StageSelected, _, cx| this.stage_selected(cx)))
            .on_action(cx.listener(|this, _: &UnstageSelected, _, cx| this.unstage_selected(cx)))
            .child(element.flex_1().min_h_0())
            .into_any_element()
    }

    /// Stage All Changes over the unstaged files, Unstage All Changes over the staged.
    fn all_button(&self, section: Section, cx: &mut Context<Self>) -> Stateful<Div> {
        let status = cx.theme().status().clone();
        let busy = self.busy(cx);
        match section {
            Section::Conflicted => self.resolve_all_button(cx),
            Section::Unstaged => live(button::outline(&self.hits, "wip-stage-all", "Stage All Changes", status.created, cx), busy, cx, |this, cx| this.stage_all(cx)),
            Section::Staged => live(button::outline(&self.hits, "wip-unstage-all", "Unstage All Changes", status.error, cx), busy, cx, |this, cx| this.unstage_all(cx)),
        }
    }

    fn section(&self, section: Section, shown: &Rc<Shown>, cx: &mut Context<Self>) -> Div {
        let colors = cx.theme().colors().clone();
        let open = self.list(section).open;
        let chevron = if open { Icon::ChevronDown } else { Icon::ChevronRight };
        let title = format!("{} ({})", section.title(shown.conflicting()), shown.files(section).len());
        let header = button::ghost(&self.hits, format!("{}-header", section.list()), cx)
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .h(px(26.))
            .px_1()
            .rounded_sm()
            .text_color(colors.text)
            .child(chevron.svg(12., colors.text_muted))
            .child(SharedString::from(title))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle(section, cx)));
        let rule = div().flex_none().h(px(1.)).bg(colors.border_variant);
        let all = (!shown.files(section).is_empty()).then(|| self.all_button(section, cx));
        let header = div().flex().flex_none().items_center().justify_between().child(header).children(all);
        let column = div().flex().flex_col().min_h_0().px_1().child(header).child(rule);
        if open { column.flex_1().child(self.file_list(section, shown, cx)) } else { column.flex_none() }
    }

    fn render_shown(&self, shown: Rc<Shown>, cx: &mut Context<Self>) -> Div {
        let colors = cx.theme().colors().clone();
        let asking = self.asking.as_ref().map(|asked| asked.render(&self.hits, "wip-confirm", |this: &mut WipPanel, yes, _, cx| this.answer(yes, cx), cx).mx_1());
        let header = if shown.conflicting() { self.conflict_header(&shown, cx) } else { self.header(&shown, cx) };
        let sections: Vec<Div> = shown
            .sections()
            .iter()
            .filter(|s| !shown.conflicting() || **s != Section::Unstaged || !shown.unstaged.is_empty())
            .map(|s| self.section(*s, &shown, cx))
            .collect();
        div()
            .flex()
            .flex_col()
            .size_full()
            .gap_1()
            .child(header)
            .child(self.toolbar(cx).border_b_1().border_color(colors.border_variant))
            .children(sections)
            .children(asking)
            .children(self.commit_area(&shown, cx))
    }
}

impl Render for WipPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _stall = stall::section("render: wip panel");
        let colors = cx.theme().colors().clone();
        let bg = blend(colors.text.opacity(PANEL_STEP), colors.editor_background);
        let content = match &self.state {
            State::Loading => placeholder("Reading the working directory…", None, cx),
            State::Failed(why) => placeholder("Cannot read the working directory", Some(why), cx),
            State::Shown(shown) => {
                let shown = shown.clone();
                self.render_shown(shown, cx).into_any_element()
            }
        };
        div().size_full().flex().flex_col().bg(bg).text_sm().child(content)
    }
}

/// Over a commit's panel while the tree is dirty: `N file changes in working directory` and View Change
/// (`wip-banner-view-change`), which selects the WIP row.
pub fn banner(total: usize, hits: &HitTargets, cx: &mut Context<GitView>) -> Div {
    let info = cx.theme().status().info;
    let view = button::outline(hits, "wip-banner-view-change", "View Change", info, cx).on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.select_wip(cx)));
    banner::tinted(info)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap_2()
        .m_1()
        .py_1()
        .child(div().id("wip-banner").child(SharedString::from(format!("{} in working directory", file_changes(total)))))
        .child(view)
}
