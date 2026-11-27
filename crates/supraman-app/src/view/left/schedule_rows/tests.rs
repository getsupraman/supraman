use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

use gpui::{Keystroke, SharedString};
use supraman_core::schedule::{ForeignTask, Schedule, Status, Via};
use supraman_ui::list::{self, keyed_list::KeyedList};

use super::*;

const FOLDER: &str = "alpha-1a2b3c4d";

fn schedule_row(id: &str, status: Status) -> ScheduleRow {
    let schedule: Schedule = serde_json::from_value(serde_json::json!({
        "id": id, "name": format!("Task {id}"),
        "trigger": {"kind": "daily", "at": "09:00"}, "action": {"kind": "script_then_session", "script": "gate.cmd", "prompt": {"text": "fix"}}
    }))
    .unwrap();
    ScheduleRow {
        key: FOLDER.into(),
        folder: PathBuf::from(r"C:\code\Alpha"),
        schedule,
        status,
        command_changed: false,
        next_fire: None,
        running: None,
        runs: Vec::new(),
        approvable: None,
        command_path: None,
    }
}

fn task(name: &str) -> ForeignTask {
    ForeignTask {
        folder: r"\Mine".into(),
        name: name.into(),
        enabled: true,
        trigger: "daily".into(),
        last_run_ms: None,
        last_result: None,
        next_run_ms: None,
        working_dir: None,
        via: Via::Direct,
        action: "claude -p hi".into(),
    }
}

/// The block's rows after a heading `head`, as the sidebar's list holds them.
fn listed(collapsed: &HashSet<String>) -> (KeyedList, Block) {
    let mut block = Block::default();
    block.schedules("alpha", &[schedule_row("nightly", Status::Active), schedule_row("weekly", Status::Pending)], &BTreeMap::new(), 2, collapsed);
    block.tasks(FOLDER, &[task("backup")], 2, collapsed);
    let mut list = KeyedList::default();
    let head = (SharedString::from(format!("sched:{FOLDER}")), vec![format!("schedules-row-{FOLDER}")]);
    list.set_rows(std::iter::once(head).chain(block.rows.iter().map(|(_, key, tokens)| (SharedString::from(key.clone()), tokens.clone()))).collect());
    (list, block)
}

fn down(list: &mut KeyedList) -> Option<String> {
    let key = list::key_of(&Keystroke::parse("down").unwrap()).unwrap();
    list.key(&key, |_| String::new()).map(|k| k.to_string())
}

#[test]
fn down_from_the_heading_reaches_each_schedule_and_task_and_enter_opens_the_schedule() {
    let (mut list, block) = listed(&HashSet::new());
    list.select_key(Some(format!("sched:{FOLDER}").into()));
    let mut walked = Vec::new();
    for _ in 0..4 {
        walked.push(down(&mut list));
    }
    let want = [format!("sched-row-{FOLDER}/nightly"), format!("sched-row-{FOLDER}/weekly"), format!("others:{FOLDER}"), r"other-row-\Mine\backup".to_string()];
    assert_eq!(walked, want.clone().map(Some), "each schedule, the tasks' heading and each task a row of its own");

    let part = |key: &str| block.rows.iter().position(|(_, k, _)| k == key).map(|at| &block.parts[at]).unwrap();
    assert_eq!(part(&want[0]).act(), Act::OpenSchedule { folder_key: FOLDER.into(), id: "nightly".into(), at: At::Top }, "Enter opens the schedule's page");
    assert_eq!(part(&want[1]).act(), Act::OpenSchedule { folder_key: FOLDER.into(), id: "weekly".into(), at: At::Approve }, "a pending one's at what Approve approves");
    assert_eq!(part(&want[2]).act(), Act::Toggle(format!("others:{FOLDER}")));
    assert_eq!(list.row_of(&format!("sched-approve-{FOLDER}/weekly")), list.index_of(&want[1]), "a schedule's hover icon belongs to its own row");
}

#[test]
fn folded_tasks_leave_only_their_heading() {
    let (list, _) = listed(&HashSet::from([format!("others:{FOLDER}")]));
    assert_eq!(list.len(), 4, "the heading, two schedules and the tasks' heading");
    assert_eq!(list.index_of(r"other-row-\Mine\backup"), None);
}
