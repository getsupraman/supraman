//! The Schedules group's actions; every change of a schedule's task goes through `schedule::sync`.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use supraman_core::schedule::{self, Clock, Day, ForeignTask, Overlap, Profile, Registrar, Schedule, Span, Status, SyncTarget, Trigger};
use supraman_core::secret::SecretStore;
use supraman_core::{Workspace, process};
use supraman_ui::button;
use supraman_ui::details::{self, Section};
use supraman_ui::hit::hit_target;
use supraman_ui::text_input::{InputEvent, TextInput};
use gpui::{AnyElement, AppContext as _, ClickEvent, Context, Entity, MouseButton, MouseDownEvent, ScrollHandle, SharedString, Subscription, Window, div, prelude::*, px};
use theme::ActiveTheme as _;

use super::{FleetView, Tab, TabKind};
use supraman_ui::icon::Icon;
use super::left::{REPO_ROW, SESSION_LINE, disclosure, grid, icon_slot, secondary, tree_label};
use super::menu::{Confirm, Confirmed, MenuTarget};
use supraman_ui::clipped::clipped;
use crate::fleet::WorkspaceNode;
use crate::schedules::{self, Primary, ScheduleRow, Waiting};

const DRY_RUN_WITHIN: Duration = Duration::from_secs(60);

/// The real Task Scheduler, or the selftest's fake (see C101).
pub type SharedRegistrar = Arc<dyn Registrar + Send + Sync>;

