//! The panel painted; hit keys `refpanel-{row,name,mark}-<path>`, `-filter`, `-show-all`, `-collapse`, `-expand`, `-strip-<section>`, `-sections-divider-<pane>`.

use gpui::{AnyElement, App, ClickEvent, Context, Div, FontWeight, Hsla, IntoElement, Render, Window, div, prelude::*, px};
use supraman_gitman::short_sha;
use supraman_ui::button;
use supraman_ui::hit::hit_target;
use supraman_ui::icon::Icon;
use supraman_ui::ink;
use supraman_ui::matcher::highlighted;
use supraman_ui::split::{Pane, Stack};
use supraman_ui::stall;
use supraman_ui::tooltip::Tooltip;
use supraman_ui::tree::{Item, LEAD, Row, TreeList};
use theme::ActiveTheme as _;

use super::marks::MARK;
use super::{Count, Entry, Keys, LIST, RefEvent, RefPanel, STRIP, Section, Walk};
use supraman_ui::themes::secondary;

pub(super) const ROW: f32 = 24.;
const ICON: f32 = 12.;
/// The row of a ref the graph does not walk.
const DIMMED: f32 = 0.45;

fn count_words(count: Count, section: Section, open: bool) -> String {
    if open && section.hides() { format!("{}/{}", count.shown, count.total) } else { count.total.to_string() }
}

impl RefPanel {
    fn count_of(&self, section: Section) -> Count {
        let at = Section::ALL.iter().position(|s| *s == section).unwrap_or(0);
        self.counts()[at]
    }

    /// Whether the graph walks none of `keys`; never for none.
    fn out_of_walk(&self, keys: &[String]) -> bool {
        !keys.is_empty() && keys.iter().all(|k| !self.walk.walks(k))
    }

    pub(super) fn header(&self, section: Section, open: bool, cx: &App) -> AnyElement {
        let muted = secondary(cx.theme().colors());
        div()
            .flex()
            .items_center()
            .gap_1p5()
            .w_full()
            .min_w_0()
            .child(ink::mark(section.icon().svg(ICON, muted)))
            .child(div().flex_1().min_w_0().text_xs().font_weight(FontWeight::SEMIBOLD).text_color(muted).child(section.title()))
            .child(div().flex_none().pr_1().text_xs().text_color(muted).child(count_words(self.count_of(section), section, open)))
            .into_any_element()
    }

    fn folder(&self, row: &Row<'_, Entry, Keys>, keys: &Keys, cx: &mut Context<Self>) -> Div {
        let dimmed = self.out_of_walk(&keys.hide);
        let name = div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(cx.theme().colors().text).child(row.name.clone());
        div().flex().items_center().w_full().min_w_0().child(name.when(dimmed, |d| d.opacity(DIMMED)))
    }

    /// A ref's icon in the tree's lead slot: a detached HEAD's branch, the main worktree's house, a linked one's tree.
    pub(crate) fn lead_icon(entry: &Entry) -> Option<Icon> {
        match entry {
            Entry::Detached { .. } => Some(Icon::GitBranch),
            Entry::Worktree { main: true, .. } => Some(Icon::Home),
            Entry::Worktree { main: false, .. } => Some(Icon::Worktree),
            _ => None,
        }
    }

    pub(super) fn lead(row: Row<'_, Entry, Keys>, cx: &App) -> Option<AnyElement> {
        let Item::File(entry) = row.item else { return None };
        Some(Self::lead_icon(entry)?.svg(ICON, secondary(cx.theme().colors())).into_any_element())
    }

    /// The muted words after a label: a worktree's folder, a stash's branch, a detached HEAD's commit.
    fn after(entry: &Entry) -> Option<String> {
        match entry {
            Entry::Worktree { folder, .. } => folder.clone(),
            Entry::Stash { branch, .. } => Some(format!("on: {branch}")),
            Entry::Detached { sha } => Some(short_sha(sha).to_string()),
            _ => None,
        }
    }

    fn track(entry: &Entry) -> Option<String> {
        let Entry::Branch { ahead, behind, .. } = entry else { return None };
        Some(super::track(*ahead, *behind)).filter(|t| !t.is_empty())
    }

    fn file(&self, list: &str, row: &Row<'_, Entry, Keys>, entry: &Entry, cx: &mut Context<Self>) -> Div {
        let (muted, text) = (secondary(cx.theme().colors()), cx.theme().colors().text);
        let dimmed = !entry.walked(&self.walk);
        let key = format!("{list}-name-{}", row.path);
        let mut label = highlighted(key.clone(), entry.label(), &entry.marks(&self.filter()), cx).relative().flex_shrink(1.).text_color(text).child(hit_target(&self.hits, key));
        if let Entry::Stash { label: words, branch, .. } = entry {
            label = label.tooltip(Tooltip::text(format!("{words} on: {branch}")));
        }
        let after = Self::after(entry).map(|w| div().flex_none().max_w(px(120.)).overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(muted).child(w));
        let content = div().flex().flex_1().min_w_0().items_center().gap_1().child(label).children(after);
        let track = Self::track(entry).map(|t| div().flex_none().px_1().text_xs().text_color(muted).child(t).into_any_element());
        let track = track.or_else(|| matches!(entry, Entry::Worktree { .. }).then(|| self.summary_element(list, row.path, &self.whole(list, row.path), cx)).flatten());
        div()
            .flex()
            .items_center()
            .w_full()
            .min_w_0()
            .child(content.when(dimmed, |d| d.opacity(DIMMED)))
            .children(track)
    }

