//! A bordered group of two or more options, one of them chosen and filled: a view toggle such as
//! Path | Tree. The owner keeps which one is chosen.

use gpui::{ClickEvent, Context, Div, SharedString, Window, div, prelude::*, px};
use theme::ActiveTheme as _;

use crate::button::segment;
use crate::hit::HitTargets;
use crate::icon::Icon;

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
/// its index.
pub fn segmented<V: 'static>(
    hits: &HitTargets,
    id: &str,
    segments: &[Segment],
    chosen: usize,
    on_change: impl Fn(&mut V, usize, &mut Window, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> Div {
    let border = cx.theme().colors().border;
    let options = segments.iter().enumerate().map(|(i, s)| {
        let on_change = on_change.clone();
        segment(hits, format!("{id}-{}", s.key), s.icon, s.label.clone(), i == chosen, cx)
            .when(i > 0, |d| d.border_l_1().border_color(border))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                if i != chosen {
                    on_change(this, i, window, cx);
                    cx.notify();
                }
            }))
    });
    div().flex().flex_none().h(px(24.)).rounded_sm().border_1().border_color(border).overflow_hidden().children(options.collect::<Vec<_>>())
}

#[cfg(test)]
mod tests {
    use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};

    use super::*;

    struct Toggle {
        hits: HitTargets,
        chosen: usize,
        changes: Vec<usize>,
    }

    impl Render for Toggle {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let segments = [Segment::new("path").label("Path"), Segment::new("tree").icon(Icon::Folder).label("Tree"), Segment::new("flat").icon(Icon::Tab)];
            let group = segmented(&self.hits, "view", &segments, self.chosen, |this: &mut Toggle, i, _, _| {
                this.chosen = i;
                this.changes.push(i);
            }, cx);
            div().size_full().child(group)
        }
    }

    fn toggle(cx: &mut TestAppContext) -> (Entity<Toggle>, &mut VisualTestContext) {
        cx.update(|cx| crate::init_globals(cx, "test"));
        let (toggle, cx) = cx.add_window_view(|_, _| Toggle { hits: HitTargets::default(), chosen: 0, changes: Vec::new() });
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
}
