//! Managing workspaces from the window: Add (folder → the discovery `fm workspace add` runs → a review →
//! the registry), Remove (Stop first, then the registry entry and its desired state; no file is
//! deleted), Rescan, repo Enable/Disable, and what the context menu's other items do.

use std::{
    path::{Path, PathBuf},
    sync::atomic::Ordering,
};

use foreman_claude::{Claude, Session, now_ms};
use foreman_core::{CoreError, DEFAULT_DEPTH, Registry, Repo, Workspace, discover, rescan, validate_name};
use foreman_supervisor::DesiredState;
use gpui::{AppContext as _, ClipboardItem, Context, Entity, Pixels, Point, Subscription, Window};

use super::FleetView;
use super::menu::{Confirm, ContextMenu, MenuAction, MenuTarget, is_foremans, repo_items, session_items, workspace_items};
use crate::fleet;
use crate::text_input::{InputEvent, TextInput};
use crate::work_view;

/// The Add workspace review: what discovery found under `root`, each repo ticked to register it
/// enabled (unticked ones are registered disabled), and the name, a [`TextInput`] that takes only what
/// a workspace name can hold ([`name_char`]).
pub struct AddDraft {
    pub root: PathBuf,
    pub name: Entity<TextInput>,
    pub repos: Vec<(Repo, bool)>,
    pub scanning: bool,
    pub error: Option<String>,
    /// Why the last key typed into the name was not taken.
    pub hint: Option<String>,
    _name_events: Subscription,
}

/// A Remove waiting for its workspace's sessions to stop.
pub struct Removing {
    pub requested_at: i64,
    pub wanted: Vec<String>,
}

/// What a workspace name takes of a typed character: lowercase letters, digits and `-` as
/// [`validate_name`] wants, capitals lowered, a space as `-`.
fn name_char(c: char) -> Option<char> {
    match c {
        ' ' | '_' => Some('-'),
        c if c.is_ascii_alphanumeric() || c == '-' => Some(c.to_ascii_lowercase()),
        _ => None,
    }
}

