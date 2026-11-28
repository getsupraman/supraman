//! The tabs of the window: each a [`Tab`] of a [`TabKind`], and the helpers that find, mark and cycle them.

use std::path::PathBuf;

use gpui::{Context, Entity};
use supraman_claude::{Session, now_ms};
use supraman_runtime::nudge::Nudge;
use supraman_term::Terminal;

use super::{FleetView, schedules, details, start, term};
use crate::work_view::WorkView;

pub enum TabKind {
    Attach { session_id: String },
    Shell,
    Work(Entity<WorkView>),
    Git(Entity<crate::git_view::GitView>),
    Schedule(schedules::SchedulePage),
    Details(details::DetailsPage),
    Start(start::StartPage),
    /// Reopened from the last run, waiting for a click on Resume.
    Saved(crate::saved_tabs::SavedTab),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Detach {
    None,
    Pending,
    Detaching,
    /// The tab and its process stay: nothing is killed that could hold the session.
    Failed,
}

pub struct Tab {
    pub id: u64,
    pub title: String,
    pub kind: TabKind,
    pub terminal: Option<Entity<Terminal>>,
    pub error: Option<String>,
    pub exited: bool,
    pub detach: Detach,
    pub cwd: Option<PathBuf>,
    /// The branch at `cwd`, read off the UI thread ([`FleetView::read_tab_branches`]).
    pub branch: Option<String>,
    pub find: Option<term::Find>,
    pub program_title: Option<String>,
    pub session: Option<String>,
    pub hand: bool,
    pub workman: Option<String>,
    pub hand_key: Option<String>,
    pub restart_to: Option<String>,
    pub opened_at: i64,
    pub claude: Option<String>,
    /// The full uuid it was launched or attached with, and that session's name.
    pub launched_session: Option<String>,
    pub session_name: Option<String>,
    pub asks_schedule: Option<start::AsksSchedule>,
    pub started: Option<crate::saved_tabs::SavedShell>,
    pub(in crate::view) watchers: Vec<Nudge>,
}

impl Tab {
    pub fn new(id: u64, title: String, kind: TabKind, cwd: Option<PathBuf>) -> Self {
        Self {
            id,
            title,
            kind,
            terminal: None,
            error: None,
            exited: false,
            detach: Detach::None,
            cwd,
            branch: None,
            find: None,
            program_title: None,
            session: None,
            hand: false,
            workman: None,
            hand_key: None,
            restart_to: None,
            opened_at: now_ms(),
            claude: None,
            launched_session: None,
            session_name: None,
            asks_schedule: None,
            started: None,
            watchers: Vec::new(),
        }
    }

    pub(in crate::view) fn wake_watchers(&self) {
        self.watchers.iter().for_each(Nudge::wake);
    }

    pub fn label(&self) -> String {
        let mut title = self.program_title.clone().unwrap_or_else(|| self.title.clone());
        if self.exited {
            title.push_str(" (exited)");
        }
        match self.detach {
            Detach::Pending | Detach::Detaching => title.push_str(" (detaching…)"),
            Detach::Failed => title.push_str(" (detach failed)"),
            Detach::None => {}
        }
        title
    }
}

impl FleetView {
    pub(in crate::view) fn active_terminal(&self) -> Option<Entity<Terminal>> {
        self.active.and_then(|id| self.tab(id)).and_then(|t| t.terminal.clone())
    }

