//! The Git tab's Undo and Redo: the journal's preview read off the UI thread after every refresh, each undo or
//! redo run through the tab's [`WriteGate`], and the question before work in progress goes to a stash.

use std::sync::{Arc, Mutex, PoisonError};

use gpui::{AppContext as _, ClickEvent, Context, Entity, EventEmitter, Render, SharedString, Subscription, Window, div, prelude::*};
use supraman_gitman::Repo;
use supraman_gitman::write::journal::{self, Done, Preview, Refused};
use supraman_gitman::write::{LockSet, Writer};
use supraman_ui::button;
use supraman_ui::confirm::Confirm;
use supraman_ui::hit::HitTargets;
use supraman_ui::icon::Icon;
use supraman_ui::tooltip::Tooltip;
use theme::ActiveTheme as _;

use super::write_gate::{WriteGate, Written};

pub const UNDO_KEY: &str = "git-undo";
pub const REDO_KEY: &str = "git-redo";

/// An undo or a redo ran: the toast's words, said as done or as why nothing was done.
pub enum UndoEvent {
    Done(String),
    Why(String),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Way {
    Undo,
    Redo,
}

type Outcome = Arc<Mutex<Option<Result<Done, Refused>>>>;

pub struct UndoBar {
    hits: HitTargets,
    gate: Entity<WriteGate>,
    writer: Option<Writer>,
    preview: Preview,
    /// Bumped by each read, so only the newest preview lands.
    read: u64,
    /// The ticket of the undo or redo under way.
    running: Option<u64>,
    /// The newest undo or redo asked for.
    way: Option<Way>,
    outcome: Outcome,
    stash: Option<Confirm<Way>>,
    _written: Subscription,
}

impl EventEmitter<UndoEvent> for UndoBar {}

impl UndoBar {
    pub fn new(gate: Entity<WriteGate>, hits: HitTargets, cx: &mut Context<Self>) -> UndoBar {
        let written = cx.subscribe(&gate, |this: &mut UndoBar, _, w: &Written, cx| this.written(w, cx));
        cx.observe(&gate, |_, _, cx| cx.notify()).detach();
        UndoBar { hits, gate, writer: None, preview: Preview::default(), read: 0, running: None, way: None, outcome: Arc::default(), stash: None, _written: written }
    }

    #[cfg(test)]
    pub fn preview(&self) -> &Preview {
        &self.preview
    }

    /// Reads the preview again; the first read opens the repo's writer.
    pub fn refresh(&mut self, repo: &Repo, cx: &mut Context<Self>) {
        self.read += 1;
        let (read, writer, repo) = (self.read, self.writer.clone(), repo.clone());
        let task = cx.background_spawn(async move {
            let writer = match writer {
                Some(w) => w,
                None => Writer::open(&repo).map_err(|e| e.to_string())?,
            };
            let preview = journal::preview(&writer).map_err(|e| e.to_string())?;
            Ok::<_, String>((writer, preview))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                if this.read != read {
                    return;
                }
                match result {
                    Ok((writer, preview)) => {
                        this.writer = Some(writer);
                        this.preview = preview;
                    }
                    Err(why) => log::warn!("git tab: cannot read the undo journal: {why}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn undo(&mut self, cx: &mut Context<Self>) {
        if self.preview.can_undo() {
            self.run(Way::Undo, false, cx);
        }
    }

    pub fn redo(&mut self, cx: &mut Context<Self>) {
        if self.preview.can_redo() {
            self.run(Way::Redo, false, cx);
        }
    }

    fn run(&mut self, way: Way, stash: bool, cx: &mut Context<Self>) {
        let Some(writer) = self.writer.clone() else { return };
        self.way = Some(way);
        let outcome = self.outcome.clone();
        let job = move |s: &supraman_gitman::write::Session<'_>| {
            let done = match way {
                Way::Undo => journal::undo(s, stash),
                Way::Redo => journal::redo(s, stash),
            };
            *outcome.lock().unwrap_or_else(PoisonError::into_inner) = Some(done);
            Ok(())
        };
        self.running = self.gate.update(cx, |g, cx| g.request(writer, LockSet::undo(), job, cx));
        cx.notify();
    }

    fn written(&mut self, written: &Written, cx: &mut Context<Self>) {
        if self.running != Some(written.ticket) {
            return;
        }
        self.running = None;
        let Some(outcome) = self.outcome.lock().unwrap_or_else(PoisonError::into_inner).take() else { return };
        match (written.result.clone(), outcome) {
            (Err(why), _) => cx.emit(UndoEvent::Why(why)),
            (Ok(()), Ok(done)) => cx.emit(UndoEvent::Done(done.toast)),
            (Ok(()), Err(Refused::WouldStash { files, action })) => {
                let way = self.way.unwrap_or(Way::Undo);
                let verb = if way == Way::Undo { "undo" } else { "redo" };
                let text = format!("{files} changed files go to a stash before the {verb} of {action}. Go on?");
                self.stash = Some(Confirm::ask(text, format!("Stash and {verb}"), way));
            }
            (Ok(()), Err(refused)) => cx.emit(UndoEvent::Why(refused.to_string())),
        }
        cx.notify();
    }

    /// The answer to the stash question; a no changes nothing.
    pub fn answer(&mut self, yes: bool, cx: &mut Context<Self>) {
        if let Some(way) = self.stash.take().and_then(|asked| asked.action).filter(|_| yes) {
            self.run(way, true, cx);
        }
        cx.notify();
    }

    #[cfg(test)]
    pub fn asking(&self) -> Option<&str> {
        self.stash.as_ref().map(|c| c.text.as_str())
    }

    fn button(&self, key: &'static str, icon: Icon, caption: &'static str, enabled: bool, tooltip: String, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.gate.read(cx).busy();
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
        let stash = self.stash.as_ref().map(|asked| asked.render(&self.hits, "git-undo-stash", |this: &mut Self, yes, _, cx| this.answer(yes, cx), cx).m_2());
        div().flex().flex_col().child(bar).children(stash)
    }
}
