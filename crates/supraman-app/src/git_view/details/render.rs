//! The commit panel's sections: the header, the message box over its height bar, the author block, the
//! counts, the file toolbar and the list.

use std::rc::Rc;

use gpui::{AnyElement, App, ClickEvent, ClipboardItem, Context, Div, FontWeight, Hsla, Render, SharedString, Window, div, prelude::*, px};
use supraman_core::time;
use supraman_ui::button;
use supraman_ui::fonts;
use supraman_ui::icon::Icon;
use supraman_ui::split::HeightBar;
use supraman_ui::stall;
use supraman_ui::tooltip::Tooltip;
use theme::ActiveTheme as _;

use super::files::{self, kind_word};
use super::{CommitPanel, FileView, MESSAGE_HEIGHT, PanelEvent, Shown, State};
use crate::git_view::colours::{Palette, blend};
use crate::git_view::graph::avatar;
use crate::git_view::load::initials;

/// The panel and the message box as steps of the theme's text over its editor background.
pub(crate) const PANEL_STEP: f32 = 0.025;
const BOX_STEP: f32 = 0.06;

fn short(sha: &str) -> SharedString {
    SharedString::from(sha.get(..7).unwrap_or(sha).to_string())
}

/// The author's date as the graph's date column says it.
pub(super) fn authored(shown: &Shown) -> String {
    time::stamp(shown.authored_ms)
}

/// A co-author's colour: one of the lanes, picked by their email so it stays theirs.
fn person_colour(email: &str) -> usize {
    email.bytes().fold(0usize, |h, b| h.wrapping_mul(31).wrapping_add(b as usize)) % supraman_gitman::LANE_COLOURS
}

pub(crate) fn placeholder(title: &str, detail: Option<&str>, cx: &App) -> AnyElement {
    let colors = cx.theme().colors().clone();
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap_1()
        .pt_8()
        .px_4()
        .child(div().text_color(colors.text).child(SharedString::from(title.to_string())))
        .children(detail.map(|d| div().text_xs().text_color(colors.text_muted).child(SharedString::from(d.to_string()))))
        .into_any_element()
}

