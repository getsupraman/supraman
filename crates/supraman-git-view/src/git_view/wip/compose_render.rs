//! The commit area under the WIP panel's lists; a cherry-pick, revert or am shows only its steps, an external rebase only its Abort.

use gpui::{ClickEvent, Context, Div, SharedString, Stateful, div, prelude::*, px};
use supraman_gitman::OperationState;
use supraman_gitman::write::conflicts::Sequence;
use supraman_gitman::write::wip::Step;
use supraman_ui::banner;
use supraman_ui::button;
use supraman_ui::icon::Icon;
use supraman_ui::tooltip::Tooltip;
use theme::ActiveTheme as _;

use super::compose::{Composer, failure_banner, summary_row};
use super::render::live;
use super::actions::noun;
use super::{Ask, Shown, WipPanel};
use crate::git_view::keys::{MESSAGE_CONTEXT, StageAllAndCommit};

impl WipPanel {
    fn amend_row(&self, c: &Composer, cx: &mut Context<Self>) -> Stateful<Div> {
        div()
            .id("wip-amend-row")
            .flex()
            .items_center()
            .gap_2()
            .cursor_pointer()
            .child(button::checkbox(&self.hits, "wip-amend", c.amend, cx))
            .child("Amend previous commit")
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_amend(cx)))
    }

    /// The summary with its count of characters left, and the description.
    fn message_box(&self, c: &Composer, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_col()
            .gap_1()
            .key_context(MESSAGE_CONTEXT)
            .on_action(cx.listener(|this, _: &StageAllAndCommit, _, cx| this.commit(true, cx)))
            .child(summary_row(&self.hits, &c.summary, "wip-counter", cx))
            .child(c.description.clone())
    }

    fn failure(&self, c: &Composer, cx: &mut Context<Self>) -> Option<Div> {
        let why = c.failure.clone()?;
        Some(failure_banner(&self.hits, "wip-commit-failure", why, |this: &mut Self, cx| {
            if let Some(c) = &mut this.composer {
                c.failure = None;
            }
            cx.notify();
        }, cx))
    }

    /// Commit options, folded; Skip hooks only where the repo has commit hooks.
    fn options(&self, c: &Composer, cx: &mut Context<Self>) -> Option<Div> {
        if !self.hooks {
            return None;
        }
        let muted = cx.theme().colors().text_muted;
        let chevron = if c.options_open { Icon::ChevronDown } else { Icon::ChevronRight };
        let fold = button::ghost(&self.hits, "wip-commit-options", cx)
            .flex()
            .items_center()
            .gap_1()
            .px_1()
            .rounded_sm()
            .text_color(muted)
            .child(chevron.svg(12., muted))
            .child("Commit options")
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                if let Some(c) = &mut this.composer {
                    c.options_open = !c.options_open;
                }
                cx.notify();
            }));
        let skip = c.options_open.then(|| {
            div()
                .id("wip-skip-hooks-row")
                .flex()
                .items_center()
                .gap_2()
                .pl_5()
                .cursor_pointer()
                .child(button::checkbox(&self.hits, "wip-skip-hooks", c.skip_hooks, cx))
                .child("Skip hooks")
                .tooltip(Tooltip::text("Commit without running the repository's commit hooks (--no-verify)"))
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    if let Some(c) = &mut this.composer {
                        c.skip_hooks = !c.skip_hooks;
                    }
                    cx.notify();
                }))
        });
        Some(div().flex().flex_col().gap_1().child(fold).children(skip))
    }

    fn abort(&self, shown: &Shown, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let (step, words) = match shown.state {
            OperationState::Merge => (Step::AbortMerge, "Abort merge".to_string()),
            OperationState::Rebase { .. } => (Step::AbortRebase, "Abort rebase".to_string()),
            state => {
                let seq = Sequence::of(state)?;
                (Step::Abort(seq), format!("Abort {}", noun(seq)))
            }
        };
        let busy = self.busy(cx);
        Some(live(button::outline(&self.hits, "wip-abort", SharedString::from(words), cx.theme().status().error, cx), busy, cx, move |this, cx| this.ask(Ask::Step(step), cx)))
    }

    fn commit_bar(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let (words, act) = self.commit_button(false, cx)?;
        let created = cx.theme().status().created;
        let bar = button::outline(&self.hits, "wip-commit", SharedString::from(words.clone()), created, cx)
            .flex()
            .items_center()
            .justify_center()
            .gap_1()
            .h(px(32.))
            .bg(banner::tint(created))
            .child(Icon::GitCommit.svg(14., cx.theme().colors().text));
        let busy = self.busy(cx);
        Some(if act.is_none() && busy.is_none() { button::dimmed(bar) } else { live(bar, busy, cx, |this, cx| this.commit(false, cx)) })
    }

    pub(super) fn commit_area(&self, shown: &Shown, cx: &mut Context<Self>) -> Option<Div> {
        let c = self.composer.as_ref()?;
        let border = cx.theme().colors().border_variant;
        let area = div().flex().flex_col().flex_none().gap_2().p_2().border_t_1().border_color(border);
        let message = matches!(shown.state, OperationState::None | OperationState::Merge | OperationState::Rebase { external: false });
        let area = match shown.state {
            OperationState::None => area.child(self.amend_row(c, cx)),
            _ => area,
        };
        let area = if message { area.child(self.message_box(c, cx)).children(self.failure(c, cx)).children(self.options(c, cx)) } else { area };
        Some(area.children(self.abort(shown, cx)).children(self.commit_bar(cx)))
    }
}
