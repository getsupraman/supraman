//! A tab or a session row dropped on a terminal pastes that session's SendMessage name there.

use supraman_ui::tabs::{DraggedSession, DraggedTab};
use gpui::{Context, Div, SharedString, div, prelude::*};
use theme::ActiveTheme as _;

use super::{FleetView, Tab, TabKind, manage::send_name_of};

#[derive(Debug, PartialEq, Eq)]
pub enum Source<'a> {
    /// A tab by its id, with the `session_id` it runs.
    Tab(u64, Option<&'a str>),
    Session(&'a str),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Dropped {
    Nothing,
    NoSession,
    /// The SendMessage name of this `session_id` is pasted.
    Lookup(String),
}

/// What a drop on the terminal of tab `target`, which runs `target_session`, does.
pub fn dropped(target: u64, target_session: Option<&str>, source: Source) -> Dropped {
    let (tab, session) = match source {
        Source::Tab(id, session) => (Some(id), session),
        Source::Session(session) => (None, Some(session)),
    };
    if tab == Some(target) || session.is_some_and(|s| Some(s) == target_session) {
        return Dropped::Nothing;
    }
    session.map_or(Dropped::NoSession, |s| Dropped::Lookup(s.to_string()))
}

impl FleetView {
    pub(super) fn tab_session_id(&self, tab: &Tab) -> Option<String> {
        match &tab.kind {
            TabKind::Attach { session_id } => self.sessions.iter().find(|s| &s.id == session_id).map(|s| s.session_id.clone()),
            TabKind::Shell => tab.claude.clone(),
            TabKind::Work(_) | TabKind::Schedule(_) | TabKind::Details(_) | TabKind::Start(_) | TabKind::Saved(_) => None,
        }
    }

    /// Over the terminal of `tab_id`, shown only while a payload it takes hovers it.
    pub(super) fn terminal_drop(&self, tab_id: u64, cx: &mut Context<Self>) -> Div {
        let own = self.tab(tab_id).and_then(|t| self.tab_session_id(t));
        let colors = cx.theme().colors();
        div()
            .invisible()
            .absolute()
            .top_0()
            .right_0()
            .bottom_0()
            .left_0()
            .bg(colors.drop_target_background)
            .border_2()
            .border_color(colors.drop_target_border)
            .drag_over::<DraggedTab>(|style, _, _, _| style.visible())
            .drag_over::<DraggedSession>(|style, _, _, _| style.visible())
            .can_drop(move |dragged, _, _| {
                if let Some(tab) = dragged.downcast_ref::<DraggedTab>() {
                    tab.id != tab_id
                } else if let Some(row) = dragged.downcast_ref::<DraggedSession>() {
                    own.as_deref() != Some(row.session_id.as_str())
                } else {
                    false
                }
            })
            .on_drop(cx.listener(move |this, dragged: &DraggedTab, _, cx| {
                let session = this.tab(dragged.id).and_then(|t| this.tab_session_id(t));
                this.drop_on_terminal(tab_id, Source::Tab(dragged.id, session.as_deref()), dragged.title.clone(), cx);
            }))
            .on_drop(cx.listener(move |this, dragged: &DraggedSession, _, cx| {
                this.drop_on_terminal(tab_id, Source::Session(&dragged.session_id), dragged.title.clone(), cx);
            }))
    }

    fn drop_on_terminal(&mut self, target: u64, source: Source, title: SharedString, cx: &mut Context<Self>) {
        let own = self.tab(target).and_then(|t| self.tab_session_id(t));
        match dropped(target, own.as_deref(), source) {
            Dropped::Nothing => {}
            Dropped::NoSession => self.show_pane_toast(format!("{title} runs no claude session: nothing pasted."), cx),
            Dropped::Lookup(session_id) => {
                let lookup = cx.background_executor().spawn(async move { send_name_of(&session_id) });
                cx.spawn(async move |this, cx| {
                    let name = lookup.await;
                    this.update(cx, |this, cx| match name {
                        Some(name) => {
                            if let Some(terminal) = this.tab(target).and_then(|t| t.terminal.clone()) {
                                terminal.update(cx, |t, _| t.paste(&name));
                            }
                        }
                        None => this.show_pane_toast(format!("{title} has no SendMessage name yet: nothing pasted."), cx),
                    })
                    .ok();
                })
                .detach();
            }
        }
    }

    fn show_pane_toast(&mut self, text: String, cx: &mut Context<Self>) {
        self.pane_toast.show_why(text, |this: &mut Self| &mut this.pane_toast, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drop_pastes_another_sessions_name_and_never_its_own() {
        assert_eq!(dropped(1, Some("s1"), Source::Tab(2, Some("s2"))), Dropped::Lookup("s2".into()));
        assert_eq!(dropped(1, None, Source::Session("s2")), Dropped::Lookup("s2".into()), "onto a plain shell");
        assert_eq!(dropped(1, Some("s1"), Source::Tab(1, Some("s1"))), Dropped::Nothing, "a tab onto its own terminal");
        assert_eq!(dropped(1, None, Source::Tab(1, None)), Dropped::Nothing, "a plain shell onto its own terminal");
        assert_eq!(dropped(1, Some("s1"), Source::Session("s1")), Dropped::Nothing, "a row onto the terminal running it");
        assert_eq!(dropped(1, Some("s1"), Source::Tab(2, Some("s1"))), Dropped::Nothing, "another tab on the same session");
        assert_eq!(dropped(1, Some("s1"), Source::Tab(2, None)), Dropped::NoSession, "a tab running no claude");
    }
}
