use std::collections::HashSet;

use gpui::{AnyElement, ClickEvent, Context, SharedString, div, prelude::*, px};

use super::{TOP_ROW, disclosure, grid, icon_slot, secondary, top_text, tree_label};
use crate::fleet::UNLISTED_TITLE;
use crate::view::FleetView;

/// Other sessions' key in the sidebar's rows and its `collapsed` set.
pub(in crate::view) const KEY: &str = "repo::unlisted";
const HIT: &str = "unlisted";

impl FleetView {
    /// Other sessions' row: muted, no icon, no count, and no folder's actions; a click folds or opens it.
    pub(super) fn render_unlisted_row(&self, open: bool, cx: &mut Context<Self>) -> AnyElement {
        let keys = supraman_ui::controls::button::RowKeys { id: KEY.into(), group: KEY.into(), hit: format!("{HIT}-row").into() };
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
            .child(icon_slot(&self.hits, HIT, None))
            .child(tree_label(&self.hits, HIT, div().flex_none().whitespace_nowrap().child(SharedString::from(UNLISTED_TITLE))))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle(KEY.to_string(), cx)))
            .into_any_element()
    }
}

/// The sidebar's `collapsed` set at the start: Other sessions folded.
pub(in crate::view) fn first_collapsed() -> HashSet<String> {
    HashSet::from([KEY.to_string()])
}

/// The hit keys Other sessions' row paints, for the sidebar's [`supraman_ui::list::keyed_list::KeyedList`].
pub(super) fn tokens() -> Vec<String> {
    vec![format!("{HIT}-row"), format!("tree-icon-{HIT}"), format!("tree-label-{HIT}")]
}

#[cfg(test)]
mod tests {
    #[test]
    fn other_sessions_starts_folded() {
        assert!(super::first_collapsed().contains(super::KEY));
    }
}
