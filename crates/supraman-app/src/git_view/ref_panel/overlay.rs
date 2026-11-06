//! A section of the collapsed panel floating over the graph beside its strip icon: its header, then its tree with
//! the open panel's rows, eyes and solos. Hit keys: `refpanel-overlay`, and `refpanel-overlay-row-<path>`,
//! `-eye-<path>`, `-solo-<path>` with the path below the section.

use gpui::{AnyElement, Context, FocusHandle, KeyDownEvent, MouseDownEvent, Pixels, Point, SharedString, Window, deferred, div, prelude::*, px};
use supraman_ui::hit::hit_target;
use supraman_ui::tree::{TreeList, TreeState};
use theme::ActiveTheme as _;

use super::render::ROW;
use super::{Entry, Keys, LIST, RefPanel, STRIP, Section, fold};

pub const OVERLAY: &str = "refpanel-overlay";
/// The most rows the overlay shows before its tree scrolls.
const MOST_ROWS: usize = 18;

pub(super) struct Overlay {
    section: Section,
    tree: TreeState<Entry, Keys>,
    focus: FocusHandle,
}

impl RefPanel {
    /// The section whose overlay is open.
    pub fn overlay(&self) -> Option<Section> {
        self.overlay.as_ref().map(|o| o.section)
    }

    /// The overlay's rows' paths below its section, top to bottom.
    pub fn overlay_rows(&self) -> Vec<SharedString> {
        self.overlay.as_ref().map(|o| o.tree.visible()).unwrap_or_default()
    }

    /// Opens `section`'s overlay with the keyboard in it, or closes it when it is the one open.
    pub fn toggle_overlay(&mut self, section: Section, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlay() == Some(section) {
            return self.close_overlay(window, cx);
        }
        let tree = TreeState::new(cx);
        tree.match_with(|entry: &Entry, filter| entry.matches(filter));
        let focus = cx.focus_handle();
        self.overlay = Some(Overlay { section, tree: tree.clone(), focus: focus.clone() });
        self.refill_overlay();
        tree.expand_all();
        tree.set_filter(&self.filter());
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

    pub(super) fn refill_overlay(&self) {
        let Some(overlay) = &self.overlay else { return };
        let prefix = format!("{}/", overlay.section.key());
        let rows = self.entries.iter().filter_map(|(path, entry)| path.strip_prefix(&prefix).map(|rest| (rest.to_string(), entry.clone())));
        overlay.tree.set_entries_given(rows, fold);
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
        let (section, tree, focus) = self.overlay.as_ref().map(|o| (o.section, o.tree.clone(), o.focus.clone()))?;
        let rows = tree.visible().len().clamp(1, MOST_ROWS);
        let list = TreeList::new(OVERLAY, &tree, &self.hits, |this: &mut RefPanel, row, _, cx| this.paint(OVERLAY, row, cx))
            .row_height(px(ROW))
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