    /// A row of `list`: a section's pane in the panel, or the overlay's.
    pub(super) fn paint(&mut self, list: &str, row: Row<'_, Entry, Keys>, cx: &mut Context<Self>) -> AnyElement {
        let content = match row.item {
            Item::Folder { aggregate, .. } => self.folder(&row, aggregate, cx),
            Item::File(entry) => self.file(list, &row, entry, cx),
        };
        self.interactive(list, row.path, content, cx)
    }

    fn bar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let input = self.filter_box(window, cx);
        if std::mem::take(&mut self.focus_filter) {
            input.update(cx, |i, cx| i.focus_all(window, cx));
        }
        let show_all = (self.walk != Walk::default()).then(|| {
            button::ghost(&self.hits, format!("{LIST}-show-all"), cx)
                .flex_none()
                .px_1()
                .rounded_sm()
                .text_xs()
                .text_color(cx.theme().colors().text_accent)
                .child("Show all")
                .tooltip(Tooltip::text("Show every ref in the graph: none hidden, none soloed"))
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.show_all(cx)))
        });
        let collapse = button::icon(&self.hits, format!("{LIST}-collapse"), Icon::ChevronLeft.svg(14., secondary(cx.theme().colors())), px(22.), cx)
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
        let open = self.overlay();
        let selected = cx.theme().colors().element_selected;
        let (accent, muted, border) = (cx.theme().colors().text_accent, secondary(cx.theme().colors()), cx.theme().colors().border);
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
                .when(open == Some(section), |b| b.bg(selected))
                .child(section.icon().svg(14., muted))
                .child(div().text_xs().text_color(accent).child(count.total.to_string()))
                .tooltip(Tooltip::text(section.title()))
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| this.toggle_overlay(section, window, cx)))
        });
        div().flex().flex_col().items_center().gap_2().pt_1().w(STRIP).h_full().child(expand).children(sections)
    }

    /// A section's header, under `refpanel-row-<section>`: a click opens or closes it.
    fn pane_header(&self, section: Section, open: bool, cx: &mut Context<Self>) -> AnyElement {
        let chevron = if open { Icon::ChevronDown } else { Icon::ChevronRight };
        let key = section.key();
        button::ghost(&self.hits, format!("{LIST}-row-{key}"), cx)
            .flex()
            .items_center()
            .h(px(ROW))
            .px_2()
            .border_t_1()
            .border_color(cx.theme().colors().border_variant)
            .child(ink::mark(div().flex().flex_none().items_center().justify_center().w(px(LEAD)).child(chevron.svg(ICON, secondary(cx.theme().colors())))))
            .child(self.header(section, open, cx))
            .children((section == Section::Worktrees).then(|| self.refresh_wips(cx)).flatten())
            .on_mouse_down(gpui::MouseButton::Right, cx.listener(move |this, e: &gpui::MouseDownEvent, _, cx| {
                cx.stop_propagation();
                this.open_section_menu(section, e.position, cx);
            }))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.tree.toggle(key);
                cx.emit(RefEvent::Layout);
                cx.notify();
            }))
            .into_any_element()
    }

    fn section_list(&self, section: Section, cx: &mut Context<Self>) -> AnyElement {
        TreeList::new(LIST, self.tree.get(section), &self.hits, |this: &mut RefPanel, row, _, cx| this.paint(LIST, row, cx))
            .row_height(px(ROW))
            .mark(px(MARK), |this, row, _, cx| this.paint_mark(LIST, row, cx))
            .lead(|_, row, _, cx| Self::lead(row, cx))
            .on_select(|this, path, _, cx| {
                this.picked(path, cx);
            })
            .on_toggle(|_, _, cx| cx.emit(RefEvent::Layout))
            .render(cx)
            .size_full()
            .into_any_element()
    }

    /// The sections the filter keeps, stacked: a closed one its header, the open ones sharing the rest by kept heights, else rows.
    fn list(&self, cx: &mut Context<Self>) -> AnyElement {
        let shown: Vec<Section> = Section::ALL.into_iter().filter(|s| self.tree.shows(*s)).collect();
        let at = |section: Section| Section::ALL.iter().position(|s| *s == section).unwrap_or(0);
        let panes = shown
            .iter()
            .map(|section| {
                let open = self.tree.is_open(section.key());
                let kept = self.heights[at(*section)];
                let height = if kept > px(0.) { kept } else { px(ROW * (1 + self.tree.get(*section).row_count()) as f32) };
                Pane { header: self.pane_header(*section, open, cx), body: open.then(|| self.section_list(*section, cx)), height }
            })
            .collect();
        let stack = Stack { id: format!("{LIST}-sections").into(), min: px(ROW * 3.) };
        let resized = shown.clone();
        stack
            .render(&self.stack, panes, &self.hits, move |this: &mut RefPanel, heights, cx| {
                for (section, height) in resized.iter().zip(heights) {
                    this.heights[at(*section)] = height;
                }
                cx.notify();
            }, |_: &mut RefPanel, cx| cx.emit(RefEvent::Layout), cx)
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
            let overlay = self.render_overlay(cx);
            let menu = self.render_menu(cx);
            return div().relative().h_full().bg(bg).text_sm().child(self.strip(cx)).children(overlay).children(menu);
        }
        let bar = self.bar(window, cx);
        let menu = self.render_menu(cx);
        div().size_full().flex().flex_col().bg(bg).text_sm().child(bar).child(self.list(cx)).children(menu)
    }
}
