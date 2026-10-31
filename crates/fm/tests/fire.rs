use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use fm::fire::{self, Host, Report, Request, RunCtx};
use fm::run::{RunLog, SessionEnd};
use foreman_core::schedule::{self, Action, ActionKind, Fired, Outcome, Overlap, Prompt, RunEvent, Schedule, Span, Stage, Status, Trigger};
use foreman_core::secret::{MemoryStore, SecretStore};
use foreman_core::{Folder, Registry, Workspace};

const SECRET: &str = "s3cr3t-Value-81f2";

/// One `.cmd` per behaviour, in folder Alpha's `scripts/`.
const SCRIPTS: [(&str, &str); 7] = [
    ("exit0.cmd", "@exit /b 0\r\n"),
    ("gate.cmd", "@echo gate output\r\n@echo token %API_TOKEN%\r\n@exit /b 10\r\n"),
    ("fail.cmd", "@echo first line 1>&2\r\n@echo boom %API_TOKEN% 1>&2\r\n@exit /b 1\r\n"),
    ("slow.cmd", "@echo before the wait 1>&2\r\n@ping -n 30 127.0.0.1 >nul\r\n@exit /b 0\r\n"),
    (
        "env.cmd",
        "@echo %FOREMAN_HOME%^|%FOREMAN_WORKSPACE%^|%FOREMAN_SCHEDULE%^|%FOREMAN_RUN%^|%FOREMAN_FOLDER%^|%FOREMAN_LAST_RUN%^|%FOREMAN_LAST_OUTCOME%^|%CD%> \"%FOREMAN_STATE_DIR%\\env.txt\"\r\n@exit /b 0\r\n",
    ),
    ("secret.cmd", "@if \"%API_TOKEN%\"==\"s3cr3t-Value-81f2\" exit /b 10\r\n@exit /b 1\r\n"),
    ("once.cmd", "@if exist \"%FOREMAN_STATE_DIR%\\once\" exit /b 0\r\n@echo x> \"%FOREMAN_STATE_DIR%\\once\"\r\n@ping -n 30 127.0.0.1 >nul\r\n@exit /b 0\r\n"),
];

struct Fixture {
    _tmp: tempfile::TempDir,
    home: PathBuf,
    alpha: PathBuf,
    store: MemoryStore,
}

fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let alpha = tmp.path().join("ws").join("Alpha");
    fs::create_dir_all(alpha.join("scripts")).unwrap();
    fs::create_dir_all(&home).unwrap();
    for (name, body) in SCRIPTS {
        fs::write(alpha.join("scripts").join(name), body).unwrap();
    }
    let w = Workspace::new("demo", tmp.path().join("ws"), vec![Folder::new("Alpha", &alpha)]);
    Registry::at(&home).save(&w, false).unwrap();
    Fixture { _tmp: tmp, home, alpha, store: MemoryStore::default() }
}

fn script_schedule(id: &str, script: &str, kind: ActionKind) -> Schedule {
    Schedule {
        id: id.into(),
        name: id.into(),
        description: String::new(),
        folder: "Alpha".into(),
        status: Status::Pending,
        created_by: schedule::CreatedBy::Fm,
        created_at: 0,
        trigger: Trigger::Daily { at: "09:00".parse().unwrap() },
        action: Action {
            kind,
            prompt: (kind != ActionKind::Script).then(|| Prompt::Text("Fix what the gate found.".into())),
            script: Some(format!("scripts/{script}")),
            script_args: Vec::new(),
            script_timeout: Span(60),
            worktree: false,
            model: String::new(),
        },
        permissions: Default::default(),
        caps: Default::default(),
        policy: Default::default(),
        approval: None,
    }
}

impl Fixture {
    fn workspace(&self) -> Workspace {
        Registry::at(&self.home).load("demo").unwrap()
    }

    /// Adds `s` and stands in for René's Approve in the window (C100: fm itself never approves).
    fn add_approved(&self, s: Schedule) -> String {
        let id = s.id.clone();
        schedule::edit(&self.home, &self.workspace(), |all| {
            all.push(s);
            Ok(())
        })
        .unwrap();
        schedule::approve(&self.home, &self.store, &self.workspace(), &id, 1).unwrap().approval.unwrap().fingerprint
    }

    fn change(&self, id: &str, how: impl FnOnce(&mut Schedule)) {
        schedule::edit(&self.home, &self.workspace(), |all| {
            how(all.iter_mut().find(|s| s.id == id).unwrap());
            Ok(())
        })
        .unwrap();
    }

