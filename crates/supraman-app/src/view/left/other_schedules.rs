use std::collections::BTreeMap;

use gpui::{AnyElement, ClickEvent, Context, SharedString, div, prelude::*, px};

use super::schedule_rows::{Block, Rows};
use super::{LeftRow, TOP_ROW, disclosure, grid, icon_slot, secondary, top_text, tree_label};
use crate::fleet::{Tree, UNROWED};
use crate::view::FleetView;
use supraman_ui::hit::hit_target;

/// Other schedules' key in the sidebar's rows and its `collapsed` set; it starts open.
pub(in crate::view) const KEY: &str = "other-schedules";
pub(in crate::view) const TITLE: &str = "Other schedules";
const CAPTION: &str = "In Task Scheduler, read only";

impl FleetView {
    /// The folder keys of the schedules and runs no row shows ([`Tree::schedule_row`]), sorted.
    pub(in crate::view) fn unrowed_keys(&self, tree: &Tree) -> Vec<String> {
        let read = self.scheduling.schedules.iter().filter(|(k, rows)| !rows.is_empty() && tree.schedule_row(k).is_none()).map(|(k, _)| k.clone());
        let mut keys: Vec<String> = read.chain(tree.unrowed_keys().map(str::to_string)).collect();
        keys.sort();
        keys.dedup();
        keys
    }

    /// The hit keys Other schedules' own row paints, for the sidebar's [`supraman_ui::list::keyed_list::KeyedList`].
    pub(super) fn other_schedules_tokens() -> Vec<String> {
        vec![format!("{KEY}-row"), format!("tree-label-{KEY}")]
    }

    /// Other schedules after the entries: the schedules whose folder has no row, as ordinary schedule rows with their
    /// runs, then Task Scheduler's tasks in no folder; its rows nothing while it holds neither.
    pub(super) fn other_schedule_rows(&self, tree: &Tree, others: &crate::schedules::Others, rows: &mut Rows, block: &mut Block) {
        rows.push((LeftRow::Others, KEY.into(), Self::other_schedules_tokens()));
        let keys = self.unrowed_keys(tree);
        if keys.is_empty() && others.unowned.is_empty() || self.sidebar.collapsed.contains(KEY) {
            return;
        }
        let none = BTreeMap::new();
        for key in &keys {
            let scheds = self.scheduling.schedules.get(key).map(Vec::as_slice).unwrap_or_default();
            let runs = tree.unrowed_runs(key).unwrap_or(&none);
            block.schedules(UNROWED, scheds, runs, 1, &self.sidebar.collapsed);
            block.gone_runs(UNROWED, scheds, runs, 1);
            rows.append(&mut block.rows);
        }
        if !others.unowned.is_empty() {
            rows.push((LeftRow::OthersCaption, format!("{KEY}-caption"), vec![format!("{KEY}-caption")]));
            block.task_rows(&others.unowned, 1);
            rows.append(&mut block.rows);
        }
    }

    /// Other schedules' own row while it holds a schedule or task.
    pub(super) fn render_other_schedules(&self, tree: &Tree, cx: &mut Context<Self>) -> AnyElement {
        if self.unrowed_keys(tree).is_empty() && self.others().unowned.is_empty() {
            return div().into_any_element();
        }
        self.render_other_schedules_row(!self.sidebar.collapsed.contains(KEY), cx)
    }

    pub(super) fn render_other_schedules_caption(&self, cx: &mut Context<Self>) -> AnyElement {
        div().relative().pl(grid::indent(1)).pt_1().text_xs().text_color(secondary(cx)).child(CAPTION).child(hit_target(&self.hits, format!("{KEY}-caption"))).into_any_element()
    }

    /// Other sessions' look: muted, no icon, no count; a click folds or opens it.
    fn render_other_schedules_row(&self, open: bool, cx: &mut Context<Self>) -> AnyElement {
        let keys = supraman_ui::controls::button::RowKeys { id: KEY.into(), group: KEY.into(), hit: format!("{KEY}-row").into() };
        self.sidebar_row(keys, Some(KEY), cx)
            .flex()
            .flex_none()
            .items_center()
            .gap(grid::gap())
            .h(px(TOP_ROW))
            .pl(grid::indent(0))
            .pr_2()
            .map(top_text)
            .text_color(secondary(cx))
            .child(disclosure(Some(open), secondary(cx)))
            .child(icon_slot(&self.hits, KEY, None))
            .child(tree_label(&self.hits, KEY, div().flex_none().whitespace_nowrap().child(SharedString::from(TITLE))))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle(KEY.to_string(), cx)))
            .into_any_element()
    }
}
