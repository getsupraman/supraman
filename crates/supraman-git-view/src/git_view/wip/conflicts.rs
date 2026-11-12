//! The conflict panel: the WIP panel's header while an operation is under way, the conflicted files' buttons and their writes.

use supraman_core::text::count;
use gpui::{Context, Div, SharedString, Stateful, div, prelude::*, px};
use supraman_gitman::write::conflicts::{self, Marked, Sequence, Side};
use supraman_gitman::{OperationState, UnderWay};
use supraman_ui::button;
use supraman_ui::hit::hit_target;
use supraman_ui::icon::Icon;
use supraman_ui::tooltip::Tooltip;
use theme::ActiveTheme as _;

use super::actions::{Doing, Kept, index_write};
use super::compose::split;
use super::render::live;
use super::{Shown, WipPanel};
use crate::git_view::cells::{Face, pill};
use crate::git_view::colours::Palette;

/// `2 file conflicts in working directory`: the graph's conflict node and the conflict panel's header.
pub(crate) fn conflicts_sentence(n: usize) -> String {
    format!("{} in working directory", count(n, "file conflict", "file conflicts"))
}

/// The header's first line: what conflicts, or what is under way while nothing does.
pub(crate) fn headline(state: OperationState, conflicted: usize) -> String {
    let what = match state {
        OperationState::None => return conflicts_sentence(conflicted),
        OperationState::Rebase { external: true } => return "External rebase in progress".into(),
        OperationState::Merge => "Merge",
        OperationState::Rebase { .. } => "Rebase",
        state => match Sequence::of(state) {
            Some(Sequence::CherryPick) => "Cherry-pick",
            Some(Sequence::Revert) => "Revert",
            _ => "Patch",
        },
    };
    if conflicted > 0 { format!("{what} conflicts detected") } else { format!("{what} in progress") }
}

/// The header's second line: a verb, the incoming side, a joiner, the current side, and the step under way.
pub(crate) fn sides(state: OperationState, under_way: &UnderWay) -> Option<(&'static str, &'static str, String)> {
    let step = under_way.step.map_or_else(String::new, |(at, of)| format!("(commit {at} of {of})"));
    Some(match state {
        OperationState::Merge => ("merging", "into", String::new()),
        OperationState::Rebase { .. } => ("rebasing", "onto", step),
        OperationState::CherryPick { .. } => ("cherry-picking", "onto", String::new()),
        OperationState::Revert { .. } => ("reverting", "on", String::new()),
        OperationState::Am => ("applying", "on", step),
        OperationState::None => return None,
    })
}

impl WipPanel {
    pub(crate) fn take(&mut self, paths: Vec<String>, side: Side, cx: &mut Context<Self>) {
        self.request(Doing::Take(side), index_write(), move |s| conflicts::take(s, &paths, side), cx);
    }

    /// Mark resolved of `paths` (`None`: every conflicted file); asks first while one holds conflict markers, unless `anyway`.
    pub(crate) fn resolve(&mut self, paths: Option<Vec<String>>, anyway: bool, cx: &mut Context<Self>) {
        let which = paths.clone();
        let job = self.writes.keeping(Kept::Marked, move |s| conflicts::mark_resolved(s, which.as_deref(), anyway));
        self.request(Doing::Resolve { paths, anyway }, index_write(), job, cx);
    }

    pub(crate) fn unresolve(&mut self, paths: Vec<String>, cx: &mut Context<Self>) {
        self.request(Doing::Unresolve, index_write(), move |s| conflicts::mark_conflicted(s, &paths), cx);
    }

    /// The files the last mark resolved left conflicted for their markers.
    pub(super) fn markers_left(&self) -> Option<Vec<String>> {
        match self.writes.kept() {
            Some(Kept::Marked(Marked::Markers(files))) => Some(files),
            _ => None,
        }
    }

    /// A merge's message in an empty commit summary once the merge is shown.
    pub(super) fn fill_merge_message(&mut self, shown: &Shown, cx: &mut Context<Self>) {
        let Some(message) = shown.under_way.as_ref().and_then(|u| u.message.as_deref()).filter(|_| shown.state == OperationState::Merge) else { return };
        let Some(c) = &self.composer else { return };
        if self.pending_summary(cx).is_empty() {
            let (summary, description) = split(message);
            c.set(&summary, &description, cx);
        }
    }

    fn side_pill(&self, key: &'static str, name: &str, palette: &Palette) -> Div {
        let colour = self.ref_colours.get(name).copied().unwrap_or(self.colour);
        pill(format!("{key}-name"), Face::named(name.to_string()), colour, palette, &self.hits).relative().child(hit_target(&self.hits, key.into()))
    }

