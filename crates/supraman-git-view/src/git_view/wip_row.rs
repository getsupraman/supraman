//! The graph's WIP row with the commit summary's second field (the conflict node while files conflict), and the staging keys.

use std::collections::HashMap;

use gpui::{AnyElement, App, Div, MouseButton, MouseDownEvent, SharedString, Window, div, prelude::*, px};
use gpui::Context;
use supraman_ui::hit::{HitTargets, hit_target};
use supraman_ui::icon::Icon;
use supraman_ui::text_input;
use supraman_gitman::WipCounts;
use theme::ActiveTheme as _;

use super::colours::Palette;
use super::wip::conflicts::conflicts_sentence;
use super::{GitView, Selected, cells, keys};

/// The WIP row's summary field's width (GitKraken's).
const FIELD: f32 = 140.;
pub(crate) const FIELD_KEY: &str = "git-wip-field";

impl GitView {
    /// The summary's field and the counts; `// WIP` and the counts until the field is made.
    pub(super) fn wip_message(&mut self, ix: usize, palette: &Palette, cx: &mut Context<Self>) -> AnyElement {
        if let Some(conflicts) = self.wip.map(|w| w.conflicted).filter(|n| *n > 0) {
            let rows = self.pages.read(cx).rows.clone();
            return cells::wip_message(&rows[ix], conflict_node(conflicts, "git-conflict-node".into(), &self.hits, palette, cx), "", palette);
        }
        let counts = self.wip.map(|w| w.words()).unwrap_or_default();
        let input = self.wip_panel.read(cx).composer().map(|c| c.row.clone());
        let rows = self.pages.read(cx).rows.clone();
        let row = &rows[ix];
        let Some(input) = input else { return cells::message(ix, row, Some(counts.as_str()), palette, &self.hits) };
        if self.selected(cx) != Some(Selected::Wip) {
            let summary = input.read(cx).text().to_string();
            return cells::wip_message(row, div().relative().flex_none().child(still_field(summary, palette, cx)).child(hit_target(&self.hits, FIELD_KEY.into())), SharedString::from(counts), palette);
        }
        let order = self.wip_panel.read(cx).message_order(true, cx).unwrap_or_default();
        let field = div()
            .relative()
            .w(px(FIELD))
            .flex_none()
            .key_context(keys::MESSAGE_CONTEXT)
            .on_action(cx.listener(|this, _: &keys::StageAllAndCommit, _, cx| this.wip_panel.update(cx, |p, cx| p.commit(true, cx))))
            .on_mouse_down(MouseButton::Left, cx.listener(|_, _: &MouseDownEvent, _, cx| cx.stop_propagation()))
            .child(input)
            .child(hit_target(&self.hits, FIELD_KEY.into()));
        cells::wip_message(row, order.on(div().flex_none()).child(field), SharedString::from(counts), palette)
    }

    /// Ctrl+Shift+S: every file staged, the WIP row selected and its summary given the keyboard.
    pub(super) fn stage_all_keyed(&mut self, cx: &mut Context<Self>) {
        self.wip_panel.update(cx, |p, cx| p.stage_all(cx));
        self.focus_message(cx);
    }

    /// Ctrl+Shift+M: the WIP row selected and the summary given the keyboard on the next frame.
    pub(super) fn focus_message(&mut self, cx: &mut Context<Self>) {
        if self.selected(cx) != Some(Selected::Wip) {
            self.select_wip(cx);
        }
        self.focus_summary = true;
        cx.notify();
    }

    /// A selftest hook (`selftest_shots`): the WIP row selected with `summary` in the commit area, as typed.
    pub fn draft_commit(&mut self, summary: &str, cx: &mut Context<Self>) {
        self.select_wip(cx);
        self.wip_panel.update(cx, |p, cx| {
            if let Some(c) = p.composer() {
                c.set(summary, "", cx);
            }
        });
        cx.notify();
    }

    /// The WIP row selected when files became conflicted since `before`.
    pub(super) fn select_conflict(&mut self, before: Option<WipCounts>, cx: &mut Context<Self>) {
        let conflicted = |wip: Option<WipCounts>| wip.is_some_and(|w| w.conflicted > 0);
        if conflicted(self.wip) && !conflicted(before) && self.selected(cx) != Some(Selected::Wip) {
            self.select_wip(cx);
        }
    }

    /// Each branch's lane colour in the loaded rows, by its pill's name and each ref's.
    pub(super) fn ref_colours(&self, cx: &App) -> HashMap<String, usize> {
        self.pages
            .read(cx)
            .rows
            .iter()
            .flat_map(|r| r.row.refs.iter().flat_map(move |g| std::iter::once(g.name.clone()).chain(g.refs.iter().map(|l| l.name.clone())).map(move |n| (n, r.row.colour))))
            .collect()
    }

    pub(super) fn take_focus_summary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.focus_summary) && self.selected(cx) == Some(Selected::Wip) {
            self.wip_panel.update(cx, |p, cx| p.focus_message(window, cx));
        }
    }
}

/// [`conflicts_sentence`] after the conflict mark, painted under `key`.
pub(super) fn conflict_node(n: u32, key: String, hits: &HitTargets, palette: &Palette, cx: &App) -> Div {
    let words = conflicts_sentence(n as usize);
    div()
        .relative()
        .flex()
        .items_center()
        .gap_1p5()
        .min_w_0()
        .whitespace_nowrap()
        .text_color(palette.text)
        .child(Icon::Conflict.svg(14., cx.theme().status().conflict))
        .child(SharedString::from(words))
        .child(hit_target(hits, key))
}

/// The field as the row shows it until the row is selected: a click on it selects the row.
fn still_field(summary: String, palette: &Palette, cx: &App) -> Div {
    let (text, colour) = if summary.is_empty() { ("// WIP".to_string(), palette.muted) } else { (summary, palette.text) };
    text_input::quiet_frame(false, cx).w(px(FIELD)).flex_none().whitespace_nowrap().text_color(colour).child(SharedString::from(text))
}

#[cfg(test)]
mod tests;
