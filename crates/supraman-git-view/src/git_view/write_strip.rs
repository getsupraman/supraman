//! The toolbar's one strip of the write under way: a remote call's words and progress, else a write's that has run `CANCEL_AFTER`, then Cancel.

use gpui::{ClickEvent, Context, Div, Entity, div, prelude::*};
use supraman_ui::button;
use theme::ActiveTheme as _;

use super::GitView;
use super::remote::bar::CANCEL_KEY as REMOTE_CANCEL_KEY;
use super::write_gate::{WriteGate, Written};

pub(crate) const CANCEL_KEY: &str = "git-write-cancel";

/// `said`, then Cancel under `key`.
fn strip(view: &GitView, key: &'static str, said: String, on_cancel: fn(&mut GitView, &mut Context<GitView>), cx: &mut Context<GitView>) -> Div {
    let cancel = button::action(&view.hits, key, "Cancel", button::Tone::Neutral, button::Fill::Outline, cx).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| on_cancel(this, cx)));
    div().flex().items_center().gap_2().px_2().text_xs().text_color(cx.theme().colors().text_muted).child(said).child(cancel)
}

impl GitView {
    pub(in crate::git_view) fn write_strip(&self, cx: &mut Context<Self>) -> Option<Div> {
        let remote = self.remote.read(cx).progress().map(|(doing, progress)| match progress {
            Some(p) => format!("{doing}: {} {}%", p.phase, p.percent),
            None => format!("{doing}…"),
        });
        if let Some(said) = remote {
            return Some(strip(self, REMOTE_CANCEL_KEY, said, |this, cx| this.remote.update(cx, |r, cx| r.cancel(cx)), cx));
        }
        let said = self.gate.read(cx).long().map(|(words, secs)| format!("{words}… {secs} s"))?;
        Some(strip(self, CANCEL_KEY, said, |this, cx| this.gate.update(cx, |g, _| g.cancel()), cx))
    }

    /// A cancelled write's toast, what it left, and the graph read again.
    pub(in crate::git_view) fn gate_written(&mut self, _: Entity<WriteGate>, w: &Written, cx: &mut Context<Self>) {
        if let Some(left) = &w.cancelled {
            self.toast.show_why(left.clone(), |v: &mut GitView| &mut v.toast, cx);
            self.refresh_after_write(cx);
        }
    }
}
