//! The tab's prompt: its owners' first question or form, modal over the tab alone (`Dialog::in_tab`).

use gpui::{AnyElement, Context, Window};

use super::GitView;

impl GitView {
    /// The first owner's prompt, its fields made once a window is at hand.
    pub(super) fn prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.wip_panel
            .update(cx, |o, cx| o.prompt(cx))
            .or_else(|| self.undo.update(cx, |o, cx| o.prompt(cx)))
            .or_else(|| self.stash.update(cx, |o, cx| o.prompt(window, cx)))
            .or_else(|| self.branch.update(cx, |o, cx| o.prompt(window, cx)))
            .or_else(|| self.commit.update(cx, |o, cx| o.prompt(cx)))
            .or_else(|| self.panel.update(cx, |o, cx| o.prompt(cx)))
            .or_else(|| self.remote.update(cx, |o, cx| o.prompt(window, cx)))
    }
}
