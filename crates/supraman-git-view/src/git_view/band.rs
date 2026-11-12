//! The tab's band: the first question or prompt of its owners, over the tab's toolbar.

use gpui::{App, Context, Div, FocusHandle, Window, prelude::*};

use super::GitView;

/// The band's focus and whether one is shown.
pub(super) struct Band {
    focus: FocusHandle,
    shown: bool,
}

impl Band {
    pub(super) fn new(cx: &mut App) -> Band {
        Band { focus: cx.focus_handle(), shown: false }
    }
}

impl GitView {
    /// The first owner's band; one without a field takes the keyboard while it is shown, [`supraman_ui::focus`] gives it back after.
    pub(super) fn band(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<Div> {
        self.remote.update(cx, |r, cx| r.ensure_fields(window, cx));
        let asked = |band: Option<Div>| band.map(|b| (b, false));
        let found = asked(self.wip_panel.update(cx, |o, cx| o.band(cx)))
            .or_else(|| asked(self.undo.update(cx, |o, cx| o.band(cx))))
            .or_else(|| asked(self.stash.update(cx, |o, cx| o.band(cx))))
            .or_else(|| asked(self.branch.update(cx, |o, cx| o.band(cx))))
            .or_else(|| asked(self.commit.update(cx, |o, cx| o.band(cx))))
            .or_else(|| asked(self.panel.update(cx, |o, cx| o.band(cx))))
            .or_else(|| self.remote.update(cx, |o, cx| o.band(cx)));
        let was = std::mem::replace(&mut self.band.shown, found.is_some());
        if let Some((_, false)) = &found
            && !was
        {
            window.focus(&self.band.focus, cx);
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
