use std::path::PathBuf;

use supraman_claude::{Claude, Session};
use gpui::{ClipboardItem, Context, Pixels, Point, Window};

use super::FleetView;
use super::menu::{Confirm, Confirmed, ContextMenu, MenuAction, MenuItem, MenuTarget, folder_items, group_items, is_foremans, other_schedule_items, schedule_items, schedules_items, session_items};
use crate::fleet::{self, SessionRow};
use crate::work_view;

pub(super) fn send_name_of(session_id: &str) -> Option<String> {
    use supraman_claude::peer;
    let live = peer::live_entries(&peer::sessions_dir()?, &peer::System);
    peer::send_name(&live, session_id)
}

impl FleetView {
    /// The action bar's +: a folder picked, added at the top level's end.
    pub fn add_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dialogs.menu = None;
        if let Some(path) = self.dialogs.pick_folder.take() {
            self.add_top_folder(path, cx);
            return;
        }
        supraman_ui::folder_picker::pick_folder("Add a folder to Supraman", window, cx, |this, path, _, cx| this.add_top_folder(path, cx));
    }

    pub fn show_toast(&mut self, text: String, cx: &mut Context<Self>) {
        self.toast.show(text, |this: &mut Self| &mut this.toast, cx);
    }

    pub fn open_menu(&mut self, target: MenuTarget, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.dialogs.menu = self.menu_for(&target, None, cx).map(|(title, items)| ContextMenu::new(title, position, items));
        cx.notify();
        let session_id = match &target {
            MenuTarget::Session { session_id, .. } => Some(session_id.clone()),
            MenuTarget::Group(_) | MenuTarget::Folder(_) | MenuTarget::Schedule { .. } | MenuTarget::Schedules(_) | MenuTarget::OtherSchedule(_) => None,
        };
        let (Some(menu), Some(session_id)) = (&self.dialogs.menu, session_id) else {
            return;
        };
        let title = menu.title.clone();
        let lookup = cx.background_executor().spawn(async move { send_name_of(&session_id) });
        cx.spawn(async move |this, cx| {
            let Some(send_name) = lookup.await else {
                return;
            };
            this.update(cx, |this, cx| {
                if !this.dialogs.menu.as_ref().is_some_and(|m| m.title == title && m.position == position) {
                    return;
                }
                if let Some((_, items)) = this.menu_for(&target, Some(send_name), cx)
                    && let Some(menu) = &mut this.dialogs.menu
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
            MenuTarget::Group(name) => tree.entry(name).map(|node| (name.clone(), group_items(node, self.has_closed_background(name)))),
            MenuTarget::Folder(key) => tree
                .folders()
                .find(|(_, f)| &f.key == key)
                .map(|(entry, f)| (f.place(), folder_items(entry, f, self.has_closed_background(&entry.name)))),
            MenuTarget::Session { entry, place, session_id } => tree
                .rows()
                .find(|r| &r.session.session_id == session_id)
                .map(|row| (fleet::display_name(&row.session).to_string(), self.session_menu(entry, place, row, send_name, cx))),
            MenuTarget::Schedule { workspace, id } => {
                self.schedule_row(workspace, id).map(|row| (format!("{workspace} · {}", row.title()), schedule_items(row)))
            }
            MenuTarget::Schedules(workspace) => Some((format!("{workspace} · Schedules"), schedules_items(workspace))),
            MenuTarget::OtherSchedule(path) => self.others().find(path).map(|t| (t.path(), other_schedule_items(t))),
        }
    }

    pub(super) fn session_menu(&self, entry: &str, place: &str, row: &SessionRow, send_name: Option<String>, cx: &gpui::App) -> Vec<MenuItem> {
        let claimed = self.claimed_item(entry, &row.session.name, cx);
        session_items(entry, place, row, self.resumable(row), claimed, send_name, self.deletable(row))
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
            MenuAction::Work(w) => {
                self.open_work(&w, cx);
            }
            MenuAction::NewTask { entry, folder, scheduled } => {
                self.open_start(&entry, folder, scheduled, window, cx);
            }
            MenuAction::OpenTaskScheduler => self.open_folder(PathBuf::from("taskschd.msc"), cx),
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
            MenuAction::Attach { place, session } => {
                self.attach(&place, &session, cx);
            }
            MenuAction::Resume { place, session } => {
                self.resume(&place, &session, cx);
            }
            MenuAction::StopSession(session) if is_foremans(&session) => self.stop_session(session, cx),
            MenuAction::StopSession(session) => self.ask_stop_foreign(session),
            MenuAction::ToggleStar(session_id) => self.toggle_star(&session_id, cx),
            MenuAction::Copy(text) => {
                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                self.show_toast(format!("Copied {text}"), cx);
            }
            MenuAction::ShowInWork { workspace, item_id } => {
                self.open_work_item(&workspace, &item_id, cx);
            }
            MenuAction::Delete(session) => self.ask_delete(session),
            MenuAction::DeleteClosed(entry) => self.ask_delete_closed(entry, cx),
            MenuAction::Details(MenuTarget::Schedule { workspace, id }) => {
                self.open_schedule(&workspace, &id, super::schedules::At::Top, cx);
            }
            MenuAction::Details(target) => {
                self.open_details(target, cx);
            }
            schedule @ (MenuAction::Approve { .. }
            | MenuAction::RunNow { .. }
            | MenuAction::EditSchedule { .. }
            | MenuAction::DryRun { .. }
            | MenuAction::SetPaused { .. }
            | MenuAction::History { .. }
            | MenuAction::RemoveSchedule { .. }) => self.run_schedule_action(schedule, window, cx),
            tab @ (MenuAction::OpenGit(_) | MenuAction::OpenWorkman { .. } | MenuAction::CloseTab(_) | MenuAction::CloseOtherTabs(_) | MenuAction::CloseTabsRight(_) | MenuAction::ReopenClosed) => {
                self.run_tab_action(tab, cx);
            }
        }
        cx.notify();
    }

    fn run_schedule_action(&mut self, action: MenuAction, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            MenuAction::Approve { workspace, id } => {
                self.open_schedule(&workspace, &id, super::schedules::At::Approve, cx);
            }
            MenuAction::RunNow { workspace, id } => self.run_schedule_now(&workspace, &id, cx),
            MenuAction::EditSchedule { workspace, id } => self.open_schedule_edit(&workspace, &id, window, cx),
            MenuAction::DryRun { workspace, id } => self.dry_run_schedule(&workspace, &id, cx),
            MenuAction::SetPaused { workspace, id, paused } => self.set_schedule_paused(workspace, id, paused, cx),
            MenuAction::History { workspace, id } => self.schedule_history(&workspace, &id, cx),
            MenuAction::RemoveSchedule { workspace, id } => self.ask_remove_schedule(&workspace, &id, cx),
            _ => {}
        }
    }

    fn ask_stop_foreign(&mut self, session: Session) {
        self.dialogs.confirm = Some(Confirm::ask(
            format!(
                "{} ({}) was not launched by Supraman. Stop it? Its transcript stays and it can be resumed.",
                if session.name.is_empty() { "This session" } else { &session.name },
                session.id
            ),
            "Stop",
            Confirmed::StopSession(session),
        ));
    }

    pub fn answer_confirm(&mut self, yes: bool, cx: &mut Context<Self>) {
        if let Some(Confirm { action: Some(action), .. }) = self.dialogs.confirm.take()
            && yes
        {
            match action {
                Confirmed::StopSession(session) => self.stop_session(session, cx),
                Confirmed::Delete { session, discard } => self.delete_now(session, discard, cx),
                Confirmed::DeleteClosed { entry, sessions, skipped } => self.delete_closed_now(entry, sessions, skipped, cx),
                Confirmed::RemoveSchedule { workspace, id } => self.remove_schedule(workspace, id, cx),
                Confirmed::Update => self.update_confirmed(cx),
            }
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