    /// The warning (or an external rebase's own icon), the headline, and the sides as the graph colours their pills.
    pub(super) fn conflict_header(&self, shown: &Shown, cx: &mut Context<Self>) -> Div {
        let palette = Palette::of(cx.theme());
        let status = cx.theme().status();
        let (icon, colour) = match shown.state {
            OperationState::Rebase { external: true } => (Icon::Resume, status.info),
            _ => (Icon::Warning, status.warning),
        };
        let title = div().id("wip-conflict-header").relative().child(headline(shown.state, shown.conflicted.len())).child(hit_target(&self.hits, "wip-conflict-header".into()));
        let first = div().flex().items_center().gap_1p5().child(icon.svg(14., colour)).child(title);
        let second = shown.under_way.as_ref().and_then(|u| Some((u, sides(shown.state, u)?))).map(|(u, (verb, joiner, step))| {
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_1p5()
                .text_color(palette.muted)
                .child(verb)
                .child(self.side_pill("wip-conflict-incoming", &u.incoming, &palette))
                .child(joiner)
                .child(self.side_pill("wip-conflict-current", &u.current, &palette))
                .when(!step.is_empty(), |d| d.child(SharedString::from(step)))
        });
        div().flex().flex_col().flex_none().items_center().gap_1().py_1p5().px_2().text_color(palette.text).child(first).children(second)
    }

    /// A conflicted file's conflict, and on hover Take current, Take incoming and Mark resolved as icons.
    pub(super) fn conflicted_tail(&mut self, path: &str, group: SharedString, cx: &mut Context<Self>) -> Div {
        let (hits, busy, shown) = (self.hits.clone(), self.busy(cx), self.shown());
        let muted = cx.theme().colors().text_muted;
        let words = shown.as_ref().and_then(|s| s.conflicts.get(path)).map_or("conflicted", |c| c.words());
        let (current, incoming) = shown.as_ref().and_then(|s| s.under_way.clone()).map_or_else(Default::default, |u| (u.current, u.incoming));
        let icon = |key: String, icon: Icon, tip: String, cx: &mut Context<WipPanel>| {
            let button = button::icon(&hits, key, icon.svg(12., muted), px(20.), cx);
            if busy.is_none() { button.tooltip(Tooltip::text(tip)) } else { button }
        };
        let named = |what: &str, name: &str| if name.is_empty() { format!("Take {what} whole") } else { format!("Take {what} ({name}) whole") };
        let mut buttons = Vec::new();
        for (side, which, glyph, name) in [(Side::Current, "current", Icon::Home, current), (Side::Incoming, "incoming", Icon::ArrowUpRight, incoming)] {
            let paths = vec![path.to_string()];
            let button = icon(format!("wip-take-{which}-{path}"), glyph, named(which, &name), cx);
            buttons.push(live(button, busy, cx, move |this, cx| this.take(paths.clone(), side, cx)));
        }
        let paths = vec![path.to_string()];
        let resolve = icon(format!("wip-resolve-{path}"), Icon::Check, "Mark resolved".into(), cx);
        buttons.push(live(resolve, busy, cx, move |this, cx| this.resolve(Some(paths.clone()), false, cx)));
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(div().text_xs().text_color(muted).child(words))
            .children(buttons.into_iter().map(|b| button::reveal(b, group.clone(), false)))
    }

    /// A resolved file's Mark Conflicted on hover, where the operation's commits say its stages.
    pub(super) fn resolved_tail(&mut self, path: &str, group: SharedString, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let shown = self.shown()?;
        if !matches!(shown.state, OperationState::Merge | OperationState::Rebase { .. } | OperationState::CherryPick { .. } | OperationState::Revert { .. }) {
            return None;
        }
        let (busy, paths) = (self.busy(cx), vec![path.to_string()]);
        let button = button::action(&self.hits, format!("wip-unresolve-{path}"), "Mark Conflicted", button::Tone::Neutral, button::Fill::Outline, cx);
        Some(button::reveal(live(button, busy, cx, move |this, cx| this.unresolve(paths.clone(), cx)), group, false))
    }

    /// Mark all resolved over the conflicted files: only those are staged.
    pub(super) fn resolve_all_button(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let busy = self.busy(cx);
        live(button::action(&self.hits, "wip-resolve-all", "Mark all resolved", button::Tone::Success, button::Fill::Outline, cx), busy, cx, |this, cx| this.resolve(None, false, cx))
    }
}

#[cfg(test)]
mod tests;
