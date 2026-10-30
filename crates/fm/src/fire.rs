//! `fm schedule fire` (what a schedule's task runs) and `fm schedule run` (Run now): the checks, the overlap
//! lock and the script stage of docs/research/scheduler.md, "The life of `fm schedule fire`". The session
//! stage is [`Host::session`].

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{Local, TimeZone};
use foreman_core::process::{self, Ended, Kill};
use foreman_core::schedule::{self, ActionKind, CatchUp, Fired, Live, Outcome, Overlap, RunEvent, RunRecord, Schedule, Stage, Status};
use foreman_core::secret::SecretStore;
use foreman_core::{Registry, Settings, Workspace};

pub const EXIT_OK: u8 = 0;
pub const EXIT_FAILED: u8 = 1;
/// No such workspace or schedule; written to `fire.log` alone.
pub const EXIT_UNKNOWN: u8 = 2;
pub const EXIT_REFUSED: u8 = 3;

pub const BUDGET_POLL: Duration = Duration::from_secs(60);
pub const BUDGET_POLLS: u32 = 30;
const CATCH_UP_GRACE_MS: i64 = 10 * 60 * 1000;
const RESTART_POLL: Duration = Duration::from_millis(250);
/// The script's exit code that hands its stdout to the session stage.
pub const GATE_EXIT: i32 = 10;
/// What [`RealHost::session`] records until sched/6 launches the session.
pub const LAUNCH_PENDING: &str = "launch pending (sched/6)";

pub struct Request<'a> {
    pub home: &'a Path,
    pub workspace: &'a str,
    pub id: &'a str,
    pub trigger: Fired,
    /// The task's own `--fingerprint`; Run now has none.
    pub fingerprint: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub run: Option<String>,
    /// `None` when nothing was recorded (an unknown workspace or schedule).
    pub outcome: Option<Outcome>,
    pub why: String,
    pub exit: u8,
}

/// A run as the session stage sees it.
pub struct RunCtx<'a> {
    pub home: &'a Path,
    pub workspace: &'a Workspace,
    pub schedule: &'a Schedule,
    pub run: &'a str,
    pub stamp: &'a str,
}

/// What fire takes from the machine, so tests can stand in for it.
pub trait Host {
    fn store(&self) -> &dyn SecretStore;
    /// This home's mid-turn `fm-` sessions fill `max_working`.
    fn busy(&mut self, home: &Path) -> Result<bool, String>;
    fn sleep(&mut self, d: Duration);
    fn now_ms(&mut self) -> i64;
    /// The session stage; `gate` is the script's stdout when it exited [`GATE_EXIT`], `None` for a session action.
    fn session(&mut self, ctx: &RunCtx, gate: Option<&str>) -> (Outcome, String);
}

pub struct RealHost {
    store: Arc<dyn SecretStore>,
}

impl RealHost {
    pub fn new(home: &Path) -> Self {
        Self { store: foreman_core::secret::default_store(home) }
    }
}

impl Host for RealHost {
    fn store(&self) -> &dyn SecretStore {
        &*self.store
    }

    fn busy(&mut self, home: &Path) -> Result<bool, String> {
        let max = Settings::load(home).map_err(|e| e.to_string())?.sessions.max_working;
        let (workspaces, _) = Registry::at(home).load_all().map_err(|e| e.to_string())?;
        let names: Vec<&str> = workspaces.iter().map(|w| w.name.as_str()).collect();
        let rows = foreman_claude::list_sessions().map_err(|e| e.to_string())?;
        let own = rows.iter().filter(|r| !foreman_supervisor::is_foreign(&r.name, names.iter().copied(), std::iter::empty()));
        Ok(foreman_supervisor::working(own, &names, foreman_claude::now_ms()) >= max)
    }

    fn sleep(&mut self, d: Duration) {
        std::thread::sleep(d);
    }

    fn now_ms(&mut self) -> i64 {
        foreman_core::time::now_ms()
    }

    fn session(&mut self, _ctx: &RunCtx, _gate: Option<&str>) -> (Outcome, String) {
        (Outcome::Skipped, LAUNCH_PENDING.into())
    }
}

