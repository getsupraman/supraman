use std::{
    path::{Path, PathBuf},
    sync::atomic::Ordering,
};

use supraman_claude::{Claude, Session, now_ms};
use supraman_core::{Registry, Workspace};
use supraman_supervisor::remove_workspace;
use gpui::{AppContext as _, ClipboardItem, Context, Entity, Pixels, Point, Window};

use super::FleetView;
use super::menu::{Confirm, Confirmed, ContextMenu, MenuAction, MenuItem, MenuTarget, add_items, is_foremans, other_schedule_items, repo_items, schedule_items, schedules_items, session_items, workspace_items};
use crate::fleet::{self, SessionRow};
use super::workspace_dialog::{DialogEvent, Mode, WorkspaceDialog, supervised_elsewhere};
use crate::work_view;

pub struct Removing {
    pub requested_at: i64,
    pub wanted: Vec<String>,
}

pub(super) fn send_name_of(session_id: &str) -> Option<String> {
    use supraman_claude::peer;
    let live = peer::live_entries(&peer::sessions_dir()?, &peer::System);
    peer::send_name(&live, session_id)
}

fn registry(home: Option<&Path>) -> Result<Registry, String> {
    home.map(Registry::at).ok_or_else(|| "no Supraman home".to_string())
}

impl FleetView {
    pub fn add_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        if let Some(path) = self.pick_folder.take() {
            self.open_add(Mode::Parent(path), window, cx);
            return;
        }
        supraman_ui::folder_picker::pick_folder("Add as a Supraman workspace", window, cx, |this, path, window, cx| this.open_add(Mode::Parent(path), window, cx));
    }

    pub fn open_add_menu(&mut self, cx: &mut Context<Self>) {
        let position = self.hits.borrow().get("add-workspace").map(|h| h.bounds.bottom_left()).unwrap_or_default();
        self.menu = Some(ContextMenu { title: "Add workspace".into(), position, items: add_items() });
        cx.notify();
    }

    pub fn open_add(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        let (home, hits) = (self.home.clone(), self.hits.clone());
        let dialog = cx.new(|cx| WorkspaceDialog::new(mode, home, &self.workspaces, hits, window, cx));
        let events = cx.subscribe_in(&dialog, window, Self::on_add_event);
        self.adding = Some((dialog, events));
        cx.notify();
    }

    pub fn add_dialog(&self) -> Option<&Entity<WorkspaceDialog>> {
        self.adding.as_ref().map(|(d, _)| d)
    }

    /// The dialog keeps Enter and Esc after a click on one of its rows took the keys from its fields.
    pub(super) fn add_key(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(dialog) = self.add_dialog().cloned() else {
            return false;
        };
        match key {
            "enter" => dialog.update(cx, |d, cx| d.save(window, cx)),
            "escape" => self.close_add(window, cx),
            _ => return false,
        }
        true
    }

    pub fn close_add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.adding = None;
        self.focus.focus(window, cx);
        cx.notify();
    }

    fn on_add_event(&mut self, _: &Entity<WorkspaceDialog>, event: &DialogEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            DialogEvent::Cancel if self.dismiss_popup() => cx.notify(),
            DialogEvent::Cancel => self.close_add(window, cx),
            DialogEvent::Edit { edits, .. } if edits.is_empty() => self.close_add(window, cx),
            DialogEvent::Edit { workspace, edits } => self.request_edits(workspace, edits.clone(), cx),
            DialogEvent::Saved { workspace, taken } => {
                let name = workspace.name.clone();
                self.close_add(window, cx);
                self.collapsed.remove(&format!("workspace:{name}"));
                self.reveal_new(super::left::PendingReveal::Row(format!("ws:{name}")));
                self.put_workspace(workspace.clone());
                self.reload_workspaces(cx);
                self.show_toast(format!("Added workspace {name}.{}", supervised_elsewhere(taken)), cx);
            }
        }
    }

    /// Reads the registry off the UI thread after a write of this window; a poll read before it, or a reload started
    /// before a later one, is not applied (`registry_writes`).
    pub(crate) fn reload_workspaces(&mut self, cx: &mut Context<Self>) {
        let generation = self.registry_writes.fetch_add(1, Ordering::SeqCst) + 1;
        let registry = registry(self.home.as_deref());
        let read = cx.background_executor().spawn(async move { super::load_registry(registry) });
        cx.spawn(async move |this, cx| {
            let read = read.await;
            this.update(cx, |this, cx| {
                if this.registry_writes.load(Ordering::SeqCst) == generation {
                    this.apply_registry(read);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn apply_registry(&mut self, read: super::RegistryRead) {
        match read {
            Ok((workspaces, skipped)) => {
                if self.workspaces != workspaces {
                    self.transcripts_nudge.wake();
                }
                self.workspaces = workspaces;
                self.invalidate_tree();
                self.registry_skipped = skipped;
                self.registry_error = None;
            }
            Err(e) => self.registry_error = Some(e),
        }
        let names: Vec<String> = self.workspaces.iter().map(|w| w.name.clone()).collect();
        self.removing.retain(|name, _| names.contains(name));
    }

    /// `workspace` as just saved, in the list at once; the reload behind it brings the registry's own.
    pub(crate) fn put_workspace(&mut self, workspace: Workspace) {
        match self.workspaces.iter_mut().find(|w| w.name == workspace.name) {
            Some(w) => *w = workspace,
            None => self.workspaces.push(workspace),
        }
        self.invalidate_tree();
    }

    pub fn show_toast(&mut self, text: String, cx: &mut Context<Self>) {
        self.toast.show(text, |this: &mut Self| &mut this.toast, cx);
    }

    pub fn open_menu(&mut self, target: MenuTarget, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.menu = self.menu_for(&target, None, cx).map(|(title, items)| ContextMenu { title, position, items });
        cx.notify();
        let session_id = match &target {
            MenuTarget::Session { session_id, .. } => Some(session_id.clone()),
            MenuTarget::Workspace(w) => self.tree().workspaces.iter().find(|n| &n.name == w).and_then(|n| n.own.as_ref()).map(|r| r.session.session_id.clone()),
            MenuTarget::Repo { .. } | MenuTarget::Schedule { .. } | MenuTarget::Schedules(_) | MenuTarget::OtherSchedule(_) => None,
        };
        let (Some(menu), Some(session_id)) = (&self.menu, session_id) else {
            return;
        };
        let title = menu.title.clone();
        let lookup = cx.background_executor().spawn(async move { send_name_of(&session_id) });
        cx.spawn(async move |this, cx| {
            let Some(send_name) = lookup.await else {
                return;
            };
            this.update(cx, |this, cx| {
                if !this.menu.as_ref().is_some_and(|m| m.title == title && m.position == position) {
                    return;
                }
                if let Some((_, items)) = this.menu_for(&target, Some(send_name), cx)
                    && let Some(menu) = &mut this.menu
                {
                    menu.items = items;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn menu_for(&self, target: &MenuTarget, send_name: Option<String>, cx: &Context<Self>) -> Option<(String, Vec<MenuItem>)> {
        let tree = self.tree();
        match target {
            MenuTarget::Workspace(w) => tree.workspaces.iter().find(|n| &n.name == w).map(|node| {
                let items_of = |row: Option<&SessionRow>, repo: &str, send_name: Option<String>| {
                    row.map(|r| self.session_menu(w, repo, r, send_name, cx)).unwrap_or_default()
                };
                let own = items_of(node.own.as_ref(), "workspace", send_name);
                let relay = items_of(node.relay.as_ref(), "relay", None);
                (w.clone(), workspace_items(node, self.has_closed_background(w), own, relay))
            }),
            MenuTarget::Repo { workspace, repo } => tree
                .workspaces
                .iter()
                .find(|n| &n.name == workspace)
                .and_then(|n| n.repos.iter().find(|r| &r.name == repo))
                .map(|r| (format!("{workspace} · ./{repo}"), repo_items(workspace, r))),
            MenuTarget::Session { workspace, repo, session_id } => {
                let node = tree.workspaces.iter().find(|n| &n.name == workspace);
                node.into_iter()
                    .flat_map(|n| n.rows())
                    .find(|r| &r.session.session_id == session_id)
                    .map(|row| (fleet::display_name(&row.session).to_string(), self.session_menu(workspace, repo, row, send_name, cx)))
            }
            MenuTarget::Schedule { workspace, id } => {
                self.schedule_row(workspace, id).map(|row| (format!("{workspace} · {}", row.title()), schedule_items(row)))
            }
            MenuTarget::Schedules(workspace) => Some((format!("{workspace} · Schedules"), schedules_items(workspace))),
            MenuTarget::OtherSchedule(path) => self.others().find(path).map(|t| (t.path(), other_schedule_items(t))),
        }
    }

    pub(super) fn session_menu(&self, workspace: &str, repo: &str, row: &SessionRow, send_name: Option<String>, cx: &Context<Self>) -> Vec<MenuItem> {
        let claimed = self.claimed_item(workspace, &row.session.name, cx);
        let adopt = match self.adoption(workspace, repo, row) {
            Some(fleet::Adoption::Offer { name }) => Some(name),
            _ => None,
        };
        session_items(workspace, repo, row, self.resumable(row), claimed, send_name, self.deletable(row), adopt)
    }

    pub(crate) fn adoption(&self, workspace: &str, folder: &str, row: &SessionRow) -> Option<fleet::Adoption> {
        fleet::adoption(&self.workspaces, workspace, folder, row, &self.sessions, &self.desired)
    }

    pub(super) fn claimed_item(&self, workspace: &str, name: &str, cx: &gpui::App) -> Option<String> {
        if name.is_empty() {
            return None;
        }
        self.tabs.iter().find_map(|t| match &t.kind {
            super::TabKind::Work(view) if view.read(cx).workspace == workspace => view.read(cx).boards.values().find_map(|b| match b {
                work_view::BoardState::Ready(board) => board
                    .rows
                    .iter()
                    .filter_map(|r| r.claim.as_ref())
                    .find(|c| !c.expired && c.holder() == name)
                    .map(|c| c.item.id.clone()),
                _ => None,
            }),
            _ => None,
        })
    }

    pub fn run_action(&mut self, action: MenuAction, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            MenuAction::Start(w) => self.start_workspace(&w, cx),
            MenuAction::Stop(w) => self.stop_workspace(&w, cx),
            MenuAction::Work(w) => {
                self.open_work(&w, cx);
            }
            MenuAction::NewTask { workspace, folder, scheduled } => {
                self.open_start(&workspace, folder, scheduled, window, cx);
            }
            MenuAction::Rescan(w) => self.rescan_workspace(&w, window, cx),
            MenuAction::OpenTaskScheduler => self.open_folder(PathBuf::from("taskschd.msc"), cx),
            MenuAction::EditFolders(w) => self.edit_folders(&w, window, cx),
            MenuAction::AddFromFolder => self.add_workspace(window, cx),
            MenuAction::AddCustom => self.open_add(Mode::Custom, window, cx),
            MenuAction::Remove(w) => {
                let started = self.desired.get(&w).is_some_and(|d| d.0);
                let stop = if started { "Its supervision stops first (Stop), then " } else { "" };
                self.confirm = Some(Confirm::ask(
                    format!(
                        "Remove workspace {w} from Supraman? {stop}its registry entry and desired state go. No files are deleted, and no session is stopped beyond what Stop does."
                    ),
                    "Remove",
                    Confirmed::Remove(w),
                ));
            }
            MenuAction::OpenFolder(path) => self.open_folder(path, cx),
            MenuAction::OpenTerminal { cwd, place } => {
                self.open_terminal_here(cwd, &place, cx);
            }
            MenuAction::NewShell(shell) => {
                self.new_shell_of(&shell, cx);
            }
            MenuAction::NewClaude { cwd, title } => {
                self.new_claude(cwd, title, window, cx);
            }
            MenuAction::SetRepoEnabled { workspace, repo, enabled } => self.request_edit(&workspace, supraman_core::Edit::SetEnabled { folder: repo, enabled }, cx),
            MenuAction::Attach { workspace, repo, session } => {
                self.attach(&workspace, &repo, &session, window, cx);
            }
            MenuAction::Resume { workspace, repo, session } => {
                self.resume(&workspace, &repo, &session, window, cx);
            }
            MenuAction::StopSession(session) if is_foremans(&session) => self.stop_session(session, cx),
            MenuAction::StopSession(session) => {
                self.confirm = Some(Confirm::ask(
                    format!(
                        "{} ({}) was not launched by Supraman. Stop it? Its transcript stays and it can be resumed.",
                        if session.name.is_empty() { "This session" } else { &session.name },
                        session.id
                    ),
                    "Stop",
                    Confirmed::StopSession(session),
                ));
            }
            MenuAction::Adopt { workspace, folder, session, name } => {
                let label = if session.name.is_empty() { format!("session {}", session.id) } else { format!("{} ({})", session.name, session.id) };
                let stop = if fleet::is_alive(&session.health(now_ms())) { "It is stopped (stop, never rm), then its" } else { "Its" };
                let started = self.desired.get(&workspace).is_some_and(|d| d.0);
                let later = if started { String::new() } else { format!(" Workspace {workspace} is not started: the copy starts when you Start it.") };
                self.confirm = Some(Confirm::ask(
                    format!(
                        "Adopt {label} into {workspace}? {stop} conversation continues as {name}, supervised: a copy under a new id with a fresh Workman token. The original stays stopped and listed.{later}"
                    ),
                    "Adopt",
                    Confirmed::Adopt { workspace, folder, session, name },
                ));
            }
            MenuAction::ToggleStar(session_id) => self.toggle_star(&session_id, cx),
            MenuAction::Copy(text) => {
                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                self.show_toast(format!("Copied {text}"), cx);
            }
            MenuAction::ShowInWork { workspace, item_id } => {
                self.open_work_item(&workspace, &item_id, cx);
            }
            MenuAction::Delete(session) => self.ask_delete(session),
            MenuAction::DeleteClosed(workspace) => self.ask_delete_closed(workspace, cx),
            MenuAction::Details(MenuTarget::Schedule { workspace, id }) => {
                self.open_schedule(&workspace, &id, super::schedules::At::Top, cx);
            }
            MenuAction::Details(target) => {
                self.open_details(target, cx);
            }
            MenuAction::Approve { workspace, id } => {
                self.open_schedule(&workspace, &id, super::schedules::At::Approve, cx);
            }
            MenuAction::RunNow { workspace, id } => self.run_schedule_now(&workspace, &id, cx),
            MenuAction::EditSchedule { workspace, id } => self.open_schedule_edit(&workspace, &id, window, cx),
            MenuAction::DryRun { workspace, id } => self.dry_run_schedule(&workspace, &id, cx),
            MenuAction::SetPaused { workspace, id, paused } => self.set_schedule_paused(workspace, id, paused, cx),
            MenuAction::History { workspace, id } => self.schedule_history(&workspace, &id, cx),
            MenuAction::RemoveSchedule { workspace, id } => self.ask_remove_schedule(&workspace, &id, cx),
            tab @ (MenuAction::OpenGit(_) | MenuAction::CloseTab(_) | MenuAction::CloseOtherTabs(_) | MenuAction::CloseTabsRight(_) | MenuAction::ReopenClosed) => {
                self.run_tab_action(tab, cx);
            }
        }
        cx.notify();
    }

    pub fn answer_confirm(&mut self, yes: bool, cx: &mut Context<Self>) {
        if let Some(Confirm { action: Some(action), .. }) = self.confirm.take()
            && yes
        {
            match action {
                Confirmed::Remove(w) => self.remove_workspace(&w, cx),
                Confirmed::StopSession(session) => self.stop_session(session, cx),
                Confirmed::Adopt { workspace, folder, session, name } => self.adopt(workspace, folder, session, name, cx),
                Confirmed::Delete { session, discard } => self.delete_now(session, discard, cx),
                Confirmed::DeleteClosed { workspace, sessions, skipped } => self.delete_closed_now(workspace, sessions, skipped, cx),
                Confirmed::Edit { workspace, edits } => self.apply_edits(&workspace, edits, cx),
                Confirmed::RemoveSchedule { workspace, id } => self.remove_schedule(workspace, id, cx),
                Confirmed::Update => self.update_confirmed(cx),
            }
        }
        cx.notify();
    }

    /// Nothing on disk outside the Supraman home is touched.
    pub fn remove_workspace(&mut self, name: &str, cx: &mut Context<Self>) {
        let (started, wanted) = self.desired.get(name).cloned().unwrap_or_default();
        if started {
            self.stop_workspace(name, cx);
        }
        self.removing.insert(name.to_string(), Removing { requested_at: now_ms(), wanted });
        self.finish_removals(cx);
    }

    /// Only a snapshot taken after the Remove was asked counts.
    pub fn finish_removals(&mut self, cx: &mut Context<Self>) {
        let now = now_ms();
        let ready: Vec<String> = self
            .removing
            .iter()
            .filter(|(_, r)| {
                let alive = self.sessions.iter().any(|s| r.wanted.contains(&s.name) && fleet::is_alive(&s.health(now)));
                r.wanted.is_empty() || (self.polled_at > r.requested_at && !alive)
            })
            .map(|(name, _)| name.clone())
            .collect();
        for name in ready {
            self.removing.remove(&name);
            let home = self.home.clone();
            let task = cx.background_executor().spawn(async move {
                let home = home.ok_or_else(|| format!("{name}: no Supraman home"))?;
                remove_workspace(&home, &name).map_err(|e| format!("{name}: {e}"))?;
                Ok::<_, String>(name)
            });
            cx.spawn(async move |this, cx| {
                let done = task.await;
                this.update(cx, |this, cx| this.removed(done, cx)).ok();
            })
            .detach();
        }
        cx.notify();
    }

    fn removed(&mut self, done: Result<String, String>, cx: &mut Context<Self>) {
        match done {
            Ok(name) => {
                self.desired.remove(&name);
                self.invalidate_tree();
                self.relays.remove(&name);
                self.messages.remove(&name);
                if self.compose.as_ref().is_some_and(|c| c.workspace == name) {
                    self.compose = None;
                }
                self.workspaces.retain(|w| w.name != name);
                self.reload_workspaces(cx);
                self.show_toast(format!("Removed workspace {name}; no files were deleted."), cx);
            }
            Err(e) => self.action_error = Some(format!("Cannot remove: {e}")),
        }
        cx.notify();
    }

    fn open_folder(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let open = self.open_with;
        let task = cx.background_executor().spawn(async move { open(&path).map_err(|e| (path, e)) });
        cx.spawn(async move |this, cx| {
            if let Err((path, e)) = task.await {
                this.update(cx, |this, cx| {
                    this.action_error = Some(format!("Cannot open {}: {e}", path.display()));
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    /// The folder's session is wanted only once the original stopped: a copy of a live conversation goes on beside it.
    fn adopt(&mut self, workspace: String, folder: String, session: Session, name: String, cx: &mut Context<Self>) {
        let Some(home) = self.home.clone() else {
            return;
        };
        let alive = fleet::is_alive(&session.health(now_ms()));
        let (id, session_id) = (session.id.clone(), session.session_id.clone());
        let ws = workspace.clone();
        let task = cx.background_executor().spawn(async move {
            if alive {
                Claude::default().stop(&id).map_err(|e| anyhow::anyhow!("cannot stop {id}: {e}"))?;
            }
            crate::supervise::adopt(&home, &ws, &folder, &session_id)
        });
        cx.spawn(async move |this, cx| {
            let done = task.await;
            this.update(cx, |this, cx| {
                match done {
                    Ok(true) => this.show_toast(format!("{} stopped; it continues as {name} at the supervisor's next tick.", session.id), cx),
                    Ok(false) => this.show_toast(format!("{} stopped; it continues as {name} when you Start {workspace}.", session.id), cx),
                    Err(e) => this.action_error = Some(format!("Cannot adopt {}: {e:#}", session.id)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn stop_session(&mut self, session: Session, cx: &mut Context<Self>) {
        let id = session.id.clone();
        let task = cx.background_executor().spawn(async move { Claude::default().stop(&id) });
        cx.spawn(async move |this, cx| {
            let done = task.await;
            this.update(cx, |this, cx| {
                match done {
                    Ok(_) => this.show_toast(format!("Stopped {} ({}).", session.name, session.id), cx),
                    Err(e) => this.action_error = Some(format!("Cannot stop {}: {e}", session.id)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}
