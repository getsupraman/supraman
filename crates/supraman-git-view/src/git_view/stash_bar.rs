//! The toolbar's Stash and Pop beside Undo and Redo, a stash row's right-click on the graph, and the stash
//! writes' outcome in the tab: the toast, the refresh and the commit summary they spend or fill.

use gpui::{AnyElement, App, ClickEvent, Context, Div, Entity, MouseButton, MouseDownEvent, SharedString, div, prelude::*};
use supraman_gitman::FileChange;
use supraman_ui::button;
use supraman_ui::icon::Icon;
use supraman_ui::tooltip::Tooltip;
use theme::ActiveTheme as _;

use super::GitView;
use super::ref_panel::Entry;
use super::stash::{StashAct, StashActions, StashEvent, StashRef};

pub(crate) const STASH_KEY: &str = "git-stash";
pub(crate) const POP_KEY: &str = "git-pop";
pub(crate) const NOTHING_TO_STASH: &str = "There are no local changes to stash";
pub(crate) const NOTHING_TO_POP: &str = "There is no stash to pop";

impl GitView {
    #[cfg(test)]
    pub(crate) fn stash_actions(&self) -> &Entity<StashActions> {
        &self.stash
    }

    fn dirty(&self) -> bool {
        self.wip.is_some_and(|w| w.total() > 0)
    }

    /// The newest stash, which Pop takes.
    pub(crate) fn top_stash(&self, cx: &App) -> Option<StashRef> {
        self.ref_panel.read(cx).entries().iter().find_map(|(_, e)| match e {
            Entry::Stash { sha, label, .. } => Some(StashRef { sha: sha.clone(), label: label.clone() }),
            _ => None,
        })
    }

    /// Every change stashed with the commit summary as its message.
    pub(crate) fn stash_all(&mut self, cx: &mut Context<Self>) {
        if !self.dirty() {
            return self.toast.show_why(NOTHING_TO_STASH.into(), |v: &mut GitView| &mut v.toast, cx);
        }
        let message = self.wip_panel.read(cx).pending_summary(cx);
        self.stash.update(cx, |s, cx| s.stash_all(message, cx));
    }

    pub(crate) fn pop_top(&mut self, cx: &mut Context<Self>) {
        match self.top_stash(cx) {
            Some(stash) => self.stash_act(StashAct::Pop, stash, cx),
            None => self.toast.show_why(NOTHING_TO_POP.into(), |v: &mut GitView| &mut v.toast, cx),
        }
    }

    /// `files` of the WIP stashed with the commit summary as its message, asked first.
    pub(super) fn stash_files(&mut self, files: Vec<FileChange>, cx: &mut Context<Self>) {
        let message = self.wip_panel.read(cx).pending_summary(cx);
        self.stash.update(cx, |s, cx| s.stash_files(files, message, cx));
    }

    /// A stash row's menu choice, from the ref panel or the graph.
    pub(crate) fn stash_act(&mut self, act: StashAct, stash: StashRef, cx: &mut Context<Self>) {
        let dirty = self.dirty();
        self.stash.update(cx, |s, cx| match act {
            StashAct::Apply => s.apply(stash, false, dirty, cx),
            StashAct::Pop => s.apply(stash, true, dirty, cx),
            StashAct::Delete => s.delete(vec![stash], cx),
            StashAct::EditMessage => s.edit_message(stash, cx),
        });
    }

    pub(super) fn stash_event(&mut self, _: Entity<StashActions>, e: &StashEvent, cx: &mut Context<Self>) {
        match e {
            StashEvent::Wrote(toast) => {
                if let Some(toast) = toast {
                    self.toast.show(toast.clone(), |v: &mut GitView| &mut v.toast, cx);
                }
                self.refresh_after_write(cx);
            }
            StashEvent::Why(why) => self.toast.show_why(why.clone(), |v: &mut GitView| &mut v.toast, cx),
            StashEvent::Applied(message) => self.wip_panel.update(cx, |p, cx| p.fill_summary(message, cx)),
            StashEvent::Stashed => self.wip_panel.update(cx, |p, cx| p.clear_summary(cx)),
            StashEvent::Picked(act, stash) => self.stash_act(*act, stash.clone(), cx),
        }
    }

    /// Why Stash and why Pop cannot act now, each `None` while it can.
    pub(crate) fn stash_why_not(&self, cx: &App) -> [Option<String>; 2] {
        let busy = self.stash.read(cx).busy(cx).map(str::to_string);
        let stash = busy.clone().or_else(|| (!self.dirty()).then(|| NOTHING_TO_STASH.to_string()));
        let pop = busy.or_else(|| self.top_stash(cx).is_none().then(|| NOTHING_TO_POP.to_string()));
        [stash, pop]
    }

    fn stash_button(&self, key: &'static str, icon: Icon, caption: &'static str, why_not: Option<String>, tooltip: String, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors();
        let fg = if why_not.is_none() { colors.text } else { colors.text_muted };
        let button = button::toolbar(&self.hits, key, icon.svg(14., fg), false, fg, cx).child(SharedString::from(caption));
        if let Some(why) = why_not {
            return button::dimmed(button.tooltip(Tooltip::text(why))).into_any_element();
        }
        button
            .tooltip(Tooltip::text(tooltip))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| match key {
                STASH_KEY => this.stash_all(cx),
                _ => this.pop_top(cx),
            }))
            .into_any_element()
    }

    /// Stash (every change, untracked files too) and Pop (the newest stash), each dimmed with why it cannot act.
    pub(super) fn stash_buttons(&self, cx: &mut Context<Self>) -> Div {
        let [stash_why, pop_why] = self.stash_why_not(cx);
        let top = self.top_stash(cx);
        let pop_tip = top.map_or_else(String::new, |s| format!("Pop \u{201c}{}\u{201d}", s.label));
        div()
            .flex()
            .gap_1()
            .py_1()
            .child(self.stash_button(STASH_KEY, Icon::Stash, "Stash", stash_why, "Stash every change, untracked files too".into(), cx))
            .child(self.stash_button(POP_KEY, Icon::ArrowUpRight, "Pop", pop_why, pop_tip, cx))
    }

    /// A graph cell of row `ix`, taking the right-click of a stash's row.
    pub(super) fn stash_row(&self, ix: usize, cell: AnyElement, cx: &mut Context<Self>) -> AnyElement {
        let Some(stash) = self.pages.read(cx).rows.get(ix).and_then(|r| r.stash()) else { return cell };
        let stash = StashRef { sha: stash.sha.clone(), label: stash.label() };
        div()
            .size_full()
            .on_mouse_down(MouseButton::Right, cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                let stash = stash.clone();
                this.stash.update(cx, |s, cx| s.open_menu(stash, e.position, cx));
            }))
            .child(cell)
            .into_any_element()
    }
}
