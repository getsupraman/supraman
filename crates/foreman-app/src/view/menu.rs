//! The left panel's right-click menu: what a workspace, repo or session row offers, as [`MenuAction`]s
//! that [`FleetView::run_action`] carries out. One menu at a time; Esc, a click outside it, or choosing an
//! item closes it. Every item is also reachable without the menu where it already was.

use std::path::PathBuf;

use foreman_claude::{Kind, Session};
use gpui::{AnyElement, ClickEvent, Context, MouseDownEvent, Pixels, Point, SharedString, anchored, deferred, div, prelude::*, px};
use theme::ActiveTheme as _;

use super::{FleetView, hit_target};
use crate::fleet::{self, RepoNode, SessionRow, WorkspaceNode};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuAction {
    Start(String),
    Stop(String),
    Work(String),
    Task(String),
    Rescan(String),
    /// Asks first ([`Confirm`]); [`MenuAction::RemoveNow`] once confirmed.
    Remove(String),
    RemoveNow(String),
    OpenFolder(PathBuf),
    OpenTerminal { cwd: PathBuf, title: String },
    NewClaude { cwd: PathBuf, title: String },
    SetRepoEnabled { workspace: String, repo: String, enabled: bool },
    Attach { workspace: String, repo: String, session: Session },
    Resume { workspace: String, repo: String, session: Session },
    /// A session Foreman did not launch asks first; [`MenuAction::StopSessionNow`] stops it.
    StopSession(Session),
    StopSessionNow(Session),
    Copy(String),
    ShowInWork { workspace: String, item_id: String },
    /// Stars or unstars a session by its `session_id` (m5/27).
    ToggleStar(String),
    /// Asks first; [`MenuAction::DeleteNow`] deletes (m5/38, [`super::delete`]).
    Delete(Session),
    /// `discard`: the `--discard-unpushed` value rm printed, only after its own second question.
    DeleteNow { session: Session, discard: Option<String> },
    /// "Delete closed background sessions…" of a workspace: asks with the plan first.
    DeleteClosed(String),
    DeleteClosedNow { workspace: String, sessions: Vec<Session>, skipped: Vec<(String, String)> },
    /// The OK of a report shown in the confirm box: nothing to do.
    Dismiss,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem {
    /// Unique within the menu; painted under `menu-<key>`.
    pub key: &'static str,
    pub label: String,
    pub action: MenuAction,
}

/// What the right-click was on, resolved against the tree when the menu opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuTarget {
    Workspace(String),
    Repo { workspace: String, repo: String },
    /// By `session_id`, which every row has (an interactive one has no 8-hex `id`).
    Session { workspace: String, repo: String, session_id: String },
}

pub struct ContextMenu {
    pub title: String,
    pub position: Point<Pixels>,
    pub items: Vec<MenuItem>,
}

/// A question before an action that cannot be taken back: Remove…, Stop on a session that is not Foreman's.
pub struct Confirm {
    pub text: String,
    pub yes: String,
    pub action: MenuAction,
}

fn item(key: &'static str, label: impl Into<String>, action: MenuAction) -> MenuItem {
    MenuItem { key, label: label.into(), action }
}

/// `closed_background`: the workspace has a background session with no process ([`super::delete::bulk_plan`]).
pub fn workspace_items(node: &WorkspaceNode, can_task: bool, closed_background: bool) -> Vec<MenuItem> {
    let w = &node.name;
    let mut items = vec![if node.started == Some(true) {
        item("stop", "Stop", MenuAction::Stop(w.clone()))
    } else {
        item("start", "Start", MenuAction::Start(w.clone()))
    }];
    items.push(item("work", "Work", MenuAction::Work(w.clone())));
    if can_task {
        items.push(item("task", "Task…", MenuAction::Task(w.clone())));
    }
    items.extend([
        item("rescan", "Rescan repos", MenuAction::Rescan(w.clone())),
        item("open-folder", "Open folder", MenuAction::OpenFolder(node.root.clone())),
        item("open-terminal", "Open terminal here", MenuAction::OpenTerminal { cwd: node.root.clone(), title: format!("pwsh · {w}") }),
    ]);
    if closed_background {
        items.push(item("delete-closed", "Delete closed background sessions…", MenuAction::DeleteClosed(w.clone())));
    }
    items.push(item("remove", "Remove…", MenuAction::Remove(w.clone())));
    items
}

pub fn repo_items(workspace: &str, repo: &RepoNode) -> Vec<MenuItem> {
    let title = |what: &str| format!("{workspace} · {} · {what}", repo.name);
    let mut items = Vec::new();
    if repo.enabled {
        items.push(item("new-claude", "New claude here", MenuAction::NewClaude { cwd: repo.path.clone(), title: title("claude") }));
    }
    items.extend([
        item("open-terminal", "Open terminal here", MenuAction::OpenTerminal { cwd: repo.path.clone(), title: title("pwsh") }),
        item("open-folder", "Open folder", MenuAction::OpenFolder(repo.path.clone())),
    ]);
    let (key, label) = if repo.enabled { ("disable", "Disable") } else { ("enable", "Enable") };
    items.push(item(
        key,
        label,
        MenuAction::SetRepoEnabled { workspace: workspace.to_string(), repo: repo.name.clone(), enabled: !repo.enabled },
    ));
    items
}

