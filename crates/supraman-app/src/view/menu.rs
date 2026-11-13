use std::path::PathBuf;

use supraman_claude::{Kind, Session};
use supraman_ui::menu::item;
use gpui::{AnyElement, Context};

use supraman_core::schedule::{ForeignTask, Status};

use supraman_core::folders::Pos;

use super::FleetView;
use super::folder_edits::FolderEdit;
use crate::fleet::{self, EntryNode, FolderNode, SessionRow};
use crate::schedules::{Primary, ScheduleRow};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuAction {
    Work(String),
    /// The prompt tab of an entry; `folder` preselects one of its folders.
    NewTask { entry: String, folder: Option<PathBuf>, scheduled: bool },
    OpenFolder(PathBuf),
    OpenTerminal { cwd: PathBuf, place: String },
    NewShell(supraman_core::shells::Shell),
    NewClaude { cwd: PathBuf, title: String },
    /// `place`: where the row is, its tab's title before the session's name.
    Attach { place: String, session: Session },
    Resume { place: String, session: Session },
    StopSession(Session),
    Copy(String),
    ShowInWork { workspace: String, item_id: String },
    ToggleStar(String),
    Delete(Session),
    DeleteClosed(String),
    Details(MenuTarget),
    Approve { folder_key: String, id: String },
    RunNow { folder_key: String, id: String },
    EditSchedule { folder_key: String, id: String },
    DryRun { folder_key: String, id: String },
    SetPaused { folder_key: String, id: String, paused: bool },
    History { folder_key: String, id: String },
    RemoveSchedule { folder_key: String, id: String },
    OpenTaskScheduler,
    /// The Git tab of the repository whose top this is ([`super::tab_menu`]).
    OpenGit(PathBuf),
    /// The Work tab of `workspace` on `project` ([`super::tab_menu`]).
    OpenWorkman { workspace: String, project: String },
    CloseTab(u64),
    CloseOtherTabs(u64),
    CloseTabsRight(u64),
    ReopenClosed,
    /// A folder picked, then added at the end of the top level or of the group named.
    AddFolder(Option<String>),
    AddGroup,
    RenameGroup(String),
    RemoveFolder { pos: Pos, path: PathBuf },
    RemoveGroup(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Confirmed {
    StopSession(Session),
    /// `discard`: the `--discard-unpushed` value rm printed, only after its own second question.
    Delete { session: Session, discard: Option<String> },
    DeleteClosed { entry: String, sessions: Vec<Session>, skipped: Vec<(String, String)> },
    RemoveSchedule { folder_key: String, id: String },
    EditFolders(FolderEdit),
    /// The banner's Update, while sessions are attached in tabs.
    Update,
}

pub type MenuItem = supraman_ui::menu::MenuItem<MenuAction>;
pub type ContextMenu = supraman_ui::menu::ContextMenu<MenuAction>;
pub type Confirm = supraman_ui::confirm::Confirm<Confirmed>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuTarget {
    /// A group's row, by its name.
    Group(String),
    /// A folder's row, by its [`FolderNode::key`].
    Folder(String),
    /// By `session_id`, which every row has (an interactive one has no 8-hex `id`); `place` is its folder's key or its schedule's id.
    Session { entry: String, place: String, session_id: String },
    /// A schedule by its folder's [`supraman_core::schedule::folder_key`] and its id.
    Schedule { folder_key: String, id: String },
    /// A folder's Schedules row: its entry and its folder.
    Schedules { entry: String, folder: PathBuf },
    /// A Task Scheduler task outside Supraman, by its path.
    OtherSchedule(String),
}

/// A group's row: its folders' list and what reaches all of them.
pub fn group_items(node: &EntryNode, closed_background: bool) -> Vec<MenuItem> {
    let name = &node.name;
    let mut items = vec![item("add-folder", "Add Folder…", MenuAction::AddFolder(Some(name.clone())))];
    items.extend(new_task_items(name, None));
    items.extend([
        item("work", "Open Work", MenuAction::Work(name.clone())),
        item("rename-group", "Rename…", MenuAction::RenameGroup(name.clone())),
        item("details", "Details", MenuAction::Details(MenuTarget::Group(name.clone()))),
    ]);
    if closed_background {
        items.push(item("delete-closed", "Delete closed background sessions…", MenuAction::DeleteClosed(name.clone())));
    }
    items.push(item("remove-group", "Remove Group…", MenuAction::RemoveGroup(name.clone())));
    items
}

/// The action bar's add button.
pub fn add_items() -> Vec<MenuItem> {
    vec![item("add-folder", "Add Folder…", MenuAction::AddFolder(None)), item("add-group", "Add Group…", MenuAction::AddGroup)]
}

pub fn new_task_items(entry: &str, folder: Option<&std::path::Path>) -> [MenuItem; 2] {
    let action = |scheduled| MenuAction::NewTask { entry: entry.to_string(), folder: folder.map(std::path::Path::to_path_buf), scheduled };
    [item("new-task", "New task…", action(false)), item("new-scheduled-task", "New scheduled task…", action(true))]
}

pub fn schedules_items(entry: &str, folder: &std::path::Path) -> Vec<MenuItem> {
    new_task_items(entry, Some(folder)).into_iter().filter(|i| i.key == "new-scheduled-task").collect()
}

/// A task outside Supraman: only what reads it.
pub fn other_schedule_items(task: &ForeignTask) -> Vec<MenuItem> {
    vec![
        item("open-task-scheduler", "Open in Task Scheduler", MenuAction::OpenTaskScheduler).under("Read only: Supraman never changes, runs or deletes it"),
        item("copy-action", "Copy action", MenuAction::Copy(task.action.clone())),
    ]
}

/// A folder's row at `pos`; a top-level folder's also has its entry's Work and bulk delete.
pub fn folder_items(entry: &EntryNode, folder: &FolderNode, pos: Pos, closed_background: bool) -> Vec<MenuItem> {
    let place = folder.place();
    let mut items = vec![item("new-claude", "New claude here", MenuAction::NewClaude { cwd: folder.path.clone(), title: format!("{place} · claude") })];
    items.extend(new_task_items(&entry.name, Some(&folder.path)));
    if folder.kind == supraman_core::FolderKind::Git {
        items.push(item("open-git", "Open Git", MenuAction::OpenGit(folder.path.clone())));
    }
    if !entry.group {
        items.push(item("work", "Open Work", MenuAction::Work(entry.name.clone())));
    }
    items.extend([
        item("open-terminal", "Open terminal here", MenuAction::OpenTerminal { cwd: folder.path.clone(), place }),
        item("open-folder", "Open folder", MenuAction::OpenFolder(folder.path.clone())),
        item("details", "Details", MenuAction::Details(MenuTarget::Folder(folder.key.clone()))),
    ]);
    if !entry.group && closed_background {
        items.push(item("delete-closed", "Delete closed background sessions…", MenuAction::DeleteClosed(entry.name.clone())));
    }
    items.push(item("remove-folder", "Remove Folder", MenuAction::RemoveFolder { pos, path: folder.path.clone() }));
    items
}

pub fn session_items(
    entry: &str,
    place: &str,
    row: &SessionRow,
    resumable: bool,
    claimed: Option<String>,
    send_name: Option<String>,
    deletable: bool,
) -> Vec<MenuItem> {
    let s = &row.session;
    let title = place.replace('/', " · ");
    let mut items = Vec::new();
    if row.attachable {
        items.push(item("attach", "Attach", MenuAction::Attach { place: title, session: s.clone() }));
    } else if resumable {
        items.push(item("resume", "Resume", MenuAction::Resume { place: title, session: s.clone() }));
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
        items.push(item("show-in-work", "Show in Work tab", MenuAction::ShowInWork { workspace: entry.to_string(), item_id }));
    }
    if !s.session_id.is_empty() {
        let label = if row.starred { "Unstar" } else { "Star" };
        items.push(item("star", label, MenuAction::ToggleStar(s.session_id.clone())));
    }
    items.push(item("details", "Details", MenuAction::Details(MenuTarget::Session { entry: entry.to_string(), place: place.to_string(), session_id: s.session_id.clone() })));
    if deletable {
        items.push(item("delete", "Delete…", MenuAction::Delete(s.clone())));
    }
    items
}

/// The row's one action first, then the rest; Approve is offered while the schedule is pending or its slash command changed.
pub fn schedule_items(row: &ScheduleRow) -> Vec<MenuItem> {
    let (w, id) = (row.key.clone(), row.schedule.id.clone());
    let mut items = vec![match row.primary() {
        Primary::Approve => item("approve", "Approve…", MenuAction::Approve { folder_key: w.clone(), id: id.clone() }),
        Primary::RunNow => item("run-now", "Run now", MenuAction::RunNow { folder_key: w.clone(), id: id.clone() }),
    }];
    if row.command_changed {
        items.push(item("approve", "Approve…", MenuAction::Approve { folder_key: w.clone(), id: id.clone() }));
    }
    items.push(item("edit-schedule", "Edit…", MenuAction::EditSchedule { folder_key: w.clone(), id: id.clone() }));
    items.push(item("dry-run", "Dry run", MenuAction::DryRun { folder_key: w.clone(), id: id.clone() }));
    items.push(match row.schedule.status {
        Status::Paused => item("resume", "Resume", MenuAction::SetPaused { folder_key: w.clone(), id: id.clone(), paused: false }),
        _ => item("pause", "Pause", MenuAction::SetPaused { folder_key: w.clone(), id: id.clone(), paused: true }),
    });
    items.push(item("history", "History", MenuAction::History { folder_key: w.clone(), id: id.clone() }));
    items.push(item("details", "Details", MenuAction::Details(MenuTarget::Schedule { folder_key: w.clone(), id: id.clone() })));
    items.push(item("remove-schedule", "Remove…", MenuAction::RemoveSchedule { folder_key: w, id }));
    items
}

/// Named `fm-…`: launched by Supraman (a scheduled run, a selftest's), so Stop does not ask.
pub fn is_foremans(session: &Session) -> bool {
    session.name.starts_with("fm-")
}

impl FleetView {
    pub(super) fn render_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.dialogs.menu.as_ref()?;
        Some(menu.render(
            &self.hits,
            |this: &mut Self, action, window, cx| {
                this.dialogs.menu = None;
                this.run_action(action, window, cx);
            },
            |this, cx| {
                this.dialogs.menu = None;
                cx.notify();
            },
            cx,
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use supraman_claude::{Health, parse};

    use super::*;

    fn folder(key: &str) -> FolderNode {
        FolderNode { key: key.into(), name: "Alpha".into(), path: r"C:\demo\Alpha".into(), kind: supraman_core::FolderKind::Git, branch: None, sessions: vec![], more: 0, scheduled: Default::default() }
    }

    fn node(group: bool) -> EntryNode {
        let key = if group { "demo/Alpha" } else { "Alpha" };
        EntryNode { name: if group { "demo".into() } else { key.into() }, group, folders: vec![folder(key)] }
    }

    fn keys(items: &[MenuItem]) -> Vec<&str> {
        items.iter().map(|i| &*i.key).collect()
    }

    fn row(json: &str, health: Health) -> SessionRow {
        let session = parse(&format!("[{json}]")).unwrap().remove(0);
        SessionRow::new(session, health)
    }

    #[test]
    fn group_menu_lists_its_items() {
        let items = group_items(&node(true), false);
        assert_eq!(keys(&items), ["add-folder", "new-task", "new-scheduled-task", "work", "rename-group", "details", "remove-group"]);
        assert_eq!((items[0].label.as_str(), &items[0].action), ("Add Folder…", &MenuAction::AddFolder(Some("demo".into()))));
        assert_eq!(items[1].label, "New task…");
        assert_eq!(items[1].action, MenuAction::NewTask { entry: "demo".into(), folder: None, scheduled: false });
        assert_eq!(items[2].label, "New scheduled task…");
        assert_eq!(items[2].action, MenuAction::NewTask { entry: "demo".into(), folder: None, scheduled: true });
        assert_eq!((items[3].label.as_str(), &items[3].action), ("Open Work", &MenuAction::Work("demo".into())));
        assert_eq!((items[4].label.as_str(), &items[4].action), ("Rename…", &MenuAction::RenameGroup("demo".into())));
        assert_eq!(items[5].action, MenuAction::Details(MenuTarget::Group("demo".into())));
        assert_eq!((items[6].label.as_str(), &items[6].action), ("Remove Group…", &MenuAction::RemoveGroup("demo".into())));
    }

    #[test]
    fn the_add_button_offers_a_folder_or_a_group() {
        let items = add_items();
        assert_eq!(items.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(), ["Add Folder…", "Add Group…"]);
        assert_eq!(items[0].action, MenuAction::AddFolder(None), "at the top level");
        assert_eq!(items[1].action, MenuAction::AddGroup);
    }

    #[test]
    fn a_schedules_menu_leads_with_its_one_action_and_pauses_or_resumes() {
        let s: supraman_core::schedule::Schedule = serde_json::from_value(serde_json::json!({
            "id": "nightly", "trigger": {"kind": "daily", "at": "09:00"}, "action": {"kind": "script", "script": "gate.cmd"}
        }))
        .unwrap();
        let mut row = ScheduleRow { key: "demo".into(), folder: r"C:\demo\Alpha".into(), schedule: s, status: Status::Pending, command_changed: false, next_fire: None, running: None, runs: vec![], approvable: None, command_path: None };
        let all = ["edit-schedule", "dry-run", "pause", "history", "details", "remove-schedule"];
        assert_eq!(keys(&schedule_items(&row)), [&["approve"][..], &all].concat());
        row.status = Status::Active;
        row.schedule.status = Status::Active;
        let items = schedule_items(&row);
        assert_eq!(keys(&items), [&["run-now"][..], &all].concat());
        assert_eq!(items[0].action, MenuAction::RunNow { folder_key: "demo".into(), id: "nightly".into() });
        assert_eq!(items[3].action, MenuAction::SetPaused { folder_key: "demo".into(), id: "nightly".into(), paused: true });
        row.command_changed = true;
        let items = schedule_items(&row);
        assert_eq!(keys(&items), [&["run-now", "approve"][..], &all].concat(), "a changed slash command adds Approve after Run now");
        assert_eq!(items[1].action, MenuAction::Approve { folder_key: "demo".into(), id: "nightly".into() });
        row.command_changed = false;
        row.status = Status::Paused;
        row.schedule.status = Status::Paused;
        let items = schedule_items(&row);
        assert_eq!(keys(&items)[..4], ["run-now", "edit-schedule", "dry-run", "resume"], "Run now of a paused schedule is fire's to refuse");
        assert_eq!(items[5].action, MenuAction::Details(MenuTarget::Schedule { folder_key: "demo".into(), id: "nightly".into() }));
        assert_eq!(items[6].label, "Remove…");
    }

    #[test]
    fn a_folder_menu_preselects_its_folder_and_a_top_level_one_holds_its_entrys_items() {
        let group = node(true);
        let items = folder_items(&group, &group.folders[0], Pos::member(2, 0), true);
        assert_eq!(keys(&items), ["new-claude", "new-task", "new-scheduled-task", "open-git", "open-terminal", "open-folder", "details", "remove-folder"]);
        assert_eq!(items[0].action, MenuAction::NewClaude { cwd: r"C:\demo\Alpha".into(), title: "demo · Alpha · claude".into() });
        assert_eq!(items[1].action, MenuAction::NewTask { entry: "demo".into(), folder: Some(r"C:\demo\Alpha".into()), scheduled: false }, "the folder is preselected");
        assert_eq!(items[2].action, MenuAction::NewTask { entry: "demo".into(), folder: Some(r"C:\demo\Alpha".into()), scheduled: true });
        assert_eq!(items[3].action, MenuAction::OpenGit(r"C:\demo\Alpha".into()));
        assert_eq!(items[6].action, MenuAction::Details(MenuTarget::Folder("demo/Alpha".into())), "a group's folder has its own Details");
        assert_eq!((items[7].label.as_str(), &items[7].action), ("Remove Folder", &MenuAction::RemoveFolder { pos: Pos::member(2, 0), path: r"C:\demo\Alpha".into() }));
        let top = node(false);
        let items = folder_items(&top, &top.folders[0], Pos::top(1), true);
        assert_eq!(keys(&items), ["new-claude", "new-task", "new-scheduled-task", "open-git", "work", "open-terminal", "open-folder", "details", "delete-closed", "remove-folder"]);
        assert_eq!((items[4].label.as_str(), &items[4].action), ("Open Work", &MenuAction::Work("Alpha".into())));
        assert_eq!(items[7].action, MenuAction::Details(MenuTarget::Folder("Alpha".into())));
        assert_eq!(items[8].action, MenuAction::DeleteClosed("Alpha".into()));
        assert_eq!(items[9].action, MenuAction::RemoveFolder { pos: Pos::top(1), path: r"C:\demo\Alpha".into() });
        let mut plain = top.clone();
        plain.folders[0].kind = supraman_core::FolderKind::Plain;
        assert!(!keys(&folder_items(&plain, &plain.folders[0], Pos::top(1), false)).contains(&"open-git"), "Open Git only for a repo");
        let schedules = schedules_items("demo", Path::new(r"C:\demo\Alpha"));
        assert_eq!(keys(&schedules), ["new-scheduled-task"]);
        assert_eq!(schedules[0].action, MenuAction::NewTask { entry: "demo".into(), folder: Some(r"C:\demo\Alpha".into()), scheduled: true }, "in its folder");
    }

    #[test]
    fn session_menu_asks_before_stopping_a_session_supraman_did_not_launch() {
        let live = r#"{"id":"aaaa0001","sessionId":"aaaa0001-x","name":"mine","cwd":"C:\\demo","kind":"background","startedAt":1,"state":"working","status":"busy","pid":1}"#;
        let items = session_items("demo", "demo/Alpha", &row(live, Health::Running), false, Some("item-1".into()), None, false);
        assert_eq!(keys(&items), ["attach", "stop-session", "copy-name", "copy-id", "show-in-work", "star", "details"]);
        assert_eq!(items[1].label, "Stop…");
        assert_eq!(items[3].action, MenuAction::Copy("aaaa0001".into()));
        assert_eq!(
            items[6].action,
            MenuAction::Details(MenuTarget::Session { entry: "demo".into(), place: "demo/Alpha".into(), session_id: "aaaa0001-x".into() })
        );

        let sm = live.replace("\"mine\"", "\"fm-demo-alpha\"");
        assert_eq!(session_items("demo", "demo/Alpha", &row(&sm, Health::Running), false, None, None, false)[1].label, "Stop");

        let stopped = r#"{"id":"aaaa0002","sessionId":"aaaa0002-x","name":"","cwd":"C:\\demo","kind":"background","startedAt":1,"state":"stopped"}"#;
        assert_eq!(keys(&session_items("demo", "demo/Alpha", &row(stopped, Health::Stopped), true, None, None, false)), ["resume", "copy-id", "star", "details"]);

        let interactive = r#"{"sessionId":"bbbb0005-x","name":"term","cwd":"C:\\demo","kind":"interactive","startedAt":1,"status":"idle","pid":5}"#;
        let items = session_items("demo", "demo/Alpha", &row(interactive, Health::Idle), false, None, None, false);
        assert_eq!(keys(&items), ["copy-name", "copy-id", "star", "details"], "an interactive session is neither attached nor stopped from here");
        assert_eq!(items[1].action, MenuAction::Copy("bbbb0005".into()));
        assert_eq!(items[2].action, MenuAction::ToggleStar("bbbb0005-x".into()));
        let mut starred = row(interactive, Health::Idle);
        starred.starred = true;
        assert_eq!(session_items("demo", "demo/Alpha", &starred, false, None, None, false)[2].label, "Unstar");
    }

    #[test]
    fn a_live_session_offers_its_sendmessage_name_beside_name_and_id() {
        let interactive = r#"{"sessionId":"bbbb0005-x","name":"Fix the tree","cwd":"C:\\demo","kind":"interactive","startedAt":1,"status":"idle","pid":5}"#;
        let items = session_items("demo", "demo/Alpha", &row(interactive, Health::Unknown), false, None, Some("twin [d923d7]".into()), false);
        assert_eq!(keys(&items), ["copy-send-name", "copy-name", "copy-id", "star", "details"]);
        assert_eq!(items[0].label, "Copy SendMessage name");
        assert_eq!(items[0].action, MenuAction::Copy("twin [d923d7]".into()));
        assert_eq!(items[1].action, MenuAction::Copy("Fix the tree".into()));
    }

    #[test]
    fn delete_comes_last_when_offered_and_the_group_offers_the_bulk_delete() {
        let stopped = r#"{"id":"aaaa0002","sessionId":"aaaa0002-0000-4000-8000-000000000002","name":"old","cwd":"C:\\demo","kind":"background","startedAt":1,"state":"stopped"}"#;
        let items = session_items("demo", "demo/Alpha", &row(stopped, Health::Stopped), true, None, None, true);
        assert_eq!(keys(&items), ["resume", "copy-name", "copy-id", "star", "details", "delete"]);
        assert_eq!(items[5].label, "Delete…");
        assert!(matches!(&items[5].action, MenuAction::Delete(s) if s.id == "aaaa0002"));
        let group = group_items(&node(true), true);
        assert_eq!(keys(&group)[5..], ["details", "delete-closed", "remove-group"]);
        assert_eq!(group[6].label, "Delete closed background sessions…");
        assert_eq!(group[6].action, MenuAction::DeleteClosed("demo".into()));
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
