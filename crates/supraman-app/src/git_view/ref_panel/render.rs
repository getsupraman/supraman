//! The panel painted: the filter bar, the section headers, folders and ref rows of the tree, and the strip the
//! collapsed panel leaves. Hit keys: `refpanel-row-<path>`, `refpanel-eye-<path>`, `refpanel-filter`,
//! `refpanel-show-all`, `refpanel-collapse`, `refpanel-expand`, `refpanel-strip-<section>`.

use gpui::{AnyElement, App, ClickEvent, Context, Div, FontWeight, Hsla, IntoElement, Render, SharedString, Window, div, prelude::*, px};
use supraman_ui::button;
use supraman_ui::hit::hit_target;
use supraman_ui::icon::Icon;
use supraman_ui::matcher::highlighted;
use supraman_ui::stall;
use supraman_ui::table::row_group;
use supraman_ui::tooltip::Tooltip;
use supraman_ui::tree::{Item, LEAD, Row, TreeList};
use theme::ActiveTheme as _;

use super::{Count, Entry, Keys, LIST, RefEvent, RefPanel, STRIP, Section};
use crate::view::secondary;

const ROW: f32 = 24.;
const ICON: f32 = 12.;
const EYE: f32 = 18.;
/// A hidden ref's row.
const DIMMED: f32 = 0.45;

fn count_words(count: Count, section: Section, open: bool) -> String {
    if open && section.hides() { format!("{}/{}", count.shown, count.total) } else { count.total.to_string() }
}

impl RefPanel {
    fn count_of(&self, section: Section) -> Count {
        let at = Section::ALL.iter().position(|s| *s == section).unwrap_or(0);
        self.counts()[at]
    }