/// `resumable`: [`FleetView::resumable`]; `claimed`: the WorkState item the session holds a claim on, as an
/// open Work tab knows it; `send_name`: what SendMessage takes for it ([`foreman_claude::peer::send_name`]),
/// which only a running session has; `deletable`: [`FleetView::deletable`].
pub fn session_items(
    workspace: &str,
    repo: &str,
    row: &SessionRow,
    resumable: bool,
    claimed: Option<String>,
    send_name: Option<String>,
    deletable: bool,
) -> Vec<MenuItem> {
    let s = &row.session;
    let (w, r) = (workspace.to_string(), repo.to_string());
    let mut items = Vec::new();
    if row.attachable {
        items.push(item("attach", "Attach", MenuAction::Attach { workspace: w.clone(), repo: r.clone(), session: s.clone() }));
    } else if resumable {
        items.push(item("resume", "Resume", MenuAction::Resume { workspace: w.clone(), repo: r, session: s.clone() }));
    }
    if fleet::is_alive(&row.health) && s.kind == Kind::Background && !s.id.is_empty() {
        let label = if is_foremans(s) { "Stop" } else { "Stop…" };
        items.push(item("stop-session", label, MenuAction::StopSession(s.clone())));
    }
    if let Some(name) = send_name {
        items.push(item("copy-send-name", "Copy SendMessage name", MenuAction::Copy(name)));
    }
    if !s.name.is_empty() {
        items.push(item("copy-name", "Copy name", MenuAction::Copy(s.name.clone())));
    }
    if !s.id.is_empty() {
        items.push(item("copy-id", "Copy id", MenuAction::Copy(s.id.clone())));
    }
    if let Some(item_id) = claimed {
        items.push(item("show-in-work", "Show in Work tab", MenuAction::ShowInWork { workspace: w, item_id }));
    }
    if !s.session_id.is_empty() {
        let label = if row.starred { "Unstar" } else { "Star" };
        items.push(item("star", label, MenuAction::ToggleStar(s.session_id.clone())));
    }
    if deletable {
        items.push(item("delete", "Delete…", MenuAction::Delete(s.clone())));
    }
    items
}

/// Named `fm-…`: launched by a Foreman supervisor (or its selftest), so Stop does not ask.
pub fn is_foremans(session: &Session) -> bool {
    session.name.starts_with("fm-")
}

impl FleetView {
    /// The open menu, over everything, kept inside the window.
    pub(super) fn render_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.menu.as_ref()?;
        let colors = cx.theme().colors().clone();
        let mut list = div()
            .id("context-menu")
            .flex()
            .flex_col()
            .min_w(px(180.))
            .py_1()
            .rounded_md()
            .border_1()
            .border_color(colors.border)
            .bg(colors.elevated_surface_background)
            .shadow_md()
            .text_sm()
            .occlude()
            .on_mouse_down_out(cx.listener(|this, _: &MouseDownEvent, _, cx| {
                this.menu = None;
                cx.notify();
            }))
            .child(
                div()
                    .px_3()
                    .pb_1()
                    .text_size(px(10.))
                    .text_color(colors.text_placeholder)
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(SharedString::from(menu.title.clone())),
            );
        for entry in &menu.items {
            let key = format!("menu-{}", entry.key);
            let action = entry.action.clone();
            list = list.child(
                div()
                    .id(SharedString::from(key.clone()))
                    .relative()
                    .px_3()
                    .py(px(3.))
                    .cursor_pointer()
                    .hover(|s| s.bg(colors.element_hover))
                    .child(SharedString::from(entry.label.clone()))
                    .child(hit_target(&self.hits, key))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        this.menu = None;
                        this.run_action(action.clone(), window, cx);
                    })),
            );
        }
        Some(deferred(anchored().position(menu.position).snap_to_window_with_margin(px(8.)).child(list)).with_priority(1).into_any_element())
    }
}

#[cfg(test)]
mod tests {
    use foreman_claude::{Health, parse};

    use super::*;

    fn node(started: Option<bool>) -> WorkspaceNode {
        WorkspaceNode { name: "demo".into(), root: r"C:\demo".into(), started, supervised: vec![], repos: vec![] }
    }

    fn keys(items: &[MenuItem]) -> Vec<&str> {
        items.iter().map(|i| i.key).collect()
    }

    fn row(json: &str, health: Health) -> SessionRow {
        let session = parse(&format!("[{json}]")).unwrap().remove(0);
        SessionRow { attachable: fleet::attachable(&session, &health), session, health, hook: None, foreign: false, starred: false, closed: false }
    }