impl CommitPanel {
    fn header(&self, shown: &Shown, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().colors().text_muted;
        let sha = shown.sha.clone();
        div().flex().flex_none().justify_center().items_baseline().gap_1().py_1p5().child(div().text_color(muted).child("commit:")).child(
            fonts::figures(button::ghost(&self.hits, "commit-sha", cx))
                .px_1()
                .rounded_sm()
                .child(short(&shown.sha))
                .tooltip(Tooltip::text("Copy the full sha"))
                .on_click(move |_: &ClickEvent, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(sha.to_string()))),
        )
    }

    fn message(&self, shown: &Shown, box_bg: Hsla, cx: &mut Context<Self>) -> Div {
        let colors = cx.theme().colors().clone();
        let text = format!("{}\n\n{}", shown.summary, shown.body).trim_end().to_string();
        let copy = button::icon(&self.hits, "commit-message-copy", Icon::Copy.svg(13., colors.text_muted), px(22.), cx)
            .tooltip(Tooltip::text("Copy the message"))
            .on_click(move |_: &ClickEvent, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(text.clone())));
        let content = div()
            .id("commit-message")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.message_scroll)
            .flex()
            .flex_col()
            .gap_2()
            .p_2()
            .pr_6()
            .child(div().text_base().font_weight(FontWeight::MEDIUM).text_color(colors.text).child(shown.summary.clone()))
            .when(!shown.body.is_empty(), |d| d.child(div().text_xs().text_color(colors.text).child(shown.body.clone())));
        let boxed = div()
            .relative()
            .size_full()
            .rounded_md()
            .border_1()
            .border_color(colors.border)
            .bg(box_bg)
            .child(content)
            .child(div().absolute().top_1().right_1().child(copy));
        let bar = HeightBar { id: "commit-message-bar".into(), min: px(48.), max: px(640.), default: MESSAGE_HEIGHT };
        bar.render(boxed, self.message_height, &self.hits, |this: &mut CommitPanel, height, cx| {
            this.message_height = height;
            cx.notify();
        }, |_: &mut CommitPanel, _| {}, cx)
        .flex_none()
        .px_2()
    }

    fn author(&self, shown: &Shown, palette: &Palette, cx: &mut Context<Self>) -> Div {
        let colors = cx.theme().colors().clone();
        let parents = shown.parents.iter().enumerate().map(|(i, sha)| {
            let full = sha.clone();
            fonts::figures(button::ghost(&self.hits, format!("commit-parent-{i}"), cx))
                .px_0p5()
                .rounded_sm()
                .text_color(colors.text)
                .child(short(sha))
                .tooltip(Tooltip::text("Select this parent in the graph"))
                .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| cx.emit(PanelEvent::Parent(full.clone()))))
        });
        let parents = div()
            .flex()
            .flex_none()
            .items_baseline()
            .gap_0p5()
            .text_xs()
            .when(!shown.parents.is_empty(), |d| d.child(div().text_color(colors.text_muted).child(if shown.parents.len() > 1 { "parents:" } else { "parent:" })))
            .children(parents);
        let name = div()
            .id("commit-author")
            .text_color(colors.text)
            .child(shown.author.clone())
            .tooltip(Tooltip::text(format!("{} <{}>", shown.author, shown.author_email)));
        let when = div().flex().gap_1().text_xs().child(div().text_color(colors.text_muted).child("authored")).child(div().text_color(colors.text).child(SharedString::from(authored(shown))));
        div()
            .flex()
            .flex_none()
            .items_start()
            .gap_2()
            .px_2()
            .pt_1()
            .child(avatar(initials(&shown.author).into(), self.colour, 32., palette))
            .child(div().flex().flex_col().flex_1().min_w_0().child(name).child(when))
            .child(parents)
    }

    fn co_authors(&self, shown: &Shown, palette: &Palette, cx: &mut Context<Self>) -> Option<Div> {
        if shown.co_authors.is_empty() {
            return None;
        }
        let muted = cx.theme().colors().text_muted;
        let people = shown.co_authors.iter().enumerate().map(|(i, p)| {
            let tip = if p.email.is_empty() { p.name.clone() } else { format!("{} <{}>", p.name, p.email) };
            div()
                .id(SharedString::from(format!("commit-coauthor-{i}")))
                .relative()
                .child(avatar(initials(&p.name).into(), person_colour(&p.email), 18., palette))
                .child(supraman_ui::hit::hit_target(&self.hits, format!("commit-coauthor-{i}")))
                .tooltip(Tooltip::text(tip))
        });
        Some(div().flex().flex_none().items_center().gap_1().px_2().text_xs().child(div().text_color(muted).child("Co-authors:")).children(people))
    }

    fn counts(&self, shown: &Shown, cx: &mut Context<Self>) -> Div {
        let parts = shown.counts.parts().into_iter().map(|(kind, n)| (kind, format!("{n} {}", kind_word(kind))));
        files::tally(parts, cx).px_2().text_color(cx.theme().colors().text)
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> Div {
        let colors = cx.theme().colors().clone();
        let sort = files::sort_button(&self.hits, "commit", self.order, |this: &mut CommitPanel, order, cx| this.set_order(order, cx), cx);
        let toggle = (!self.all_files).then(|| files::view_toggle(&self.hits, "commit", self.view, |this: &mut CommitPanel, view, cx| this.set_view(view, cx), cx));
        let all = self.all_files;
        let tick = button::checkbox(&self.hits, "commit-all-files", all, cx).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_all_files(!all, cx)));
        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_between()
            .h(px(28.))
            .px_2()
            .child(sort)
            .child(div().flex().children(toggle))
            .child(div().flex().items_center().gap_1p5().text_xs().text_color(colors.text).child(tick).child("View all files"))
    }

    fn files(&mut self, shown: &Rc<Shown>, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let colors = cx.theme().colors().clone();
        let column = div().flex().flex_col().flex_1().min_h_0().px_1();
        if !self.all_files && self.view == FileView::Path {
            let rows = shown.clone();
            let list = files::path_list("commit", &self.paths, &self.hits, shown.changes.len(), move |ix| rows.changes.get(ix).map(|c| (c.path.clone(), c.kind)), |this: &mut CommitPanel, path, cx| this.file_clicked(&path.into(), cx), cx);
            return column.child(list.flex_1().min_h_0());
        }
        let (_, expanded) = self.tree();
        let label = if expanded { "Collapse All" } else { "Expand All" };
        let expand = button::ghost(&self.hits, "commit-expand", cx)
            .flex_none()
            .px_1()
            .rounded_sm()
            .text_xs()
            .text_color(colors.text)
            .child(label)
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.expand_or_collapse(cx)));
        let filter = self.all_files.then(|| self.filter_box(window, cx));
        column.children(filter.map(|f| div().relative().flex_none().px_1().pb_1().child(f).child(supraman_ui::hit::hit_target(&self.hits, "commit-filter".into())))).child(div().flex().flex_none().pb_0p5().child(expand)).child(self.render_tree(cx))
    }

    /// The tree on show, with View all files the whole one.
    fn render_tree(&self, cx: &mut Context<Self>) -> AnyElement {
        let (tree, _) = self.tree();
        files::tree_list("commit", tree, &self.hits, |this: &mut CommitPanel, path, cx| this.file_clicked(path, cx), cx).flex_1().min_h_0().into_any_element()
    }

    fn render_shown(&mut self, shown: Rc<Shown>, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let colors = cx.theme().colors().clone();
        let palette = Palette::of(cx.theme());
        let box_bg = blend(colors.text.opacity(BOX_STEP), colors.editor_background);
        let border = colors.border_variant;
        div()
            .flex()
            .flex_col()
            .size_full()
            .gap_1p5()
            .child(self.header(&shown, cx))
            .child(self.message(&shown, box_bg, cx))
            .child(self.author(&shown, &palette, cx))
            .children(self.co_authors(&shown, &palette, cx))
            .child(self.counts(&shown, cx).pt_1())
            .child(self.toolbar(cx).border_b_1().border_color(border))
            .child(self.files(&shown, window, cx))
    }
}

impl Render for CommitPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _stall = stall::section("render: commit panel");
        let colors = cx.theme().colors().clone();
        let bg = blend(colors.text.opacity(PANEL_STEP), colors.editor_background);
        let content = match &self.state {
            State::Empty => placeholder("No commit selected", Some("Select a commit to see its message and files."), cx),
            State::Loading => placeholder("Reading the commit…", None, cx),
            State::Failed(why) => placeholder("Cannot read this commit", Some(why), cx),
            State::Shown(shown) => {
                let shown = shown.clone();
                self.render_shown(shown, window, cx).into_any_element()
            }
        };
        div().size_full().flex().flex_col().bg(bg).text_sm().child(content)
    }
}
