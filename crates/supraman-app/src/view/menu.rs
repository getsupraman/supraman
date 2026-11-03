use std::path::PathBuf;

use supraman_claude::{Kind, Session};
use supraman_ui::menu::item;
use gpui::{AnyElement, Context};

use supraman_core::schedule::{ForeignTask, Status};

use super::FleetView;
use crate::fleet::{self, RepoNode, SessionRow, WorkspaceNode};
use crate::schedules::{Primary, ScheduleRow};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuAction {
    Start(String),
    Stop(String),
    Work(String),
    /// The prompt tab; `folder` preselects one of the workspace's.
    NewTask { workspace: String, folder: Option<PathBuf>, scheduled: bool },
    Rescan(String),
    EditFolders(String),
    AddFromFolder,
    AddEmpty,
    Remove(String),
    OpenFolder(PathBuf),
    OpenTerminal { cwd: PathBuf, title: String },
    NewClaude { cwd: PathBuf, title: String },
    SetRepoEnabled { workspace: String, repo: String, enabled: bool },
    Attach { workspace: String, repo: String, session: Session },
    Resume { workspace: String, repo: String, session: Session },
    StopSession(Session),
    Adopt { workspace: String, folder: String, session: Session, name: String },
    Copy(String),
    ShowInWork { workspace: String, item_id: String },
    ToggleStar(String),
    Delete(Session),
    DeleteClosed(String),
    Details(MenuTarget),
    Approve { workspace: String, id: String },
    RunNow { workspace: String, id: String },
    EditSchedule { workspace: String, id: String },
    DryRun { workspace: String, id: String },
    SetPaused { workspace: String, id: String, paused: bool },
    History { workspace: String, id: String },
    RemoveSchedule { workspace: String, id: String },
    OpenTaskScheduler,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Confirmed {
    Remove(String),
    StopSession(Session),
    Adopt { workspace: String, folder: String, session: Session, name: String },
    /// `discard`: the `--discard-unpushed` value rm printed, only after its own second question.
    Delete { session: Session, discard: Option<String> },
    DeleteClosed { workspace: String, sessions: Vec<Session>, skipped: Vec<(String, String)> },
    Edit { workspace: String, edit: supraman_core::Edit },
    RemoveSchedule { workspace: String, id: String },
    /// The banner's Update, while sessions are attached in tabs.
    Update,
}

pub type MenuItem = supraman_ui::menu::MenuItem<MenuAction>;
pub type ContextMenu = supraman_ui::menu::ContextMenu<MenuAction>;
pub type Confirm = supraman_ui::confirm::Confirm<Confirmed>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuTarget {
    Workspace(String),
    Repo { workspace: String, repo: String },
    /// By `session_id`, which every row has (an interactive one has no 8-hex `id`).
    Session { workspace: String, repo: String, session_id: String },
    Schedule { workspace: String, id: String },
    Schedules(String),
    /// A Task Scheduler task outside Supraman, by its path.
    OtherSchedule(String),
}

/// `own` and `relay` are [`session_items`] of the workspace session and the relay, which have no row of their
/// own: they follow under a caption each, so nothing on those sessions is out of reach.
pub fn workspace_items(node: &WorkspaceNode, closed_background: bool, own: Vec<MenuItem>, relay: Vec<MenuItem>) -> Vec<MenuItem> {
    let w = &node.name;
    let mut items = vec![if node.started == Some(true) {
        item("stop", "Stop", MenuAction::Stop(w.clone()))
    } else {
        item("start", "Start", MenuAction::Start(w.clone()))
    }];
    items.push(item("work", "Work", MenuAction::Work(w.clone())));
    items.extend(new_task_items(w, None));
    items.push(item("rescan", "Rescan repos", MenuAction::Rescan(w.clone())));
    items.push(item("edit-folders", "Edit folders…", MenuAction::EditFolders(w.clone())));
    if let Some(home) = &node.home {
        items.extend([
            item("open-folder", "Open folder", MenuAction::OpenFolder(home.clone())),
            item("open-terminal", "Open terminal here", MenuAction::OpenTerminal { cwd: home.clone(), title: format!("pwsh · {w}") }),
        ]);
    }
    items.push(item("details", "Details", MenuAction::Details(MenuTarget::Workspace(w.clone()))));
    if closed_background {
        items.push(item("delete-closed", "Delete closed background sessions…", MenuAction::DeleteClosed(w.clone())));
    }
    items.push(item("remove", "Remove…", MenuAction::Remove(w.clone())));
    if let Some(row) = &node.own {
        items.extend(section(own, Section::Own, format!("workspace session · {}", row.session.name)));
    }
    if let Some(row) = &node.relay {
        items.extend(section(relay, Section::Relay, format!("relay · {}", row.session.name)));
    }
    items
}

#[derive(Clone, Copy)]
enum Section {
    Own,
    Relay,
}

fn section(items: Vec<MenuItem>, of: Section, caption: String) -> Vec<MenuItem> {
    let mut items: Vec<MenuItem> = items.into_iter().map(|i| MenuItem { key: section_key(of, i.key), ..i }).collect();
    if let Some(first) = items.first_mut() {
        first.section = Some(caption);
    }
    items
}

/// A [`session_items`] key inside the workspace menu, where it must not clash with the workspace's own.
fn section_key(of: Section, key: &'static str) -> &'static str {
    match (of, key) {
        (Section::Own, "attach") => "session-attach",
        (Section::Own, "resume") => "session-resume",
        (Section::Own, "stop-session") => "session-stop",
        (Section::Own, "copy-send-name") => "session-copy-send-name",
        (Section::Own, "copy-name") => "session-copy-name",
        (Section::Own, "copy-id") => "session-copy-id",
        (Section::Own, "show-in-work") => "session-show-in-work",
        (Section::Own, "star") => "session-star",
        (Section::Own, "delete") => "session-delete",
        (Section::Own, "details") => "session-details",
        (Section::Relay, "attach") => "relay-attach",
        (Section::Relay, "resume") => "relay-resume",
        (Section::Relay, "stop-session") => "relay-stop",
        (Section::Relay, "copy-send-name") => "relay-copy-send-name",
        (Section::Relay, "copy-name") => "relay-copy-name",
        (Section::Relay, "copy-id") => "relay-copy-id",
        (Section::Relay, "show-in-work") => "relay-show-in-work",
        (Section::Relay, "star") => "relay-star",
        (Section::Relay, "delete") => "relay-delete",
        (Section::Relay, "details") => "relay-details",
        (_, other) => other,
    }
}

/// The prompt tab's two openings, in `folder` when given.
pub fn new_task_items(workspace: &str, folder: Option<&std::path::Path>) -> [MenuItem; 2] {
    let action = |scheduled| MenuAction::NewTask { workspace: workspace.to_string(), folder: folder.map(std::path::Path::to_path_buf), scheduled };
    [item("new-task", "New task…", action(false)), item("new-scheduled-task", "New scheduled task…", action(true))]
}

pub fn schedules_items(workspace: &str) -> Vec<MenuItem> {
    new_task_items(workspace, None).into_iter().filter(|i| i.key == "new-scheduled-task").collect()
}

/// A task outside Supraman: only what reads it.
pub fn other_schedule_items(task: &ForeignTask) -> Vec<MenuItem> {
    vec![
        item("open-task-scheduler", "Open in Task Scheduler", MenuAction::OpenTaskScheduler).under("Read only: Supraman never changes, runs or deletes it"),
        item("copy-action", "Copy action", MenuAction::Copy(task.action.clone())),
    ]
}

/// Add workspace: today's scan of a parent folder, or a workspace with no folder yet.
pub fn add_items() -> Vec<MenuItem> {
    vec![item("add-from-folder", "From a parent folder…", MenuAction::AddFromFolder), item("add-empty", "Empty workspace", MenuAction::AddEmpty)]
}

pub fn repo_items(workspace: &str, repo: &RepoNode) -> Vec<MenuItem> {
    let title = |what: &str| format!("{workspace} · {} · {what}", repo.name);
    let mut items = Vec::new();
    if repo.enabled && repo.supervised_in.is_none() {
        items.push(item("new-claude", "New claude here", MenuAction::NewClaude { cwd: repo.path.clone(), title: title("claude") }));
    }
    items.extend(new_task_items(workspace, Some(&repo.path)));
    items.extend([
        item("open-terminal", "Open terminal here", MenuAction::OpenTerminal { cwd: repo.path.clone(), title: title("pwsh") }),
        item("open-folder", "Open folder", MenuAction::OpenFolder(repo.path.clone())),
    ]);
    let (key, label) = if repo.enabled { ("disable", "Disable") } else { ("enable", "Enable") };
    if repo.listed && (repo.enabled || repo.supervised_in.is_none()) {
        items.push(item(
            key,
            label,
            MenuAction::SetRepoEnabled { workspace: workspace.to_string(), repo: repo.name.clone(), enabled: !repo.enabled },
        ));
    }
    items
}

pub fn session_items(
    workspace: &str,
    repo: &str,
    row: &SessionRow,
    resumable: bool,
    claimed: Option<String>,
    send_name: Option<String>,
    deletable: bool,
    adopt: Option<String>,
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
    if let Some(name) = adopt {
        items.push(item("adopt", "Adopt…", MenuAction::Adopt { workspace: w.clone(), folder: repo.to_string(), session: s.clone(), name }));
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
    items.push(item("details", "Details", MenuAction::Details(MenuTarget::Session { workspace: workspace.to_string(), repo: repo.to_string(), session_id: s.session_id.clone() })));
    if deletable {
        items.push(item("delete", "Delete…", MenuAction::Delete(s.clone())));
    }
    items
}

/// The row's one action first, then the rest; Approve is offered while the schedule is pending or its slash command changed.
pub fn schedule_items(row: &ScheduleRow) -> Vec<MenuItem> {
    let (w, id) = (row.workspace.clone(), row.schedule.id.clone());
    let mut items = vec![match row.primary() {
        Primary::Approve => item("approve", "Approve…", MenuAction::Approve { workspace: w.clone(), id: id.clone() }),
        Primary::RunNow => item("run-now", "Run now", MenuAction::RunNow { workspace: w.clone(), id: id.clone() }),
    }];
    if row.command_changed {
        items.push(item("approve", "Approve…", MenuAction::Approve { workspace: w.clone(), id: id.clone() }));
    }
    items.push(item("edit-schedule", "Edit…", MenuAction::EditSchedule { workspace: w.clone(), id: id.clone() }));
    items.push(item("dry-run", "Dry run", MenuAction::DryRun { workspace: w.clone(), id: id.clone() }));
    items.push(match row.schedule.status {
        Status::Paused => item("resume", "Resume", MenuAction::SetPaused { workspace: w.clone(), id: id.clone(), paused: false }),
        _ => item("pause", "Pause", MenuAction::SetPaused { workspace: w.clone(), id: id.clone(), paused: true }),
    });
    items.push(item("history", "History", MenuAction::History { workspace: w.clone(), id: id.clone() }));
    items.push(item("details", "Details", MenuAction::Details(MenuTarget::Schedule { workspace: w.clone(), id: id.clone() })));
    items.push(item("remove-schedule", "Remove…", MenuAction::RemoveSchedule { workspace: w, id }));
    items
}

/// Named `fm-…`: launched by a Foreman supervisor (or its selftest), so Stop does not ask.
pub fn is_foremans(session: &Session) -> bool {
    session.name.starts_with("fm-")
}

impl FleetView {
    pub(super) fn render_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.menu.as_ref()?;
        Some(menu.render(
            &self.hits,
            |this: &mut Self, action, window, cx| {
                this.menu = None;
                this.run_action(action, window, cx);
            },
            |this, cx| {
                this.menu = None;
                cx.notify();
            },
            cx,
        ))
    }
}

#[cfg(test)]
mod tests {
    use supraman_claude::{Health, parse};

    use super::*;

    fn node(started: Option<bool>) -> WorkspaceNode {
        WorkspaceNode { name: "demo".into(), home: Some(r"C:\demo".into()), started, own: None, relay: None, supervised: vec![], scheduled: Default::default(), repos: vec![] }
    }

    fn keys(items: &[MenuItem]) -> Vec<&str> {
        items.iter().map(|i| i.key).collect()
    }

    fn row(json: &str, health: Health) -> SessionRow {
        let session = parse(&format!("[{json}]")).unwrap().remove(0);
        SessionRow::new(session, health)
    }

    #[test]
    fn workspace_menu_follows_its_state() {
        let stopped = workspace_items(&node(None), false, vec![], vec![]);
        assert_eq!(keys(&stopped), ["start", "work", "new-task", "new-scheduled-task", "rescan", "edit-folders", "open-folder", "open-terminal", "details", "remove"]);
        assert_eq!(stopped[2].label, "New task…");
        assert_eq!(stopped[2].action, MenuAction::NewTask { workspace: "demo".into(), folder: None, scheduled: false });
        assert_eq!(stopped[3].label, "New scheduled task…");
        assert_eq!(stopped[3].action, MenuAction::NewTask { workspace: "demo".into(), folder: None, scheduled: true });
        let started = workspace_items(&node(Some(true)), false, vec![], vec![]);
        assert_eq!(keys(&started), ["stop", "work", "new-task", "new-scheduled-task", "rescan", "edit-folders", "open-folder", "open-terminal", "details", "remove"]);
        assert!(!started.iter().any(|i| i.label == "Task…"), "the relay's task box is not on the menu");
        assert_eq!(started.last().unwrap().action, MenuAction::Remove("demo".into()));
        assert_eq!(started[8].action, MenuAction::Details(MenuTarget::Workspace("demo".into())));
    }

    #[test]
    fn the_workspace_menu_holds_every_item_of_its_workspace_session_and_relay_under_a_caption() {
        let own = r#"{"id":"aaaa0010","sessionId":"aaaa0010-x","name":"fm-demo-workspace","cwd":"C:\\demo","kind":"background","startedAt":1,"state":"working","status":"busy","pid":1}"#;
        let relay = own.replace("aaaa0010", "aaaa0011").replace("workspace\"", "relay\"");
        let mut n = node(Some(true));
        n.own = Some(row(own, Health::Running));
        n.relay = Some(row(&relay, Health::Idle));
        let own_items = session_items("demo", "workspace", n.own.as_ref().unwrap(), false, Some("item-1".into()), Some("ctrl [abc123]".into()), false, None);
        let relay_items = session_items("demo", "relay", n.relay.as_ref().unwrap(), false, None, None, false, None);
        let (own_n, relay_n) = (own_items.len(), relay_items.len());
        let items = workspace_items(&n, false, own_items, relay_items);
        assert_eq!(
            keys(&items),
            [
                "stop", "work", "new-task", "new-scheduled-task", "rescan", "edit-folders", "open-folder", "open-terminal", "details", "remove",
                "session-attach", "session-stop", "session-copy-send-name", "session-copy-name", "session-copy-id", "session-show-in-work", "session-star", "session-details",
                "relay-attach", "relay-stop", "relay-copy-name", "relay-copy-id", "relay-star", "relay-details",
            ]
        );
        assert_eq!(items.len(), 10 + own_n + relay_n, "nothing dropped");
        let captions: Vec<(&str, &str)> = items.iter().filter_map(|i| i.section.as_deref().map(|c| (i.key, c))).collect();
        assert_eq!(captions, [("session-attach", "workspace session · fm-demo-workspace"), ("relay-attach", "relay · fm-demo-relay")]);
        assert!(matches!(&items[10].action, MenuAction::Attach { repo, session, .. } if repo == "workspace" && session.id == "aaaa0010"));
        assert_eq!(items[11].label, "Stop", "an fm- session stops without a question");
        let keys = keys(&items);
        assert_eq!(keys.iter().collect::<std::collections::HashSet<_>>().len(), keys.len(), "keys unique");
    }

    #[test]
    fn a_schedules_menu_leads_with_its_one_action_and_pauses_or_resumes() {
        let s: supraman_core::schedule::Schedule = serde_json::from_value(serde_json::json!({
            "id": "nightly", "folder": "Alpha", "trigger": {"kind": "daily", "at": "09:00"}, "action": {"kind": "script", "script": "gate.cmd"}
        }))
        .unwrap();
        let mut row = ScheduleRow { workspace: "demo".into(), schedule: s, status: Status::Pending, command_changed: false, next_fire: None, running: None, runs: vec![], approvable: None, folder_path: None, command_path: None };
        let all = ["edit-schedule", "dry-run", "pause", "history", "details", "remove-schedule"];
        assert_eq!(keys(&schedule_items(&row)), [&["approve"][..], &all].concat());
        row.status = Status::Active;
        row.schedule.status = Status::Active;
        let items = schedule_items(&row);
        assert_eq!(keys(&items), [&["run-now"][..], &all].concat());
        assert_eq!(items[0].action, MenuAction::RunNow { workspace: "demo".into(), id: "nightly".into() });
        assert_eq!(items[3].action, MenuAction::SetPaused { workspace: "demo".into(), id: "nightly".into(), paused: true });
        row.command_changed = true;
        let items = schedule_items(&row);
        assert_eq!(keys(&items), [&["run-now", "approve"][..], &all].concat(), "a changed slash command adds Approve after Run now");
        assert_eq!(items[1].action, MenuAction::Approve { workspace: "demo".into(), id: "nightly".into() });
        row.command_changed = false;
        row.status = Status::Paused;
        row.schedule.status = Status::Paused;
        let items = schedule_items(&row);
        assert_eq!(keys(&items)[..4], ["run-now", "edit-schedule", "dry-run", "resume"], "Run now of a paused schedule is fire's to refuse");
        assert_eq!(items[5].action, MenuAction::Details(MenuTarget::Schedule { workspace: "demo".into(), id: "nightly".into() }));
        assert_eq!(items[6].label, "Remove…");
    }

    #[test]
    fn repo_menu_offers_enable_or_disable() {
        let mut repo = RepoNode {
            name: "Alpha".into(),
            path: r"C:\demo\Alpha".into(),
            kind: supraman_core::FolderKind::Git,
            branch: None,
            enabled: true,
            listed: true,
            supervised_in: None,
            also_in: vec![],
            sessions: vec![],
            more: 0,
        };
        let items = repo_items("demo", &repo);
        assert_eq!(keys(&items), ["new-claude", "new-task", "new-scheduled-task", "open-terminal", "open-folder", "disable"]);
        assert_eq!(items[1].action, MenuAction::NewTask { workspace: "demo".into(), folder: Some(r"C:\demo\Alpha".into()), scheduled: false }, "the folder is preselected");
        assert_eq!(items[2].action, MenuAction::NewTask { workspace: "demo".into(), folder: Some(r"C:\demo\Alpha".into()), scheduled: true });
        repo.enabled = false;
        let items = repo_items("demo", &repo);
        assert_eq!(keys(&items), ["new-task", "new-scheduled-task", "open-terminal", "open-folder", "enable"]);
        assert_eq!(
            items[4].action,
            MenuAction::SetRepoEnabled { workspace: "demo".into(), repo: "Alpha".into(), enabled: true }
        );
        repo.supervised_in = Some("other".into());
        assert_eq!(keys(&repo_items("demo", &repo)), ["new-task", "new-scheduled-task", "open-terminal", "open-folder"], "supervised in another workspace: no Enable");
        repo.enabled = true;
        assert_eq!(keys(&repo_items("demo", &repo)), ["new-task", "new-scheduled-task", "open-terminal", "open-folder", "disable"], "enabled in two: this one can let go");
        let home_folder = RepoNode { enabled: false, listed: false, supervised_in: None, ..repo };
        assert_eq!(keys(&repo_items("demo", &home_folder)), ["new-task", "new-scheduled-task", "open-terminal", "open-folder"], "a folder of the home is not the workspace's to enable");
        let group = schedules_items("demo");
        assert_eq!(keys(&group), ["new-scheduled-task"]);
        assert_eq!(group[0].action, MenuAction::NewTask { workspace: "demo".into(), folder: None, scheduled: true });
    }

    #[test]
    fn session_menu_asks_before_stopping_a_session_supraman_did_not_launch() {
        let live = r#"{"id":"aaaa0001","sessionId":"aaaa0001-x","name":"mine","cwd":"C:\\demo","kind":"background","startedAt":1,"state":"working","status":"busy","pid":1}"#;
        let items = session_items("demo", "Alpha", &row(live, Health::Running), false, Some("item-1".into()), None, false, None);
        assert_eq!(keys(&items), ["attach", "stop-session", "copy-name", "copy-id", "show-in-work", "star", "details"]);
        assert_eq!(items[1].label, "Stop…");
        assert_eq!(items[3].action, MenuAction::Copy("aaaa0001".into()));
        assert_eq!(
            items[6].action,
            MenuAction::Details(MenuTarget::Session { workspace: "demo".into(), repo: "Alpha".into(), session_id: "aaaa0001-x".into() })
        );

        let adopt = session_items("demo", "Alpha", &row(live, Health::Running), false, None, None, false, Some("fm-demo-alpha".into()));
        assert_eq!(keys(&adopt), ["attach", "stop-session", "adopt", "copy-name", "copy-id", "star", "details"]);
        assert_eq!(adopt[2].label, "Adopt…");
        assert!(matches!(&adopt[2].action, MenuAction::Adopt { workspace, folder, session, name } if workspace == "demo" && folder == "Alpha" && session.id == "aaaa0001" && name == "fm-demo-alpha"));

        let sm = live.replace("\"mine\"", "\"fm-demo-alpha\"");
        assert_eq!(session_items("demo", "Alpha", &row(&sm, Health::Running), false, None, None, false, None)[1].label, "Stop");

        let stopped = r#"{"id":"aaaa0002","sessionId":"aaaa0002-x","name":"","cwd":"C:\\demo","kind":"background","startedAt":1,"state":"stopped"}"#;
        assert_eq!(keys(&session_items("demo", "Alpha", &row(stopped, Health::Stopped), true, None, None, false, None)), ["resume", "copy-id", "star", "details"]);

        let interactive = r#"{"sessionId":"bbbb0005-x","name":"term","cwd":"C:\\demo","kind":"interactive","startedAt":1,"status":"idle","pid":5}"#;
        let items = session_items("demo", "Alpha", &row(interactive, Health::Idle), false, None, None, false, None);
        assert_eq!(keys(&items), ["copy-name", "copy-id", "star", "details"], "an interactive session is neither attached nor stopped from here");
        assert_eq!(items[1].action, MenuAction::Copy("bbbb0005".into()));
        assert_eq!(items[2].action, MenuAction::ToggleStar("bbbb0005-x".into()));
        let mut starred = row(interactive, Health::Idle);
        starred.starred = true;
        assert_eq!(session_items("demo", "Alpha", &starred, false, None, None, false, None)[2].label, "Unstar");
    }

    #[test]
    fn a_live_session_offers_its_sendmessage_name_beside_name_and_id() {
        let interactive = r#"{"sessionId":"bbbb0005-x","name":"Fix the tree","cwd":"C:\\demo","kind":"interactive","startedAt":1,"status":"idle","pid":5}"#;
        let items = session_items("demo", "Alpha", &row(interactive, Health::Unknown), false, None, Some("twin [d923d7]".into()), false, None);
        assert_eq!(keys(&items), ["copy-send-name", "copy-name", "copy-id", "star", "details"]);
        assert_eq!(items[0].label, "Copy SendMessage name");
        assert_eq!(items[0].action, MenuAction::Copy("twin [d923d7]".into()));
        assert_eq!(items[1].action, MenuAction::Copy("Fix the tree".into()));
    }

    #[test]
    fn delete_comes_last_when_offered_and_the_workspace_offers_the_bulk_delete_before_remove() {
        let stopped = r#"{"id":"aaaa0002","sessionId":"aaaa0002-0000-4000-8000-000000000002","name":"old","cwd":"C:\\demo","kind":"background","startedAt":1,"state":"stopped"}"#;
        let items = session_items("demo", "Alpha", &row(stopped, Health::Stopped), true, None, None, true, None);
        assert_eq!(keys(&items), ["resume", "copy-name", "copy-id", "star", "details", "delete"]);
        assert_eq!(items[5].label, "Delete…");
        assert!(matches!(&items[5].action, MenuAction::Delete(s) if s.id == "aaaa0002"));
        let ws = workspace_items(&node(Some(false)), true, vec![], vec![]);
        assert_eq!(keys(&ws), ["start", "work", "new-task", "new-scheduled-task", "rescan", "edit-folders", "open-folder", "open-terminal", "details", "delete-closed", "remove"]);
        assert_eq!(ws[9].label, "Delete closed background sessions…");
        assert_eq!(ws[9].action, MenuAction::DeleteClosed("demo".into()));
    }

    #[test]
    fn an_other_schedule_offers_only_what_reads_it() {
        let task = ForeignTask {
            folder: r"\Mine".into(),
            name: "Nightly review".into(),
            enabled: true,
            trigger: "daily at 09:00".into(),
            last_run_ms: None,
            last_result: None,
            next_run_ms: None,
            working_dir: None,
            via: supraman_core::schedule::Via::Direct,
            action: "claude.exe -p review".into(),
        };
        let items = other_schedule_items(&task);
        assert_eq!(keys(&items), ["open-task-scheduler", "copy-action"]);
        assert_eq!(items[0].action, MenuAction::OpenTaskScheduler);
        assert!(items[0].section.as_deref().is_some_and(|s| s.contains("never changes, runs or deletes")));
        assert_eq!(items[1].action, MenuAction::Copy("claude.exe -p review".into()));
    }
}
