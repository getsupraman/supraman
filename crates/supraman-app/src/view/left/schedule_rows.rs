//! A Schedules block as the sidebar's rows: each schedule, each run under it, a folder's Task Scheduler tasks and
//! each run whose schedule is gone a row of its own, so the list keys reach every one.

use std::collections::{BTreeMap, HashSet};

use gpui::{AnyElement, Context};
use supraman_core::schedule::{ForeignTask, Status};

use super::{LeftRow, grid};
use crate::fleet::{self, SessionRow, UNROWED};
use crate::schedules::ScheduleRow;
use crate::view::FleetView;
use crate::view::schedules::{At, key};

/// The sidebar's rows of one frame: the row, its key and the tokens of its hit keys.
pub(in crate::view) type Rows = Vec<(LeftRow, String, Vec<String>)>;

/// What a [`LeftRow::Scheduled`] row shows.
#[derive(Clone)]
pub(in crate::view) enum Part {
    Schedule { entry: String, row: ScheduleRow, level: usize, runs: usize, open: bool },
    Run { entry: String, place: String, row: SessionRow, key: String, level: usize },
    /// A folder's Task Scheduler tasks' own heading.
    Tasks { folder_key: String, level: usize, open: bool },
    Task { task: ForeignTask, level: usize },
}

/// What Enter on a part does.
#[derive(Debug, PartialEq, Eq)]
pub(in crate::view) enum Act {
    OpenSchedule { folder_key: String, id: String, at: At },
    OpenRun,
    Toggle(String),
    Nothing,
}

/// The rows a block adds and the parts they show.
#[derive(Default)]
pub(in crate::view) struct Block {
    pub rows: Rows,
    pub parts: Vec<Part>,
}

impl Block {
    fn push(&mut self, key: String, tokens: Vec<String>, part: Part) {
        self.rows.push((LeftRow::Scheduled(self.parts.len()), key, tokens));
        self.parts.push(part);
    }

    /// Each schedule's row at `level`, its runs under it while it is open.
    pub fn schedules(&mut self, entry: &str, rows: &[ScheduleRow], runs: &BTreeMap<String, Vec<SessionRow>>, level: usize, collapsed: &HashSet<String>) {
        for row in rows {
            let k = key(&row.key, &row.schedule.id);
            let runs = runs.get(&row.schedule.id).map(Vec::as_slice).unwrap_or_default();
            let open = !collapsed.contains(&format!("schedule:{k}"));
            let tokens = ["sched-row-", "sched-state-", "sched-error-", "sched-name-", "sched-folder-", "sched-approve-", "sched-run-", "schedule-"].iter().map(|p| format!("{p}{k}")).collect();
            self.push(format!("sched-row-{k}"), tokens, Part::Schedule { entry: entry.to_string(), row: row.clone(), level, runs: runs.len(), open });
            if open {
                for run in runs {
                    self.run(entry, &row.schedule.id, run, level + 1);
                }
            }
        }
    }

    /// The runs of `runs` whose schedule is none of `rows`, at `level`.
    pub fn gone_runs(&mut self, entry: &str, rows: &[ScheduleRow], runs: &BTreeMap<String, Vec<SessionRow>>, level: usize) {
        for (id, runs) in runs.iter().filter(|(id, _)| !rows.iter().any(|r| &r.schedule.id == *id)) {
            for run in runs {
                self.run(entry, id, run, level);
            }
        }
    }

    fn run(&mut self, entry: &str, place: &str, row: &SessionRow, level: usize) {
        let key = format!("run:{}{}", row.session.session_id, row.scope);
        let tokens = vec![row.session.session_id.clone(), row.session.id.clone()];
        self.push(key.clone(), tokens, Part::Run { entry: entry.to_string(), place: place.to_string(), row: row.clone(), key, level });
    }

    /// A folder's tasks under their heading at `level`, the tasks one in while it is open.
    pub fn tasks(&mut self, folder_key: &str, tasks: &[ForeignTask], level: usize, collapsed: &HashSet<String>) {
        if tasks.is_empty() {
            return;
        }
        let open = !collapsed.contains(&format!("others:{folder_key}"));
        let tokens = vec![format!("others-row-{folder_key}"), format!("others-{folder_key}")];
        self.push(format!("others:{folder_key}"), tokens, Part::Tasks { folder_key: folder_key.to_string(), level, open });
        if open {
            self.task_rows(tasks, level + 1);
        }
    }