/// `<ws>/<id>`: a schedule's key in hit keys and maps.
pub fn key(workspace: &str, id: &str) -> String {
    format!("{workspace}/{id}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerKind {
    Weekly,
    Daily,
    Interval,
}

/// A schedule's Details tab: the page renders its row as of the last poll.
pub struct SchedulePage {
    pub workspace: String,
    pub id: String,
    pub scroll: ScrollHandle,
    /// The last Dry run's output, or why it failed.
    pub dry_run: Option<String>,
    pub dry_running: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum At {
    Top,
    /// What Approve approves: the change since the last approval, when there is one.
    Approve,
    Runs,
}

/// What the Edit dialog changes; the rest of the schedule is `sm schedule edit`'s.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Form {
    pub name: String,
    pub kind: TriggerKind,
    pub days: Vec<Day>,
    pub at: String,
    pub every: String,
    pub max_runtime: String,
    pub max_tokens: String,
    pub profile: Profile,
    pub overlap: Overlap,
}

impl Form {
    pub fn of(s: &Schedule) -> Self {
        let (kind, days, at, every) = match &s.trigger {
            Trigger::Weekly { days, at } => (TriggerKind::Weekly, days.clone(), *at, Span(2 * 3600)),
            Trigger::Daily { at } => (TriggerKind::Daily, vec![Day::Mon], *at, Span(2 * 3600)),
            Trigger::Interval { at, every } => (TriggerKind::Interval, vec![Day::Mon], *at, *every),
        };
        Self {
            name: s.name.clone(),
            kind,
            days,
            at: at.to_string(),
            every: every.to_string(),
            max_runtime: s.caps.max_runtime.to_string(),
            max_tokens: s.caps.max_tokens.to_string(),
            profile: s.permissions.profile,
            overlap: s.policy.overlap,
        }
    }

    /// All or none; `Err` names the field that does not parse, `schedule::edit` validates the rest.
    pub fn apply(&self, s: &mut Schedule) -> Result<(), String> {
        let at: Clock = self.at.trim().parse().map_err(|e| format!("At: {e}"))?;
        let trigger = match self.kind {
            TriggerKind::Weekly if self.days.is_empty() => return Err("Days: choose at least one day".into()),
            TriggerKind::Weekly => {
                let mut days = self.days.clone();
                days.sort();
                days.dedup();
                Trigger::Weekly { days, at }
            }
            TriggerKind::Daily => Trigger::Daily { at },
            TriggerKind::Interval => Trigger::Interval { at, every: self.every.trim().parse().map_err(|e| format!("Every: {e}"))? },
        };
        let max_runtime: Span = self.max_runtime.trim().parse().map_err(|e| format!("Max runtime: {e}"))?;
        let max_tokens: u64 =
            self.max_tokens.trim().replace(['_', ','], "").parse().map_err(|_| format!("Max tokens: '{}' is not a whole number", self.max_tokens.trim()))?;
        s.trigger = trigger;
        s.name = self.name.trim().to_string();
        s.caps.max_runtime = max_runtime;
        s.caps.max_tokens = max_tokens;
        s.permissions.profile = self.profile;
        s.policy.overlap = self.overlap;
        Ok(())
    }
}

pub struct ScheduleEdit {
    pub workspace: String,
    pub id: String,
    pub kind: TriggerKind,
    pub days: Vec<Day>,
    pub profile: Profile,
    pub overlap: Overlap,
    pub name: Entity<TextInput>,
    pub at: Entity<TextInput>,
    pub every: Entity<TextInput>,
    pub max_runtime: Entity<TextInput>,
    pub max_tokens: Entity<TextInput>,
    pub error: Option<String>,
    saving: bool,
    _events: Vec<Subscription>,
}

impl ScheduleEdit {
    pub fn form(&self, cx: &gpui::App) -> Form {
        let text = |e: &Entity<TextInput>| e.read(cx).text().to_string();
        Form {
            name: text(&self.name),
            kind: self.kind,
            days: self.days.clone(),
            at: text(&self.at),
            every: text(&self.every),
            max_runtime: text(&self.max_runtime),
            max_tokens: text(&self.max_tokens),
            profile: self.profile,
            overlap: self.overlap,
        }
    }
}

fn sync_now(home: &Path, registrar: &dyn Registrar, store: &dyn SecretStore, workspace: &Workspace) -> Result<schedule::Synced, String> {
    let schedules = schedule::load(home, &workspace.name).map_err(|e| e.to_string())?;
    let smw = schedule::smw_exe();
    let target = SyncTarget { home, store, level: supraman_claude::task_run_level(), smw: &smw, now_ms: supraman_core::time::now_ms() };
    schedule::sync(registrar, &target, workspace, &schedules).map_err(|e| format!("Task Scheduler refused the sync of {}: {e}", workspace.name))
}

/// Why a step failed: the edit (nothing changed) or the sync after it (the schedule changed, its task did not).
enum Failed {
    Edit(String),
    Sync(String),
}

impl FleetView {
    pub fn schedule_row(&self, workspace: &str, id: &str) -> Option<&ScheduleRow> {
        self.schedules.get(workspace)?.iter().find(|r| r.schedule.id == id)
    }

    fn registered_workspace(&self, workspace: &str) -> Option<Workspace> {
        self.workspaces.iter().find(|w| w.name == workspace).cloned()
    }

    /// The row's one action, also its double-click; Approve opens the Details tab on what it approves.
    pub fn schedule_primary(&mut self, workspace: &str, id: &str, cx: &mut Context<Self>) {
        match self.schedule_row(workspace, id).map(ScheduleRow::primary) {
            Some(Primary::Approve) => {
                self.open_schedule(workspace, id, At::Approve, cx);
            }
            Some(Primary::RunNow) => self.run_schedule_now(workspace, id, cx),
            None => {}
        }
    }

    pub fn schedule_page(&self, workspace: &str, id: &str) -> Option<(u64, &SchedulePage)> {
        self.tabs.iter().find_map(|t| match &t.kind {
            TabKind::Schedule(p) if p.workspace == workspace && p.id == id => Some((t.id, p)),
            _ => None,
        })
    }

    fn schedule_page_mut(&mut self, workspace: &str, id: &str) -> Option<&mut SchedulePage> {
        self.tabs.iter_mut().find_map(|t| match &mut t.kind {
            TabKind::Schedule(p) if p.workspace == workspace && p.id == id => Some(p),
            _ => None,
        })
    }

    pub fn page_sections(&self, row: &ScheduleRow, page: Option<&SchedulePage>) -> Vec<Section> {
        let now = supraman_core::time::now_ms();
        schedules::sections(row, &schedules::zone_words(now), page.and_then(|p| p.dry_run.as_deref()), now)
    }

    pub fn open_schedule(&mut self, workspace: &str, id: &str, at: At, cx: &mut Context<Self>) -> Option<u64> {
        let row = self.schedule_row(workspace, id)?.clone();
        self.menu = None;
        let tab = match self.schedule_page(workspace, id) {
            Some((tab, _)) => tab,
            None => {
                let page = SchedulePage { workspace: workspace.to_string(), id: id.to_string(), scroll: ScrollHandle::new(), dry_run: None, dry_running: false };
                let tab = self.next_tab_id;
                self.next_tab_id += 1;
                self.tabs.push(Tab::new(tab, format!("Schedule · {}", row.title()), TabKind::Schedule(page), row.folder_path.clone()));
                tab
            }
        };
        let key = match at {
            At::Top => None,
            At::Approve => Some("changed"),
            At::Runs => Some("runs"),
        };
        if let Some(key) = key
            && let Some((_, page)) = self.schedule_page(workspace, id)
            && let Some(ix) = details::index_of(&self.page_sections(&row, Some(page)), key)
        {
            page.scroll.scroll_to_item(ix);
        }
        self.active = Some(tab);
        cx.notify();
        Some(tab)
    }

    /// The fingerprint the page shows, which [`Self::approve_schedule`] refuses once the file holds another.
    pub fn approve_shown(&mut self, workspace: &str, id: &str, cx: &mut Context<Self>) {
        match self.schedule_row(workspace, id).map(|r| r.approvable.clone()) {
            Some(Some(Ok(covered))) => self.approve_schedule(workspace.to_string(), id.to_string(), covered.fingerprint, cx),
            Some(Some(Err(why))) => self.action_error = Some(format!("Cannot approve {workspace}/{id}: {why}")),
            _ => {}
        }
        cx.notify();
    }

    /// The app's only approval (see C100).
    pub fn approve_schedule(&mut self, workspace: String, id: String, fingerprint: String, cx: &mut Context<Self>) {
        let store = self.schedule_store.clone();
        self.change_schedule(workspace, id, "approved; its task is registered", cx, move |home, w, id| {
            let s = schedule::load(home, &w.name).map_err(|e| e.to_string())?.into_iter().find(|s| s.id == id).ok_or("the schedule is gone")?;
            let now = schedule::covered(w, &s).map_err(|e| e.to_string())?;
            if now.fingerprint != fingerprint {
                return Err("it changed after the page showed it: read the page again, then Approve".into());
            }
            schedule::approve(home, &*store, w, id, supraman_core::time::now_ms()).map(|_| ()).map_err(|e| e.to_string())
        });
    }

    pub fn set_schedule_paused(&mut self, workspace: String, id: String, paused: bool, cx: &mut Context<Self>) {
        let done = if paused { "paused; its task is removed" } else { "resumed" };
        self.change_schedule(workspace, id, done, cx, move |home, w, id| {
            schedule::edit(home, w, |all| {
                let s = all.iter_mut().find(|s| s.id == id).ok_or_else(|| supraman_core::CoreError::NoSchedule { workspace: w.name.clone(), id: id.to_string() })?;
                s.status = if paused { Status::Paused } else { Status::Active };
                Ok(())
            })
            .map_err(|e| e.to_string())
        });
    }

    pub fn ask_remove_schedule(&mut self, workspace: &str, id: &str, cx: &mut Context<Self>) {
        self.confirm = Some(Confirm::ask(
            format!("Remove schedule {id} from {workspace}? Its task and its entry go. Its run records stay, and nothing in any folder is touched."),
            "Remove",
            Confirmed::RemoveSchedule { workspace: workspace.to_string(), id: id.to_string() },
        ));
        cx.notify();
    }

    pub fn remove_schedule(&mut self, workspace: String, id: String, cx: &mut Context<Self>) {
        self.change_schedule(workspace, id, "removed with its task", cx, |home, w, id| {
            schedule::edit(home, w, |all| {
                let before = all.len();
                all.retain(|s| s.id != id);
                if all.len() == before {
                    return Err(supraman_core::CoreError::NoSchedule { workspace: w.name.clone(), id: id.to_string() });
                }
                Ok(())
            })
            .map_err(|e| e.to_string())
        });
    }

    /// A sync failure stays on the row and in the Inbox until a sync of the workspace succeeds.
    fn change_schedule(
        &mut self,
        workspace: String,
        id: String,
        done: &'static str,
        cx: &mut Context<Self>,
        edit: impl FnOnce(&Path, &Workspace, &str) -> Result<(), String> + Send + 'static,
    ) {
        let (Some(home), Some(w)) = (self.home.clone(), self.registered_workspace(&workspace)) else {
            return;
        };
        let (registrar, store) = (self.registrar.clone(), self.schedule_store.clone());
        let row = key(&workspace, &id);
        let task = cx.background_executor().spawn(async move {
            edit(&home, &w, &id).map_err(Failed::Edit)?;
            sync_now(&home, &*registrar, &*store, &w).map_err(Failed::Sync)?;
            Ok::<_, Failed>(id)
        });
        let ws = workspace;
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(id) => {
                        this.sync_errors.retain(|k, _| !k.starts_with(&format!("{ws}/")));
                        this.show_toast(format!("Schedule {id} of {ws} {done}."), cx);
                    }
                    Err(Failed::Edit(e)) => this.action_error = Some(format!("Cannot change schedule of {ws}: {e}")),
                    Err(Failed::Sync(e)) => {
                        this.sync_errors.insert(row, e);
                    }
                }
                this.poll_nudge.wake();
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Detached and off the UI thread; the row shows the run from its live file.
    pub fn run_schedule_now(&mut self, workspace: &str, id: &str, cx: &mut Context<Self>) {
        let Some(home) = self.home.clone() else {
            return;
        };
        let (ws, id) = (workspace.to_string(), id.to_string());
        let args = (ws.clone(), id.clone());
        let task = cx.background_executor().spawn(async move {
            let smw = schedule::smw_exe();
            process::spawn_detached(process::command(&smw).arg("schedule").arg("run").arg("--home").arg(&home).arg(&args.0).arg(&args.1))
                .map_err(|e| format!("cannot start {}: {e}", smw.display()))
        });
        cx.spawn(async move |this, cx| {
            let started = task.await;
            this.update(cx, |this, cx| {
                match started {
                    Ok(_) => this.show_toast(format!("Run now: {ws}/{id} started."), cx),
                    Err(e) => this.action_error = Some(format!("Cannot run {ws}/{id}: {e}")),
                }
                this.poll_nudge.wake();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn dry_run_schedule(&mut self, workspace: &str, id: &str, cx: &mut Context<Self>) {
        let Some(home) = self.home.clone() else {
            return;
        };
        self.open_schedule(workspace, id, At::Top, cx);
        if let Some(page) = self.schedule_page_mut(workspace, id) {
            page.dry_running = true;
        }
        let (ws, id) = (workspace.to_string(), id.to_string());
        let args = (ws.clone(), id.clone());
        let task = cx.background_executor().spawn(async move {
            let sm = supraman_core::install::sm_exe().map_err(|e| e.to_string())?;
            let out = process::run(
                process::command(&sm).args(["schedule", "dry-run", &args.0, &args.1]).env(supraman_core::HOME_VAR, &home),
                DRY_RUN_WITHIN,
                process::Kill::Tree,
            )
            .map_err(|e| format!("{}: {e}", sm.display()))?;
            if out.success() { Ok(out.stdout) } else { Err(format!("{}{}", out.stdout, out.stderr).trim().to_string()) }
        });
        cx.spawn(async move |this, cx| {
            let out = task.await;
            this.update(cx, |this, cx| {
                let text = match out {
                    Ok(text) => text,
                    Err(e) => format!("The dry run failed: {e}"),
                };
                if let Some(page) = this.schedule_page_mut(&ws, &id) {
                    page.dry_run = Some(text);
                    page.dry_running = false;
                    page.scroll.scroll_to_bottom();
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn schedule_history(&mut self, workspace: &str, id: &str, cx: &mut Context<Self>) {
        self.open_schedule(workspace, id, At::Runs, cx);
    }

    pub fn open_schedule_edit(&mut self, workspace: &str, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.schedule_row(workspace, id) else {
            return;
        };
        let form = Form::of(&row.schedule);
        let mut events = Vec::new();
        let mut input = |placeholder: &'static str, text: &str, window: &mut Window, cx: &mut Context<Self>| {
            let input = cx.new(|cx| {
                let mut i = TextInput::new(placeholder, None, window, cx);
                i.set_text(text, cx);
                i
            });
            events.push(cx.subscribe_in(&input, window, |this: &mut Self, _, event: &InputEvent, window, cx| this.on_schedule_edit_event(event, window, cx)));
            input
        };
        let name = input("name", &form.name, window, cx);
        let at = input("HH:MM", &form.at, window, cx);
        let every = input("2h", &form.every, window, cx);
        let max_runtime = input("1h", &form.max_runtime, window, cx);
        let max_tokens = input("3000000", &form.max_tokens, window, cx);
        name.update(cx, |n, cx| n.focus_all(window, cx));
        self.menu = None;
        self.schedule_edit = Some(ScheduleEdit {
            workspace: workspace.to_string(),
            id: id.to_string(),
            kind: form.kind,
            days: form.days,
            profile: form.profile,
            overlap: form.overlap,
            name,
            at,
            every,
            max_runtime,
            max_tokens,
            error: None,
            saving: false,
            _events: events,
        });
        cx.notify();
    }

    fn on_schedule_edit_event(&mut self, event: &InputEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            InputEvent::Confirm | InputEvent::ConfirmBack | InputEvent::Submit => self.save_schedule_edit(cx),
            InputEvent::Cancel => {
                self.schedule_edit = None;
                self.focus.focus(window, cx);
            }
            InputEvent::Edited | InputEvent::Refused(_) => {
                if let Some(e) = &mut self.schedule_edit {
                    e.error = None;
                }
            }
        }
        cx.notify();
    }

    /// A covered change makes the schedule pending, so it needs Approve again.
    pub fn save_schedule_edit(&mut self, cx: &mut Context<Self>) {
        let Some(edit) = &self.schedule_edit else {
            return;
        };
        if edit.saving {
            return;
        }
        let (Some(home), Some(w)) = (self.home.clone(), self.registered_workspace(&edit.workspace)) else {
            return;
        };
        let (form, id) = (edit.form(cx), edit.id.clone());
        if let Some(edit) = &mut self.schedule_edit {
            edit.saving = true;
        }
        let (registrar, store) = (self.registrar.clone(), self.schedule_store.clone());
        let task = {
            let (w, id) = (w.clone(), id.clone());
            cx.background_executor().spawn(async move {
                let status = schedule::edit(&home, &w, |all| {
                    let s = all.iter_mut().find(|s| s.id == id).ok_or_else(|| supraman_core::CoreError::NoSchedule { workspace: w.name.clone(), id: id.clone() })?;
                    form.apply(s).map_err(|why| supraman_core::CoreError::InvalidSchedule { id: id.clone(), why })?;
                    Ok(s.status)
                })
                .map_err(|e| e.to_string())?;
                Ok::<_, String>((status, sync_now(&home, &*registrar, &*store, &w)))
            })
        };
        cx.spawn(async move |this, cx| {
            let saved = task.await;
            this.update(cx, |this, cx| {
                let editing = this.schedule_edit.as_ref().is_some_and(|e| e.workspace == w.name && e.id == id);
                match saved {
                    Ok((status, synced)) => {
                        if editing {
                            this.schedule_edit = None;
                        }
                        let pending = if status == Status::Pending { "; it is pending until you Approve it" } else { "" };
                        this.show_toast(format!("Saved schedule {id} of {}{pending}.", w.name), cx);
                        if let Err(e) = synced {
                            this.sync_errors.insert(key(&w.name, &id), e);
                        }
                        this.poll_nudge.wake();
                    }
                    Err(e) => {
                        if let Some(edit) = this.schedule_edit.as_mut().filter(|_| editing) {
                            edit.saving = false;
                            edit.error = Some(e);
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

    pub fn dismiss_schedule_notice(&mut self, workspace: &str, run: &str, cx: &mut Context<Self>) {
        let save = self.runs_dismissed.dismiss(workspace, run);
        self.save_off_thread(save, "Cannot save the dismissed run", cx);
        cx.notify();
    }

    /// The Details tab of `page`'s schedule; `None` once the schedule is gone.
    pub(super) fn render_schedule_page(&self, page: &SchedulePage, cx: &mut Context<Self>) -> Option<AnyElement> {
        let row = self.schedule_row(&page.workspace, &page.id)?.clone();
        let colors = cx.theme().colors().clone();
        let status = cx.theme().status().clone();
        let sections = self.page_sections(&row, Some(page));
        let (pill, asks) = schedules::status_words(&row);
        let pill_color = match (asks, row.status) {
            (true, _) => status.warning,
            (false, Status::Paused) => colors.text_muted,
            (false, _) => status.success,
        };
        let k = key(&page.workspace, &page.id);
        let (w, id) = (page.workspace.clone(), page.id.clone());
        let act = |f: fn(&mut Self, &str, &str, &mut Window, &mut Context<Self>)| {
            let (w, id) = (w.clone(), id.clone());
            cx.listener(move |this: &mut Self, _: &ClickEvent, window, cx| {
                cx.stop_propagation();
                f(this, &w, &id, window, cx);
            })
        };
        let mut actions = Vec::new();
        if matches!(row.approvable, Some(Ok(_))) {
            actions.push(
                button::outline(&self.hits, format!("sched-page-approve-{k}"), "Approve", status.warning, cx)
                    .on_click(act(|this, w, id, _, cx| this.approve_shown(w, id, cx)))
                    .into_any_element(),
            );
        }
        let paused = row.schedule.status == Status::Paused;
        actions.push(
            button::bordered(&self.hits, format!("sched-page-pause-{k}"), if paused { "Resume" } else { "Pause" }, cx)
                .on_click(if paused {
                    act(|this, w, id, _, cx| this.set_schedule_paused(w.to_string(), id.to_string(), false, cx))
                } else {
                    act(|this, w, id, _, cx| this.set_schedule_paused(w.to_string(), id.to_string(), true, cx))
                })
                .into_any_element(),
        );
        actions.push(button::bordered(&self.hits, format!("sched-page-run-{k}"), "Run now", cx).on_click(act(|this, w, id, _, cx| this.run_schedule_now(w, id, cx))).into_any_element());
        actions.push(
            button::bordered(&self.hits, format!("sched-page-dry-run-{k}"), if page.dry_running { "Dry run…" } else { "Dry run" }, cx)
                .on_click(act(|this, w, id, _, cx| this.dry_run_schedule(w, id, cx)))
                .into_any_element(),
        );
        actions.push(button::bordered(&self.hits, format!("sched-page-edit-{k}"), "Edit…", cx).on_click(act(|this, w, id, window, cx| this.open_schedule_edit(w, id, window, cx))).into_any_element());
        actions.push(button::bordered(&self.hits, format!("sched-page-remove-{k}"), "Remove…", cx).on_click(act(|this, w, id, _, cx| this.ask_remove_schedule(w, id, cx))).into_any_element());
        let page_key = format!("sched-page-{k}");
        let page = details::Page { key: &page_key, title: row.title().to_string(), pill: Some((pill.to_string(), pill_color)), sections: &sections, actions, scroll: &page.scroll, on_link: None, top: None, over: None, bar: None };
        Some(details::render(page, &self.hits, cx))
    }

    pub(super) fn render_schedule_edit(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let edit = self.schedule_edit.as_ref()?;
        let colors = cx.theme().colors().clone();
        let status = cx.theme().status().clone();
        let label = |text: &'static str| div().flex_none().w(px(88.)).text_xs().text_color(colors.text_muted).child(text);
        let field = |key: &'static str, input: &Entity<TextInput>| {
            div().id(key).relative().flex_1().min_w_0().child(input.clone()).child(hit_target(&self.hits, key.into()))
        };
        let line = |name: &'static str| div().flex().items_center().gap_2().min_w_0().child(label(name));
        let choice = |key: String, text: &'static str, selected: bool, cx: &mut Context<Self>| {
            button::choice(SharedString::from(key.clone()), text, selected, cx).relative().child(hit_target(&self.hits, key))
        };
        let mut kinds = line("Every");
        for (kind, text) in [(TriggerKind::Weekly, "weekly"), (TriggerKind::Daily, "daily"), (TriggerKind::Interval, "interval")] {
            kinds = kinds.child(choice(format!("sched-edit-kind-{text}"), text, edit.kind == kind, cx).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                if let Some(e) = &mut this.schedule_edit {
                    e.kind = kind;
                }
                cx.notify();
            })));
        }
        let mut days = line("Days").flex_wrap();
        for day in Day::ALL {
            days = days.child(choice(format!("sched-edit-day-{}", day.as_str()), day.as_str(), edit.days.contains(&day), cx).on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    if let Some(e) = &mut this.schedule_edit {
                        match e.days.iter().position(|d| *d == day) {
                            Some(i) => {
                                e.days.remove(i);
                            }
                            None => e.days.push(day),
                        }
                    }
                    cx.notify();
                },
            )));
        }
        let mut profiles = line("Profile");
        for p in [Profile::Read, Profile::Edit, Profile::Auto] {
            profiles = profiles.child(choice(format!("sched-edit-profile-{}", p.as_str()), p.as_str(), edit.profile == p, cx).on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    if let Some(e) = &mut this.schedule_edit {
                        e.profile = p;
                    }
                    cx.notify();
                },
            )));
        }
        let mut overlaps = line("Overlap");
        for (o, text) in [(Overlap::Skip, "skip"), (Overlap::Queue, "queue"), (Overlap::Restart, "restart")] {
            overlaps = overlaps.child(choice(format!("sched-edit-overlap-{text}"), text, edit.overlap == o, cx).on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    if let Some(e) = &mut this.schedule_edit {
                        e.overlap = o;
                    }
                    cx.notify();
                },
            )));
        }
        let buttons = div()
            .flex()
            .gap_2()
            .justify_end()
            .child(button::bordered(&self.hits, "sched-edit-save", "Save", cx).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.save_schedule_edit(cx);
            })))
            .child(button::bordered(&self.hits, "sched-edit-cancel", "Cancel", cx).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.schedule_edit = None;
                cx.notify();
            })));
        Some(
            div()
                .id("sched-edit")
                .relative()
                .flex()
                .flex_col()
                .gap_2()
                .min_w_0()
                .mx_3()
                .my_2()
                .p_2()
                .rounded_md()
                .border_1()
                .border_color(colors.border_focused)
                .bg(colors.editor_background)
                .child(hit_target(&self.hits, "sched-edit".into()))
                .child(clipped("sched-edit-title", format!("EDIT SCHEDULE · {} · {}", edit.workspace, edit.id), false).text_xs().text_color(colors.text_muted))
                .child(line("Name").child(field("sched-edit-name", &edit.name)))
                .child(kinds)
                .when(edit.kind == TriggerKind::Weekly, |d| d.child(days))
                .child(line("At").child(field("sched-edit-at", &edit.at)))
                .when(edit.kind == TriggerKind::Interval, |d| d.child(line("Interval").child(field("sched-edit-every", &edit.every))))
                .child(line("Max runtime").child(field("sched-edit-max-runtime", &edit.max_runtime)))
                .child(line("Max tokens").child(field("sched-edit-max-tokens", &edit.max_tokens)))
                .child(profiles)
                .child(overlaps)
                .child(div().text_xs().text_color(colors.text_muted).child("Everything here but the name is covered: a change makes the schedule pending until Approve."))
                .children(edit.error.clone().map(|e| {
                    div()
                        .relative()
                        .px_1()
                        .rounded_sm()
                        .bg(status.error.opacity(0.14))
                        .text_xs()
                        .text_color(status.error)
                        .child(SharedString::from(e))
                        .child(hit_target(&self.hits, "sched-edit-error".into()))
                }))
                .child(buttons)
                .into_any_element(),
        )
    }
}