    fn records(&self, run: &str) -> Vec<RunEvent> {
        schedule::read_runs(&self.home, "demo").unwrap().into_iter().filter(|r| r.run == run).map(|r| r.event).collect()
    }

    fn live(&self) -> PathBuf {
        schedule::live_dir(&self.home, "demo").unwrap()
    }
}

struct TestHost<'a> {
    store: &'a MemoryStore,
    busy: VecDeque<bool>,
    busy_asked: usize,
    slept: Vec<Duration>,
    now: i64,
    gates: Vec<Option<String>>,
}

impl<'a> TestHost<'a> {
    fn new(f: &'a Fixture) -> Self {
        Self::at(f, 1_793_595_600_000)
    }

    fn at(f: &'a Fixture, now: i64) -> Self {
        Self { store: &f.store, busy: VecDeque::new(), busy_asked: 0, slept: Vec::new(), now, gates: Vec::new() }
    }
}

impl Host for TestHost<'_> {
    fn store(&self) -> &dyn SecretStore {
        self.store
    }

    fn busy(&mut self, _home: &Path) -> Result<bool, String> {
        self.busy_asked += 1;
        Ok(self.busy.pop_front().unwrap_or(false))
    }

    fn sleep(&mut self, d: Duration) {
        self.slept.push(d);
    }

    fn now_ms(&mut self) -> i64 {
        self.now += 1000;
        self.now
    }

    fn session(&mut self, _ctx: &RunCtx, gate: Option<&str>, _log: &mut dyn RunLog) -> SessionEnd {
        self.gates.push(gate.map(str::to_string));
        SessionEnd { outcome: Outcome::Done, why: "the test's session stage".into(), tokens: None, recorded: false }
    }

    fn end_dead_session(&mut self, _home: &Path, _session: &schedule::LiveSession) -> Result<(), String> {
        Ok(())
    }
}

fn fire_as(f: &Fixture, id: &str, trigger: Fired, fingerprint: Option<&str>, host: &mut TestHost) -> Report {
    fire::fire(&Request { home: &f.home, workspace: "demo", id, trigger, fingerprint }, host)
}

fn run_now(f: &Fixture, id: &str, host: &mut TestHost) -> Report {
    fire_as(f, id, Fired::RunNow, None, host)
}

fn script_stage(events: &[RunEvent]) -> (Option<i32>, String) {
    events
        .iter()
        .find_map(|e| match e {
            RunEvent::Stage(Stage::Script { code, stderr, .. }) => Some((*code, stderr.clone())),
            _ => None,
        })
        .expect("a script stage")
}

fn ended(events: &[RunEvent]) -> (Outcome, String) {
    match events.last() {
        Some(RunEvent::Ended { outcome, why, .. }) => (*outcome, why.clone()),
        other => panic!("the run did not end: {other:?}"),
    }
}

/// c1: exit 0 is nothing to do; the run is recorded started, script stage, ended.
#[test]
fn fire_exit_0_records_nothing_to_do() {
    let f = fixture();
    let fp = f.add_approved(script_schedule("gate", "exit0.cmd", ActionKind::ScriptThenSession));
    let mut host = TestHost::new(&f);
    let report = fire_as(&f, "gate", Fired::Task, Some(&fp), &mut host);
    assert_eq!((report.outcome, report.exit), (Some(Outcome::NothingToDo), 0), "{report:?}");
    let events = f.records(report.run.as_deref().unwrap());
    assert!(matches!(&events[0], RunEvent::Started { trigger: Fired::Task, fingerprint, .. } if *fingerprint == fp), "{events:?}");
    assert_eq!(script_stage(&events).0, Some(0));
    assert_eq!(ended(&events).0, Outcome::NothingToDo);
    assert!(host.gates.is_empty(), "no session stage");
    assert!(!f.live().join("gate.json").exists(), "the live file goes at the end");
    let log = fs::read_to_string(schedule::fire_log(&f.home)).unwrap();
    assert!(log.contains("demo/gate task gate-") && log.contains(" nothing_to_do"), "{log}");
}

