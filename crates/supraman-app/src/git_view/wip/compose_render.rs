//! The commit area under the WIP panel's lists: Amend previous commit, the summary with its counter, the
//! description, a failed commit's words, Commit options, a merge's or rebase's Abort, and the commit button.

use gpui::{ClickEvent, Context, Div, SharedString, Stateful, div, prelude::*, px};
use supraman_gitman::OperationState;
use supraman_gitman::write::wip::Step;
use supraman_ui::banner;
use supraman_ui::button;
use supraman_ui::hit::hit_target;
use supraman_ui::icon::Icon;
use supraman_ui::tooltip::Tooltip;
use theme::ActiveTheme as _;

use super::compose::{Composer, SUMMARY_TOOLTIP, counter_colour, left};
use super::render::live;
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

    /// The summary, its count of characters left (negative past 72, in the warning colour), and the description.
    fn message_box(&self, c: &Composer, cx: &mut Context<Self>) -> Div {
        let left = left(c.summary.read(cx).text());
        let colour = counter_colour(left, cx);
        let counter = div().id("wip-counter").flex_none().w(px(32.)).text_right().text_color(colour).child(SharedString::from(left.to_string())).tooltip(Tooltip::text(SUMMARY_TOOLTIP));
        div()
            .flex()
            .flex_col()
            .gap_1()
            .key_context(MESSAGE_CONTEXT)
            .on_action(cx.listener(|this, _: &StageAllAndCommit, _, cx| this.commit(true, cx)))
            .child(div().flex().items_center().gap_1().child(div().flex_1().min_w_0().child(c.summary.clone())).child(counter))
            .child(c.description.clone())
    }

    fn failure(&self, c: &Composer, cx: &mut Context<Self>) -> Option<Div> {
        let why = c.failure.clone()?;
        let close = button::icon(&self.hits, "wip-commit-failure-dismiss", Icon::Close.svg(12., cx.theme().colors().text_muted), px(20.), cx).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            if let Some(c) = &mut this.composer {
                c.failure = None;
            }
            cx.notify();
        }));
        Some(banner::tinted(cx.theme().status().error).child(banner::dismissable(div().relative().child(SharedString::from(why)).child(hit_target(&self.hits, "wip-commit-failure".into())), close)))
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
            OperationState::Merge => (Step::AbortMerge, "Abort merge"),
            OperationState::Rebase { .. } => (Step::AbortRebase, "Abort rebase"),
            _ => return None,
        };
        let busy = self.busy(cx);
        Some(live(button::outline(&self.hits, "wip-abort", words, cx.theme().status().error, cx), busy, cx, move |this, cx| this.ask(Ask::Step(step), cx)))
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
        Some(
            div()
                .flex()
                .flex_col()
                .flex_none()
                .gap_2()
                .p_2()
                .border_t_1()
                .border_color(border)
                .child(self.amend_row(c, cx))
                .child(self.message_box(c, cx))
                .children(self.failure(c, cx))
                .children(self.options(c, cx))
                .children(self.abort(shown, cx))
                .children(self.commit_bar(cx)),
        )
    }
}
