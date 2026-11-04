use std::{
    path::{Path, PathBuf},
    sync::atomic::Ordering,
};

use supraman_claude::{Claude, Session, now_ms};
use supraman_core::{CoreError, DEFAULT_DEPTH, Folder, Registry, Scan, Workspace, discover, name_char, rescan, validate_name};
use supraman_supervisor::remove_workspace;
use gpui::{AppContext as _, ClipboardItem, Context, Entity, Pixels, Point, Subscription, Window};

use super::FleetView;
use super::menu::{Confirm, Confirmed, ContextMenu, MenuAction, MenuItem, MenuTarget, add_items, is_foremans, other_schedule_items, repo_items, schedule_items, schedules_items, session_items, workspace_items};
use crate::fleet::{self, SessionRow};
use supraman_ui::text_input::{InputEvent, TextInput};
use crate::work_view;

pub struct AddDraft {
    /// No folder: a workspace whose folders Edit folders… adds afterwards.
    pub empty: bool,
    pub root: PathBuf,
    pub name: Entity<TextInput>,
    pub repos: Vec<(Folder, bool)>,
    pub scanning: bool,
    /// Save's registry write runs; the dialog closes when it lands.
    pub saving: bool,
    pub error: Option<String>,
    pub hint: Option<String>,
    _name_events: Subscription,
}

pub struct Removing {
    pub requested_at: i64,
    pub wanted: Vec<String>,
}

impl AddDraft {
    pub fn new(root: PathBuf, window: &mut Window, cx: &mut Context<FleetView>) -> Self {
        let name = cx.new(|cx| TextInput::new("workspace name", Some(name_char), window, cx));
        let events = cx.subscribe_in(&name, window, |this: &mut FleetView, _, event: &InputEvent, window, cx| this.on_name_event(event, window, cx));
        name.update(cx, |name, cx| name.focus_all(window, cx));
        Self { empty: false, root, name, repos: Vec::new(), scanning: true, saving: false, error: None, hint: None, _name_events: events }
    }