/// c1: exit 10 hands stdout to the session stage, whose outcome ends the run.
#[test]
fn fire_exit_10_hands_stdout_to_the_session_stage() {
    let f = fixture();
    f.add_approved(script_schedule("gate", "gate.cmd", ActionKind::ScriptThenSession));
    let mut host = TestHost::new(&f);
    let report = run_now(&f, "gate", &mut host);
    assert_eq!((report.outcome, report.why.as_str(), report.exit), (Some(Outcome::Done), "the test's session stage", 0));
    let gate = host.gates.pop().unwrap().unwrap();
    assert!(gate.contains("gate output"), "{gate:?}");
    let events = f.records(report.run.as_deref().unwrap());
    assert!(matches!(&events[0], RunEvent::Started { trigger: Fired::RunNow, .. }));
    assert_eq!(script_stage(&events).0, Some(10));

    let session = f.add_approved({
        let mut s = script_schedule("talk", "gate.cmd", ActionKind::Session);
        s.action.script = None;
        s
    });
    assert!(!session.is_empty());
    let report = run_now(&f, "talk", &mut host);
    assert_eq!(report.outcome, Some(Outcome::Done));
    assert_eq!(host.gates.pop(), Some(None), "a session action has no gate output");
}

/// c1: any other exit and a timeout are failed, with the tail of stderr.
#[test]
fn fire_exit_1_and_a_timeout_record_failed_with_the_stderr_tail() {
    let f = fixture();
    f.add_approved(script_schedule("fails", "fail.cmd", ActionKind::ScriptThenSession));
    let mut host = TestHost::new(&f);
    let report = run_now(&f, "fails", &mut host);
    assert_eq!((report.outcome, report.why.as_str(), report.exit), (Some(Outcome::Failed), "the script exited 1", 1));
    let events = f.records(report.run.as_deref().unwrap());
    let (code, stderr) = script_stage(&events);
    assert_eq!(code, Some(1));
    assert!(stderr.contains("first line") && stderr.contains("boom"), "{stderr:?}");

    let mut slow = script_schedule("slow", "slow.cmd", ActionKind::Script);
    slow.action.script_timeout = Span(2);
    f.add_approved(slow);
    let started = Instant::now();
    let report = run_now(&f, "slow", &mut host);
    assert!(started.elapsed() < Duration::from_secs(20), "{:?}", started.elapsed());
    assert_eq!((report.outcome, report.why.as_str()), (Some(Outcome::Failed), "the script did not finish within 2s and was killed"));
    let (code, stderr) = script_stage(&f.records(report.run.as_deref().unwrap()));
    assert_eq!(code, None);
    assert!(stderr.contains("before the wait"), "{stderr:?}");
}

/// A script action has no session: 0 and 10 are both ok, and it waits for no budget slot.
#[test]
fn fire_a_script_action_records_ok_for_0_and_10() {
    let f = fixture();
    f.add_approved(script_schedule("zero", "exit0.cmd", ActionKind::Script));
    f.add_approved(script_schedule("ten", "gate.cmd", ActionKind::Script));
    let mut host = TestHost::new(&f);
    host.busy.extend([true; 100]);
    assert_eq!(run_now(&f, "zero", &mut host).outcome, Some(Outcome::Ok));
    assert_eq!(run_now(&f, "ten", &mut host).outcome, Some(Outcome::Ok));
    assert!(host.gates.is_empty() && host.busy_asked == 0 && host.slept.is_empty());
}

/// The script runs in the folder's checkout with the doc's environment and its state dir.
#[test]
fn fire_gives_the_script_its_environment_and_state_dir() {
    let f = fixture();
    f.add_approved(script_schedule("env", "env.cmd", ActionKind::Script));
    let mut host = TestHost::new(&f);
    let first = run_now(&f, "env", &mut host);
    let state = schedule::state_dir(&f.home, "demo", "env").unwrap();
    let line = fs::read_to_string(state.join("env.txt")).unwrap();
    let fields: Vec<&str> = line.trim_end().split('|').collect();
    assert_eq!(fields[..5], [f.home.to_str().unwrap(), "demo", "env", first.run.as_deref().unwrap(), f.alpha.to_str().unwrap()], "{line}");
    assert_eq!((fields[5], fields[6]), ("", ""), "no previous run");
    assert_eq!(fields[7], f.alpha.to_str().unwrap(), "the cwd is the folder");

    let second = run_now(&f, "env", &mut host);
    assert_ne!(second.run, first.run);
    let line = fs::read_to_string(state.join("env.txt")).unwrap();
    let fields: Vec<&str> = line.trim_end().split('|').collect();
    let first_start = schedule::runs(&f.home, "demo").unwrap().into_iter().find(|r| Some(&r.run) == first.run.as_ref()).unwrap().started.unwrap().0;
    assert_eq!((fields[5], fields[6]), (first_start.to_string().as_str(), "ok"));
}

