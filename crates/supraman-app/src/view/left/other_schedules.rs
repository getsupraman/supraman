use std::collections::BTreeMap;

use gpui::{AnyElement, ClickEvent, Context, SharedString, div, prelude::*, px};
use theme::ActiveTheme as _;

use super::{TOP_ROW, disclosure, grid, icon_slot, secondary, top_text, tree_label};
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

    /// The hit keys Other schedules paints, for the sidebar's [`supraman_ui::keyed_list::KeyedList`].
    pub(super) fn other_schedules_tokens(&self, tree: &Tree) -> Vec<String> {
        let mut tokens = vec![format!("{KEY}-row"), format!("tree-label-{KEY}"), format!("{KEY}-caption"), "other-row-".into(), "other-name-".into(), "other-summary-".into()];
        for key in self.unrowed_keys(tree) {
            tokens.extend(["sched-row-", "sched-state-", "sched-error-", "sched-name-", "sched-folder-"].iter().map(|p| format!("{p}{key}")));
        }
        tokens
    }

    /// Other schedules after the entries: the schedules whose folder has no row, as ordinary schedule rows with their
    /// runs, then Task Scheduler's tasks in no folder; nothing while it has neither.
    pub(super) fn render_other_schedules(&self, tree: &Tree, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let keys = self.unrowed_keys(tree);
        let tasks = self.others().unowned;
        if keys.is_empty() && tasks.is_empty() {
            return Vec::new();
        }
        let open = !self.sidebar.collapsed.contains(KEY);
        let mut out = vec![self.render_other_schedules_row(open, cx)];
        if !open {
            return out;
        }
        let none = BTreeMap::new();
        for key in &keys {
            let rows = self.scheduling.schedules.get(key).map(Vec::as_slice).unwrap_or_default();
            let runs = tree.unrowed_runs(key).unwrap_or(&none);
            out.extend(self.render_schedule_list(UNROWED, rows, runs, 1, cx));
            out.extend(self.render_gone_runs(UNROWED, rows, runs, 1, cx));
        }
        if !tasks.is_empty() {
            let caption = div().relative().pl(grid::indent(1)).pt_1().text_xs().text_color(secondary(cx)).child(CAPTION).child(hit_target(&self.hits, format!("{KEY}-caption")));
            out.push(caption.into_any_element());
            out.extend(tasks.iter().map(|t| self.render_other_row(t, 1, cx)));
        }
        out
    }

    /// Other sessions' look: muted, no icon, no count; a click folds or opens it.
    fn render_other_schedules_row(&self, open: bool, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        div()
            .id(KEY)
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(grid::gap())
            .h(px(TOP_ROW))
            .pl(grid::indent(0))
            .pr_2()
            .map(top_text)
            .text_color(secondary(cx))
            .cursor_pointer()
            .hover(|s| s.bg(colors.ghost_element_hover))
            .active(|s| s.bg(colors.ghost_element_active))
            .child(disclosure(Some(open), secondary(cx)))
            .child(icon_slot(&self.hits, KEY, None))
            .child(tree_label(&self.hits, KEY, div().flex_none().whitespace_nowrap().child(SharedString::from(TITLE))))
            .child(hit_target(&self.hits, format!("{KEY}-row")))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle(KEY.to_string(), cx)))
            .into_any_element()
    }
}
