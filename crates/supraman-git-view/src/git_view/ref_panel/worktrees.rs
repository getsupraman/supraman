//! The WORKTREES section's own parts (GitKraken 12.6.2): a locked or prunable worktree's mark, each other
//! worktree's work in progress at its row's right, and the header's Refresh WIPs. Hit keys: `<list>-mark-<path>`,
//! `<list>-wip-<path>` (the summary), `<list>-wip-reading-<path>` (its spinner while the first read runs),
//! `refpanel-worktrees-refresh` (with `-turning` while a read runs).

use std::collections::HashMap;

use gpui::{AnyElement, ClickEvent, Context, SharedString, div, prelude::*, px};
use supraman_core::paths::path_key;
use supraman_gitman::Worktree;
use supraman_ui::button;
use supraman_ui::hit::hit_target;
use supraman_ui::icon::{self, Icon};
use supraman_ui::themes::secondary;
use supraman_ui::tooltip::Tooltip;
use theme::ActiveTheme as _;

use super::marks::MARK;
use super::{Entry, LIST, RefEvent, RefPanel};
use crate::git_view::worktree_rows::WorktreeView;

const ICON: f32 = 12.;

/// The other worktrees' work in progress as the tab last read it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RefWips {
    pub(crate) reading: bool,
    /// Whether a read has landed.
    pub(crate) landed: bool,
    /// The tab's own worktree's [`path_key`]: its WIP is the graph's WIP row.
    pub(crate) own: Option<String>,
    /// Each other worktree's words, by [`path_key`].
    pub(crate) said: HashMap<String, WorktreeView>,
}

/// What a worktree row shows at its right.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Summary {
    Reading,
    /// `2↑ 1↓`, then `+1 ✎2 −1`; either may be empty.
    Said { upstream: String, counts: String },
}

impl RefPanel {
    pub(crate) fn set_wips(&mut self, wips: RefWips, cx: &mut Context<Self>) {
        if self.wips != wips {
            self.wips = wips;
            cx.notify();
        }
    }

    #[cfg(test)]
    pub(crate) fn wips_reading(&self) -> bool {
        self.wips.reading
    }

    /// The worktree of the row at the panel's whole `path`.
    pub(crate) fn worktree(&self, path: &str) -> Option<&Worktree> {
        self.worktrees.get(path.strip_prefix("worktrees/")?.parse::<usize>().ok()?)
    }

    /// The summary of the worktree row at the panel's whole `path`: none for the tab's own.
    pub(crate) fn summary(&self, path: &str) -> Option<Summary> {
        let key = path_key(&self.worktree(path)?.path);
        if self.wips.own.as_deref() == Some(key.as_str()) {
            return None;
        }
        match self.wips.said.get(&key) {
            Some(view) if !view.upstream.is_empty() || !view.counts.is_empty() => Some(Summary::Said { upstream: view.upstream.clone(), counts: view.counts.clone() }),
            Some(_) => None,
            None => (self.wips.reading && !self.wips.landed).then_some(Summary::Reading),
        }
    }

    pub(super) fn summary_element(&self, list: &str, row: &str, whole: &str, cx: &mut Context<Self>) -> Option<AnyElement> {
        let muted = secondary(cx.theme().colors());
        let line = div().relative().flex().flex_none().items_center().gap_1p5().px_1().text_xs().whitespace_nowrap().text_color(muted);
        Some(match self.summary(whole)? {
            Summary::Reading => {
                let key = format!("{list}-wip-reading-{row}");
                line.child(icon::turning(Icon::Refresh.svg(ICON, muted), SharedString::from(key.clone()))).child(hit_target(&self.hits, key)).into_any_element()
            }
            Summary::Said { upstream, counts } => {
                let part = |words: String| (!words.is_empty()).then(|| div().child(SharedString::from(words)));
                line.children(part(upstream)).children(part(counts)).child(hit_target(&self.hits, format!("{list}-wip-{row}"))).into_any_element()
            }
        })
    }

    /// A worktree row's mark: prunable, else locked, each with git's reason as its tooltip.
    pub(super) fn worktree_mark(&self, key: String, whole: &str, cx: &mut Context<Self>) -> Option<AnyElement> {
        let w = self.worktree(whole)?;
        let (icon, colour, tip) = if w.prunable {
            (Icon::Warning, cx.theme().status().warning, words("Prunable", &w.prune_reason))
        } else if w.locked {
            (Icon::Lock, secondary(cx.theme().colors()), words("Locked", &w.lock_reason))
        } else {
            return None;
        };
        let mark = div().id(SharedString::from(key.clone())).relative().flex().items_center().justify_center().w(px(MARK)).child(icon.svg(ICON, colour)).child(hit_target(&self.hits, key));
        Some(mark.tooltip(Tooltip::text(tip)).into_any_element())
    }

    /// Refresh WIPs on the WORKTREES header while the repository has another worktree; it turns while a read runs.
    pub(super) fn refresh_wips(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.entries.iter().filter(|(_, e)| matches!(e, Entry::Worktree { .. })).count() < 2 {
            return None;
        }
        let muted = secondary(cx.theme().colors());
        let button = button::toolbar_turning(&self.hits, format!("{LIST}-worktrees-refresh"), Icon::Refresh.svg(ICON, muted), false, self.wips.reading, muted, cx)
            .h(px(20.))
            .min_w(px(20.))
            .px_0()
            .tooltip(Tooltip::text("Refresh WIPs"))
            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                cx.emit(RefEvent::RefreshWips);
            }));
        Some(button.into_any_element())
    }
}

fn words(what: &str, reason: &str) -> String {
    if reason.is_empty() { what.to_string() } else { format!("{what}: {reason}") }
}

#[cfg(test)]
mod tests;