/// `--home <home> <workspace> <id> [--fingerprint <hex>]`, as the task gives them; `print` echoes the result.
pub fn fire_cmd(args: &[&str], print: bool) -> u8 {
    let (mut home, mut fingerprint, mut positional) = (None, None, Vec::new());
    let mut it = args.iter();
    while let Some(&arg) = it.next() {
        match arg {
            "--home" => home = it.next().copied(),
            "--fingerprint" => fingerprint = it.next().copied(),
            p => positional.push(p),
        }
    }
    let (Some(home), [workspace, id]) = (home, positional.as_slice()) else {
        if print {
            eprintln!("error: schedule fire needs --home <home> <workspace> <id> --fingerprint <hex>");
        }
        return EXIT_UNKNOWN;
    };
    let home = Path::new(home);
    let req = Request { home, workspace, id, trigger: Fired::Task, fingerprint };
    let report = fire(&req, &mut RealHost::new(home));
    if print {
        print_report(&report);
    }
    report.exit
}

pub fn print_report(r: &Report) {
    let line = report_line(r);
    if r.exit == EXIT_OK { println!("{line}") } else { eprintln!("{line}") }
}

fn report_line(r: &Report) -> String {
    let outcome = r.outcome.map(outcome_name).unwrap_or("refused");
    let why = if r.why.is_empty() { String::new() } else { format!(": {}", r.why) };
    format!("{} {outcome}{why}", r.run.as_deref().unwrap_or("-"))
}

fn outcome_name(o: Outcome) -> &'static str {
    match o {
        Outcome::NothingToDo => "nothing_to_do",
        Outcome::Ok => "ok",
        Outcome::Done => "done",
        Outcome::NeedsYou => "needs_you",
        Outcome::Failed => "failed",
        Outcome::Refused => "refused",
        Outcome::Skipped => "skipped",
        Outcome::CappedRuntime => "capped_runtime",
        Outcome::CappedTokens => "capped_tokens",
        Outcome::Interrupted => "interrupted",
    }
}

/// One fire of schedule `req.id`, recorded in the run records and `fire.log`.
pub fn fire(req: &Request, host: &mut dyn Host) -> Report {
    let report = fire_inner(req, host);
    let trigger = match req.trigger {
        Fired::Task => "task",
        Fired::RunNow => "run_now",
        Fired::CatchUp => "catch_up",
    };
    log(req.home, &format!("{}/{} {trigger} {}", req.workspace, req.id, report_line(&report)));
    report
}

fn log(home: &Path, line: &str) {
    let path = schedule::fire_log(home);
    let line = format!("{} {line}\n", Local::now().format("%Y-%m-%dT%H:%M:%S%:z"));
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let _ = foreman_core::fs::append_capped(&path, line.as_bytes(), schedule::FIRE_LOG_CAP);
}

/// Appends one run's lines; `started` is written once, before the first other line.
struct Recorder<'a> {
    home: &'a Path,
    workspace: &'a str,
    schedule: &'a str,
    run: String,
    trigger: Fired,
    fingerprint: String,
    fired_ms: i64,
    started: bool,
}

impl Recorder<'_> {
    fn append(&mut self, ts: i64, event: RunEvent) {
        let record = RunRecord { ts, run: self.run.clone(), schedule: self.schedule.to_string(), event };
        if let Err(e) = schedule::append_run(self.home, self.workspace, &record) {
            log(self.home, &format!("{}/{} {}: cannot record the run: {e}", self.workspace, self.schedule, self.run));
        }
    }

    fn start(&mut self, ts: i64) {
        if !self.started {
            self.started = true;
            let event = RunEvent::Started { trigger: self.trigger, fingerprint: self.fingerprint.clone(), fm_pid: std::process::id() };
            self.append(ts, event);
        }
    }

    fn stage(&mut self, ts: i64, stage: Stage) {
        self.start(ts);
        self.append(ts, RunEvent::Stage(stage));
    }

    fn end(&mut self, ts: i64, outcome: Outcome, why: impl Into<String>, exit: u8) -> Report {
        self.start(ts);
        let why = why.into();
        let ms = u64::try_from(ts - self.fired_ms).unwrap_or(0);
        self.append(ts, RunEvent::Ended { outcome, why: why.clone(), tokens: None, ms });
        Report { run: Some(self.run.clone()), outcome: Some(outcome), why, exit }
    }

    fn ended(&mut self, ts: i64, outcome: Outcome, why: impl Into<String>) -> Report {
        let exit = if outcome.succeeded() { EXIT_OK } else { EXIT_FAILED };
        self.end(ts, outcome, why, exit)
    }
}

