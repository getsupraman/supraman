//! The graph's WIP row, where the commit summary has its second field (the conflict node in its place while
//! files are conflicted), and the tab's staging keys.

use std::collections::HashMap;

use gpui::{AnyElement, App, Div, MouseButton, MouseDownEvent, SharedString, Window, div, prelude::*, px};
use gpui::Context;
use supraman_ui::hit::{HitTargets, hit_target};
use supraman_ui::icon::Icon;
use theme::ActiveTheme as _;

use super::colours::Palette;
use super::load::Wip;
use super::{GitView, Selected, cells, keys};

/// The WIP row's summary field's width.
const FIELD: f32 = 160.;

impl GitView {
    /// The summary's field and the counts; `// WIP` and the counts until the field is made.
    pub(super) fn wip_message(&mut self, ix: usize, palette: &Palette, cx: &mut Context<Self>) -> AnyElement {
        if let Some(conflicts) = self.wip.map(|w| w.conflicts).filter(|n| *n > 0) {
            return cells::wip_message(&self.rows[ix], conflict_node(conflicts, &self.hits, palette, cx), String::new(), palette);
        }
        let counts = self.wip.map(|w| w.counts()).unwrap_or_default();
        let input = self.wip_panel.read(cx).composer().map(|c| c.row.clone());
        let row = &self.rows[ix];
        let Some(input) = input else { return cells::message(ix, row, Some(counts.as_str()), palette, &self.hits) };
        if self.selected() != Some(Selected::Wip) {
            let summary = input.read(cx).text().to_string();
            return cells::wip_message(row, still_field(summary, palette, cx), counts, palette);
        }
        let field = div()
            .w(px(FIELD))
            .flex_none()
            .key_context(keys::MESSAGE_CONTEXT)
            .on_action(cx.listener(|this, _: &keys::StageAllAndCommit, _, cx| this.wip_panel.update(cx, |p, cx| p.commit(true, cx))))
            .on_mouse_down(MouseButton::Left, cx.listener(|_, _: &MouseDownEvent, _, cx| cx.stop_propagation()))
            .child(input);
        cells::wip_message(row, field, counts, palette)
    }

    /// Ctrl+Shift+S: every file staged, the WIP row selected and its summary given the keyboard.
    pub(super) fn stage_all_keyed(&mut self, cx: &mut Context<Self>) {
        self.wip_panel.update(cx, |p, cx| p.stage_all(cx));
        self.focus_message(cx);
    }

    /// Ctrl+Shift+M: the WIP row selected and the summary given the keyboard on the next frame.
    pub(super) fn focus_message(&mut self, cx: &mut Context<Self>) {
        if self.selected() != Some(Selected::Wip) {
            self.select_wip(cx);
        }
        self.focus_summary = true;
        cx.notify();
    }

    /// The WIP row selected with `summary` in the commit area, as typed (the README's shot).
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
    pub(super) fn select_conflict(&mut self, before: Option<Wip>, cx: &mut Context<Self>) {
        let conflicted = |wip: Option<Wip>| wip.is_some_and(|w| w.conflicts > 0);
        if conflicted(self.wip) && !conflicted(before) && self.selected() != Some(Selected::Wip) {
            self.select_wip(cx);
        }
    }

    /// Each branch's lane colour in the loaded rows, by its pill's name and each ref's.
    pub(super) fn ref_colours(&self) -> HashMap<String, usize> {
        self.rows
            .iter()
            .flat_map(|r| r.row.refs.iter().flat_map(move |g| std::iter::once(g.name.clone()).chain(g.refs.iter().map(|l| l.name.clone())).map(move |n| (n, r.row.colour))))
            .collect()
    }

    pub(super) fn take_focus_summary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.focus_summary) && self.selected() == Some(Selected::Wip) {
            self.wip_panel.update(cx, |p, cx| p.focus_message(window, cx));
        }
    }
}

/// `n file conflicts in working directory` after the conflict mark, painted under `git-conflict-node`.
fn conflict_node(n: usize, hits: &HitTargets, palette: &Palette, cx: &App) -> Div {
    let words = if n == 1 { "1 file conflict in working directory".to_string() } else { format!("{n} file conflicts in working directory") };
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
        .child(hit_target(hits, "git-conflict-node".into()))
}

/// The field as the row shows it until the row is selected: a click on it selects the row.
fn still_field(summary: String, palette: &Palette, cx: &App) -> Div {
    let colors = cx.theme().colors();
    let (text, colour) = if summary.is_empty() { ("// WIP".to_string(), palette.muted) } else { (summary, palette.text) };
    div()
        .w(px(FIELD))
        .flex_none()
        .flex()
        .items_center()
        .h(px(26.))
        .px(px(8.))
        .rounded_sm()
        .border_1()
        .border_color(colors.border)
        .bg(colors.editor_background)
        .overflow_hidden()
        .whitespace_nowrap()
        .text_color(colour)
        .child(SharedString::from(text))
}
