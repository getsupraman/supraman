use std::{
    path::{Path, PathBuf},
    sync::atomic::Ordering,
};

use foreman_claude::{Claude, Session, now_ms};
use foreman_core::{CoreError, DEFAULT_DEPTH, Folder, Registry, Scan, Workspace, discover, rescan, validate_name};
use foreman_supervisor::DesiredState;
use gpui::{AppContext as _, ClipboardItem, Context, Entity, Pixels, Point, Subscription, Window};

use super::FleetView;
use super::menu::{Confirm, Confirmed, ContextMenu, MenuAction, MenuItem, MenuTarget, add_items, is_foremans, repo_items, session_items, workspace_items};
use crate::fleet::{self, SessionRow};
use foreman_ui::text_input::{InputEvent, TextInput};
use crate::work_view;

pub struct AddDraft {
    /// No folder: a workspace whose folders Edit folders… adds afterwards.
    pub empty: bool,
    pub root: PathBuf,
    pub name: Entity<TextInput>,
    pub repos: Vec<(Folder, bool)>,
    pub scanning: bool,
    pub error: Option<String>,
    pub hint: Option<String>,
    _name_events: Subscription,
}

pub struct Removing {
    pub requested_at: i64,
    pub wanted: Vec<String>,
}

fn name_char(c: char) -> Option<char> {
    match c {
        ' ' | '_' => Some('-'),
        c if c.is_ascii_alphanumeric() || c == '-' => Some(c.to_ascii_lowercase()),
        _ => None,
    }
}

