//! The graph's column header menu (docs/research/gitkraken-12.6.1.md §B); the hidden columns are kept in the layout file.

use gpui::{AnyElement, Context, Pixels, Point};
use supraman_ui::menu::{ContextMenu, MenuItem, item};
use supraman_ui::store::save_off_thread;

use super::{GitView, columns, layout};

/// The columns the compact layout shows.
const COMPACT: [&str; 3] = ["refs", "graph", "message"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ColumnAct {
    Toggle(String),
    Default,
    Compact,
}

impl GitView {
    /// The indices of the columns shown, in their order.
    pub(super) fn shown_columns(&self) -> Vec<usize> {
        columns(None).iter().enumerate().filter(|(_, c)| !self.hidden_columns.iter().any(|h| *h == c.id.as_ref())).map(|(i, _)| i).collect()
    }

    pub(super) fn header_menu(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let shown = self.shown_columns();
        let mut items: Vec<MenuItem<ColumnAct>> = columns(None)
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let on = shown.contains(&i);
                let tick = item(format!("column-{}", c.id), c.caption.to_string(), ColumnAct::Toggle(c.id.to_string())).ticked(on);
                if on && shown.len() == 1 { tick.dimmed("One column stays shown") } else { tick }
            })
            .collect();
        items.push(item("column-default", "Reset columns to default", ColumnAct::Default).rule());
        items.push(item("column-compact", "Compact layout", ColumnAct::Compact));
        self.column_menu = Some(ContextMenu::new("Columns".into(), position, items));
        cx.notify();
    }

    fn column_act(&mut self, act: ColumnAct, cx: &mut Context<Self>) {
        self.column_menu = None;
        let defaults: Vec<Pixels> = columns(None).iter().map(|c| c.default).collect();
        match act {
            ColumnAct::Toggle(id) => {
                if let Some(at) = self.hidden_columns.iter().position(|h| *h == id) {
                    self.hidden_columns.remove(at);
                } else if self.shown_columns().len() > 1 {
                    self.hidden_columns.push(id);
                }
            }
            ColumnAct::Default => {
                self.hidden_columns.clear();
                self.widths = defaults;
                self.keep_layout(cx);
            }
            ColumnAct::Compact => {
                self.hidden_columns = columns(None).iter().map(|c| c.id.to_string()).filter(|id| !COMPACT.contains(&id.as_str())).collect();
                self.widths = defaults;
                self.keep_layout(cx);
            }
        }
        let save = layout::hide_columns(self.hidden_columns.clone(), cx);
        save_off_thread(save, cx, Self::kept);
        cx.notify();
    }

    pub(super) fn render_column_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.column_menu.as_ref()?;
        Some(menu.render(&self.hits, |this: &mut GitView, act, _, cx| this.column_act(act, cx), |this: &mut GitView, cx| {
            this.column_menu = None;
            cx.notify();
        }, cx))
    }
}

#[cfg(test)]
mod tests {
    use gpui::{Entity, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, VisualTestContext};
    use supraman_gitman::fixture::Fixture;

    use crate::git_view::GitView;
    use crate::git_view::test_support::{Opts, open, redraw};
    use crate::git_view::test_support::{click, repaint};

    fn right_click(view: &Entity<GitView>, key: &str, cx: &mut VisualTestContext) {
        let position = repaint(view, cx).borrow().get(key).unwrap_or_else(|| panic!("{key} is painted")).bounds.center();
        let (button, modifiers) = (MouseButton::Right, Modifiers::none());
        cx.simulate_event(MouseDownEvent { position, button, modifiers, click_count: 1, first_mouse: false });
        cx.simulate_event(MouseUpEvent { position, button, modifiers, click_count: 1 });
        cx.run_until_parked();
        redraw(view, cx);
    }

    /// The header menu's items: `+` ticked, `-` not, `!` dimmed.
    fn ticks(view: &Entity<GitView>, cx: &mut VisualTestContext) -> Vec<String> {
        view.read_with(cx, |v, _| {
            v.column_menu.as_ref().map_or_else(Vec::new, |m| {
                m.items.iter().map(|i| format!("{}{}{}", i.ticked.map_or("", |t| if t { "+" } else { "-" }), if i.dimmed.is_some() { "!" } else { "" }, i.label)).collect()
            })
        })
    }

    fn shown(view: &Entity<GitView>, cx: &mut VisualTestContext) -> Vec<&'static str> {
        let hits = repaint(view, cx);
        ["refs", "graph", "message", "date", "sha"].into_iter().filter(|c| hits.borrow().contains_key(&format!("git-col-{c}"))).collect()
    }

    #[gpui::test]
    fn the_header_menu_ticks_columns_keeps_the_last_and_resets_the_layouts(cx: &mut gpui::TestAppContext) {
        let f = Fixture::copy();
        let (view, cx) = open(&f.worktree, Opts::default(), cx);
        right_click(&view, "git-col-date", cx);
        assert_eq!(ticks(&view, cx), ["+Branch / Tag", "+Graph", "+Commit Message", "+Commit Date / Time", "+Sha", "Reset columns to default", "Compact layout"]);
        click(&view, "menu-column-date", cx);
        assert_eq!(shown(&view, cx), ["refs", "graph", "message", "sha"], "unticked: hidden");

        for column in ["refs", "graph", "message"] {
            right_click(&view, "git-col-sha", cx);
            click(&view, &format!("menu-column-{column}"), cx);
        }
        assert_eq!(shown(&view, cx), ["sha"]);
        right_click(&view, "git-col-sha", cx);
        assert_eq!(ticks(&view, cx)[4], "+!Sha", "the last column shown cannot be unticked");
        click(&view, "menu-column-sha", cx);
        assert_eq!(shown(&view, cx), ["sha"]);

        view.update(cx, |v, _| v.column_menu = None);
        right_click(&view, "git-col-sha", cx);
        click(&view, "menu-column-compact", cx);
        assert_eq!(shown(&view, cx), ["refs", "graph", "message"], "the compact layout");
        right_click(&view, "git-col-message", cx);
        click(&view, "menu-column-default", cx);
        assert_eq!(shown(&view, cx), ["refs", "graph", "message", "date", "sha"], "the default layout");
    }
}