impl AddDraft {
    /// The review of `root`, its name focused.
    pub fn new(root: PathBuf, window: &mut Window, cx: &mut Context<FleetView>) -> Self {
        let name = cx.new(|cx| TextInput::new("workspace name", Some(name_char), window, cx));
        let events = cx.subscribe_in(&name, window, |this: &mut FleetView, _, event: &InputEvent, window, cx| this.on_name_event(event, window, cx));
        name.update(cx, |name, cx| name.focus_all(window, cx));
        Self { root, name, repos: Vec::new(), scanning: true, error: None, hint: None, _name_events: events }
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

/// What an event of the name does to the review: Enter saves, Esc cancels, an edit clears the error and
/// the hint, a character a name cannot hold says so under the field.
pub fn name_event(event: &InputEvent, error: &mut Option<String>, hint: &mut Option<String>) -> NameKey {
    match event {
        InputEvent::Confirm => NameKey::Save,
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

fn registry(home: Option<&Path>) -> Result<Registry, String> {
    home.map(Registry::at).ok_or_else(|| "no Foreman home".to_string())
}

/// What the review says about an error: the user's own mistakes plainly, the rest with their cause.
fn add_error(e: &CoreError) -> String {
    match e {
        CoreError::AlreadyExists(name) => format!("The name {name} is taken: choose another."),
        CoreError::RootTaken { by, .. } => format!("This folder is already registered, as workspace {by}."),
        CoreError::InvalidName(_) => "Use lowercase letters, digits and '-' only for the name.".into(),
        other => other.to_string(),
    }
}

impl FleetView {
    /// Add workspace: a folder picker, then the review ([`AddDraft`]) in the left panel. The one entry
    /// point for every Add button; [`FleetView::pick_folder`] stands in for the picker (the selftest).
    /// The picker is modal on its own thread: once it returns, the window is activated and the review's
    /// name takes the focus, so the keys reach it whatever Windows gave the activation to (m5/26).
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

    /// The review of `root`: discovery runs off the UI thread (it walks the folder).
    pub fn begin_add(&mut self, root: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(c) = &mut self.compose {
            c.focused = false;
        }
        self.adding = Some(AddDraft::new(root.clone(), window, cx));
        cx.notify();
        let scan = cx.background_executor().spawn(async move {
            match discover(&root, None, DEFAULT_DEPTH) {
                // a folder name that slugs to nothing valid still gets its repos listed
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
                        draft.root = workspace.root;
                        draft.name.update(cx, |name, cx| name.set_text(&workspace.name, cx));
                        draft.repos = workspace.repos.into_iter().map(|r| (r, true)).collect();
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
            NameKey::Save => self.save_add(cx),
            // Esc closes an open menu or question first
            NameKey::Cancel if self.menu.is_some() || self.confirm.is_some() => {
                if self.menu.take().is_none() {
                    self.confirm = None;
                }
            }
            NameKey::Cancel => self.adding = None,
            NameKey::Edited => {}
        }
        if self.adding.is_none() {
            self.focus.focus(window, cx);
        }
        cx.notify();
    }

    /// Registers the reviewed workspace; a refusal (no repos, invalid or taken name, folder already
    /// registered) stays in the review.
    pub fn save_add(&mut self, cx: &mut Context<Self>) {
        let Some(draft) = &mut self.adding else {
            return;
        };
        if draft.scanning {
            draft.error = Some("Still reading the folder: Save once its repos are listed.".into());
            cx.notify();
            return;
        }
        let name = draft.name_text(cx);
        let result = (|| {
            if draft.repos.is_empty() {
                return Err(format!("No git repos found directly under {}.", draft.root.display()));
            }
            validate_name(&name).map_err(|e| add_error(&e))?;
            let repos = draft.repos.iter().map(|(r, on)| Repo { enabled: *on, ..r.clone() }).collect();
            let workspace = Workspace::new(name, draft.root.clone(), repos);
            let registry = registry(self.home.as_deref())?;
            registry.check_new(&workspace).map_err(|e| add_error(&e))?;
            registry.save(&workspace, false).map_err(|e| add_error(&e))?;
            Ok(workspace.name)
        })();
        match result {
            Ok(name) => {
                self.adding = None;
                self.collapsed.remove(&format!("workspace:{name}"));
                self.reload_workspaces();
                self.action_info = Some(format!("Added workspace {name}."));
            }
            Err(e) => draft.error = Some(e),
        }
        cx.notify();
    }

    pub fn cancel_add(&mut self, cx: &mut Context<Self>) {
        self.adding = None;
        cx.notify();
    }

    /// The registry as it is now, without waiting for the next poll.
    pub(crate) fn reload_workspaces(&mut self) {
        self.registry_writes.fetch_add(1, Ordering::SeqCst);
        match registry(self.home.as_deref()).and_then(|r| r.list().map_err(|e| e.to_string())) {
            Ok(workspaces) => {
                self.workspaces = workspaces;
                self.registry_error = None;
            }
            Err(e) => self.registry_error = Some(e),
        }
        let names: Vec<String> = self.workspaces.iter().map(|w| w.name.clone()).collect();
        self.removing.retain(|name, _| names.contains(name));
    }

    /// Opens the menu of the row under the pointer; nothing when the row is gone.
    pub fn open_menu(&mut self, target: MenuTarget, position: Point<Pixels>, cx: &mut Context<Self>) {
        let tree = self.tree();
        let menu = match &target {
            MenuTarget::Workspace(w) => tree.workspaces.iter().find(|n| &n.name == w).map(|node| {
                let can_task = node.started == Some(true) && self.relay_refusal(w).is_none();
                (w.clone(), workspace_items(node, can_task))
            }),
            MenuTarget::Repo { workspace, repo } => tree
                .workspaces
                .iter()
                .find(|n| &n.name == workspace)
                .and_then(|n| n.repos.iter().find(|r| &r.name == repo))
                .map(|r| (format!("{workspace} · ./{repo}"), repo_items(workspace, r))),
            MenuTarget::Session { workspace, repo, session_id } => {
                let node = tree.workspaces.iter().find(|n| &n.name == workspace);
                let rows = node.into_iter().flat_map(|n| n.supervised.iter().chain(n.repos.iter().flat_map(|r| &r.sessions)));
                rows.into_iter().find(|r| &r.session.session_id == session_id).map(|row| {
                    let claimed = self.claimed_item(workspace, &row.session.name, cx);
                    let name = if row.session.name.is_empty() { fleet::short_id(&row.session).to_string() } else { row.session.name.clone() };
                    (name, session_items(workspace, repo, row, self.resumable(row), claimed))
                })
            }
        };
        self.menu = menu.map(|(title, items)| ContextMenu { title, position, items });
        cx.notify();
    }

    /// The item `name` holds a claim on, as the workspace's open Work tab has read it (none without one).
    fn claimed_item(&self, workspace: &str, name: &str, cx: &Context<Self>) -> Option<String> {
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
        self.action_info = None;
        match action {
            MenuAction::Start(w) => self.start_workspace(&w, cx),
            MenuAction::Stop(w) => self.stop_workspace(&w, cx),
            MenuAction::Work(w) => {
                self.open_work(&w, cx);
            }
            MenuAction::Task(w) => self.open_compose(&w, None, cx),
            MenuAction::Rescan(w) => self.rescan_workspace(&w, cx),
            MenuAction::Remove(w) => {
                let started = self.desired.get(&w).is_some_and(|d| d.0);
                let stop = if started { "Its supervision stops first (Stop), then " } else { "" };
                self.confirm = Some(Confirm {
                    text: format!(
                        "Remove workspace {w} from Foreman? {stop}its registry entry and desired state go. No files are deleted, and no session is stopped beyond what Stop does."
                    ),
                    yes: "Remove".into(),
                    action: MenuAction::RemoveNow(w),
                });
            }
            MenuAction::RemoveNow(w) => self.remove_workspace(&w, cx),
            MenuAction::OpenFolder(path) => self.open_folder(&path),
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
                self.confirm = Some(Confirm {
                    text: format!(
                        "{} ({}) was not launched by Foreman. Stop it? Its transcript stays and it can be resumed.",
                        if session.name.is_empty() { "This session" } else { &session.name },
                        session.id
                    ),
                    yes: "Stop".into(),
                    action: MenuAction::StopSessionNow(session),
                });
            }
            MenuAction::StopSessionNow(session) => self.stop_session(session, cx),
            MenuAction::ToggleStar(session_id) => self.toggle_star(&session_id, cx),
            MenuAction::Copy(text) => {
                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                self.action_info = Some(format!("Copied {text}"));
            }
            MenuAction::ShowInWork { workspace, item_id } => {
                self.open_work_item(&workspace, &item_id, cx);
            }
        }
        cx.notify();
    }

    pub fn answer_confirm(&mut self, yes: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(confirm) = self.confirm.take()
            && yes
        {
            self.run_action(confirm.action, window, cx);
        }
        cx.notify();
    }

    /// Stop first when started; the entry and its desired state go once no session it wants is alive
    /// ([`FleetView::finish_removals`], after each poll). Nothing on disk outside the Foreman home is touched.
    pub fn remove_workspace(&mut self, name: &str, cx: &mut Context<Self>) {
        let (started, wanted) = self.desired.get(name).cloned().unwrap_or_default();
        if started {
            self.stop_workspace(name, cx);
        }
        self.removing.insert(name.to_string(), Removing { requested_at: now_ms(), wanted });
        self.finish_removals(cx);
    }

    /// Completes each Remove whose sessions are no longer alive in a snapshot taken after it was asked.
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
                    self.action_info = Some(format!("Removed workspace {name}; no files were deleted."));
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
            let (workspace, added, removed) = rescan(&registry.load(&name)?, DEFAULT_DEPTH)?;
            registry.save(&workspace, true)?;
            Ok::<_, CoreError>((name, added, removed))
        });
        cx.spawn(async move |this, cx| {
            let done = task.await;
            this.update(cx, |this, cx| {
                match done {
                    Ok((name, added, removed)) => {
                        let list = |v: &[String]| if v.is_empty() { "none".to_string() } else { v.join(", ") };
                        this.action_info = Some(format!("Rescanned {name}: added {}; gone {}.", list(&added), list(&removed)));
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
            r.update(workspace, |w| w.repos.iter_mut().filter(|r| r.name == repo).for_each(|r| r.enabled = enabled))
                .map_err(|e| e.to_string())
        });
        match result {
            Ok(_) => self.reload_workspaces(),
            Err(e) => self.action_error = Some(format!("Cannot change {workspace} / {repo}: {e}")),
        }
    }

    fn open_folder(&mut self, path: &Path) {
        if let Err(e) = std::process::Command::new("explorer.exe").arg(path).spawn() {
            self.action_error = Some(format!("Cannot open {}: {e}", path.display()));
        }
    }

    /// `claude stop <id>`, off the UI thread; the row changes at the next poll.
    fn stop_session(&mut self, session: Session, cx: &mut Context<Self>) {
        let id = session.id.clone();
        let task = cx.background_executor().spawn(async move { Claude::default().stop(&id) });
        cx.spawn(async move |this, cx| {
            let done = task.await;
            this.update(cx, |this, cx| {
                match done {
                    Ok(_) => this.action_info = Some(format!("Stopped {} ({}).", session.name, session.id)),
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
        use crate::text_input::filtered;
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
        assert_eq!(
            add_error(&CoreError::RootTaken { root: r"C:\x".into(), by: "y".into() }),
            "This folder is already registered, as workspace y."
        );
    }
}