impl FleetView {
    /// A run whose schedule is gone stays under the workspace.
    pub(super) fn render_schedules(&self, node: &WorkspaceNode, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let rows = self.schedules.get(&node.name).cloned().unwrap_or_default();
        let others = self.others();
        let others = others.of_workspace(&node.name);
        let mut out = Vec::new();
        if !rows.is_empty() || !others.is_empty() {
            let gkey = format!("schedules:{}", node.name);
            let open = !self.collapsed.contains(&gkey);
            out.push(self.render_schedules_row(&node.name, rows.len() + others.len(), open, cx));
            if open {
                for row in &rows {
                    let runs = node.scheduled.get(&row.schedule.id).map(Vec::as_slice).unwrap_or_default();
                    let skey = format!("schedule:{}", key(&node.name, &row.schedule.id));
                    let sopen = !self.collapsed.contains(&skey);
                    out.push(self.render_schedule_row(row, runs.len(), sopen, cx));
                    if sopen {
                        out.extend(runs.iter().map(|r| self.render_session(&node.name, &row.schedule.id, r, grid::indent(3), cx)));
                    }
                }
                out.extend(self.render_others(Some(&node.name), others, 2, cx));
            }
        }
        for (id, runs) in node.scheduled.iter().filter(|(id, _)| !rows.iter().any(|r| &r.schedule.id == *id)) {
            out.extend(runs.iter().map(|r| self.render_session(&node.name, id, r, grid::indent(1), cx)));
        }
        out
    }

