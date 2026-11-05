//! The WIP panel's parts: the header with the branch, the sort and Path | Tree toolbar, the two collapsible
//! lists, and the read-only line where a commit box would be.

use std::rc::Rc;

use gpui::{AnyElement, ClickEvent, Context, Div, Render, SharedString, Window, div, prelude::*, px};
use supraman_ui::banner;
use supraman_ui::button;
use supraman_ui::hit::HitTargets;
use supraman_ui::icon::Icon;
use supraman_ui::stall;
use theme::ActiveTheme as _;

use super::{Branch, Section, Shown, State, WipPanel, file_changes};
use crate::git_view::GitView;
use crate::git_view::cells::pill_box;
use crate::git_view::colours::{Palette, blend};
use crate::git_view::details::{FileView, PANEL_STEP, files, placeholder};

const READ_ONLY: &str = "The Git tab only reads for now: stage, discard and commit in your own git tool.";

impl WipPanel {
    fn header(&self, shown: &Shown, cx: &mut Context<Self>) -> Div {
        let palette = Palette::of(cx.theme());
        let name = match &shown.branch {
            Branch::Named(name) => Some(name.clone()),
            Branch::Detached(sha) => sha.clone(),
        };
        let pill = name.map(|n| pill_box(self.colour, &palette).id("wip-branch").child(SharedString::from(n)));
        div()
            .flex()
            .flex_none()
            .justify_center()
            .items_center()
            .gap_1p5()
            .py_1p5()
            .px_2()
            .text_color(palette.text)
            .child(div().id("wip-header").child(Self::header_words(shown)))
            .children(pill)
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
                files::path_list(id, &list.paths, &self.hits, shown.files(section).len(), file, picked, cx)
            }
            FileView::Tree => files::tree_list(id, &list.tree, &self.hits, move |this: &mut WipPanel, path, cx| picked(this, path.to_string(), cx), cx),
        };
        element.flex_1().min_h_0().into_any_element()
    }

    fn section(&self, section: Section, shown: &Rc<Shown>, cx: &mut Context<Self>) -> Div {
        let colors = cx.theme().colors().clone();
        let open = self.list(section).open;
        let chevron = if open { Icon::ChevronDown } else { Icon::ChevronRight };
        let title = format!("{} ({})", section.title(), shown.files(section).len());
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
        let column = div().flex().flex_col().min_h_0().px_1().child(header).child(rule);
        if open { column.flex_1().child(self.file_list(section, shown, cx)) } else { column.flex_none() }
    }

    fn render_shown(&self, shown: Rc<Shown>, cx: &mut Context<Self>) -> Div {
        let colors = cx.theme().colors().clone();
        div()
            .flex()
            .flex_col()
            .size_full()
            .gap_1()
            .child(self.header(&shown, cx))
            .child(self.toolbar(cx).border_b_1().border_color(colors.border_variant))
            .child(self.section(Section::Unstaged, &shown, cx))
            .child(self.section(Section::Staged, &shown, cx))
            .child(div().id("wip-read-only").flex_none().px_2().py_2().border_t_1().border_color(colors.border_variant).text_xs().text_color(colors.text_muted).child(READ_ONLY))
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