/// c2: unapproved, paused, changed since approval and a stale task fingerprint are refused and recorded.
#[test]
fn fire_refuses_and_records_unapproved_paused_and_changed_schedules() {
    let f = fixture();
    let refused = |report: &Report, why: &str| {
        assert_eq!((report.outcome, report.exit), (Some(Outcome::Refused), 3), "{report:?}");
        assert!(report.why.contains(why), "{report:?}");
        let events = f.records(report.run.as_deref().unwrap());
        assert!(matches!(events[0], RunEvent::Started { .. }) && ended(&events).0 == Outcome::Refused, "{events:?}");
        assert!(!events.iter().any(|e| matches!(e, RunEvent::Stage(_))), "nothing ran");
    };
    let mut host = TestHost::new(&f);

    schedule::edit(&f.home, &f.workspace(), |all| {
        all.push(script_schedule("fresh", "exit0.cmd", ActionKind::Script));
        Ok(())
    })
    .unwrap();
    refused(&run_now(&f, "fresh", &mut host), "the schedule is pending");

    let fp = f.add_approved(script_schedule("gate", "exit0.cmd", ActionKind::Script));
    refused(&fire_as(&f, "gate", Fired::Task, Some("0000"), &mut host), "the task's fingerprint is not the approved one");
    assert_eq!(fire_as(&f, "gate", Fired::Task, Some(&fp), &mut host).outcome, Some(Outcome::Ok));

    f.change("gate", |s| s.status = Status::Paused);
    refused(&fire_as(&f, "gate", Fired::Task, Some(&fp), &mut host), "the schedule is paused");
    f.change("gate", |s| s.status = Status::Active);
    assert_eq!(f.workspace().name, "demo");
    assert_eq!(run_now(&f, "gate", &mut host).outcome, Some(Outcome::Ok), "resumed, still approved");

    fs::write(f.alpha.join("scripts").join("exit0.cmd"), "@exit /b 10\r\n").unwrap();
    refused(&fire_as(&f, "gate", Fired::Task, Some(&fp), &mut host), "changed since its approval");
    refused(&run_now(&f, "gate", &mut host), "changed since its approval");

    let other = MemoryStore::default();
    let mut stranger = TestHost::new(&f);
    stranger.store = &other;
    fs::write(f.alpha.join("scripts").join("exit0.cmd"), SCRIPTS[0].1).unwrap();
    refused(&run_now(&f, "gate", &mut stranger), "no approval key");
}

#[test]
fn fire_of_an_unknown_schedule_exits_2_and_writes_fire_log_only() {
    let f = fixture();
    let mut host = TestHost::new(&f);
    for (ws, id) in [("demo", "nope"), ("ghost", "x")] {
        let report = fire::fire(&Request { home: &f.home, workspace: ws, id, trigger: Fired::Task, fingerprint: Some("ab") }, &mut host);
        assert_eq!((report.outcome, report.exit, report.run), (None, 2, None));
    }
    assert!(schedule::read_runs(&f.home, "demo").unwrap().is_empty());
    let log = fs::read_to_string(schedule::fire_log(&f.home)).unwrap();
    assert!(log.contains("demo/nope task - refused: workspace demo has no schedule nope") && log.contains("ghost/x task - refused: "), "{log}");
}

fn with_overlap(id: &str, script: &str, overlap: Overlap) -> Schedule {
    let mut s = script_schedule(id, script, ActionKind::ScriptThenSession);
    s.policy.overlap = overlap;
    s
}

/// c3, skip: a held lock skips the fire and names the run in flight.
#[test]
fn fire_overlap_skip_records_skipped_while_the_lock_is_held() {
    let f = fixture();
    f.add_approved(with_overlap("gate", "exit0.cmd", Overlap::Skip));
    let held = foreman_core::fs::lock_file(&f.live().join("gate.lock")).unwrap();
    let body = r#"{"run":"gate-20261102-090000","fm_pid":1,"started_at":1,"stage":"script"}"#;
    fs::write(f.live().join("gate.json"), body).unwrap();
    let report = run_now(&f, "gate", &mut TestHost::new(&f));
    assert_eq!((report.outcome, report.why.as_str(), report.exit), (Some(Outcome::Skipped), "overlap: run gate-20261102-090000 is in flight", 0));
    assert!(f.live().join("gate.json").exists(), "the running run's live file stays");
    drop(held);
}