    /// Each task's row at `level`.
    pub fn task_rows(&mut self, tasks: &[ForeignTask], level: usize) {
        for task in tasks {
            let path = task.path();
            let tokens = ["other-row-", "other-name-", "other-summary-", "other-"].iter().map(|p| format!("{p}{path}")).collect();
            self.push(format!("other-row-{path}"), tokens, Part::Task { task: task.clone(), level });
        }
    }
}

impl Part {
    /// Enter on it: as a click on a schedule's name, a double-click on a run, a click on the tasks' heading.
    pub fn act(&self) -> Act {
        match self {
            Part::Schedule { row, .. } => {
                let at = if row.status == Status::Pending { At::Approve } else { At::Top };
                Act::OpenSchedule { folder_key: row.key.clone(), id: row.schedule.id.clone(), at }
            }
            Part::Run { .. } => Act::OpenRun,
            Part::Tasks { folder_key, .. } => Act::Toggle(format!("others:{folder_key}")),
            Part::Task { .. } => Act::Nothing,
        }
    }

    /// Its type-ahead text.
    pub fn text(&self) -> String {
        match self {
            Part::Schedule { row, .. } => row.title().to_string(),
            Part::Run { row, .. } => fleet::display_name(&row.session).to_string(),
            Part::Tasks { .. } => super::other_schedules::TITLE.into(),
            Part::Task { task, .. } => task.name.clone(),
        }
    }
}

impl FleetView {
    /// A folder's Schedules block under its row `at`: its heading while it holds a schedule or task, then, while open,
    /// each schedule with its runs and its tasks; the runs whose schedule is gone after, whether open or not.
    pub(super) fn folder_schedule_rows(&self, entry: &fleet::EntryNode, at: (usize, usize), others: &crate::schedules::Others, rows: &mut Rows, block: &mut Block) {
        let folder = &entry.folders[at.1];
        let fkey = supraman_core::schedule::folder_key(&folder.path);
        let scheds = self.scheduling.schedules.get(&fkey).map(Vec::as_slice).unwrap_or_default();
        let tasks = others.of_folder(&fkey);
        let level = super::folder_level(entry, folder) + 1;
        let collapsed = &self.sidebar.collapsed;
        if !scheds.is_empty() || !tasks.is_empty() {
            rows.push((LeftRow::Schedules(at.0, at.1), format!("sched:{fkey}"), vec![format!("schedules-row-{fkey}"), format!("schedules-{fkey}")]));
            if !collapsed.contains(&format!("schedules:{fkey}")) {
                block.schedules(&entry.name, scheds, &folder.scheduled, level + 1, collapsed);
                block.tasks(&fkey, tasks, level + 1, collapsed);
            }
        }
        block.gone_runs(&entry.name, scheds, &folder.scheduled, level);
        rows.append(&mut block.rows);
    }

    pub(super) fn render_part(&self, part: &Part, cx: &mut Context<Self>) -> AnyElement {
        match part {
            Part::Schedule { entry, row, level, runs, open } => self.render_schedule_row(row, *level, *runs, *open, entry == UNROWED, cx),
            Part::Run { entry, place, row, key, level } => self.render_session(entry, place, row, Some(key), grid::indent(*level), cx),
            Part::Tasks { folder_key, level, open } => self.render_tasks_row(folder_key, *level, *open, cx),
            Part::Task { task, level } => self.render_other_row(task, *level, cx),
        }
    }

    pub(super) fn activate_part(&mut self, part: &Part, cx: &mut Context<Self>) {
        match (part.act(), part) {
            (Act::OpenSchedule { folder_key, id, at }, _) => {
                self.open_schedule(&folder_key, &id, at, cx);
            }
            (Act::OpenRun, Part::Run { place, row, .. }) => self.open_row(&place.replace('/', " · "), row, cx),
            (Act::Toggle(key), _) => self.toggle(key, cx),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests;
