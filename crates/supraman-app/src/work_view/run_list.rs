//! The middle column's run page: breadcrumb, title, how much is done, the prompt chips, one square per item, the
//! filter, and a row per item: its state's dot, its run-relative ref, its title and its criteria count.

use gpui::{ClickEvent, FontWeight};
use supraman_ui::clipped::clamped;
use supraman_ui::hit::hit_target;
use supraman_ui::icon::Icon;
use supraman_ui::marks::{dot, squares};
use supraman_ui::segmented::{Segment, segmented};
use supraman_ui::{button, fonts};
use supraman_workman::BoardRow;

use super::actions::COPY_SIDE;
use super::state::{FILTERS, StateKey};
use super::*;

pub const NOTHING_MATCHES: &str = "Nothing matches this filter.";

pub fn dot_key(qualified_ref: &str, key: StateKey) -> String {
    format!("work-dot-{qualified_ref}={}", key.as_str())
}

pub fn done_line(done: usize, total: usize) -> String {
    format!("{done} of {total} done.")
}

/// Every text a row paints, in order; the row paints nothing else.
pub fn row_texts(row: &BoardRow) -> Vec<String> {
    let mut out = vec![row.item.ref_.clone(), row.item.title.clone()];
    if row.criteria.total > 0 {
        out.push(format!("{}/{}", row.criteria.checked, row.criteria.total));
    }
    out
}

pub fn filter_labels(board: &Board) -> Vec<String> {
    FILTERS.iter().map(|(f, _, label)| format!("{label} {}", state::listed(board, *f).len())).collect()
}

impl WorkView {
    pub(super) fn render_run_page(&self, run: &str, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let root = div().id("work-list").size_full().flex().flex_col().gap_2().px_5().pt_4().pb_6().overflow_y_scroll().track_scroll(&self.list_scroll);
        let board = match self.boards.get(run) {
            Some(BoardState::Ready(board)) => board,
            Some(BoardState::Failed(message)) => return root.child(div().text_color(cx.theme().status().error).child(SharedString::from(message.clone()))).into_any_element(),
            _ => return root.child(div().text_color(colors.text_muted).child("Reading the run…")).into_any_element(),
        };
        let project = board.project.as_ref().map(|p| p.title.clone()).unwrap_or_default();
        let crumb = div()
            .flex()
            .items_center()
            .gap_1()
            .text_xs()
            .text_color(colors.text_muted)
            .when(!project.is_empty(), |d| d.child(SharedString::from(format!("{project} /"))))
            .child(div().font_family(fonts::MONO).child(SharedString::from(board.run.ref_.clone())))
            .child(self.ref_actions("work-crumb", &board.run, cx));
        let title = div().text_xl().font_weight(FontWeight::SEMIBOLD).whitespace_normal().child(SharedString::from(board.run.title.clone()));
        let (keys, done) = state::tally(board);
        let marks: Vec<_> = keys.iter().map(|k| k.mark(cx)).collect();
        let segments: Vec<Segment> = FILTERS.iter().zip(filter_labels(board)).map(|((_, key, _), label)| Segment::new(*key).label(label)).collect();
        let chosen = FILTERS.iter().position(|(f, _, _)| *f == self.filter).unwrap_or(0);
        let filter = segmented(&self.hits, "work-filter", &segments, chosen, |this: &mut WorkView, i, _, cx| this.set_filter(FILTERS[i].0, cx), cx);
        let rows = state::listed(board, self.filter);
        let mut list = div().flex().flex_col().rounded_md().border_1().border_color(colors.border_variant).py_1();
        if rows.is_empty() {
            list = list.child(div().relative().px_3().py_2().text_sm().text_color(colors.text_muted).child(NOTHING_MATCHES).child(hit_target(&self.hits, "work-list-empty".into())));
        }
        for row in rows {
            list = list.child(self.render_row(row, cx));
        }
        root.child(crumb)
            .child(title)
            .child(div().text_sm().text_color(colors.text_muted).child(SharedString::from(done_line(done, keys.len()))))
            .children(self.render_prompts(run, "run", cx))
            .child(div().relative().child(squares(&marks)).child(hit_target(&self.hits, "work-squares".into())))
            .child(filter)
            .child(list)
            .into_any_element()
    }

    fn render_row(&self, row: &BoardRow, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let item = &row.item;
        let key = format!("work-row-{}", item.qualified_ref);
        let selected = self.selected.as_deref() == Some(item.id.as_str());
        let state = state::of_row(row);
        let indent = px(12. + 16. * row.depth.saturating_sub(1) as f32);
        let (id, copied) = (item.id.clone(), item.clone());
        let mut texts = row_texts(row).into_iter();
        let (ref_, title, criteria) = (texts.next().unwrap_or_default(), texts.next().unwrap_or_default(), texts.next());
        let copy = button::revealed(&self.hits, format!("work-row-copy-{}", item.qualified_ref), Icon::Copy.svg(11., colors.text_muted), px(COPY_SIDE), key.clone(), selected, cx)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.copy_ref(&copied, cx);
            }));
        div()
            .id(SharedString::from(key.clone()))
            .group(SharedString::from(key.clone()))
            .relative()
            .flex()
            .items_start()
            .gap_2()
            .pl(indent)
            .pr_3()
            .py(px(4.))
            .text_sm()
            .cursor_pointer()
            .when(selected, |d| d.bg(colors.element_selected))
            .when(!selected, |d| d.hover(|s| s.bg(colors.ghost_element_hover)))
            .child(div().relative().flex_none().h(px(COPY_SIDE)).flex().items_center().child(dot(state.key.mark(cx), 8.)).child(hit_target(&self.hits, dot_key(&item.qualified_ref, state.key))))
            .child(div().flex_none().h(px(COPY_SIDE)).flex().items_center().font_family(fonts::MONO).text_xs().text_color(colors.text_muted).child(SharedString::from(ref_)))
            .child(copy)
            .child(div().flex_1().min_w_0().pt(px(1.)).child(clamped(format!("work-row-title-{}", item.qualified_ref), title, 2)))
            .when_some(criteria, |d, criteria| {
                d.child(fonts::figures(div().flex_none().h(px(COPY_SIDE)).flex().items_center().text_xs().text_color(colors.text_muted)).child(SharedString::from(criteria)))
            })
            .child(hit_target(&self.hits, key))
            .on_mouse_down(gpui::MouseButton::Right, self.on_row_mouse_down(item, cx))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                window.focus(&this.focus, cx);
                this.detail_focused = false;
                this.select(id.clone(), cx);
            }))
            .into_any_element()
    }
}
