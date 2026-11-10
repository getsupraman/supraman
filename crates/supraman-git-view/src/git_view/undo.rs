//! The Git tab's Undo and Redo through its [`WriteGate`], the journal's preview read off the UI thread after each refresh.

use gpui::{AppContext as _, ClickEvent, Context, Entity, EventEmitter, Render, SharedString, Window, div, prelude::*};
use supraman_gitman::Repo;
use supraman_gitman::write::journal::{self, Done, Preview, Refused};
use supraman_gitman::write::LockSet;
use supraman_ui::button;
use supraman_ui::confirm::Confirm;
use supraman_ui::hit::HitTargets;
use supraman_ui::icon::Icon;
use supraman_ui::tooltip::Tooltip;
use theme::ActiveTheme as _;

use super::write_gate::{WriteGate, Writes, Written};

pub(crate) const UNDO_KEY: &str = "git-undo";
pub(crate) const REDO_KEY: &str = "git-redo";

/// An undo or a redo ran: the toast's words, said as done or as why nothing was done.
pub(crate) enum UndoEvent {
    Done(String),
    Why(String),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Way {
    Undo,
    Redo,
}

pub(crate) struct UndoBar {
    hits: HitTargets,
    writes: Writes<Way, Way, Result<Done, Refused>>,
    preview: Preview,
    /// Bumped by each read, so only the newest preview lands.
    read: u64,
}

impl EventEmitter<UndoEvent> for UndoBar {}

impl UndoBar {
    pub(crate) fn new(gate: Entity<WriteGate>, hits: HitTargets, cx: &mut Context<Self>) -> UndoBar {
        UndoBar { hits, writes: Writes::new(gate, Self::written, cx), preview: Preview::default(), read: 0 }
    }

    #[cfg(test)]
    pub(crate) fn preview(&self) -> &Preview {
        &self.preview
    }

    /// Reads the preview again once the tab's writer is open.
    pub(crate) fn refresh(&mut self, repo: &Repo, cx: &mut Context<Self>) {
        self.read += 1;
        let (read, opening) = (self.read, self.writes.gate().update(cx, |g, cx| g.open(repo, cx)));
        let task = cx.background_spawn(async move {
            let writer = opening.await.ok_or_else(|| "the repository did not open".to_string())?;
            journal::preview(&writer).map_err(|e| e.to_string())
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                if this.read != read {
                    return;
                }
                match result {
                    Ok(preview) => this.preview = preview,
                    Err(why) => log::warn!("git tab: cannot read the undo journal: {why}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn undo(&mut self, cx: &mut Context<Self>) {
        if self.preview.can_undo() {
            self.run(Way::Undo, false, cx);
        }
    }

    pub(crate) fn redo(&mut self, cx: &mut Context<Self>) {
        if self.preview.can_redo() {
            self.run(Way::Redo, false, cx);
        }
    }

    fn run(&mut self, way: Way, stash: bool, cx: &mut Context<Self>) {
        let job = self.writes.keeping(|done| done, move |s| {
            Ok(match way {
                Way::Undo => journal::undo(s, stash),
                Way::Redo => journal::redo(s, stash),
            })
        });
        self.writes.request(way, LockSet::undo(), job, cx).ok();
    }

    fn written(&mut self, written: &Written, cx: &mut Context<Self>) {
        let Some(way) = self.writes.written(written) else { return };
        let Some(outcome) = self.writes.kept() else { return };
        match (written.result.clone(), outcome) {
            (Err(why), _) => cx.emit(UndoEvent::Why(why)),
            (Ok(()), Ok(done)) => cx.emit(UndoEvent::Done(done.toast)),
            (Ok(()), Err(Refused::WouldStash { files, action })) => {
                let verb = if way == Way::Undo { "undo" } else { "redo" };
                let text = format!("{files} changed files go to a stash before the {verb} of {action}. Go on?");
                self.writes.asking = Some(Confirm::ask(text, format!("Stash and {verb}"), way));
            }
            (Ok(()), Err(refused)) => cx.emit(UndoEvent::Why(refused.to_string())),
        }
        cx.notify();
    }

    /// The answer to the stash question; a no changes nothing.
    pub(crate) fn answer(&mut self, yes: bool, cx: &mut Context<Self>) {
        if let Some(way) = self.writes.answer(yes) {
            self.run(way, true, cx);
        }
        cx.notify();
    }

    #[cfg(test)]
    pub(crate) fn asking(&self) -> Option<&str> {
        self.writes.asked()
    }

    fn button(&self, key: &'static str, icon: Icon, caption: &'static str, enabled: bool, tooltip: String, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.writes.busy(cx);
        let (enabled, tooltip) = match busy {
            Some(why) => (false, why.to_string()),
            None => (enabled, tooltip),
        };
        let colors = cx.theme().colors();
        let fg = if enabled { colors.text } else { colors.text_muted };
        let button = button::toolbar(&self.hits, key, icon.svg(14., fg), false, fg, cx).child(SharedString::from(caption)).tooltip(Tooltip::text(tooltip));
        if !enabled {
            return button::dimmed(button);
        }
        button.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| match key {
            UNDO_KEY => this.undo(cx),
            _ => this.redo(cx),
        }))
    }
}

impl Render for UndoBar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (undo, redo) = (self.preview.can_undo(), self.preview.can_redo());
        let bar = div()
            .flex()
            .gap_1()
            .px_2()
            .py_1()
            .child(self.button(UNDO_KEY, Icon::Undo, "Undo", undo, self.preview.undo_tooltip(), cx))
            .child(self.button(REDO_KEY, Icon::Redo, "Redo", redo, self.preview.redo_tooltip(), cx));
        div().flex().flex_col().child(bar)
    }
}

impl UndoBar {
    pub(crate) fn band(&self, cx: &mut Context<Self>) -> Option<gpui::Div> {
        self.writes.band(&self.hits, "git-undo-stash", Self::answer, cx)
    }
}

#[cfg(test)]
mod tests;
