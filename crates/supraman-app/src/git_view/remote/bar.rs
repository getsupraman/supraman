//! The toolbar's Pull split button (the default, and ▾ for each pull and the default's choice) and Push, each
//! dimmed with why it cannot act, and the strip of the call under way with its progress and Cancel. Hit keys:
//! `git-pull`, `git-pull-menu`, `git-push`, `git-remote-cancel`.

use gpui::{AnyElement, App, ClickEvent, Context, Div, Entity, SharedString, div, prelude::*};
use supraman_gitman::write::remote::PullChoice;
use supraman_ui::button;
use supraman_ui::icon::Icon;
use supraman_ui::tooltip::Tooltip;
use theme::ActiveTheme as _;

use super::{Button, RemoteActions, RemoteEvent};
use crate::git_view::GitView;

pub const PULL_KEY: &str = "git-pull";
pub const PULL_MENU_KEY: &str = "git-pull-menu";
pub const PUSH_KEY: &str = "git-push";
pub const CANCEL_KEY: &str = "git-remote-cancel";

pub fn words(choice: PullChoice) -> &'static str {
    match choice {
        PullChoice::Fetch => "Fetch all",
        PullChoice::FastForwardIfPossible => "Pull (fast-forward if possible)",
        PullChoice::FastForwardOnly => "Pull (fast-forward only)",
        PullChoice::Rebase => "Pull (rebase)",
    }
}

/// The Pull button's caption for the default `choice`.
pub fn caption(choice: PullChoice) -> &'static str {
    if choice == PullChoice::Fetch { "Fetch" } else { "Pull" }
}

pub fn key_of(choice: PullChoice) -> &'static str {
    match choice {
        PullChoice::Fetch => "fetch",
        PullChoice::FastForwardIfPossible => "ff",
        PullChoice::FastForwardOnly => "ff-only",
        PullChoice::Rebase => "rebase",
    }
}

impl GitView {
    pub(in crate::git_view) fn remote_event(&mut self, _: Entity<RemoteActions>, e: &RemoteEvent, cx: &mut Context<Self>) {
        match e {
            RemoteEvent::Wrote(toast) => {
                if let Some(toast) = toast {
                    self.toast.show(toast.clone(), |v: &mut GitView| &mut v.toast, cx);
                }
                self.refresh_after_write(cx);
            }
            RemoteEvent::Why(why) => self.toast.show_why(why.clone(), |v: &mut GitView| &mut v.toast, cx),
            RemoteEvent::Facts(facts) => {
                let facts = facts.clone();
                self.branch.update(cx, |b, _| b.set_remote_facts(facts));
                if let Some(writer) = self.remote.read(cx).writer().cloned() {
                    crate::git_view::background::join(self, &writer, cx);
                }
            }
        }
    }

    /// Why Pull and why Push cannot act now, each `None` while it can.
    pub fn remote_why_not(&self, cx: &App) -> [Option<String>; 2] {
        let remote = self.remote.read(cx);
        [remote.why_not(Button::Pull(None), cx), remote.why_not(Button::Push, cx)]
    }

    /// What the Pull button does, and each remote whose background fetches fail.
    pub fn pull_tooltip(&self, choice: PullChoice, cx: &Context<Self>) -> String {
        let failures = crate::git_view::background::failures(cx.entity_id(), cx);
        std::iter::once(words(choice).to_string()).chain(failures).collect::<Vec<_>>().join("
")
    }

    fn remote_button(&self, key: &'static str, icon: Icon, caption: SharedString, why_not: Option<String>, tooltip: String, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors();
        let fg = if why_not.is_none() { colors.text } else { colors.text_muted };
        let button = button::toolbar(&self.hits, key, icon.svg(14., fg), false, fg, cx).child(caption);
        if let Some(why) = why_not {
            return button::dimmed(button.tooltip(Tooltip::text(why))).into_any_element();
        }
        button
            .tooltip(Tooltip::text(tooltip))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| match key {
                PULL_KEY => this.remote.update(cx, |r, cx| r.pull_default(cx)),
                _ => this.remote.update(cx, |r, cx| r.push_head(cx)),
            }))
            .into_any_element()
    }

    /// Pull with its ▾, Push, and while a call runs its words, progress and Cancel.
    pub(in crate::git_view) fn remote_buttons(&self, cx: &mut Context<Self>) -> Div {
        let [pull_why, push_why] = self.remote_why_not(cx);
        let remote = self.remote.read(cx);
        let choice = remote.pull_choice();
        let caption = caption(choice);
        let push_tip = match remote.current().and_then(|c| remote.upstream(c)) {
            Some((r, b)) => format!("Push to {r}/{b}"),
            None => "Push, choosing its upstream first".into(),
        };
        let strip = remote.progress().map(|(doing, progress)| {
            let said = match progress {
                Some(p) => format!("{doing}: {} {}%", p.phase, p.percent),
                None => format!("{doing}…"),
            };
            let cancel = button::bordered(&self.hits, CANCEL_KEY, "Cancel", cx).on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.remote.update(cx, |r, cx| r.cancel(cx))));
            div().flex().items_center().gap_2().px_2().text_xs().text_color(cx.theme().colors().text_muted).child(said).child(cancel)
        });
        let colors = cx.theme().colors();
        let menu = button::toolbar(&self.hits, PULL_MENU_KEY, Icon::ChevronDown.svg(12., colors.text_muted), false, colors.text_muted, cx)
            .tooltip(Tooltip::text("Choose a pull, or the default the button does"))
            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, e: &gpui::MouseDownEvent, _, cx| {
                cx.stop_propagation();
                let at = e.position;
                this.remote.update(cx, |r, cx| r.open_pull_menu(at, cx));
            }));
        div()
            .flex()
            .items_center()
            .gap_1()
            .py_1()
            .child(self.remote_button(PULL_KEY, Icon::SortDown, caption.into(), pull_why, self.pull_tooltip(choice, cx), cx))
            .child(menu)
            .child(self.remote_button(PUSH_KEY, Icon::SortUp, "Push".into(), push_why, push_tip, cx))
            .children(strip)
    }
}