    /// The eye of a ref or folder whose refs hide: shown on the row's hover, struck and always shown while hidden.
    fn eye(&self, path: &SharedString, keys: Vec<String>, cx: &mut Context<Self>) -> AnyElement {
        let hidden = self.all_hidden(&keys);
        let (icon, tip) = if hidden { (Icon::EyeOff, "Show in the graph") } else { (Icon::Eye, "Hide from the graph") };
        button::revealed(&self.hits, format!("{LIST}-eye-{path}"), icon.svg(ICON, secondary(cx)), px(EYE), row_group(LIST), hidden, cx)
            .tooltip(Tooltip::text(tip))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.toggle_hidden(&keys, cx);
            }))
            .into_any_element()
    }

    fn header(&self, section: Section, open: bool, cx: &App) -> AnyElement {
        let muted = secondary(cx);
        div()
            .flex()
            .items_center()
            .gap_1p5()
            .w_full()
            .min_w_0()
            .child(section.icon().svg(ICON, muted))
            .child(div().flex_1().min_w_0().text_xs().font_weight(FontWeight::SEMIBOLD).text_color(muted).child(section.title()))
            .child(div().flex_none().pr_1().text_xs().text_color(muted).child(count_words(self.count_of(section), section, open)))
            .into_any_element()
    }

    fn folder(&self, row: &Row<'_, Entry, Keys>, keys: &Keys, cx: &mut Context<Self>) -> AnyElement {
        let hidden = self.all_hidden(&keys.0);
        let eye = (!keys.0.is_empty()).then(|| self.eye(row.path, keys.0.clone(), cx));
        let name = div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(cx.theme().colors().text).child(row.name.clone());
        div().flex().items_center().w_full().min_w_0().child(name.when(hidden, |d| d.opacity(DIMMED))).children(eye).into_any_element()
    }

    /// A ref's mark in the tree's lead slot: the current branch's tick, a worktree's house or tree.
    fn lead(entry: &Entry, cx: &App) -> Div {
        let slot = div().flex().flex_none().items_center().justify_center().w(px(LEAD));
        let muted = secondary(cx);
        match entry {
            Entry::Branch { current: true, .. } => slot.child(button::tick(true, cx.theme().colors().text_accent, cx).size(px(12.))),
            Entry::Detached { .. } => slot.child(Icon::GitBranch.svg(ICON, muted)),
            Entry::Worktree { main: true, .. } => slot.child(Icon::Home.svg(ICON, muted)),
            Entry::Worktree { main: false, .. } => slot.child(Icon::Worktree.svg(ICON, muted)),
            _ => slot,
        }
    }

    /// The muted words after a label: a worktree's folder, a stash's branch, a detached HEAD's commit.
    fn after(entry: &Entry) -> Option<String> {
        match entry {
            Entry::Worktree { folder, .. } => folder.clone(),
            Entry::Stash { branch, .. } => Some(format!("on: {branch}")),
            Entry::Detached { sha } => Some(sha.chars().take(7).collect()),
            _ => None,
        }
    }

    fn track(entry: &Entry) -> Option<String> {
        let Entry::Branch { ahead, behind, .. } = entry else { return None };
        let parts: Vec<String> = [(*ahead, "↑"), (*behind, "↓")].iter().filter(|(n, _)| *n > 0).map(|(n, arrow)| format!("{n}{arrow}")).collect();
        (!parts.is_empty()).then(|| parts.join(" "))
    }

    fn file(&self, row: &Row<'_, Entry, Keys>, entry: &Entry, cx: &mut Context<Self>) -> AnyElement {
        let (muted, text) = (secondary(cx), cx.theme().colors().text);
        let keys: Vec<String> = entry.hide_key().into_iter().collect();
        let hidden = self.all_hidden(&keys);
        let mut label = highlighted(format!("{LIST}-name-{}", row.path), entry.label(), &entry.marks(&self.filter()), cx).flex_shrink(1.).text_color(text);
        if let Entry::Stash { label: words, branch, .. } = entry {
            label = label.tooltip(Tooltip::text(format!("{words} on: {branch}")));
        }
        let after = Self::after(entry).map(|w| div().flex_none().max_w(px(120.)).overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(muted).child(w));
        let content = div().flex().flex_1().min_w_0().items_center().gap_1().child(label).children(after);
        let track = Self::track(entry).map(|t| div().flex_none().px_1().text_xs().text_color(muted).child(t));
        let eye = (!keys.is_empty()).then(|| self.eye(row.path, keys, cx));
        div()
            .flex()
            .items_center()
            .w_full()
            .min_w_0()
            .child(Self::lead(entry, cx))
            .child(content.when(hidden, |d| d.opacity(DIMMED)))
            .children(track)
            .children(eye)
            .into_any_element()
    }

    fn paint(&mut self, row: Row<'_, Entry, Keys>, cx: &mut Context<Self>) -> AnyElement {
        match row.item {
            Item::Folder { open, .. } if row.depth == 0 => match Section::of(row.path) {
                Some(section) => self.header(section, open, cx),
                None => div().into_any_element(),
            },
            Item::Folder { aggregate, .. } => self.folder(&row, aggregate, cx),
            Item::File(entry) => self.file(&row, entry, cx),
        }
    }

    fn bar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let input = self.filter_box(window, cx);
        if std::mem::take(&mut self.focus_filter) {
            input.update(cx, |i, cx| i.focus_all(window, cx));
        }
        let show_all = (!self.hidden.is_empty()).then(|| {
            button::ghost(&self.hits, format!("{LIST}-show-all"), cx)
                .flex_none()
                .px_1()
                .rounded_sm()
                .text_xs()
                .text_color(cx.theme().colors().text_accent)
                .child("Show all")
                .tooltip(Tooltip::text("Show every hidden ref in the graph"))
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.show_all(cx)))
        });
        let collapse = button::icon(&self.hits, format!("{LIST}-collapse"), Icon::ChevronLeft.svg(14., secondary(cx)), px(22.), cx)
            .tooltip(Tooltip::text("Collapse the ref panel (Ctrl+J)"))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_shut(cx)));
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .px_1()
            .py_1()
            .child(div().relative().flex_1().min_w_0().child(input).child(hit_target(&self.hits, format!("{LIST}-filter"))))
            .children(show_all)
            .child(collapse)
    }

    fn strip(&self, cx: &mut Context<Self>) -> Div {
        let (accent, muted, border) = (cx.theme().colors().text_accent, secondary(cx), cx.theme().colors().border);
        let expand = button::icon(&self.hits, format!("{LIST}-expand"), Icon::ChevronRight.svg(14., muted), px(24.), cx)
            .rounded_full()
            .border_1()
            .border_color(border)
            .tooltip(Tooltip::text("Expand the ref panel (Ctrl+J)"))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_shut(cx)));
        let counts = self.counts();
        let sections = Section::ALL.into_iter().zip(counts).map(|(section, count)| {
            button::ghost(&self.hits, format!("{LIST}-strip-{}", section.key()), cx)
                .flex()
                .flex_col()
                .items_center()
                .gap_0p5()
                .w(px(30.))
                .py_1()
                .rounded_sm()
                .child(section.icon().svg(14., muted))
                .child(div().text_xs().text_color(accent).child(count.total.to_string()))
                .tooltip(Tooltip::text(section.title()))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.open_section(section, cx)))
        });
        div().flex().flex_col().items_center().gap_2().pt_1().w(STRIP).h_full().child(expand).children(sections)
    }

    fn list(&self, cx: &mut Context<Self>) -> AnyElement {
        TreeList::new(LIST, &self.tree, &self.hits, |this: &mut RefPanel, row, _, cx| this.paint(row, cx))
            .row_height(px(ROW))
            .on_select(|this, path, _, cx| this.picked(path, cx))
            .on_toggle(|_, _, cx| cx.emit(RefEvent::Layout))
            .render(cx)
            .flex_1()
            .min_h_0()
            .into_any_element()
    }
}

impl Render for RefPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _stall = stall::section("render: ref panel");
        let bg: Hsla = cx.theme().colors().panel_background;
        if self.shut {
            return div().h_full().bg(bg).text_sm().child(self.strip(cx));
        }
        let bar = self.bar(window, cx);
        div().size_full().flex().flex_col().bg(bg).text_sm().child(bar).child(self.list(cx))
    }
}