/// c3, queue: one waiter takes the lock once it is free; a second one skips.
#[test]
fn fire_overlap_queue_waits_for_the_lock_and_a_second_waiter_skips() {
    let f = fixture();
    f.add_approved(with_overlap("gate", "exit0.cmd", Overlap::Queue));
    let held = foreman_core::fs::lock_file(&f.live().join("gate.lock")).unwrap();
    std::thread::scope(|scope| {
        let waiter = scope.spawn(|| run_now(&f, "gate", &mut TestHost::at(&f, 1_793_599_200_000)));
        let queued = f.live().join("gate.queued");
        let deadline = Instant::now() + Duration::from_secs(20);
        while !(queued.exists() && foreman_core::fs::try_lock_file(&queued).unwrap().is_none()) {
            assert!(Instant::now() < deadline, "the first waiter never queued");
            std::thread::sleep(Duration::from_millis(20));
        }
        let second = run_now(&f, "gate", &mut TestHost::new(&f));
        assert_eq!(second.outcome, Some(Outcome::Skipped));
        assert!(second.why.ends_with("and another run waits for it"), "{second:?}");
        assert!(!waiter.is_finished(), "still waiting for the lock");
        drop(held);
        assert_eq!(waiter.join().unwrap().outcome, Some(Outcome::NothingToDo));
    });
}

/// c3, restart: the fire asks the owner to stop and runs once the lock is free.
#[test]
fn fire_overlap_restart_asks_the_owner_and_runs_after_it() {
    let f = fixture();
    f.add_approved(with_overlap("gate", "exit0.cmd", Overlap::Restart));
    let held = foreman_core::fs::lock_file(&f.live().join("gate.lock")).unwrap();
    std::thread::scope(|scope| {
        let restarter = scope.spawn(|| run_now(&f, "gate", &mut TestHost::new(&f)));
        let restart = f.live().join("gate.restart");
        let deadline = Instant::now() + Duration::from_secs(20);
        while !restart.exists() {
            assert!(Instant::now() < deadline, "no restart asked");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!restarter.is_finished());
        drop(held);
        assert_eq!(restarter.join().unwrap().outcome, Some(Outcome::NothingToDo));
    });
    assert!(!f.live().join("gate.restart").exists(), "the request is spent");
}

/// c3, restart, the owner's side: a running script is killed and its run ends interrupted.
#[test]
fn fire_restart_interrupts_the_running_script() {
    let f = fixture();
    f.add_approved(with_overlap("long", "once.cmd", Overlap::Restart));
    std::thread::scope(|scope| {
        let owner = scope.spawn(|| run_now(&f, "long", &mut TestHost::at(&f, 1_793_599_200_000)));
        let live = f.live().join("long.json");
        let deadline = Instant::now() + Duration::from_secs(20);
        while !live.exists() {
            assert!(Instant::now() < deadline, "the owner never started");
            std::thread::sleep(Duration::from_millis(20));
        }
        std::thread::sleep(Duration::from_millis(500));
        let started = Instant::now();
        let restarter = run_now(&f, "long", &mut TestHost::new(&f));
        assert!(started.elapsed() < Duration::from_secs(15), "{:?}", started.elapsed());
        let owner = owner.join().unwrap();
        assert_eq!((owner.outcome, owner.why.as_str(), owner.exit), (Some(Outcome::Interrupted), "restarted", 1));
        assert_eq!(restarter.outcome, Some(Outcome::NothingToDo), "the second run finds the marker and exits 0");
    });
}

/// Step 4: a full budget is asked once a minute; after 30 minutes the fire is skipped as busy.
#[test]
fn fire_waits_for_a_budget_slot_once_a_minute_then_skips_busy() {
    let f = fixture();
    f.add_approved(script_schedule("gate", "exit0.cmd", ActionKind::ScriptThenSession));
    let mut host = TestHost::new(&f);
    host.busy.extend([true, true]);
    assert_eq!(run_now(&f, "gate", &mut host).outcome, Some(Outcome::NothingToDo));
    assert_eq!(host.slept, [fire::BUDGET_POLL; 2]);
    assert_eq!(fire::BUDGET_POLL, Duration::from_secs(60));

    let mut host = TestHost::new(&f);
    host.busy.extend([true; 100]);
    let report = run_now(&f, "gate", &mut host);
    assert_eq!((report.outcome, report.exit), (Some(Outcome::Skipped), 0));
    assert!(report.why.starts_with("busy"), "{report:?}");
    assert_eq!(host.slept.iter().sum::<Duration>(), Duration::from_secs(30 * 60));
    assert!(!f.records(report.run.as_deref().unwrap()).iter().any(|e| matches!(e, RunEvent::Stage(_))));
}

