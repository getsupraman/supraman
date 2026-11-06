//! A section of the collapsed panel floating over the graph beside its strip icon: its header, then its tree with
//! the open panel's rows and marks. Hit keys: `refpanel-overlay`, and `refpanel-overlay-row-<path>`,
//! `-mark-<path>` with the path below the section.

use gpui::{AnyElement, Context, FocusHandle, KeyDownEvent, MouseDownEvent, Pixels, Point, SharedString, Window, deferred, div, prelude::*, px};
use supraman_ui::hit::hit_target;
use supraman_ui::tree::{Tree, TreeList, TreeState};
use theme::ActiveTheme as _;

use super::marks::MARK;
use super::render::ROW;
use super::{Entry, Keys, LIST, RefPanel, STRIP, Section, fold};

pub const OVERLAY: &str = "refpanel-overlay";
/// The most rows the overlay shows before its tree scrolls.
const MOST_ROWS: usize = 18;

pub(super) struct Overlay {
    section: Section,
    tree: TreeState<Entry, Keys>,
    focus: FocusHandle,
    /// Bumped by each build; a build that finishes under a newer one is dropped.
    build: u64,
    /// Whether its tree has been built once: it paints only from then.
    ready: bool,
}

/// `section`'s rows below its folder, for `cx.background_spawn`.
fn section_tree(entries: &[(String, Entry)], section: Section) -> Tree<Entry, Keys> {
    let prefix = format!("{}/", section.key());
    Tree::build_given(entries.iter().filter_map(|(path, entry)| path.strip_prefix(&prefix).map(|rest| (rest.to_string(), entry.clone()))), fold)
}

impl RefPanel {
    /// The section whose overlay is open.
    pub fn overlay(&self) -> Option<Section> {
        self.overlay.as_ref().map(|o| o.section)
    }

    /// The overlay's rows' paths below its section, top to bottom.
    #[cfg(test)]
    pub fn overlay_rows(&self) -> Vec<SharedString> {
        self.overlay.as_ref().map(|o| o.tree.visible()).unwrap_or_default()
    }

    /// Opens `section`'s overlay with the keyboard in it once its tree is built, or closes it when it is the one open.
    pub fn toggle_overlay(&mut self, section: Section, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlay() == Some(section) {
            return self.close_overlay(window, cx);
        }
        let tree = TreeState::new(cx);
        tree.match_with(|entry: &Entry, filter| entry.matches(filter));
        let focus = cx.focus_handle();
        self.overlay = Some(Overlay { section, tree, focus: focus.clone(), build: 0, ready: false });
        self.refill_overlay(cx);
        window.focus(&focus, cx);
        cx.notify();
    }

    /// Closes the overlay; the tab takes the keyboard when the overlay held it.
    pub fn close_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(overlay) = self.overlay.take() else { return };
        if overlay.focus.contains_focused(window, cx) {
            window.focus(&self.tab, cx);
        }
        cx.notify();
    }

    /// Builds the open overlay's tree off the UI thread; the rows shown until then stay.
    pub(super) fn refill_overlay(&mut self, cx: &mut Context<Self>) {
        let Some(overlay) = &mut self.overlay else { return };
        overlay.build += 1;
        let (section, build, entries) = (overlay.section, overlay.build, self.entries.clone());
        let built = cx.background_spawn(async move { section_tree(&entries, section) });
        cx.spawn(async move |this, cx| {
            let tree = built.await;
            this.update(cx, |this, cx| this.overlay_built(section, build, tree, cx)).ok();
        })
        .detach();
    }

    fn overlay_built(&mut self, section: Section, build: u64, tree: Tree<Entry, Keys>, cx: &mut Context<Self>) {
        let filter = self.filter();
        let Some(overlay) = self.overlay.as_mut().filter(|o| o.section == section && o.build == build) else { return };
        overlay.tree.set_tree(tree);
        if !std::mem::replace(&mut overlay.ready, true) {
            overlay.tree.expand_all();
            overlay.tree.set_filter(&filter);
        }
        cx.notify();
    }

    fn overlay_picked(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(section) = self.overlay() else { return };
        if self.picked(&format!("{}/{path}", section.key()), cx) {
            self.close_overlay(window, cx);
        }
    }

    /// Whether `at` is on the strip icon of the open overlay, whose own click closes it.
    fn on_own_icon(&self, at: Point<Pixels>) -> bool {
        let Some(section) = self.overlay() else { return false };
        self.hits.borrow().get(&format!("{LIST}-strip-{}", section.key())).is_some_and(|hit| hit.bounds.contains(&at))
    }

    pub(super) fn render_overlay(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (section, tree, focus) = self.overlay.as_ref().filter(|o| o.ready).map(|o| (o.section, o.tree.clone(), o.focus.clone()))?;
        let rows = tree.visible().len().clamp(1, MOST_ROWS);
        let list = TreeList::new(OVERLAY, &tree, &self.hits, |this: &mut RefPanel, row, _, cx| this.paint(OVERLAY, row, cx))
            .row_height(px(ROW))
            .mark(px(MARK), |this, row, _, cx| this.paint_mark(OVERLAY, row, cx))
            .on_select(|this, path, window, cx| this.overlay_picked(path, window, cx))
            .render(cx)
            .flex_shrink(1.)
            .min_h_0()
            .h(px(ROW * rows as f32));
        let colors = cx.theme().colors();
        let header = div().flex_none().px_2().py_1().child(self.header(section, true, cx));
        let card = div()
            .id(OVERLAY)
            .track_focus(&focus)
            .absolute()
            .top_0()
            .left(STRIP)
            .w(self.width)
            .max_h_full()
            .flex()
            .flex_col()
            .py_1()
            .bg(colors.elevated_surface_background)
            .border_1()
            .border_color(colors.border)
            .rounded_md()
            .shadow_lg()
            .occlude()
            .child(hit_target(&self.hits, OVERLAY.into()))
            .child(header)
            .child(list)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                if e.keystroke.key == "escape" {
                    cx.stop_propagation();
                    this.close_overlay(window, cx);
                }
            }))
            .on_mouse_down_out(cx.listener(|this, e: &MouseDownEvent, window, cx| {
                if !this.on_own_icon(e.position) {
                    this.close_overlay(window, cx);
                }
            }));
        Some(deferred(card).with_priority(1).into_any_element())
    }
}
