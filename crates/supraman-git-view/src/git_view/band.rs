//! The tab's band: the first question or prompt of its owners, across the tab under its toolbar.

use gpui::{App, Context, Div, FocusHandle, Window, prelude::*};

use super::GitView;

/// The band's focus, whether one is shown, and where the keyboard was before it came.
pub(super) struct Band {
    focus: FocusHandle,
    shown: bool,
    before: Option<FocusHandle>,
}

impl Band {
    pub(super) fn new(cx: &mut App) -> Band {
        Band { focus: cx.focus_handle(), shown: false, before: None }
    }
}

impl GitView {
    /// The first owner's band; one without a field takes the keyboard while it is shown and gives it back after.
    pub(super) fn band(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<Div> {
        self.remote.update(cx, |r, cx| r.ensure_fields(window, cx));
        let asked = |band: Option<Div>| band.map(|b| (b, false));
        let found = asked(self.gate.update(cx, |o, cx| o.band(cx)))
            .or_else(|| asked(self.wip_panel.update(cx, |o, cx| o.band(cx))))
            .or_else(|| asked(self.undo.update(cx, |o, cx| o.band(cx))))
            .or_else(|| asked(self.stash.update(cx, |o, cx| o.band(cx))))
            .or_else(|| asked(self.branch.update(cx, |o, cx| o.band(cx))))
            .or_else(|| asked(self.commit.update(cx, |o, cx| o.band(cx))))
            .or_else(|| asked(self.panel.update(cx, |o, cx| o.band(cx))))
            .or_else(|| self.remote.update(cx, |o, cx| o.band(cx)));
        let was = std::mem::replace(&mut self.band.shown, found.is_some());
        match &found {
            Some((_, field)) if !was => {
                self.band.before = window.focused(cx);
                if !field {
                    window.focus(&self.band.focus, cx);
                }
            }
            None if was => {
                let before = self.band.before.take();
                if self.band.focus.is_focused(window) || window.focused(cx).is_none() {
                    let back = before.unwrap_or_else(|| self.table.focus_handle().clone());
                    window.focus(&back, cx);
                }
            }
            _ => {}
        }
        found.map(|(band, _)| band.track_focus(&self.band.focus))
    }
}

#[cfg(test)]
impl GitView {
    pub(crate) fn band_focused(&self, window: &Window) -> bool {
        self.band.focus.is_focused(window)
    }
}