/// Step 3: with catch_up = skip, a task's start more than 10 minutes late is skipped as missed; Run now is not.
#[test]
fn fire_skips_a_missed_start_when_catch_up_is_skip() {
    let f = fixture();
    let mut s = script_schedule("gate", "exit0.cmd", ActionKind::Script);
    s.policy.catch_up = schedule::CatchUp::Skip;
    let fp = f.add_approved(s);
    use chrono::TimeZone;
    let noon = chrono::Local.with_ymd_and_hms(2026, 11, 2, 12, 0, 0).single().unwrap().timestamp_millis();
    let report = fire_as(&f, "gate", Fired::Task, Some(&fp), &mut TestHost::at(&f, noon));
    assert_eq!(report.outcome, Some(Outcome::Skipped));
    assert!(report.why.starts_with("missed: the start at 2026-11-02 09:00"), "{report:?}");
    assert_eq!(run_now(&f, "gate", &mut TestHost::at(&f, noon)).outcome, Some(Outcome::Ok));
    let on_time = chrono::Local.with_ymd_and_hms(2026, 11, 2, 9, 0, 0).single().unwrap().timestamp_millis();
    assert_eq!(fire_as(&f, "gate", Fired::Task, Some(&fp), &mut TestHost::at(&f, on_time)).outcome, Some(Outcome::Ok));
}