fn unknown(why: String) -> Report {
    Report { run: None, outcome: None, why, exit: EXIT_UNKNOWN }
}

fn fire_inner(req: &Request, host: &mut dyn Host) -> Report {
    let home = req.home;
    let w = match Registry::at(home).load(req.workspace) {
        Ok(w) => w,
        Err(e) => return unknown(e.to_string()),
    };
    let s = match schedule::load(home, &w.name).map(|all| all.into_iter().find(|s| s.id == req.id)) {
        Ok(Some(s)) => s,
        Ok(None) => return unknown(format!("workspace {} has no schedule {}", w.name, req.id)),
        Err(e) => return unknown(e.to_string()),
    };
    let Some(folder) = w.folder(&s.folder).map(|f| f.path.clone()) else {
        return unknown(format!("workspace {} has no folder {}", w.name, s.folder));
    };
    let live = match schedule::live_dir(home, &w.name) {
        Ok(dir) => Paths::new(dir, &s.id),
        Err(e) => return unknown(e.to_string()),
    };
    let fired_ms = host.now_ms();
    let stamp = fresh_stamp(home, &w.name, &s.id, &live, fired_ms);
    let mut rec = Recorder {
        home,
        workspace: &w.name,
        schedule: &s.id,
        run: schedule::run_id(&s.id, &stamp),
        trigger: req.trigger,
        fingerprint: String::new(),
        fired_ms,
        started: false,
    };

    if s.status != Status::Active {
        let status = if s.status == Status::Paused { "paused" } else { "pending: it was never approved, or changed since" };
        return rec.end(host.now_ms(), Outcome::Refused, format!("the schedule is {status}"), EXIT_REFUSED);
    }
    match schedule::verify_approval(host.store(), &w, &s) {
        Ok(c) => rec.fingerprint = c.fingerprint,
        Err(refusal) => {
            return rec.end(host.now_ms(), Outcome::Refused, refusal.to_string(), EXIT_REFUSED);
        }
    }
    if req.fingerprint.is_some_and(|f| f != rec.fingerprint) {
        let why = "the task's fingerprint is not the approved one; the task is out of date until the Foreman window syncs it";
        return rec.end(host.now_ms(), Outcome::Refused, why, EXIT_REFUSED);
    }

    if req.trigger == Fired::Task
        && s.policy.catch_up == CatchUp::Skip
        && let Some(now) = Local.timestamp_millis_opt(fired_ms).single()
        && let Some(prev) = schedule::previous_fire(&s.trigger, &now)
        && fired_ms - prev.timestamp_millis() > CATCH_UP_GRACE_MS
    {
        return rec.ended(host.now_ms(), Outcome::Skipped, format!("missed: the start at {} is more than 10 minutes ago", prev.format("%Y-%m-%d %H:%M")));
    }

    if s.action.kind != ActionKind::Script {
        let mut polls = 0;
        loop {
            match host.busy(home) {
                Ok(false) => break,
                Ok(true) if polls == BUDGET_POLLS => {
                    return rec.ended(host.now_ms(), Outcome::Skipped, "busy: max_working sessions were mid-turn for 30 minutes");
                }
                Ok(true) => {}
                Err(e) => {
                    log(home, &format!("{}/{} {}: cannot read the budget, firing anyway: {e}", w.name, s.id, rec.run));
                    break;
                }
            }
            host.sleep(BUDGET_POLL);
            polls += 1;
        }
    }

    let _lock = match overlap(&live, s.policy.overlap) {
        Ok(lock) => lock,
        Err(Overlapped::Skip(why)) => return rec.ended(host.now_ms(), Outcome::Skipped, why),
        Err(Overlapped::Io(why)) => return rec.ended(host.now_ms(), Outcome::Failed, why),
    };
    let _ = fs::remove_file(&live.restart);
    recover_dead_run(home, &w.name, &s.id, &live, host.now_ms());

    let last = last_run(home, &w.name, &s.id);
    let started_ms = host.now_ms();
    let run = rec.run.clone();
    let write_live = |stage: &str| {
        let pid = std::process::id();
        let body = Live { run: run.clone(), fm_pid: pid, fm_started: process::started(pid), started_at: started_ms, stage: stage.into() };
        let _ = foreman_core::fs::write_atomic(&live.json, serde_json::to_vec(&body).expect("plain data serializes"));
    };
    write_live(if s.action.kind == ActionKind::Session { "session" } else { "script" });
    rec.start(started_ms);

    let ctx = RunCtx { home, workspace: &w, schedule: &s, run: &run, stamp: &stamp };
    let (outcome, why) = match s.action.kind {
        ActionKind::Session => host.session(&ctx, None),
        kind => match secrets(host.store(), &w.name, &s) {
            Err(why) => (Outcome::Failed, why),
            Ok(secrets) => {
                let env = ScriptEnv { home, workspace: &w.name, id: &s.id, run: &run, folder: &folder, last };
                let (end, stage) = script_stage(&env, &s, &secrets, &live.restart, host);
                if let Some(stage) = stage {
                    rec.stage(host.now_ms(), stage);
                }
                match end {
                    ScriptEnd::Gate(_) | ScriptEnd::Nothing if kind == ActionKind::Script => (Outcome::Ok, String::new()),
                    ScriptEnd::Gate(stdout) => {
                        write_live("session");
                        host.session(&ctx, Some(&stdout))
                    }
                    ScriptEnd::Nothing => (Outcome::NothingToDo, String::new()),
                    ScriptEnd::Failed(why) => (Outcome::Failed, why),
                    ScriptEnd::Restarted => (Outcome::Interrupted, "restarted".into()),
                }
            }
        },
    };
    let report = rec.ended(host.now_ms(), outcome, why);
    let _ = fs::remove_file(&live.json);
    report
}

