use std::path::{Path, PathBuf};

use supraman_claude::{find_transcript, now_ms};
use supraman_core::Settings;
use futures::StreamExt as _;
use gpui::{AnyElement, App, ClickEvent, Context, SharedString, div, prelude::*};
use theme::ActiveTheme as _;

use super::{FleetView, Tab, TabKind};
use crate::saved_tabs::{self, Reopen, SavedTab, SavedTabs, World};
use crate::{fleet, tab_card};

const SAVE_FAILED: &str = "Cannot save the open tabs";

type Planned = (Vec<(String, Result<Reopen, String>)>, Option<usize>, Option<String>);

impl FleetView {
    pub(super) fn tabs_writer(home: Option<PathBuf>, cx: &mut Context<Self>) -> futures::channel::mpsc::UnboundedSender<SavedTabs> {
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<SavedTabs>();
        let Some(home) = home else {
            return tx;
        };
        cx.spawn(async move |this, cx| {
            while let Some(mut tabs) = rx.next().await {
                while let Ok(newer) = rx.try_recv() {
                    tabs = newer;
                }
                let home = home.clone();
                let saved = cx.background_executor().spawn(async move { saved_tabs::save(&home, &tabs) }).await;
                let alive = this.update(cx, |this, cx| {
                    let before = this.saving.tab_notices.len();
                    this.saving.tab_notices.retain(|n| !n.starts_with(SAVE_FAILED));
                    if let Err(e) = saved {
                        this.saving.tab_notices.push(e);
                    }
                    if this.saving.tab_notices.len() != before || !this.saving.tab_notices.is_empty() {
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    return;
                }
            }
        })
        .detach();
        tx
    }

    /// The open tabs as the next start reopens them: exited ones are left out.
    pub fn saved_tabs(&self, cx: &App) -> SavedTabs {
        let mut saved = SavedTabs::default();
        for tab in self.tabs.iter().filter(|t| !t.exited) {
            let Some(one) = self.saved_tab(tab, cx) else { continue };
            if self.active == Some(tab.id) {
                saved.active = Some(saved.tabs.len());
            }
            saved.tabs.push(one);
        }
        saved
    }

    pub(super) fn saved_tab(&self, tab: &Tab, cx: &App) -> Option<SavedTab> {
        let title = tab.title.clone();
        let named = |sid: &str| self.sessions.iter().chain(&self.listing.closed).find(|s| s.session_id == sid).map(|s| s.name.clone()).filter(|n| !n.is_empty());
        Some(match &tab.kind {
            TabKind::Saved(saved) => saved.clone(),
            TabKind::Schedule(_) | TabKind::Details(_) | TabKind::Start(_) => return None,
            TabKind::Work(view) => SavedTab::Work { title, workspace: view.read(cx).workspace.clone() },
            TabKind::Git(view) => SavedTab::Git { title, folder: view.read(cx).folder.clone() },
            TabKind::Shell if tab.hand => {
                let session_id = tab.claude.clone().or_else(|| tab.launched_session.clone()).map(|sid| self.transcripts.continued.get(&sid).cloned().unwrap_or(sid));
                let name = session_id.as_deref().and_then(named).or_else(|| tab.session_name.clone()).unwrap_or_default();
                SavedTab::Claude { title, cwd: tab.cwd.clone()?, session_id, name }
            }
            TabKind::Shell => SavedTab::Shell { title, cwd: tab.cwd.clone()?, shell: tab.started.clone() },
            TabKind::Attach { session_id: id } => {
                let listed = self.sessions.iter().find(|s| &s.id == id);
                SavedTab::Attach {
                    title,
                    cwd: tab.cwd.clone()?,
                    id: id.clone(),
                    session_id: listed.map(|s| s.session_id.clone()).or_else(|| tab.launched_session.clone()),
                    name: listed.map(|s| s.name.clone()).or_else(|| tab.session_name.clone()).unwrap_or_default(),
                }
            }
        })
    }

    /// Called on every render, so every change to the tabs reaches the file; nothing is written while a quit closes them.
    pub(super) fn persist_tabs(&mut self, cx: &App) {
        let _stall = supraman_ui::stall::section("render: persist_tabs");
        if !self.saving.tabs_restored || self.saving.tabs_frozen || self.lifecycle.quit_asked || self.home.is_none() {
            return;
        }
        let now = self.saved_tabs(cx);
        if self.saving.tabs_saved.as_ref() != Some(&now) {
            self.saving.tabs_writes.unbounded_send(now.clone()).ok();
            self.saving.tabs_saved = Some(now);
        }
    }

    /// Once the first session list is in and the hand tokens of the last run are released.
    pub(super) fn reopen_tabs(&mut self, cx: &mut Context<Self>) {
        if self.saving.tabs_restored || self.saving.tabs_reopening || !self.polls.sessions_read || !self.lifecycle.hands_ready {
            return;
        }
        let Some(home) = self.home.clone() else {
            self.saving.tabs_restored = true;
            return;
        };
        self.saving.tabs_reopening = true;
        let sessions = self.sessions.clone();
        let workspaces: Vec<String> = self.workspaces.iter().map(|w| w.name.clone()).collect();
        let claude_dir = self.claude_dir.clone();
        let planned = cx.background_executor().spawn(async move {
            let settings = Settings::load(&home).unwrap_or_default().sessions;
            if !settings.reopen_tabs {
                return None;
            }
            let (saved, notice) = saved_tabs::load(&home);
            let plans = plan_tabs(saved.tabs, &sessions, &workspaces, claude_dir.as_deref(), settings.resume_tabs_on_click);
            Some((plans, saved.active, notice))
        });
        cx.spawn(async move |this, cx| {
            let planned = planned.await;
            this.update(cx, |this, cx| this.apply_reopen(planned, cx)).ok();
        })
        .detach();
    }

    fn apply_reopen(&mut self, planned: Option<Planned>, cx: &mut Context<Self>) {
        self.saving.tabs_reopening = false;
        self.saving.tabs_restored = true;
        let Some((plans, active, notice)) = planned else {
            cx.notify();
            return;
        };
        let before: Vec<u64> = self.tabs.iter().map(|t| t.id).collect();
        let active_before = self.active;
        let (mut opened, mut gone, mut active_id) = (Vec::new(), Vec::new(), None);
        for (i, (title, plan)) in plans.into_iter().enumerate() {
            match plan {
                Ok(reopen) => {
                    let id = self.open_reopened(reopen, cx);
                    if active == Some(i) {
                        active_id = Some(id);
                    }
                    opened.push(id);
                }
                Err(why) => gone.push(format!("{title} ({why})")),
            }
        }
        // the reopened tabs come first, then any opened while they loaded
        let order: Vec<u64> = opened.iter().chain(&before).copied().collect();
        self.tabs.sort_by_key(|t| order.iter().position(|id| *id == t.id));
        self.active = if before.is_empty() { active_id.or(opened.first().copied()) } else { active_before };
        self.saving.tab_notices.extend(notice);
        if !gone.is_empty() {
            let (n, tabs) = if gone.len() == 1 { (String::new(), "tab") } else { (format!("{} ", gone.len()), "tabs") };
            self.saving.tab_notices.push(format!("{n}{tabs} of the last run not reopened: {}", gone.join("; ")));
        }
        log::info!("reopened {} tabs, {} not", opened.len(), gone.len());
        cx.notify();
    }

    /// Ctrl+Alt+T: the last tab closed in this run, planned as a start reopens its tabs.
    pub(super) fn reopen_closed(&mut self, cx: &mut Context<Self>) {
        let Some(saved) = self.tab_keys.update(cx, |keys, _| keys.pop()) else {
            self.pane_toast.show_why("No closed tab to reopen.".into(), |v: &mut Self| &mut v.pane_toast, cx);
            return;
        };
        let home = self.home.clone();
        let sessions = self.sessions.clone();
        let workspaces: Vec<String> = self.workspaces.iter().map(|w| w.name.clone()).collect();
        let claude_dir = self.claude_dir.clone();
        let planned = cx.background_executor().spawn(async move {
            let resume_on_click = home.as_deref().and_then(|h| Settings::load(h).ok()).unwrap_or_default().sessions.resume_tabs_on_click;
            plan_tabs(vec![saved], &sessions, &workspaces, claude_dir.as_deref(), resume_on_click).pop()
        });
        cx.spawn(async move |this, cx| {
            let Some((title, plan)) = planned.await else { return };
            this.update(cx, |this, cx| match plan {
                Ok(reopen) => {
                    let id = this.open_reopened(reopen, cx);
                    this.activate_tab(id, cx);
                }
                Err(why) => this.pane_toast.show_why(format!("{title} not reopened: {why}"), |v: &mut Self| &mut v.pane_toast, cx),
            })
            .ok();
        })
        .detach();
    }

    fn open_reopened(&mut self, reopen: Reopen, cx: &mut Context<Self>) -> u64 {
        match reopen {
            Reopen::Shell { title, cwd, shell } => {
                let shell = shell.unwrap_or_else(|| self.default_shell().1);
                self.open_plain(title, Some(cwd), shell, cx)
            }
            Reopen::Attach { title, session } => self.attach_titled(title, &session, cx),
            Reopen::Resume { title, session } => self.resume_titled(title, &session, cx),
            Reopen::Work { workspace } => self.open_work(&workspace, cx),
            Reopen::Git { title, folder } => self.open_git(folder, title, cx),
            Reopen::Placeholder(saved) => {
                let session = saved.session();
                let id = self.push_tab(saved.title().to_string(), TabKind::Saved(saved.clone()), saved.cwd().map(Path::to_path_buf), cx);
                if let Some((tab, s)) = self.tab_mut(id).zip(session) {
                    tab.session = Some(tab_card::session_label(&s.name, &s.session_id));
                }
                id
            }
        }
    }

    /// A placeholder's Resume: in its place, attached when its session runs again by now.
    pub fn resume_saved(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.tabs.iter().position(|t| t.id == id) else { return };
        let TabKind::Saved(saved) = &self.tabs[index].kind else { return };
        let (title, Some(session)) = (saved.title().to_string(), saved.session()) else { return };
        let running = self.sessions.iter().find(|s| s.session_id == session.session_id && fleet::is_alive(&s.health(now_ms())));
        let new = match (running, &session.kind) {
            (Some(live), supraman_claude::Kind::Background) => {
                let live = live.clone();
                self.attach_titled(title, &live, cx)
            }
            (Some(_), _) => {
                self.pane_toast.show_why(format!("{title} runs in another terminal: resuming it here would run it twice."), |v: &mut Self| &mut v.pane_toast, cx);
                return;
            }
            (None, _) => self.resume_titled(title, &session, cx),
        };
        self.tabs.retain(|t| t.id != id);
        if let Some(from) = self.tabs.iter().position(|t| t.id == new) {
            let last = self.tabs.len() - 1;
            supraman_ui::tabs::reorder(&mut self.tabs, from, index.min(last));
        }
        self.active = Some(new);
        cx.notify();
    }

    pub(super) fn saved_placeholder(&self, id: u64, saved: &SavedTab, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let what = match saved {
            SavedTab::Attach { .. } => "This background session is stopped.",
            _ => "This claude session was open when Supraman closed.",
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_2()
            .bg(colors.editor_background)
            .text_color(colors.text_muted)
            .child(SharedString::from(format!("{what} Nothing runs until you resume it.")))
            .child(
                supraman_ui::button::action(&self.hits, format!("resume-tab-{id}"), "Resume", supraman_ui::button::Tone::Neutral, supraman_ui::button::Fill::Outline, cx)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.resume_saved(id, cx))),
            )
            .into_any_element()
    }

    /// For the selftest: every tab closes as a quit closes them, without touching the saved file.
    #[cfg(feature = "selftest")]
    pub fn close_tabs_unsaved(&mut self, cx: &mut Context<Self>) {
        self.saving.tabs_frozen = true;
        let ids: Vec<u64> = self.tabs.iter().map(|t| t.id).collect();
        ids.into_iter().for_each(|id| self.close_tab(id, cx));
    }

    /// For the selftest: a tab that runs nothing until resumed.
    #[cfg(feature = "selftest")]
    pub fn open_placeholder(&mut self, saved: SavedTab, cx: &mut Context<Self>) -> u64 {
        self.open_reopened(Reopen::Placeholder(saved), cx)
    }

    /// For the selftest: the tabs as a start reopens them.
    #[cfg(feature = "selftest")]
    pub fn restart_tabs(&mut self, cx: &mut Context<Self>) {
        self.saving.tabs_frozen = false;
        self.saving.tabs_restored = false;
        self.saving.tabs_saved = None;
        self.reopen_tabs(cx);
    }
}

/// Each tab followed to its newest session and planned against the sessions and folders now; blocking.
fn plan_tabs(
    tabs: Vec<SavedTab>,
    sessions: &[supraman_claude::Session],
    workspaces: &[String],
    claude_dir: Option<&Path>,
    resume_on_click: bool,
) -> Vec<(String, Result<Reopen, String>)> {
    let tabs: Vec<SavedTab> = match claude_dir {
        Some(dir) => tabs.into_iter().map(|t| t.followed(|sid| supraman_claude::newest_in(dir, sid))).collect(),
        None => tabs,
    };
    let has_transcript = |sid: &str, cwd: &Path| claude_dir.is_some_and(|dir| find_transcript(dir, sid, Some(cwd)).is_some());
    let world = World {
        sessions,
        now: now_ms(),
        workspaces,
        resume_on_click,
        is_dir: &|p| p.is_dir(),
        is_file: &|p| p.is_file(),
        has_transcript: &has_transcript,
    };
    tabs.iter().map(|t| (t.title().to_string(), saved_tabs::plan(t, &world))).collect()
}

pub(super) fn kind_of(saved: &SavedTab) -> Option<tab_card::Kind> {
    match saved {
        SavedTab::Claude { .. } => Some(tab_card::Kind::Hand),
        SavedTab::Attach { .. } => Some(tab_card::Kind::Attach),
        SavedTab::Shell { .. } | SavedTab::Work { .. } | SavedTab::Git { .. } => None,
    }
}
