//! The commit area under the WIP panel's lists; a cherry-pick, revert or am shows only its steps, an external rebase only its Abort.

use supraman_ui::tokens;
use gpui::{ClickEvent, Context, Div, SharedString, div, prelude::*};
use supraman_gitman::OperationState;
use supraman_gitman::write::conflicts::Sequence;
use supraman_gitman::write::wip::Step;
use supraman_ui::controls::button;
use supraman_ui::hit::hit_target;
use supraman_ui::icon::Icon;
use theme::ActiveTheme as _;

use super::compose::{Composer, failure_banner, summary_row};
use super::render::live;
use super::actions::noun;
use super::{Ask, Shown, WipPanel};
use crate::git_view::keys::{MESSAGE_CONTEXT, StageAllAndCommit};
use supraman_ui::controls::checkbox::Checkbox;

pub(super) const AREA: &str = "wip-commit-area";

impl WipPanel {
    fn amend_row(&self, c: &Composer, cx: &mut Context<Self>) -> Checkbox {
        Checkbox::new(&self.hits, "wip-amend", c.amend).label("Amend previous commit").focus(&c.amend_focus).on_toggle(cx.listener(|this, _: &bool, _, cx| this.toggle_amend(cx)))
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
            .rounded(tokens::radius::S)
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
            let check = Checkbox::new(&self.hits, "wip-skip-hooks", c.skip_hooks).label("Skip hooks").tooltip("Commit without running the repository's commit hooks (--no-verify)").on_toggle(cx.listener(|this, _: &bool, _, cx| {
                if let Some(c) = &mut this.composer {
                    c.skip_hooks = !c.skip_hooks;
                }
                cx.notify();
            }));
            div().flex().pl_5().child(check)
        });
        Some(div().flex().flex_col().gap_1().child(fold).children(skip))
    }

    fn abort(&self, shown: &Shown, cx: &mut Context<Self>) -> Option<button::Button> {
        let (step, words) = match shown.state {
            OperationState::Merge => (Step::AbortMerge, "Abort merge".to_string()),
            OperationState::Rebase { .. } => (Step::AbortRebase, "Abort rebase".to_string()),
            state => {
                let seq = Sequence::of(state)?;
                (Step::Abort(seq), format!("Abort {}", noun(seq)))
            }
        };
        let busy = self.busy(cx);
        Some(live(button::Button::new(&self.hits, "wip-abort").label(SharedString::from(words)).tone(button::Tone::Danger).fill(button::Fill::Outline), busy, cx, move |this, cx| this.ask(Ask::Step(step), cx)))
    }

    fn commit_bar(&self, cx: &mut Context<Self>) -> Option<button::Button> {
        let (words, act) = self.commit_button(false, cx)?;
        let (tone, fill) = (button::Tone::Success, button::Fill::Solid);
        let bar = button::Button::new(&self.hits, "wip-commit").label(SharedString::from(words.clone())).tone(tone).fill(fill).size(button::Size::Medium).w_full().icon_after(Icon::GitCommit);
        let busy = self.busy(cx);
        Some(if act.is_none() && busy.is_none() { bar.dimmed_if(true) } else { live(bar, busy, cx, |this, cx| this.commit(false, cx)) })
    }

    pub(super) fn commit_area(&self, shown: &Shown, cx: &mut Context<Self>) -> Option<Div> {
        let c = self.composer.as_ref()?;
        let border = cx.theme().colors().border_variant;
        let area = div().relative().flex().flex_col().flex_none().gap_2().p_2().border_t(tokens::stroke::HAIRLINE).border_color(border).child(hit_target(&self.hits, AREA.into()));
        let area = match self.message_order(false, cx) {
            Some(order) => order.on(area),
            None => area,
        };
        let message = matches!(shown.state, OperationState::None | OperationState::Merge | OperationState::Rebase { external: false });
        let area = match shown.state {
            OperationState::None => area.child(self.amend_row(c, cx)),
            _ => area,
        };
        let area = if message { area.child(self.message_box(c, cx)).children(self.failure(c, cx)).children(self.options(c, cx)) } else { area };
        Some(area.children(self.abort(shown, cx)).children(self.commit_bar(cx)))
    }
}