/// One schedule's files in `schedules/live/<ws>/`.
struct Paths {
    lock: PathBuf,
    queued: PathBuf,
    restart: PathBuf,
    json: PathBuf,
}

impl Paths {
    fn new(dir: PathBuf, id: &str) -> Self {
        Self {
            lock: dir.join(format!("{id}.lock")),
            queued: dir.join(format!("{id}.queued")),
            restart: dir.join(format!("{id}.restart")),
            json: dir.join(format!("{id}.json")),
        }
    }
}

enum Overlapped {
    Skip(String),
    Io(String),
}

/// The overlap lock, taken per the policy (step 5).
fn overlap(live: &Paths, policy: Overlap) -> Result<fs::File, Overlapped> {
    let io = |what: &Path, e: std::io::Error| Overlapped::Io(format!("cannot lock {}: {e}", what.display()));
    if let Some(lock) = foreman_core::fs::try_lock_file(&live.lock).map_err(|e| io(&live.lock, e))? {
        return Ok(lock);
    }
    let in_flight = || {
        let run = fs::read(&live.json).ok().and_then(|b| serde_json::from_slice::<Live>(&b).ok()).map(|l| l.run);
        format!("run {} is in flight", run.as_deref().unwrap_or("of this schedule"))
    };
    match policy {
        Overlap::Skip => Err(Overlapped::Skip(format!("overlap: {}", in_flight()))),
        Overlap::Queue => {
            let Some(_waiting) = foreman_core::fs::try_lock_file(&live.queued).map_err(|e| io(&live.queued, e))? else {
                return Err(Overlapped::Skip(format!("overlap: {} and another run waits for it", in_flight())));
            };
            foreman_core::fs::lock_file(&live.lock).map_err(|e| io(&live.lock, e))
        }
        Overlap::Restart => {
            fs::write(&live.restart, b"").map_err(|e| Overlapped::Io(format!("cannot write {}: {e}", live.restart.display())))?;
            foreman_core::fs::lock_file(&live.lock).map_err(|e| io(&live.lock, e))
        }
    }
}