/// A live file under a free lock is a dead run: it is recorded interrupted before the new run starts.
#[test]
fn fire_records_a_dead_run_left_behind_as_interrupted() {
    let f = fixture();
    f.add_approved(script_schedule("gate", "exit0.cmd", ActionKind::Script));
    fs::create_dir_all(f.live()).unwrap();
    fs::write(f.live().join("gate.json"), r#"{"run":"gate-20261101-090000","fm_pid":4242,"started_at":1,"stage":"script"}"#).unwrap();
    assert_eq!(run_now(&f, "gate", &mut TestHost::new(&f)).outcome, Some(Outcome::Ok));
    let (outcome, why) = ended(&f.records("gate-20261101-090000"));
    assert_eq!((outcome, why.as_str()), (Outcome::Interrupted, "fm (pid 4242) ended without finishing the run"));
}

/// The window's recovery: a live file whose fm is gone is ended as interrupted; one whose fm runs, or whose lock is
/// held by a fire, is left alone.
#[test]
fn the_app_recovers_a_run_whose_fm_is_gone_and_leaves_a_live_one() {
    let f = fixture();
    f.add_approved(script_schedule("gate", "exit0.cmd", ActionKind::Script));
    fs::create_dir_all(f.live()).unwrap();
    let me = std::process::id();
    let mine = format!(r#"{{"run":"gate-20261101-080000","fm_pid":{me},"fm_started":{},"started_at":1,"stage":"script"}}"#, foreman_core::process::started(me).unwrap());
    fs::write(f.live().join("gate.json"), &mine).unwrap();
    assert_eq!(fire::recover_if_dead(&f.home, "demo", "gate"), None, "its fm runs");

    let reused = format!(r#"{{"run":"gate-20261101-090000","fm_pid":{me},"fm_started":1,"started_at":1,"stage":"script"}}"#);
    fs::write(f.live().join("gate.json"), &reused).unwrap();
    let held = foreman_core::fs::try_lock_file(&f.live().join("gate.lock")).unwrap().unwrap();
    assert_eq!(fire::recover_if_dead(&f.home, "demo", "gate"), None, "a fire holds the lock");
    drop(held);
    assert_eq!(fire::recover_if_dead(&f.home, "demo", "gate").as_deref(), Some("gate-20261101-090000"), "a reused pid is not its fm");
    let (outcome, why) = ended(&f.records("gate-20261101-090000"));
    assert_eq!((outcome, why.as_str()), (Outcome::Interrupted, format!("fm (pid {me}) ended without finishing the run").as_str()));
    assert!(!f.live().join("gate.json").exists());
    assert_eq!(fire::recover_if_dead(&f.home, "demo", "gate"), None, "nothing left");
}

/// Every file under `dir`, read whole.
fn files_holding(dir: &Path, needle: &str) -> Vec<PathBuf> {
    let wide: Vec<u8> = needle.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut found = Vec::new();
    let mut dirs = vec![dir.to_path_buf()];
    while let Some(d) = dirs.pop() {
        for e in fs::read_dir(&d).unwrap().flatten() {
            let path = e.path();
            if path.is_dir() {
                dirs.push(path);
            } else {
                let bytes = fs::read(&path).unwrap();
                if bytes.windows(needle.len()).any(|w| w == needle.as_bytes()) || bytes.windows(wide.len()).any(|w| w == wide) {
                    found.push(path);
                }
            }
        }
    }
    found
}

/// c4: the secrets reach the script's environment; stderr tails and the gate output are masked; no file
/// Foreman writes holds them.
#[test]
fn fire_passes_secrets_to_the_script_and_writes_them_nowhere() {
    let f = fixture();
    let with_secret = |id: &str, script: &str| {
        let mut s = script_schedule(id, script, ActionKind::ScriptThenSession);
        s.permissions.secrets = vec!["API_TOKEN".into()];
        s
    };
    f.add_approved(with_secret("check", "secret.cmd"));
    f.add_approved(with_secret("leak", "fail.cmd"));
    f.add_approved(with_secret("gate", "gate.cmd"));
    let mut host = TestHost::new(&f);

    let missing = run_now(&f, "check", &mut host);
    assert_eq!((missing.outcome, missing.why.as_str()), (Some(Outcome::Failed), "secret API_TOKEN is not set: fm schedule secret set demo check API_TOKEN"));

    for id in ["check", "leak", "gate"] {
        f.store.set(&schedule::secret_key("demo", id, "API_TOKEN"), SECRET).unwrap();
    }
    let check = run_now(&f, "check", &mut host);
    assert_eq!(check.outcome, Some(Outcome::Done), "the script saw the value: {check:?}");

    let leak = run_now(&f, "leak", &mut host);
    let (_, stderr) = script_stage(&f.records(leak.run.as_deref().unwrap()));
    assert!(stderr.contains("boom ***") && !stderr.contains(SECRET), "{stderr:?}");

    run_now(&f, "gate", &mut host);
    let gate = host.gates.pop().unwrap().unwrap();
    assert!(gate.contains("token ***") && !gate.contains(SECRET), "{gate:?}");

    assert_eq!(files_holding(&f.home, SECRET), Vec::<PathBuf>::new());
    assert!(files_holding(&f.home, "boom ***").len() == 1, "the masked tail is in the run records");
}

fn fm_bin(name: &str, f: &Fixture, args: &[&str], stdin: &str) -> std::process::Output {
    use std::io::Write as _;
    let exe = match name {
        "fm" => env!("CARGO_BIN_EXE_fm"),
        _ => env!("CARGO_BIN_EXE_fmw"),
    };
    let mut child = Command::new(exe)
        .args(args)
        .env("SUPRAMAN_HOME", &f.home)
        .env_remove("APPDATA")
        .env_remove("FOREMAN_ROLE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    child.wait_with_output().unwrap()
}

/// The real host: Run now through `fm`, the task's line through `fmw`. Folder Alpha is no git repo, so the session
/// stage stops at its worktree, before any `claude` or WorkState call.
#[test]
fn fire_through_fm_and_fmw_with_the_homes_own_store() {
    let f = fixture();
    let mut s = script_schedule("gate", "gate.cmd", ActionKind::ScriptThenSession);
    s.permissions.profile = schedule::Profile::WorktreeEdit;
    s.action.worktree = true;
    schedule::edit(&f.home, &f.workspace(), |all| {
        all.push(s.clone());
        Ok(())
    })
    .unwrap();
    let store = foreman_core::secret::default_store(&f.home);
    let fp = schedule::approve(&f.home, &*store, &f.workspace(), "gate", 1).unwrap().approval.unwrap().fingerprint;

    let out = fm_bin("fm", &f, &["schedule", "run", "demo", "gate"], "");
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.starts_with("gate-") && stderr.contains(" failed: ") && stderr.contains("is not a git repo"), "{stderr}");

    let home = f.home.to_str().unwrap();
    let out = fm_bin("fmw", &f, &["schedule", "fire", "--home", home, "demo", "gate", "--fingerprint", "0bad"], "");
    assert_eq!(out.status.code(), Some(3), "{out:?}");
    assert!(out.stdout.is_empty() && out.stderr.is_empty(), "fmw writes to fire.log alone: {out:?}");
    let out = fm_bin("fmw", &f, &["schedule", "fire", "--home", home, "demo", "gate", "--fingerprint", &fp], "");
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let out = fm_bin("fmw", &f, &["schedule", "fire", "--home", home, "demo", "nope", "--fingerprint", &fp], "");
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    let log = fs::read_to_string(schedule::fire_log(&f.home)).unwrap();
    let lines: Vec<&str> = log.lines().collect();
    assert_eq!(lines.len(), 4, "{log}");
    assert!(lines[0].contains(" demo/gate run_now gate-") && lines[0].contains("is not a git repo"), "{log}");
    assert!(lines[1].contains(" demo/gate task gate-") && lines[1].contains("refused: the task's fingerprint"), "{log}");
    assert!(lines[2].contains(" demo/gate task gate-") && lines[2].contains("is not a git repo"), "{log}");
    assert!(lines[3].contains(" demo/nope task - refused: workspace demo has no schedule nope"), "{log}");
}

/// The window's Run now: `fmw schedule run --home`, no console output, recorded as `run_now`.
#[test]
fn fmw_runs_now_for_the_window() {
    let f = fixture();
    schedule::edit(&f.home, &f.workspace(), |all| {
        all.push(script_schedule("gate", "exit0.cmd", ActionKind::Script));
        Ok(())
    })
    .unwrap();
    let home = f.home.to_str().unwrap();
    let refused = fm_bin("fmw", &f, &["schedule", "run", "--home", home, "demo", "gate"], "");
    assert_eq!(refused.status.code(), Some(3), "pending: {refused:?}");
    let store = foreman_core::secret::default_store(&f.home);
    schedule::approve(&f.home, &*store, &f.workspace(), "gate", 1).unwrap();
    let out = fm_bin("fmw", &f, &["schedule", "run", "--home", home, "demo", "gate", "--fingerprint", "0bad"], "");
    assert_eq!(out.status.code(), Some(0), "Run now has no task fingerprint to check: {out:?}");
    assert!(out.stdout.is_empty() && out.stderr.is_empty(), "{out:?}");
    let runs = schedule::runs(&f.home, "demo").unwrap();
    let last = runs.last().unwrap();
    assert_eq!(last.started.map(|(_, t)| t), Some(Fired::RunNow));
    assert_eq!(last.ended.as_ref().map(|e| e.1), Some(Outcome::Ok));
    assert_eq!(fm_bin("fmw", &f, &["schedule", "run", "demo", "gate"], "").status.code(), Some(2), "no --home");
}

/// `fm schedule secret set` keeps the value in the home's DPAPI store under the schedule's key, and only there.
#[test]
fn fire_secret_set_stores_the_value_sealed_and_refuses_an_unlisted_name() {
    let f = fixture();
    let mut s = script_schedule("gate", "exit0.cmd", ActionKind::Script);
    s.permissions.secrets = vec!["API_TOKEN".into()];
    schedule::edit(&f.home, &f.workspace(), |all| {
        all.push(s);
        Ok(())
    })
    .unwrap();
    let out = fm_bin("fm", &f, &["schedule", "secret", "set", "demo", "gate", "API_TOKEN"], &format!("{SECRET}\r\n"));
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(!String::from_utf8_lossy(&out.stdout).contains(SECRET));
    let store = foreman_core::secret::default_store(&f.home);
    assert_eq!(store.get(&schedule::secret_key("demo", "gate", "API_TOKEN")).unwrap().as_deref(), Some(SECRET));
    assert_eq!(files_holding(&f.home, SECRET), Vec::<PathBuf>::new(), "sealed on disk");

    let out = fm_bin("fm", &f, &["schedule", "secret", "set", "demo", "gate", "OTHER"], "x\n");
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("schedule gate has no secret OTHER; its secrets are API_TOKEN"), "{out:?}");
    let out = fm_bin("fm", &f, &["schedule", "secret", "set", "demo", "gate", "API_TOKEN"], "\n");
    assert_eq!(out.status.code(), Some(1), "an empty value is refused: {out:?}");
}