    fn render_schedules_row(&self, workspace: &str, count: usize, open: bool, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let gkey = format!("schedules:{workspace}");
        let tree_key = format!("schedules-{workspace}");
        div()
            .id(SharedString::from(gkey.clone()))
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(grid::gap())
            .h(px(REPO_ROW))
            .pl(grid::indent(1))
            .pr_2()
            .text_sm()
            .cursor_pointer()
            .hover(|s| s.bg(colors.ghost_element_hover))
            .active(|s| s.bg(colors.ghost_element_active))
            .child(disclosure(Some(open), secondary(cx)))
            .child(icon_slot(&self.hits, &tree_key, Some(Icon::Clock.slotted(grid::ICON, secondary(cx)).into_any_element())))
            .child(tree_label(&self.hits, &tree_key, div().flex_none().whitespace_nowrap().child("Schedules")))
            .child(div().flex_none().text_xs().text_color(secondary(cx)).child(SharedString::from(count.to_string())))
            .child(hit_target(&self.hits, format!("schedules-row-{workspace}")))
            .on_mouse_down(MouseButton::Right, {
                let target = MenuTarget::Schedules(workspace.to_string());
                cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.open_menu(target.clone(), e.position, cx);
                })
            })
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle(gkey.clone(), cx)))
            .into_any_element()
    }

    /// Reads Task Scheduler off the UI thread, read only; a later scan's answer replaces an earlier one's.
    pub fn scan_other_schedules(&mut self, cx: &mut Context<Self>) {
        self.other_scans += 1;
        let scan = self.other_scans;
        let registrar = self.registrar.clone();
        let task = cx.background_executor().spawn(async move { schedule::scan_foreign(&*registrar) });
        cx.spawn(async move |this, cx| {
            let found = task.await;
            this.update(cx, |this, cx| {
                if this.other_scans != scan {
                    return;
                }
                match found {
                    Ok(found) => {
                        this.other_schedules = Some(found);
                        this.other_scan_error = None;
                    }
                    Err(e) => this.other_scan_error = Some(format!("Cannot read Task Scheduler: {e}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn others(&self) -> schedules::Others {
        self.other_schedules.as_ref().map(|f| schedules::Others::of(f, &self.workspaces)).unwrap_or_default()
    }

    /// The top-level Other schedules: tasks that run claude in no workspace's folder.
    pub(super) fn render_unowned_others(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let others = self.others();
        self.render_others(None, &others.unowned, 0, cx)
    }

    fn render_others(&self, workspace: Option<&str>, tasks: &[ForeignTask], level: usize, cx: &mut Context<Self>) -> Vec<AnyElement> {
        if tasks.is_empty() {
            return vec![];
        }
        let colors = cx.theme().colors().clone();
        let scope = workspace.unwrap_or_default();
        let gkey = format!("others:{scope}");
        let open = !self.collapsed.contains(&gkey);
        let (hit, tree_key) = if scope.is_empty() { ("others-row".to_string(), "others".to_string()) } else { (format!("others-row-{scope}"), format!("others-{scope}")) };
        let group = div()
            .id(SharedString::from(hit.clone()))
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(grid::gap())
            .h(px(REPO_ROW))
            .pl(grid::indent(level))
            .pr_2()
            .text_sm()
            .cursor_pointer()
            .hover(|s| s.bg(colors.ghost_element_hover))
            .active(|s| s.bg(colors.ghost_element_active))
            .child(disclosure(Some(open), secondary(cx)))
            .child(icon_slot(&self.hits, &tree_key, Some(Icon::Clock.slotted(grid::ICON, secondary(cx)).into_any_element())))
            .child(tree_label(
                &self.hits,
                &tree_key,
                div().flex_none().whitespace_nowrap().text_color(secondary(cx)).child(SharedString::from(format!("Other schedules ({})", tasks.len()))),
            ))
            .child(hit_target(&self.hits, hit))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle(gkey.clone(), cx)))
            .into_any_element();
        let mut out = vec![group];
        if open {
            out.extend(tasks.iter().map(|t| self.render_other_row(t, level + 1, cx)));
        }
        out
    }

    /// Muted: Supraman shows the task and never changes, runs or deletes it.
    fn render_other_row(&self, t: &ForeignTask, level: usize, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let path = t.path();
        let tree_key = format!("other-{path}");
        let target = MenuTarget::OtherSchedule(path.clone());
        div()
            .id(SharedString::from(format!("other-row-{path}")))
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(grid::gap())
            .h(px(SESSION_LINE))
            .pl(grid::indent(level))
            .pr_2()
            .text_sm()
            .opacity(super::left::DIMMED)
            .hover(|s| s.bg(colors.ghost_element_hover))
            .child(disclosure(None, secondary(cx)))
            .child(icon_slot(&self.hits, &tree_key, Some(Icon::Clock.slotted(grid::ICON, secondary(cx)).into_any_element())))
            .child(tree_label(&self.hits, &tree_key, clipped(format!("other-name-{path}"), t.name.clone(), false).flex_shrink(4.)))
            .child(
                clipped(format!("other-summary-{path}"), schedules::other_summary(t, supraman_core::time::now_ms()), false)
                    .flex_shrink(1.)
                    .text_xs()
                    .text_color(secondary(cx)),
            )
            .child(hit_target(&self.hits, format!("other-row-{path}")))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.open_menu(target.clone(), e.position, cx);
                }),
            )
            .into_any_element()
    }

    pub(super) fn render_schedule_row(&self, row: &ScheduleRow, runs: usize, open: bool, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let status = cx.theme().status().clone();
        let (w, id) = (row.workspace.clone(), row.schedule.id.clone());
        let k = key(&w, &id);
        let tree_key = format!("schedule-{k}");
        let group = SharedString::from(format!("schedule-group-{k}"));
        let color = match row.last_result() {
            Some(schedules::Result::Done) => status.success,
            Some(schedules::Result::NothingToDo) => status.info,
            Some(schedules::Result::Failed) => status.error,
            Some(schedules::Result::NeedsYou) => status.warning,
            Some(schedules::Result::Skipped) => status.hint,
            None => secondary(cx),
        };
        let now = supraman_core::time::now_ms();
        let pending = row.status == Status::Pending;
        let state = if row.running.is_some() { "running" } else { row.status_text() };
        let state_color = match (row.running.is_some(), row.status, row.command_changed) {
            (true, _, _) => status.success,
            (_, Status::Pending, _) | (_, _, true) => status.warning,
            _ => secondary(cx),
        };
        let action = {
            let (w, id) = (w.clone(), id.clone());
            let button = match row.primary() {
                Primary::Approve => self.hover_icon(format!("sched-approve-{k}"), Icon::Check, "Approve: opens what it approves", &group, true, cx),
                Primary::RunNow => self.hover_icon(format!("sched-run-{k}"), Icon::Play, "Run now", &group, false, cx),
            };
            button.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.schedule_primary(&w, &id, cx);
            }))
        };
        let toggle = {
            let skey = format!("schedule:{k}");
            div()
                .id(SharedString::from(format!("sched-disclosure-{k}")))
                .flex_none()
                .child(disclosure((runs > 0).then_some(open), secondary(cx)))
                .when(runs > 0, |d| {
                    d.cursor_pointer().on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        this.toggle(skey.clone(), cx);
                    }))
                })
        };
        let line = div()
            .flex()
            .items_center()
            .gap(grid::gap())
            .h(px(SESSION_LINE))
            .child(toggle)
            .child(icon_slot(&self.hits, &tree_key, Some(Icon::Clock.slotted(grid::ICON, color).into_any_element())))
            .child(tree_label(&self.hits, &tree_key, clipped(format!("sched-name-{k}"), row.title().to_string(), false).flex_shrink(4.)).cursor_pointer().on_click({
                let (w, id) = (w.clone(), id.clone());
                cx.listener(move |this, e: &ClickEvent, _, cx| {
                    if e.is_right_click() {
                        return;
                    }
                    cx.stop_propagation();
                    if e.click_count() == 2 {
                        this.schedule_primary(&w, &id, cx);
                    } else {
                        this.open_schedule(&w, &id, if pending { At::Approve } else { At::Top }, cx);
                    }
                })
            }))
            .child(
                div()
                    .id(SharedString::from(format!("sched-state-{k}")))
                    .relative()
                    .flex_none()
                    .whitespace_nowrap()
                    .text_xs()
                    .text_color(state_color)
                    .child(SharedString::from(state))
                    .child(hit_target(&self.hits, format!("sched-state-{k}"))),
            )
            .children(row.next_fire.map(|t| {
                div().flex_none().whitespace_nowrap().text_xs().text_color(secondary(cx)).child(SharedString::from(format!("next {}", supraman_core::time::near(t, now))))
            }))
            .child(div().flex_1())
            .child(div().flex().flex_none().items_center().child(action));
        let error = self.sync_errors.get(&k).cloned();
        let target = MenuTarget::Schedule { workspace: w.clone(), id: id.clone() };
        div()
            .id(SharedString::from(format!("sched-row-{k}")))
            .group(group)
            .relative()
            .flex()
            .flex_none()
            .flex_col()
            .pl(grid::indent(2))
            .pr_2()
            .py(px(1.))
            .text_sm()
            .hover(|s| s.bg(colors.ghost_element_hover))
            .when(pending, |d| d.bg(status.warning.opacity(0.08)))
            .when(row.status == Status::Paused, |d| d.opacity(super::left::DIMMED))
            .child(line)
            .children(error.map(|e| {
                clipped(format!("sched-error-{k}"), e, false).pl(px(grid::LABEL)).text_xs().text_color(status.error).relative().child(hit_target(&self.hits, format!("sched-error-{k}")))
            }))
            .child(hit_target(&self.hits, format!("sched-row-{k}")))
            .on_click(cx.listener(move |this, e: &ClickEvent, _, cx| {
                if e.click_count() == 2 {
                    this.schedule_primary(&w, &id, cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.open_menu(target.clone(), e.position, cx);
                }),
            )
            .into_any_element()
    }

    /// The runs whose Inbox notice is not dismissed, and the failed syncs.
    pub fn schedule_attention(&self) -> (Vec<schedules::Attention>, Vec<(String, String)>) {
        let runs = schedules::attention(&self.schedules).into_iter().filter(|a| !self.runs_dismissed.hides(&a.workspace, &a.run)).collect();
        (runs, self.sync_errors.iter().map(|(k, e)| (k.clone(), e.clone())).collect())
    }

    /// The pending schedules whose Inbox entry is not dismissed for what they cover now.
    pub fn schedules_waiting(&self) -> Vec<Waiting> {
        schedules::waiting(&self.schedules).into_iter().filter(|w| !self.waiting_dismissed.hides(&w.workspace, &w.key)).collect()
    }

    pub fn tray_tooltip(&self) -> String {
        let (runs, syncs) = self.schedule_attention();
        schedules::tray_tooltip(runs.len() + syncs.len(), self.schedules_waiting().len())
    }

    pub(super) fn render_schedule_inbox(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let colors = cx.theme().colors().clone();
        let status = cx.theme().status().clone();
        let now = supraman_core::time::now_ms();
        let (runs, syncs) = self.schedule_attention();
        let waiting = self.schedules_waiting();
        let card = |color: gpui::Hsla, label: &'static str, title: String, when: Option<String>, detail: String, buttons: gpui::Div| {
            div()
                .flex()
                .items_start()
                .gap_2()
                .p_2()
                .rounded_md()
                .border_l_2()
                .border_color(color)
                .bg(color.opacity(0.10))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .gap(px(2.))
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .text_sm()
                                .child(div().font_weight(gpui::FontWeight::BOLD).text_color(color).child(label))
                                .child(SharedString::from(title))
                                .children(when.map(|w| div().text_xs().text_color(colors.text_muted).child(SharedString::from(w)))),
                        )
                        .child(div().text_xs().text_color(colors.text).child(SharedString::from(detail))),
                )
                .child(buttons)
                .into_any_element()
        };
        let mut out = Vec::new();
        for w in waiting {
            let k = key(&w.workspace, &w.id);
            let open = button::bordered(&self.hits, format!("inbox-sched-open-{k}"), "Open", cx).on_click(cx.listener({
                let (ws, id) = (w.workspace.clone(), w.id.clone());
                move |this, _: &ClickEvent, _, cx| {
                    this.open_schedule(&ws, &id, At::Approve, cx);
                }
            }));
            let dismiss = button::bordered(&self.hits, format!("inbox-sched-later-{k}"), "Dismiss", cx).on_click(cx.listener({
                let (ws, dismissed) = (w.workspace.clone(), w.key.clone());
                move |this, _: &ClickEvent, _, cx| {
                    let save = this.waiting_dismissed.dismiss(&ws, &dismissed);
                    this.save_off_thread(save, "Cannot save the dismissed approval", cx);
                    cx.notify();
                }
            }));
            let (title, detail) = if w.command_changed {
                (
                    format!("{}: its slash command changed", w.title),
                    format!("Schedule {} of {} runs the changed file; Approve makes it the approved one.", w.id, w.workspace),
                )
            } else {
                (format!("{} waits for your approval", w.title), format!("Schedule {} of {}: nothing runs until you approve it.", w.id, w.workspace))
            };
            out.push(card(status.warning, "Approve", title, None, detail, div().flex().flex_none().gap_1().child(open).child(dismiss)));
        }
        for a in runs {
            let needs_you = a.outcome == schedule::Outcome::NeedsYou;
            let color = if needs_you { status.warning } else { status.error };
            let label = if needs_you { "Needs you" } else { "Run failed" };
            let detail = if a.why.is_empty() { schedules::outcome_text(a.outcome).to_string() } else { format!("{}: {}", schedules::outcome_text(a.outcome), a.why) };
            let (w, id, run) = (a.workspace.clone(), a.id.clone(), a.run.clone());
            let history = button::bordered(&self.hits, format!("inbox-sched-history-{}", a.run), "Details", cx).on_click(cx.listener({
                let (w, id) = (w.clone(), id.clone());
                move |this, _: &ClickEvent, _, cx| this.schedule_history(&w, &id, cx)
            }));
            let dismiss = button::bordered(&self.hits, format!("inbox-sched-dismiss-{}", a.run), "Dismiss", cx)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.dismiss_schedule_notice(&w, &run, cx)));
            out.push(card(color, label, format!("{} · {}", a.workspace, a.id), Some(supraman_core::time::ago(now - a.at)), detail, div().flex().flex_none().gap_1().child(history).child(dismiss)));
        }
        for (k, e) in syncs {
            let dismiss = button::bordered(&self.hits, format!("inbox-sync-dismiss-{k}"), "Dismiss", cx).on_click(cx.listener({
                let k = k.clone();
                move |this, _: &ClickEvent, _, cx| {
                    this.sync_errors.remove(&k);
                    cx.notify();
                }
            }));
            out.push(card(status.error, "Task not synced", k, None, e, div().flex().flex_none().child(dismiss)));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schedule() -> Schedule {
        toml::from_str::<schedule::ScheduleFile>(
            r#"
[[schedule]]
id = "nightly"
name = "Nightly"
folder = "Alpha"
[schedule.trigger]
kind = "weekly"
days = ["mon", "fri"]
at = "09:00"
[schedule.action]
kind = "script"
script = "gate.cmd"
"#,
        )
        .unwrap()
        .schedule
        .remove(0)
    }

    #[test]
    fn the_form_round_trips_and_names_the_field_that_does_not_parse() {
        let mut s = schedule();
        let form = Form::of(&s);
        assert_eq!((form.kind, form.days.clone(), form.at.as_str(), form.max_runtime.as_str(), form.max_tokens.as_str()), (TriggerKind::Weekly, vec![Day::Mon, Day::Fri], "09:00", "1h", "3000000"));
        let before = s.clone();
        form.apply(&mut s).unwrap();
        assert_eq!(s, before, "an untouched form changes nothing");

        let daily = Form { kind: TriggerKind::Daily, at: "7:05".into(), max_tokens: "1_000_000".into(), profile: Profile::Auto, ..form.clone() };
        daily.apply(&mut s).unwrap();
        assert_eq!(s.trigger, Trigger::Daily { at: Clock::new(7, 5).unwrap() });
        assert_eq!((s.caps.max_tokens, s.permissions.profile), (1_000_000, Profile::Auto));

        let every = Form { kind: TriggerKind::Interval, every: "30m".into(), ..form.clone() };
        every.apply(&mut s).unwrap();
        assert_eq!(s.trigger, Trigger::Interval { at: Clock::new(9, 0).unwrap(), every: Span(1800) });

        let days = Form { days: vec![Day::Fri, Day::Mon, Day::Fri], ..form.clone() };
        days.apply(&mut s).unwrap();
        assert_eq!(s.trigger, Trigger::Weekly { days: vec![Day::Mon, Day::Fri], at: Clock::new(9, 0).unwrap() }, "sorted, once each");

        for (bad, field) in [
            (Form { at: "25:00".into(), ..form.clone() }, "At:"),
            (Form { kind: TriggerKind::Interval, every: "soon".into(), ..form.clone() }, "Every:"),
            (Form { max_runtime: "1 hour".into(), ..form.clone() }, "Max runtime:"),
            (Form { max_tokens: "lots".into(), ..form.clone() }, "Max tokens:"),
            (Form { days: vec![], ..form.clone() }, "Days:"),
        ] {
            let mut s = schedule();
            let e = bad.apply(&mut s).unwrap_err();
            assert!(e.starts_with(field), "{e}");
            assert_eq!(s.caps, schedule().caps, "nothing half-applied before the failing field");
        }
    }
}
