//! A bordered group of two or more options, one of them chosen and filled: a view toggle such as
//! Path | Tree. The owner keeps which one is chosen.

use gpui::{App, ClickEvent, Context, Div, ElementId, FocusHandle, KeyDownEvent, SharedString, StyleRefinement, Window, div, prelude::*, px};
use theme::ActiveTheme as _;

use crate::button::segment;
use crate::hit::HitTargets;
use crate::icon::Icon;
use crate::tokens;

#[derive(Clone, Debug)]
pub struct Segment {
    pub key: SharedString,
    pub icon: Option<Icon>,
    pub label: Option<SharedString>,
}

impl Segment {
    pub fn new(key: impl Into<SharedString>) -> Self {
        Self { key: key.into(), icon: None, label: None }
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }
}

/// The group; each option paints under `<id>-<key>`, and a click on one not chosen calls `on_change` with
/// its index. It is one keyboard member, as [`crate::button::choices`] is: the arrows pick the option before or after
/// the chosen one, and it wears [`crate::focus::ring`] while its focus came by keyboard ([`Segmented::focus`]).
pub fn segmented<V: 'static>(
    hits: &HitTargets,
    id: &str,
    segments: &[Segment],
    chosen: usize,
    on_change: impl Fn(&mut V, usize, &mut Window, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> Segmented {
    let border = cx.theme().colors().border;
    let options = segments.iter().enumerate().map(|(i, s)| {
        let on_change = on_change.clone();
        segment(hits, format!("{id}-{}", s.key), s.icon, s.label.clone(), i == chosen, cx)
            .when(i > 0, |d| d.border_l(tokens::stroke::HAIRLINE).border_color(border))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                if i != chosen {
                    on_change(this, i, window, cx);
                    cx.notify();
                }
            }))
    });
    let group = div().flex().flex_none().h(px(24.)).rounded(tokens::radius::S).border(tokens::stroke::HAIRLINE).border_color(border).overflow_hidden().children(options.collect::<Vec<_>>());
    let count = segments.len();
    let keys = cx.listener(move |this, e: &KeyDownEvent, window, cx| {
        let back = match e.keystroke.key.as_str() {
            "left" | "up" => true,
            "right" | "down" => false,
            _ => return,
        };
        if e.keystroke.modifiers.modified() {
            return;
        }
        cx.stop_propagation();
        crate::focus::keyed(window, cx);
        if let Some(to) = crate::button::neighbour(Some(chosen), count, back) {
            on_change(this, to, window, cx);
            cx.notify();
        }
    });
    Segmented { id: SharedString::from(format!("{id}-focus")), outer: div().flex_none().relative().rounded(tokens::radius::S).on_key_down(keys).child(group), focus: None }
}

/// A [`segmented`] group, focusable as one member.
#[derive(IntoElement)]
pub struct Segmented {
    id: SharedString,
    outer: Div,
    focus: Option<FocusHandle>,
}

impl Segmented {
    /// The owner's handle, a member of its form's Tab order; else the group keeps one of its own.
    pub fn focus(mut self, handle: &FocusHandle) -> Self {
        self.focus = Some(handle.clone());
        self
    }
}

impl Styled for Segmented {
    fn style(&mut self) -> &mut StyleRefinement {
        self.outer.style()
    }
}

impl RenderOnce for Segmented {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focus = self.focus.unwrap_or_else(|| window.use_keyed_state(ElementId::Name(self.id), cx, |_, cx| cx.focus_handle()).read(cx).clone());
        self.outer.track_focus(&focus).child(crate::focus::ring(&focus, tokens::radius::S))
    }
}

#[cfg(test)]
mod tests {
    use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};

    use super::*;

    struct Toggle {
        hits: HitTargets,
        chosen: usize,
        changes: Vec<usize>,
        focus: gpui::FocusHandle,
    }

    impl Render for Toggle {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let segments = [Segment::new("path").label("Path"), Segment::new("tree").icon(Icon::Folder).label("Tree"), Segment::new("flat").icon(Icon::Tab)];
            let group = segmented(&self.hits, "view", &segments, self.chosen, |this: &mut Toggle, i, _, _| {
                this.chosen = i;
                this.changes.push(i);
            }, cx)
            .focus(&self.focus);
            crate::focus::root(div().size_full()).child(group)
        }
    }

    fn toggle(cx: &mut TestAppContext) -> (Entity<Toggle>, &mut VisualTestContext) {
        cx.update(|cx| crate::init_globals(cx, "test"));
        let (toggle, cx) = cx.add_window_view(|_, cx| Toggle { hits: HitTargets::default(), chosen: 0, changes: Vec::new(), focus: cx.focus_handle() });
        cx.run_until_parked();
        (toggle, cx)
    }

    fn click(toggle: &Entity<Toggle>, key: &str, cx: &mut VisualTestContext) {
        let hit = toggle.read_with(cx, |t, _| t.hits.borrow().get(key).copied()).unwrap_or_else(|| panic!("{key} paints"));
        cx.simulate_click(hit.bounds.center(), Modifiers::none());
        cx.run_until_parked();
    }

    #[gpui::test]
    fn a_click_on_an_option_not_chosen_reports_it(cx: &mut TestAppContext) {
        let (toggle, cx) = toggle(cx);
        click(&toggle, "view-tree", cx);
        click(&toggle, "view-tree", cx);
        click(&toggle, "view-flat", cx);
        click(&toggle, "view-path", cx);
        toggle.read_with(cx, |t, _| {
            assert_eq!(t.changes, [1, 2, 0], "a click on the chosen option reports nothing");
            assert_eq!(t.chosen, 0);
        });
    }

    #[gpui::test]
    fn the_arrows_pick_its_neighbours_and_the_ring_shows_for_keyboard_focus_only(cx: &mut TestAppContext) {
        let (toggle, cx) = toggle(cx);
        let focus = toggle.read_with(cx, |t, _| t.focus.clone());
        cx.update(|window, cx| window.focus(&focus, cx));
        cx.run_until_parked();
        assert!(!cx.update(|window, cx| crate::focus::visible(&focus, window, cx)), "no ring before a key");
        cx.simulate_keystrokes("right right right left");
        cx.run_until_parked();
        toggle.read_with(cx, |t, _| assert_eq!(t.changes, [1, 2, 1], "Right stops at the last"));
        assert!(cx.update(|window, cx| crate::focus::visible(&focus, window, cx)), "the ring once a key came");
    }
}