    #[test]
    fn workspace_menu_follows_its_state() {
        assert_eq!(keys(&workspace_items(&node(None), false, false)), ["start", "work", "rescan", "open-folder", "open-terminal", "remove"]);
        let started = workspace_items(&node(Some(true)), true, false);
        assert_eq!(keys(&started), ["stop", "work", "task", "rescan", "open-folder", "open-terminal", "remove"]);
        assert_eq!(started.last().unwrap().action, MenuAction::Remove("demo".into()));
    }

    #[test]
    fn repo_menu_offers_enable_or_disable() {
        let mut repo = RepoNode { name: "Alpha".into(), path: r"C:\demo\Alpha".into(), branch: None, enabled: true, sessions: vec![], more: 0 };
        assert_eq!(keys(&repo_items("demo", &repo)), ["new-claude", "open-terminal", "open-folder", "disable"]);
        repo.enabled = false;
        let items = repo_items("demo", &repo);
        assert_eq!(keys(&items), ["open-terminal", "open-folder", "enable"]);
        assert_eq!(
            items[2].action,
            MenuAction::SetRepoEnabled { workspace: "demo".into(), repo: "Alpha".into(), enabled: true }
        );
    }

    #[test]
    fn session_menu_asks_before_stopping_a_session_foreman_did_not_launch() {
        let live = r#"{"id":"aaaa0001","sessionId":"aaaa0001-x","name":"mine","cwd":"C:\\demo","kind":"background","startedAt":1,"state":"working","status":"busy","pid":1}"#;
        let items = session_items("demo", "Alpha", &row(live, Health::Running), false, Some("item-1".into()), None, false);
        assert_eq!(keys(&items), ["attach", "stop-session", "copy-name", "copy-id", "show-in-work", "star"]);
        assert_eq!(items[1].label, "Stop…");
        assert_eq!(items[3].action, MenuAction::Copy("aaaa0001".into()));

        let fm = live.replace("\"mine\"", "\"fm-demo-alpha\"");
        assert_eq!(session_items("demo", "Alpha", &row(&fm, Health::Running), false, None, None, false)[1].label, "Stop");

        let stopped = r#"{"id":"aaaa0002","sessionId":"aaaa0002-x","name":"","cwd":"C:\\demo","kind":"background","startedAt":1,"state":"stopped"}"#;
        assert_eq!(keys(&session_items("demo", "Alpha", &row(stopped, Health::Stopped), true, None, None, false)), ["resume", "copy-id", "star"]);

        let interactive = r#"{"sessionId":"bbbb0005-x","name":"term","cwd":"C:\\demo","kind":"interactive","startedAt":1,"status":"idle","pid":5}"#;
        let items = session_items("demo", "Alpha", &row(interactive, Health::Idle), false, None, None, false);
        assert_eq!(keys(&items), ["copy-name", "copy-id", "star"], "an interactive session is neither attached nor stopped from here");
        assert_eq!(items[1].action, MenuAction::Copy("bbbb0005".into()));
        assert_eq!(items[2].action, MenuAction::ToggleStar("bbbb0005-x".into()));
        let mut starred = row(interactive, Health::Idle);
        starred.starred = true;
        assert_eq!(session_items("demo", "Alpha", &starred, false, None, None, false)[2].label, "Unstar");
    }

    #[test]
    fn a_live_session_offers_its_sendmessage_name_beside_name_and_id() {
        let interactive = r#"{"sessionId":"bbbb0005-x","name":"Fix the tree","cwd":"C:\\demo","kind":"interactive","startedAt":1,"status":"idle","pid":5}"#;
        let items = session_items("demo", "Alpha", &row(interactive, Health::Unknown), false, None, Some("twin [d923d7]".into()), false);
        assert_eq!(keys(&items), ["copy-send-name", "copy-name", "copy-id", "star"]);
        assert_eq!(items[0].label, "Copy SendMessage name");
        assert_eq!(items[0].action, MenuAction::Copy("twin [d923d7]".into()));
        assert_eq!(items[1].action, MenuAction::Copy("Fix the tree".into()));
    }

    #[test]
    fn delete_comes_last_when_offered_and_the_workspace_offers_the_bulk_delete_before_remove() {
        let stopped = r#"{"id":"aaaa0002","sessionId":"aaaa0002-0000-4000-8000-000000000002","name":"old","cwd":"C:\\demo","kind":"background","startedAt":1,"state":"stopped"}"#;
        let items = session_items("demo", "Alpha", &row(stopped, Health::Stopped), true, None, None, true);
        assert_eq!(keys(&items), ["resume", "copy-name", "copy-id", "star", "delete"]);
        assert_eq!(items[4].label, "Delete…");
        assert!(matches!(&items[4].action, MenuAction::Delete(s) if s.id == "aaaa0002"));
        let ws = workspace_items(&node(Some(false)), false, true);
        assert_eq!(keys(&ws), ["start", "work", "rescan", "open-folder", "open-terminal", "delete-closed", "remove"]);
        assert_eq!(ws[5].label, "Delete closed background sessions…");
        assert_eq!(ws[5].action, MenuAction::DeleteClosed("demo".into()));
    }
}