    pub fn tab(&self, id: u64) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == id)
    }

    pub(in crate::view) fn tab_mut(&mut self, id: u64) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    pub fn activate_tab(&mut self, id: u64, cx: &mut Context<Self>) {
        if self.tab(id).is_some() {
            self.active = Some(id);
            cx.notify();
        }
    }

    pub fn tab_runs(tab: &Tab, session: &Session) -> bool {
        match &tab.kind {
            TabKind::Attach { session_id } => *session_id == session.id,
            TabKind::Shell => tab.claude.as_deref() == Some(session.session_id.as_str()),
            TabKind::Saved(saved) => saved.session().is_some_and(|s| s.session_id == session.session_id),
            TabKind::Work(_) | TabKind::Git(_) | TabKind::Schedule(_) | TabKind::Details(_) | TabKind::Start(_) => false,
        }
    }

    pub fn is_marked(&self, session: &Session) -> bool {
        self.active.and_then(|id| self.tab(id)).is_some_and(|t| Self::tab_runs(t, session))
    }

    pub fn session_tab(&self, session: &Session) -> Option<u64> {
        self.tabs.iter().find(|t| Self::tab_runs(t, session)).map(|t| t.id)
    }

    pub(in crate::view) fn cycle_tab(&mut self, step: isize, cx: &mut Context<Self>) {
        let at = self.active.and_then(|id| self.tabs.iter().position(|t| t.id == id));
        if let Some(next) = supraman_ui::tabs::cycled(self.tabs.len(), at, step) {
            self.activate_tab(self.tabs[next].id, cx);
        }
    }
}

pub(in crate::view) fn stale_tabs(tabs: &[Tab], elsewhere: &[(String, Option<String>)]) -> Vec<u64> {
    tabs.iter()
        .filter(|t| t.hand && !t.exited)
        .filter(|t| t.hand_key.as_ref().is_some_and(|k| elsewhere.iter().any(|(n, _)| n == k)))
        .map(|t| t.id)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use supraman_claude::Session;

    use super::*;

    #[test]
    fn a_tab_runs_its_attached_id_or_the_claude_found_under_its_shell() {
        let session = Session {
            id: "abcd0001".into(),
            session_id: "abcd0001-0000".into(),
            name: "s".into(),
            cwd: PathBuf::from(r"C:\demo"),
            kind: supraman_claude::Kind::Interactive,
            state: None,
            status: None,
            waiting_for: None,
            pid: Some(5),
            started_at: None,
        };
        let tab = |kind, claude: Option<&str>| Tab { claude: claude.map(String::from), ..Tab::new(1, "t".into(), kind, None) };
        assert!(FleetView::tab_runs(&tab(TabKind::Attach { session_id: "abcd0001".into() }, None), &session));
        assert!(!FleetView::tab_runs(&tab(TabKind::Attach { session_id: "ffff0001".into() }, None), &session));
        assert!(FleetView::tab_runs(&tab(TabKind::Shell, Some("abcd0001-0000")), &session));
        assert!(!FleetView::tab_runs(&tab(TabKind::Shell, Some("abcd0001-ffff")), &session), "/clear gave the tab a new session");
        let placeholder = |sid: &str| {
            let saved = crate::saved_tabs::SavedTab::Claude { title: "c".into(), cwd: r"C:\demo".into(), session_id: Some(sid.into()), name: "s".into() };
            tab(TabKind::Saved(saved), None)
        };
        let full = Session { session_id: "abcd0001-0000-4000-8000-000000000001".into(), ..session.clone() };
        assert!(FleetView::tab_runs(&placeholder(&full.session_id), &full), "the row opens its placeholder, not a second tab");
        assert!(!FleetView::tab_runs(&placeholder("abcd0001-0000-4000-8000-000000000002"), &full));
        assert!(!FleetView::tab_runs(&tab(TabKind::Shell, None), &session), "a shell without claude runs nothing");
    }

    #[test]
    fn only_open_hand_tabs_holding_a_token_from_elsewhere_restart() {
        let tab = |id, hand, key: Option<&str>, exited| Tab {
            hand,
            hand_key: key.map(String::from),
            exited,
            ..Tab::new(id, format!("t{id}"), TabKind::Shell, None)
        };
        let tabs = [
            tab(1, true, Some("hand-a-1"), false),
            tab(2, true, Some("hand-a-2"), false),
            tab(3, true, Some("hand-a-3"), true),
            tab(4, false, Some("hand-a-4"), false),
            tab(5, true, None, false),
        ];
        let elsewhere = [
            ("hand-a-1".to_string(), Some("https://old.example".to_string())),
            ("hand-a-3".to_string(), None),
            ("hand-a-4".to_string(), None),
        ];
        assert_eq!(stale_tabs(&tabs, &elsewhere), [1], "2 is current, 3 exited, 4 is no hand tab, 5 runs plain");
        assert!(stale_tabs(&tabs, &[]).is_empty());
    }
}