    pub fn name_text(&self, cx: &gpui::App) -> String {
        self.name.read(cx).text().to_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameKey {
    Save,
    Cancel,
    Edited,
}

pub fn name_event(event: &InputEvent, error: &mut Option<String>, hint: &mut Option<String>) -> NameKey {
    match event {
        InputEvent::Confirm | InputEvent::ConfirmBack => NameKey::Save,
        InputEvent::Cancel => NameKey::Cancel,
        InputEvent::Refused(c) => {
            *hint = Some(format!("'{c}' is not taken: a name is lowercase letters, digits and '-'."));
            NameKey::Edited
        }
        InputEvent::Edited => {
            *error = None;
            *hint = None;
            NameKey::Edited
        }
    }
}

pub(super) fn send_name_of(session_id: &str) -> Option<String> {
    use supraman_claude::peer;
    let live = peer::live_entries(&peer::sessions_dir()?, &peer::System);
    peer::send_name(&live, session_id)
}

fn registry(home: Option<&Path>) -> Result<Registry, String> {
    home.map(Registry::at).ok_or_else(|| "no Supraman home".to_string())
}

fn add_error(e: &CoreError) -> String {
    match e {
        CoreError::AlreadyExists(name) => format!("The name {name} is taken: choose another."),
        CoreError::InvalidName(_) => "Use lowercase letters, digits and '-' only for the name.".into(),
        other => other.to_string(),
    }
}

/// The toast's tail for the folders [`Registry::claim_folders`] disabled.
fn supervised_elsewhere(taken: &[(String, String)]) -> String {
    let mut by: Vec<(&str, Vec<&str>)> = Vec::new();
    for (folder, owner) in taken {
        match by.iter_mut().find(|(o, _)| o == owner) {
            Some((_, list)) => list.push(folder),
            None => by.push((owner, vec![folder])),
        }
    }
    by.iter()
        .map(|(owner, list)| {
            let (names, verb) = match list.as_slice() {
                [one] => (one.to_string(), "stays"),
                [rest @ .., last] => (format!("{} and {last}", rest.join(", ")), "stay"),
                [] => unreachable!(),
            };
            format!(" {names} {verb} supervised in {owner}, disabled here.")
        })
        .collect()
}

/// Add workspace's Save: a new workspace of `repos` under `root` (none when `empty`), checked and claimed under the registry.
fn add(registry: Registry, name: String, empty: bool, root: PathBuf, repos: Vec<Folder>) -> Result<(Workspace, Vec<(String, String)>), String> {
    if empty {
        validate_name(&name).map_err(|e| add_error(&e))?;
        let mut workspace = Workspace::new(name, PathBuf::new(), Vec::new());
        workspace.home = None;
        registry.check_new(&workspace).map_err(|e| add_error(&e))?;
        registry.save(&workspace, false).map_err(|e| add_error(&e))?;
        return Ok((workspace, Vec::new()));
    }
    validate_name(&name).map_err(|e| add_error(&e))?;
    let mut workspace = Workspace::new(name, root.clone(), repos);
    workspace.scans.push(Scan::new(root, DEFAULT_DEPTH));
    registry.check_new(&workspace).map_err(|e| add_error(&e))?;
    let taken = registry.claim_folders(&mut workspace, |_| true).map_err(|e| add_error(&e))?;
    registry.save(&workspace, false).map_err(|e| add_error(&e))?;
    Ok((workspace, taken))
}

impl FleetView {
    pub fn add_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        if let Some(path) = self.pick_folder.take() {
            self.begin_add(path, window, cx);
            return;
        }
        supraman_ui::folder_picker::pick_folder("Add as a Supraman workspace", window, cx, |this, path, window, cx| this.begin_add(path, window, cx));
    }

    pub fn open_add_menu(&mut self, cx: &mut Context<Self>) {
        let position = self.hits.borrow().get("add-workspace").map(|h| h.bounds.bottom_left()).unwrap_or_default();
        self.menu = Some(ContextMenu { title: "Add workspace".into(), position, items: add_items() });
        cx.notify();
    }

    pub fn begin_add_empty(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editing = None;
        let mut draft = AddDraft::new(PathBuf::new(), window, cx);
        (draft.empty, draft.scanning) = (true, false);
        self.adding = Some(draft);
        cx.notify();
    }

    pub fn begin_add(&mut self, root: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.editing = None;
        self.adding = Some(AddDraft::new(root.clone(), window, cx));
        cx.notify();
        let scan = cx.background_executor().spawn(async move {
            match discover(&root, None, DEFAULT_DEPTH) {
                Err(CoreError::InvalidName(_)) => discover(&root, Some("workspace"), DEFAULT_DEPTH).map(|w| (w, true)),
                other => other.map(|w| (w, false)),
            }
        });
        cx.spawn(async move |this, cx| {
            let found = scan.await;
            this.update(cx, |this, cx| {
                let Some(draft) = &mut this.adding else {
                    return;
                };
                draft.scanning = false;
                match found {
                    Ok((workspace, renamed)) => {
                        draft.root = workspace.home().map(Path::to_path_buf).unwrap_or_default();
                        draft.name.update(cx, |name, cx| name.set_text(&workspace.name, cx));
                        draft.repos = workspace.folders.into_iter().map(|r| (r, true)).collect();
                        draft.error = renamed.then(|| "The folder's name gives no valid workspace name: choose one.".into());
                    }
                    Err(e) => draft.error = Some(add_error(&e)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn toggle_add_repo(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some((_, on)) = self.adding.as_mut().and_then(|d| d.repos.get_mut(index)) {
            *on = !*on;
            cx.notify();
        }
    }

    fn on_name_event(&mut self, event: &InputEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = &mut self.adding else {
            return;
        };
        match name_event(event, &mut draft.error, &mut draft.hint) {
            NameKey::Save => self.save_add(window, cx),
            NameKey::Cancel if self.dismiss_popup() => {}
            NameKey::Cancel => self.adding = None,
            NameKey::Edited => {}
        }
        if self.adding.is_none() {
            self.focus.focus(window, cx);
        }
        cx.notify();
    }

    pub fn save_add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = &mut self.adding else {
            return;
        };
        if draft.scanning {
            draft.error = Some("Still reading the folder: Save once its repos are listed.".into());
            cx.notify();
            return;
        }
        if draft.saving {
            return;
        }
        let (name, empty, root) = (draft.name_text(cx), draft.empty, draft.root.clone());
        let repos: Vec<Folder> = draft.repos.iter().map(|(r, on)| Folder { enabled: *on, ..r.clone() }).collect();
        draft.saving = true;
        let registry = registry(self.home.as_deref());
        let save = cx.background_executor().spawn(async move { add(registry?, name, empty, root, repos) });
        cx.spawn_in(window, async move |this, cx| {
            let result = save.await;
            this.update_in(cx, |this, window, cx| {
                match result {
                    Ok((workspace, taken)) => {
                        let name = workspace.name.clone();
                        if this.adding.as_ref().is_some_and(|d| d.saving) {
                            this.adding = None;
                        }
                        this.collapsed.remove(&format!("workspace:{name}"));
                        this.reveal_new(super::left::PendingReveal::Row(format!("ws:{name}")));
                        this.put_workspace(workspace);
                        this.reload_workspaces(cx);
                        this.show_toast(format!("Added workspace {name}.{}", supervised_elsewhere(&taken)), cx);
                        if empty {
                            this.edit_folders(&name, window, cx);
                        }
                    }
                    Err(e) => {
                        if let Some(draft) = &mut this.adding {
                            draft.saving = false;
                            draft.error = Some(e);
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub fn cancel_add(&mut self, cx: &mut Context<Self>) {
        self.adding = None;
        cx.notify();
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
            MenuAction::Rescan(w) => self.rescan_workspace(&w, cx),
            MenuAction::OpenTaskScheduler => self.open_folder(PathBuf::from("taskschd.msc"), cx),
            MenuAction::EditFolders(w) => self.edit_folders(&w, window, cx),
            MenuAction::AddFromFolder => self.add_workspace(window, cx),
            MenuAction::AddEmpty => self.begin_add_empty(window, cx),
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
                Confirmed::Edit { workspace, edit } => self.apply_edit(&workspace, edit, cx),
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

    fn rescan_workspace(&mut self, name: &str, cx: &mut Context<Self>) {
        let Some(home) = self.home.clone() else {
            return;
        };
        self.scan_other_schedules(cx);
        let name = name.to_string();
        let task = cx.background_executor().spawn(async move {
            let registry = Registry::at(home);
            let (mut workspace, added, removed) = rescan(&registry.load(&name)?)?;
            registry.claim_folders(&mut workspace, |f| added.contains(&f.name))?;
            registry.save(&workspace, true)?;
            Ok::<_, CoreError>((name, added, removed))
        });
        cx.spawn(async move |this, cx| {
            let done = task.await;
            this.update(cx, |this, cx| {
                match done {
                    Ok((name, added, removed)) => {
                        let list = |v: &[String]| if v.is_empty() { "none".to_string() } else { v.join(", ") };
                        this.show_toast(format!("Rescanned {name}: added {}; gone {}.", list(&added), list(&removed)), cx);
                        this.reload_workspaces(cx);
                    }
                    Err(e) => this.action_error = Some(format!("Cannot rescan: {e}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_saves_the_workspace_with_its_folders_claimed_and_refuses_a_taken_name() {
        let tmp = tempfile::tempdir().unwrap();
        let registry = Registry::at(tmp.path());
        let repo = |n: &str| Folder::new(n, tmp.path().join("root").join(n));
        let (first, taken) = add(Registry::at(tmp.path()), "one".into(), false, tmp.path().join("root"), vec![repo("alpha")]).unwrap();
        assert_eq!((first.folders.len(), taken.len()), (1, 0));
        assert_eq!(registry.load("one").unwrap(), first, "the value shown at once is the one saved");

        let (second, taken) = add(Registry::at(tmp.path()), "two".into(), false, tmp.path().join("root"), vec![repo("alpha")]).unwrap();
        assert_eq!(taken.len(), 1, "alpha stays supervised in one");
        assert!(!second.folders[0].enabled);

        let e = add(Registry::at(tmp.path()), "one".into(), false, tmp.path().join("root"), vec![repo("beta")]).unwrap_err();
        assert_eq!(e, "The name one is taken: choose another.");
        let e = add(Registry::at(tmp.path()), "Bad Name".into(), false, tmp.path().join("root"), Vec::new()).unwrap_err();
        assert_eq!(e, "Use lowercase letters, digits and '-' only for the name.", "a folder without subfolders is no error");
        let (empty, _) = add(Registry::at(tmp.path()), "three".into(), true, PathBuf::new(), Vec::new()).unwrap();
        assert_eq!((empty.folders.len(), empty.home.is_none()), (0, true));
    }

    #[test]
    fn the_name_takes_only_what_a_workspace_name_allows() {
        use supraman_ui::text_input::filtered;
        assert_eq!(filtered("My-Repo2", name_char), ("my-repo2".to_string(), None));
        assert_eq!(filtered("a b", name_char), ("ab".to_string(), Some(' ')));
        assert_eq!(filtered("a_b", name_char), ("ab".to_string(), Some('_')));
        assert_eq!(filtered("aä7", name_char), ("a7".to_string(), Some('ä')));
        assert_eq!(filtered("ä", name_char), (String::new(), Some('ä')));
    }

    #[test]
    fn the_name_says_why_a_character_is_not_taken_until_the_next_edit() {
        let (mut error, mut hint) = (Some("e".to_string()), None);
        assert_eq!(name_event(&InputEvent::Refused('ä'), &mut error, &mut hint), NameKey::Edited);
        assert_eq!(hint.as_deref(), Some("'ä' is not taken: a name is lowercase letters, digits and '-'."));
        assert_eq!(error.as_deref(), Some("e"));
        name_event(&InputEvent::Edited, &mut error, &mut hint);
        assert_eq!((error, hint), (None, None));
        assert_eq!(name_event(&InputEvent::Confirm, &mut None, &mut None), NameKey::Save);
        assert_eq!(name_event(&InputEvent::Cancel, &mut None, &mut None), NameKey::Cancel);
    }

    #[test]
    fn errors_read_as_what_to_do() {
        assert_eq!(add_error(&CoreError::AlreadyExists("x".into())), "The name x is taken: choose another.");
        assert_eq!(supervised_elsewhere(&[]), "");
        let taken = [("Alpha".to_string(), "larchsys".to_string()), ("Beta".to_string(), "larchsys".to_string())];
        assert_eq!(supervised_elsewhere(&taken), " Alpha and Beta stay supervised in larchsys, disabled here.");
        assert_eq!(supervised_elsewhere(&taken[..1]), " Alpha stays supervised in larchsys, disabled here.");
    }
}