impl AddDraft {
    pub fn new(root: PathBuf, window: &mut Window, cx: &mut Context<FleetView>) -> Self {
        let name = cx.new(|cx| TextInput::new("workspace name", Some(name_char), window, cx));
        let events = cx.subscribe_in(&name, window, |this: &mut FleetView, _, event: &InputEvent, window, cx| this.on_name_event(event, window, cx));
        name.update(cx, |name, cx| name.focus_all(window, cx));
        Self { empty: false, root, name, repos: Vec::new(), scanning: true, error: None, hint: None, _name_events: events }
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
    use foreman_claude::peer;
    let live = peer::live_entries(&peer::sessions_dir()?, &peer::System);
    peer::send_name(&live, session_id)
}

fn registry(home: Option<&Path>) -> Result<Registry, String> {
    home.map(Registry::at).ok_or_else(|| "no Foreman home".to_string())
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

impl FleetView {
    /// The window is activated after the picker returns, so the keys reach the review whatever Windows gave the activation to.
    pub fn add_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        if let Some(path) = self.pick_folder.take() {
            self.begin_add(path, window, cx);
            return;
        }
        let paths = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Add as a Foreman workspace".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            if let Some(path) = paths.into_iter().next() {
                this.update_in(cx, |this, window, cx| {
                    window.activate_window();
                    this.begin_add(path, window, cx);
                })
                .ok();
            }
        })
        .detach();
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
                        draft.error = if draft.repos.is_empty() {
                            Some(format!("No git repos found directly under {}.", draft.root.display()))
                        } else if renamed {
                            Some("The folder's name gives no valid workspace name: choose one.".into())
                        } else {
                            None
                        };
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
        let name = draft.name_text(cx);
        let empty = draft.empty;
        let result = (|| {
            if empty {
                validate_name(&name).map_err(|e| add_error(&e))?;
                let mut workspace = Workspace::new(name, PathBuf::new(), Vec::new());
                workspace.home = None;
                let registry = registry(self.home.as_deref())?;
                registry.check_new(&workspace).map_err(|e| add_error(&e))?;
                registry.save(&workspace, false).map_err(|e| add_error(&e))?;
                return Ok((workspace.name, Vec::new()));
            }
            if draft.repos.is_empty() {
                return Err(format!("No git repos found directly under {}.", draft.root.display()));
            }
            validate_name(&name).map_err(|e| add_error(&e))?;
            let folders = draft.repos.iter().map(|(r, on)| Folder { enabled: *on, ..r.clone() }).collect();
            let mut workspace = Workspace::new(name, draft.root.clone(), folders);
            workspace.scans.push(Scan::new(draft.root.clone(), DEFAULT_DEPTH));
            let registry = registry(self.home.as_deref())?;
            registry.check_new(&workspace).map_err(|e| add_error(&e))?;
            let taken = registry.claim_folders(&mut workspace, |_| true).map_err(|e| add_error(&e))?;
            registry.save(&workspace, false).map_err(|e| add_error(&e))?;
            Ok((workspace.name, taken))
        })();
        match result {
            Ok((name, taken)) => {
                self.adding = None;
                self.collapsed.remove(&format!("workspace:{name}"));
                self.reload_workspaces();
                self.show_toast(format!("Added workspace {name}.{}", supervised_elsewhere(&taken)), cx);
                if empty {
                    self.edit_folders(&name, window, cx);
                }
            }
            Err(e) => draft.error = Some(e),
        }
        cx.notify();
    }

    pub fn cancel_add(&mut self, cx: &mut Context<Self>) {
        self.adding = None;
        cx.notify();
    }

    pub(crate) fn reload_workspaces(&mut self) {
        self.registry_writes.fetch_add(1, Ordering::SeqCst);
        match super::load_registry(registry(self.home.as_deref())) {
            Ok((workspaces, skipped)) => {
                if self.workspaces != workspaces {
                    self.transcripts_nudge.wake();
                }
                self.workspaces = workspaces;
                self.registry_skipped = skipped;
                self.registry_error = None;
            }
            Err(e) => self.registry_error = Some(e),
        }
        let names: Vec<String> = self.workspaces.iter().map(|w| w.name.clone()).collect();
        self.removing.retain(|name, _| names.contains(name));
    }

    pub fn show_toast(&mut self, text: String, cx: &mut Context<Self>) {
        self.toast.show(text, |this: &mut Self| &mut this.toast, cx);
    }

    pub fn open_menu(&mut self, target: MenuTarget, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.menu = self.menu_for(&target, None, cx).map(|(title, items)| ContextMenu { title, position, items });
        cx.notify();
        let session_id = match &target {
            MenuTarget::Session { session_id, .. } => Some(session_id.clone()),
            MenuTarget::Workspace(w) => self.tree().workspaces.into_iter().find(|n| &n.name == w).and_then(|n| n.own).map(|r| r.session.session_id),
            MenuTarget::Repo { .. } => None,
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
                let can_task = node.started == Some(true) && self.relay_refusal(w).is_none();
                let items_of = |row: Option<&SessionRow>, repo: &str, send_name: Option<String>| {
                    row.map(|r| self.session_menu(w, repo, r, send_name, cx)).unwrap_or_default()
                };
                let own = items_of(node.own.as_ref(), "workspace", send_name);
                let relay = items_of(node.relay.as_ref(), "relay", None);
                (w.clone(), workspace_items(node, can_task, self.has_closed_background(w), own, relay))
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
        }
    }

    fn session_menu(&self, workspace: &str, repo: &str, row: &SessionRow, send_name: Option<String>, cx: &Context<Self>) -> Vec<MenuItem> {
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

    pub(super) fn claimed_item(&self, workspace: &str, name: &str, cx: &Context<Self>) -> Option<String> {
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
            MenuAction::Task(w) => self.open_compose(&w, None, window, cx),
            MenuAction::Rescan(w) => self.rescan_workspace(&w, cx),
            MenuAction::EditFolders(w) => self.edit_folders(&w, window, cx),
            MenuAction::AddFromFolder => self.add_workspace(window, cx),
            MenuAction::AddEmpty => self.begin_add_empty(window, cx),
            MenuAction::Remove(w) => {
                let started = self.desired.get(&w).is_some_and(|d| d.0);
                let stop = if started { "Its supervision stops first (Stop), then " } else { "" };
                self.confirm = Some(Confirm::ask(
                    format!(
                        "Remove workspace {w} from Foreman? {stop}its registry entry and desired state go. No files are deleted, and no session is stopped beyond what Stop does."
                    ),
                    "Remove",
                    Confirmed::Remove(w),
                ));
            }
            MenuAction::OpenFolder(path) => self.open_folder(path, cx),
            MenuAction::OpenTerminal { cwd, title } => {
                self.open_shell_in(cwd, title, window, cx);
            }
            MenuAction::NewClaude { cwd, title } => {
                self.new_claude(cwd, title, window, cx);
            }
            MenuAction::SetRepoEnabled { workspace, repo, enabled } => self.set_repo_enabled(&workspace, &repo, enabled),
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
                        "{} ({}) was not launched by Foreman. Stop it? Its transcript stays and it can be resumed.",
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
                        "Adopt {label} into {workspace}? {stop} conversation continues as {name}, supervised: a copy under a new id with a fresh WorkState token. The original stays stopped and listed.{later}"
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
            }
        }
        cx.notify();
    }

    /// Nothing on disk outside the Foreman home is touched.
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
            let result = (|| -> Result<(), String> {
                let home = self.home.as_deref().ok_or("no Foreman home")?;
                let desired = DesiredState::path(home, &name).map_err(|e| e.to_string())?;
                match std::fs::remove_file(&desired) {
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(format!("{}: {e}", desired.display())),
                    _ => {}
                }
                Registry::at(home).remove(&name).map_err(|e| e.to_string())
            })();
            match result {
                Ok(()) => {
                    self.desired.remove(&name);
                    self.relays.remove(&name);
                    self.messages.remove(&name);
                    if self.compose.as_ref().is_some_and(|c| c.workspace == name) {
                        self.compose = None;
                    }
                    self.reload_workspaces();
                    self.show_toast(format!("Removed workspace {name}; no files were deleted."), cx);
                }
                Err(e) => self.action_error = Some(format!("Cannot remove {name}: {e}")),
            }
        }
        cx.notify();
    }

    fn rescan_workspace(&mut self, name: &str, cx: &mut Context<Self>) {
        let Some(home) = self.home.clone() else {
            return;
        };
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
                        this.reload_workspaces();
                    }
                    Err(e) => this.action_error = Some(format!("Cannot rescan: {e}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn set_repo_enabled(&mut self, workspace: &str, repo: &str, enabled: bool) {
        let result = registry(self.home.as_deref()).and_then(|r| {
            r.update(workspace, |w| w.folders.iter_mut().filter(|r| r.name == repo).for_each(|r| r.enabled = enabled))
                .map_err(|e| e.to_string())
        });
        match result {
            Ok(_) => self.reload_workspaces(),
            Err(e) => self.action_error = Some(format!("Cannot change {workspace} / {repo}: {e}")),
        }
    }

    fn open_folder(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        // ShellExecuteW pumps its thread's messages: on the UI thread gpui re-enters inside the click's App borrow (m5/68).
        let open = self.open_with;
        let task = cx.background_executor().spawn(async move { open(&path).map_err(|code| (path, code)) });
        cx.spawn(async move |this, cx| {
            if let Err((path, code)) = task.await {
                this.update(cx, |this, cx| {
                    this.action_error = Some(format!("Cannot open {}: ShellExecuteW returned {code}", path.display()));
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    /// Stops it first (stop, never rm), and wants the folder's session only once it stopped: a copy of a live
    /// conversation would go on beside it.
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
    fn the_name_takes_only_what_a_workspace_name_allows() {
        use foreman_ui::text_input::filtered;
        assert_eq!(filtered("My Repo_2", name_char), ("my-repo-2".to_string(), None));
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