/// A live file left under a free lock is a run whose fm died: it ends as `interrupted`.
fn recover_dead_run(home: &Path, workspace: &str, id: &str, live: &Paths, now: i64) {
    let Ok(bytes) = fs::read(&live.json) else {
        return;
    };
    if let Ok(dead) = serde_json::from_slice::<Live>(&bytes) {
        let ended = schedule::runs(home, workspace).is_ok_and(|runs| runs.iter().any(|r| r.run == dead.run && r.ended.is_some()));
        if !ended {
            let why = format!("fm (pid {}) ended without finishing the run", dead.fm_pid);
            let ms = u64::try_from(now - dead.started_at).unwrap_or(0);
            let record =
                RunRecord { ts: now, run: dead.run, schedule: id.to_string(), event: RunEvent::Ended { outcome: Outcome::Interrupted, why, tokens: None, ms } };
            let _ = schedule::append_run(home, workspace, &record);
        }
    }
    let _ = fs::remove_file(&live.json);
}

/// `yyyymmdd-hhmmss` of `at`, with `-2`, `-3`… when a run of this schedule already has that id.
fn fresh_stamp(home: &Path, workspace: &str, id: &str, live: &Paths, at: i64) -> String {
    let base = Local.timestamp_millis_opt(at).single().map(|t| schedule::run_stamp(&t)).unwrap_or_else(|| at.to_string());
    let mut taken: Vec<String> = schedule::read_runs(home, workspace).unwrap_or_default().into_iter().filter(|r| r.schedule == id).map(|r| r.run).collect();
    taken.extend(fs::read(&live.json).ok().and_then(|b| serde_json::from_slice::<Live>(&b).ok()).map(|l| l.run));
    (1..)
        .map(|n| if n == 1 { base.clone() } else { format!("{base}-{n}") })
        .find(|stamp| !taken.contains(&schedule::run_id(id, stamp)))
        .expect("an unbounded range")
}

/// `(start ms, outcome)` of the schedule's latest run.
fn last_run(home: &Path, workspace: &str, id: &str) -> Option<(i64, Option<Outcome>)> {
    let runs = schedule::runs(home, workspace).ok()?;
    runs.into_iter().rev().filter(|r| r.schedule == id).find_map(|r| r.started.map(|(ts, _)| (ts, r.ended.map(|(_, o, _)| o))))
}

/// The schedule's secrets as `(NAME, value)`, read from the store; `Err` names one that is missing or unreadable.
fn secrets(store: &dyn SecretStore, workspace: &str, s: &Schedule) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    for name in &s.permissions.secrets {
        match store.get(&schedule::secret_key(workspace, &s.id, name)) {
            Ok(Some(value)) => out.push((name.clone(), value)),
            Ok(None) => {
                return Err(format!("secret {name} is not set: fm schedule secret set {workspace} {} {name}", s.id));
            }
            Err(e) => return Err(format!("cannot read secret {name}: {e}")),
        }
    }
    Ok(out)
}

struct ScriptEnv<'a> {
    home: &'a Path,
    workspace: &'a str,
    id: &'a str,
    run: &'a str,
    folder: &'a Path,
    last: Option<(i64, Option<Outcome>)>,
}

enum ScriptEnd {
    Nothing,
    /// Exit [`GATE_EXIT`], with the stdout masked for the schedule's secrets.
    Gate(String),
    Failed(String),
    Restarted,
}

