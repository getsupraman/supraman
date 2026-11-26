//! The middle column's run page: breadcrumb, done line, prompt chips, squares, filter and a row per item.

use gpui::{ClickEvent, FontWeight};
use supraman_ui::placeholder::placeholder;
use supraman_ui::clipped::clamped;
use supraman_ui::hit::hit_target;
use supraman_ui::icon::Icon;
use supraman_ui::marks::{dot, squares};
use supraman_ui::segmented::{Segment, segmented};
use supraman_ui::tooltip::Tooltip;
use supraman_ui::label::{Label, LabelMark};
use supraman_ui::{button, fonts, tokens};
use supraman_workman::BoardRow;

use super::actions::COPY_SIDE;
use super::state::{FILTERS, StateKey};
use super::*;

pub(crate) const NOTHING_MATCHES: &str = "Nothing matches this filter.";

pub(crate) fn dot_key(qualified_ref: &str, key: StateKey) -> String {
    format!("work-dot-{qualified_ref}={}", key.as_str())
}

pub(crate) fn done_line(done: usize, total: usize) -> String {
    format!("{done} of {total} done.")
}

/// The breadcrumb's first part: the run's project's ref, and its title as the tooltip.
pub(crate) fn crumb_project(board: &Board) -> Option<(String, String)> {
    board.project.as_ref().filter(|p| !p.full().is_empty()).map(|p| (p.full().to_string(), p.title.clone()))
}

/// Every text a row paints, in order; the row paints nothing else.
pub(crate) fn row_texts(row: &BoardRow) -> Vec<String> {
    let mut out = vec![row.item.ref_.clone(), row.item.title.clone()];
    if row.criteria.total > 0 {
        out.push(format!("{}/{}", row.criteria.checked, row.criteria.total));
    }
    out
}

pub(crate) fn filter_labels(board: &Board) -> Vec<String> {
    FILTERS.iter().map(|(f, _, label)| format!("{label} {}", state::listed(board, *f).len())).collect()
}

impl WorkView {
    pub(super) fn render_run_page(&self, run: &str, focused: bool, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let root = supraman_ui::scroll::area("work-list").track(&self.list_scroll).size_full().flex().flex_col().gap_2().px_5().pt_4().pb_6();
        let board = match self.boards.get(run) {
            Some(BoardState::Ready(board)) => board,
            Some(BoardState::Failed(message)) => return root.child(placeholder("Cannot show this run", [message.clone()], cx).size_full()).into_any_element(),
            _ => return root.child(placeholder("Reading the run…", Vec::<SharedString>::new(), cx).size_full()).into_any_element(),
        };
        let crumb = div()
            .flex()
            .items_center()
            .gap_1()
            .text_xs()
            .text_color(colors.text_muted)
            .children(crumb_project(board).map(|(r, title)| {
                let ref_ = div().id("work-crumb-project").relative().font_family(fonts::MONO).child(SharedString::from(r)).child(hit_target(&self.hits, "work-crumb-project".into()));
                div().flex().gap_1().child(if title.is_empty() { ref_ } else { ref_.tooltip(Tooltip::text(title)) }).child("/")
            }))
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
            list = list.child(self.render_row(row, focused, cx));
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

    fn render_row(&self, row: &BoardRow, focused: bool, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let item = &row.item;
        let key = format!("work-row-{}", item.qualified_ref);
        let selected = self.selected.as_deref() == Some(item.id.as_str());
        let state = state::of_row(row);
        let indent = px(12. + 16. * row.depth.saturating_sub(1) as f32);
        let (id, copied) = (item.id.clone(), item.clone());
        let mut texts = row_texts(row).into_iter();
        let (ref_, title, criteria) = (texts.next().unwrap_or_default(), texts.next().unwrap_or_default(), texts.next());
        let copy = button::Button::new(&self.hits, format!("work-row-copy-{}", item.qualified_ref)).icon(Icon::Copy.svg(11., colors.text_muted)).icon_only(px(COPY_SIDE)).reveal(key.clone(), selected)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.copy_ref(&copied, cx);
            }));
        let chrome = supraman_ui::list::Chrome::new().selected(selected).focused(focused);
        let mark = div().relative().child(dot(state.key.mark(cx))).child(hit_target(&self.hits, dot_key(&item.qualified_ref, state.key)));
        let ref_ = Label::new(ref_).mark(LabelMark::Element(mark.into_any_element())).gap(tokens::space::L).size(tokens::text::SMALL).color(colors.text_muted);
        supraman_ui::list::row(&self.hits, button::RowKeys::one(key.clone().into()), chrome, cx)
            .flex()
            .items_start()
            .gap_2()
            .pl(indent)
            .pr_3()
            .py(px(4.))
            .text_sm()
            .child(div().flex_none().h(px(COPY_SIDE)).flex().items_center().font_family(fonts::MONO).child(ref_))
            .child(copy)
            .child(div().flex_1().min_w_0().child(clamped(format!("work-row-title-{}", item.qualified_ref), title, 2)))
            .when_some(criteria, |d, criteria| {
                d.child(fonts::figures(div().flex_none().h(px(COPY_SIDE)).flex().items_center().text_xs().text_color(colors.text_muted)).child(SharedString::from(criteria)))
            })
            .on_mouse_down(gpui::MouseButton::Right, self.on_row_mouse_down(item, key, cx))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                window.focus(&this.panes.main, cx);
                this.select(id.clone(), cx);
            }))
            .into_any_element()
    }
}