/// Step 7: the script in the folder's main checkout, per the script contract.
fn script_stage(env: &ScriptEnv, s: &Schedule, secrets: &[(String, String)], restart: &Path, host: &mut dyn Host) -> (ScriptEnd, Option<Stage>) {
    let script = s.action.script.as_deref().unwrap_or_default();
    let state = match schedule::state_dir(env.home, env.workspace, env.id) {
        Ok(dir) => dir,
        Err(e) => return (ScriptEnd::Failed(e.to_string()), None),
    };
    if let Err(e) = fs::create_dir_all(&state) {
        return (ScriptEnd::Failed(format!("cannot create {}: {e}", state.display())), None);
    }
    let values: Vec<String> = secrets.iter().map(|(_, v)| v.clone()).collect();
    let timeout = s.action.script_timeout.duration();
    let started = host.now_ms();
    let mut next_check = Instant::now();
    let mut stop = || {
        if Instant::now() < next_check {
            return false;
        }
        next_check = Instant::now() + RESTART_POLL;
        restart.exists()
    };
    let mut ran = Err(std::io::Error::from(std::io::ErrorKind::NotFound));
    for pwsh in [true, false] {
        let mut cmd = script_command(env.folder, script, &s.action.script_args, pwsh);
        cmd.current_dir(env.folder)
            .env("FOREMAN_HOME", env.home)
            .env("FOREMAN_WORKSPACE", env.workspace)
            .env("FOREMAN_SCHEDULE", env.id)
            .env("FOREMAN_RUN", env.run)
            .env("FOREMAN_FOLDER", env.folder)
            .env("FOREMAN_LAST_RUN", env.last.map(|(ts, _)| ts.to_string()).unwrap_or_default())
            .env("FOREMAN_LAST_OUTCOME", env.last.and_then(|(_, o)| o).map(outcome_name).unwrap_or_default())
            .env("FOREMAN_STATE_DIR", &state)
            .envs(secrets.iter().map(|(k, v)| (k, v)));
        ran = process::run_watched(&mut cmd, timeout, Kill::Tree, &mut stop);
        let missing_pwsh = matches!(&ran, Err(e) if e.kind() == std::io::ErrorKind::NotFound);
        if !(pwsh && missing_pwsh && extension(script) == "ps1") {
            break;
        }
    }
    let ms = u64::try_from(host.now_ms() - started).unwrap_or(0);
    let (ended, out) = match ran {
        Ok(done) => done,
        Err(e) => {
            return (ScriptEnd::Failed(format!("cannot start the script {script}: {e}")), None);
        }
    };
    let stage = Stage::Script { code: out.code, ms, stderr: schedule::masked_tail(&out.stderr, &values, schedule::STDERR_TAIL) };
    let end = match (ended, out.code) {
        (Ended::Exited, Some(0)) => ScriptEnd::Nothing,
        (Ended::Exited, Some(GATE_EXIT)) => ScriptEnd::Gate(schedule::masked_tail(&out.stdout, &values, usize::MAX)),
        (Ended::Exited, code) => ScriptEnd::Failed(format!("the script exited {}", code.map(|c| c.to_string()).unwrap_or_else(|| "without a code".into()))),
        (Ended::TimedOut, _) => ScriptEnd::Failed(format!("the script did not finish within {} and was killed", s.action.script_timeout)),
        (Ended::Stopped, _) => ScriptEnd::Restarted,
    };
    (end, Some(stage))
}

fn extension(script: &str) -> String {
    Path::new(script).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}

/// The command per the script's extension (the script contract's table).
fn script_command(folder: &Path, script: &str, args: &[String], pwsh: bool) -> Command {
    let path = folder.join(script);
    match extension(script).as_str() {
        "ps1" => {
            let mut cmd = process::command(if pwsh { "pwsh" } else { "powershell" });
            cmd.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"]).arg(&path).args(args);
            cmd
        }
        "cmd" | "bat" => {
            let mut cmd = process::command("cmd");
            let line =
                std::iter::once(path.to_string_lossy().into_owned()).chain(args.iter().cloned()).map(|a| format!("\"{a}\"")).collect::<Vec<_>>().join(" ");
            raw(&mut cmd, &format!("/d /s /c \"{line}\""));
            cmd
        }
        _ => {
            let mut cmd = process::command(&path);
            cmd.args(args);
            cmd
        }
    }
}

/// cmd's `/s /c "…"` line is not parsed by the MSVC rules `Command::arg` quotes for.
#[cfg(windows)]
fn raw(cmd: &mut Command, line: &str) {
    use std::os::windows::process::CommandExt as _;
    cmd.raw_arg(line);
}

#[cfg(not(windows))]
fn raw(cmd: &mut Command, line: &str) {
    cmd.arg(line);
}
