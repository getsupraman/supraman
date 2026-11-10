//! `sm schedule fire` (what a schedule's task runs) and `sm schedule run` (Run now): the checks, the overlap
//! lock and the script stage of docs/research/scheduler.md, "The life of `sm schedule fire`". The session
//! stage is [`Host::session`], [`crate::run`] on the real host.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{Local, TimeZone};
use supraman_core::process::{self, Ended, Kill};
use supraman_claude::Claude;
use supraman_core::schedule::{self, ActionKind, CatchUp, Fired, Live, LiveSession, Outcome, Overlap, RunEvent, RunRecord, Schedule, Stage, Status, Tokens};
use supraman_core::secret::SecretStore;
use supraman_core::{Registry, Settings, Workspace};
use supraman_supervisor::{AppTokens, HttpMinter, Minter, TokenProvider};

use crate::run::{Agents, RunLog, SessionEnd, SessionHost};

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
    pub fired_ms: i64,
}

/// What fire takes from the machine, so tests can stand in for it.
pub trait Host {
    fn store(&self) -> &dyn SecretStore;
    /// This home's mid-turn `fm-` sessions fill `max_working`.
    fn busy(&mut self, home: &Path) -> Result<bool, String>;
    fn sleep(&mut self, d: Duration);
    fn now_ms(&mut self) -> i64;
    /// The session stage; `gate` is the script's stdout when it exited [`GATE_EXIT`], `None` for a session action.
    fn session(&mut self, ctx: &RunCtx, gate: Option<&str>, log: &mut dyn RunLog) -> SessionEnd;
    /// The session of a run whose sm died: stopped (never rm'd); its token, config and record stay.
    fn end_dead_session(&mut self, home: &Path, session: &LiveSession) -> Result<(), String>;
}

pub struct RealHost {
    store: Arc<dyn SecretStore>,
    claude: Claude,
    tokens: AppTokens,
    minter: HttpMinter,
}

impl RealHost {
    pub fn new(home: &Path) -> Self {
        Self {
            store: supraman_core::secret::default_store(home),
            claude: Claude::default(),
            tokens: AppTokens { auth: supraman_workman::Auth::open(home) },
            minter: HttpMinter::default(),
        }
    }
}

impl SessionHost for RealHost {
    fn claude(&self) -> &dyn Agents {
        &self.claude
    }

    fn tokens(&self) -> &dyn TokenProvider {
        &self.tokens
    }

    fn minter(&self) -> &dyn Minter {
        &self.minter
    }

    fn claude_dir(&self) -> Option<PathBuf> {
        supraman_core::paths::claude_dir()
    }

    fn plugin_dir(&self) -> Option<PathBuf> {
        std::env::current_exe().ok().and_then(|exe| supraman_core::find_plugin_dir(&exe).ok())
    }

    fn now_ms(&mut self) -> i64 {
        supraman_core::time::now_ms()
    }

    fn sleep(&mut self, d: Duration) {
        std::thread::sleep(d);
    }
}

/// [`Host::end_dead_session`] with `host`'s claude.
pub fn end_dead_session(host: &dyn SessionHost, _home: &Path, session: &LiveSession) -> Result<(), String> {
    host.claude().stop(&session.id).map(drop).map_err(|e| format!("claude stop {}: {e}", session.id))
}

impl Host for RealHost {
    fn store(&self) -> &dyn SecretStore {
        &*self.store
    }

    fn busy(&mut self, home: &Path) -> Result<bool, String> {
        let max = Settings::load(home).map_err(|e| e.to_string())?.sessions.max_working;
        let (workspaces, _) = Registry::at(home).load_all().map_err(|e| e.to_string())?;
        let names: Vec<&str> = workspaces.iter().map(|w| w.name.as_str()).collect();
        let rows = supraman_claude::list_sessions().map_err(|e| e.to_string())?;
        let own = rows.iter().filter(|r| !supraman_supervisor::is_foreign(&r.name, names.iter().copied(), std::iter::empty()));
        Ok(supraman_supervisor::working(own, &names, supraman_claude::now_ms()) >= max)
    }

    fn sleep(&mut self, d: Duration) {
        std::thread::sleep(d);
    }

    fn now_ms(&mut self) -> i64 {
        supraman_core::time::now_ms()
    }

    fn session(&mut self, ctx: &RunCtx, gate: Option<&str>, log: &mut dyn RunLog) -> SessionEnd {
        crate::run::session_stage(ctx, gate, self, log)
    }

    fn end_dead_session(&mut self, home: &Path, session: &LiveSession) -> Result<(), String> {
        end_dead_session(self, home, session)
    }
}

/// `--home <home> <workspace> <id> [--fingerprint <hex>]`, as the task gives them; `print` echoes the result.
pub fn fire_cmd(args: &[&str], print: bool) -> u8 {
    fired_cmd(args, print, Fired::Task)
}

/// Run now from the Supraman window: `--home <home> <workspace> <id>`, the path of `sm schedule run`.
pub fn run_now_cmd(args: &[&str]) -> u8 {
    fired_cmd(args, false, Fired::RunNow)
}

fn fired_cmd(args: &[&str], print: bool, trigger: Fired) -> u8 {
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
    let fingerprint = fingerprint.filter(|_| trigger == Fired::Task);
    let req = Request { home, workspace, id, trigger, fingerprint };
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

pub(crate) fn log(home: &Path, line: &str) {
    let path = schedule::fire_log(home);
    let line = format!("{} {line}\n", Local::now().format("%Y-%m-%dT%H:%M:%S%:z"));
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let _ = supraman_core::fs::append_capped(&path, line.as_bytes(), schedule::FIRE_LOG_CAP);
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
        self.end_with(ts, outcome, why.into(), None, exit)
    }

    fn end_with(&mut self, ts: i64, outcome: Outcome, why: String, tokens: Option<Tokens>, exit: u8) -> Report {
        self.start(ts);
        let ms = u64::try_from(ts - self.fired_ms).unwrap_or(0);
        self.append(ts, RunEvent::Ended { outcome, why: why.clone(), tokens, ms });
        Report { run: Some(self.run.clone()), outcome: Some(outcome), why, exit }
    }

    fn ended(&mut self, ts: i64, outcome: Outcome, why: impl Into<String>) -> Report {
        self.end_with(ts, outcome, why.into(), None, exit_of(outcome))
    }
}

fn exit_of(outcome: Outcome) -> u8 {
    if outcome.succeeded() { EXIT_OK } else { EXIT_FAILED }
}

/// The live file of the run in flight: `schedules/live/<ws>/<id>.json`.
struct LiveFile<'h> {
    home: &'h Path,
    path: PathBuf,
    body: Live,
    unwritten: bool,
}

impl LiveFile<'_> {
    /// A write that fails is said once in fire.log: without the file, a dead sm's run is not recovered.
    fn write(&mut self) {
        let written = supraman_core::fs::write_atomic(&self.path, serde_json::to_vec(&self.body).expect("plain data serializes"));
        if let Err(e) = written
            && !std::mem::replace(&mut self.unwritten, true)
        {
            log(self.home, &format!("{}: cannot write the live file {}: {e}", self.body.run, self.path.display()));
        }
    }

    fn stage(&mut self, stage: &str) {
        self.body.stage = stage.into();
        self.write();
    }
}

/// What the session stage writes through: the run records and the live file.
struct SessionLog<'r, 'a> {
    rec: &'r mut Recorder<'a>,
    live: &'r mut LiveFile<'a>,
}

impl RunLog for SessionLog<'_, '_> {
    fn event(&mut self, ts: i64, event: RunEvent) {
        self.rec.start(ts);
        self.rec.append(ts, event);
    }

    fn session(&mut self, session: &LiveSession) {
        self.live.body.session = Some(session.clone());
        self.live.write();
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
        let why = "the task's fingerprint is not the approved one; the task is out of date until the Supraman window syncs it";
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
    let now = host.now_ms();
    recover_dead_run(home, &w.name, &s.id, &live, now, host);

    let last = last_run(home, &w.name, &s.id);
    let started_ms = host.now_ms();
    let run = rec.run.clone();
    let pid = std::process::id();
    let mut live_file = LiveFile {
        home,
        path: live.json.clone(),
        body: Live { run: run.clone(), fm_pid: pid, fm_started: supraman_core::platform::process::started(pid), started_at: started_ms, stage: String::new(), session: None },
        unwritten: false,
    };
    live_file.stage(if s.action.kind == ActionKind::Session { "session" } else { "script" });
    rec.start(started_ms);

    let ctx = RunCtx { home, workspace: &w, schedule: &s, run: &run, stamp: &stamp, fired_ms };
    let plain = |outcome, why: String| SessionEnd { outcome, why, tokens: None, recorded: false };
    let end = match s.action.kind {
        ActionKind::Session => host.session(&ctx, None, &mut SessionLog { rec: &mut rec, live: &mut live_file }),
        kind => match secrets(host.store(), &w.name, &s) {
            Err(why) => plain(Outcome::Failed, why),
            Ok(secrets) => {
                let env = ScriptEnv { home, workspace: &w.name, id: &s.id, run: &run, folder: &folder, last };
                let (end, stage) = script_stage(&env, &s, &secrets, &live.restart, host);
                if let Some(stage) = stage {
                    rec.stage(host.now_ms(), stage);
                }
                match end {
                    ScriptEnd::Gate(_) | ScriptEnd::Nothing if kind == ActionKind::Script => plain(Outcome::Ok, String::new()),
                    ScriptEnd::Gate(stdout) => {
                        live_file.stage("session");
                        host.session(&ctx, Some(&stdout), &mut SessionLog { rec: &mut rec, live: &mut live_file })
                    }
                    ScriptEnd::Nothing => plain(Outcome::NothingToDo, String::new()),
                    ScriptEnd::Failed(why) => plain(Outcome::Failed, why),
                    ScriptEnd::Restarted => plain(Outcome::Interrupted, "restarted".into()),
                }
            }
        },
    };
    let report = if end.recorded {
        Report { run: Some(rec.run.clone()), outcome: Some(end.outcome), why: end.why, exit: exit_of(end.outcome) }
    } else {
        rec.end_with(host.now_ms(), end.outcome, end.why, end.tokens, exit_of(end.outcome))
    };
    let _ = fs::remove_file(&live.json);
    report
}

/// One schedule's files in `schedules/live/<ws>/`.
struct Paths {
    lock: PathBuf,
    queued: PathBuf,
    restart: PathBuf,
    json: PathBuf,
    dead: PathBuf,
}

impl Paths {
    fn new(dir: PathBuf, id: &str) -> Self {
        Self {
            lock: dir.join(format!("{id}.lock")),
            queued: dir.join(format!("{id}.queued")),
            restart: dir.join(format!("{id}.restart")),
            json: dir.join(format!("{id}.json")),
            dead: dir.join(format!("{id}.dead.json")),
        }
    }
}

/// The session of a dead run whose stop failed: `<id>.dead.json`, tried again by [`recover_if_dead`].
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct DeadSession {
    run: String,
    session: LiveSession,
    tries: u32,
    next_at: i64,
}

/// How often a dead run's session is tried before it is given up and said in fire.log.
const DEAD_TRIES: u32 = 5;
/// The wait before the next try grows by this with each failed one.
const DEAD_RETRY_MS: i64 = 60_000;

fn keep_dead(home: &Path, live: &Paths, dead: &DeadSession) {
    let written = supraman_core::fs::write_atomic(&live.dead, serde_json::to_vec(dead).expect("plain data serializes"));
    if let Err(e) = written {
        log(home, &format!("{}: cannot keep its session for another try in {}: {e}", dead.run, live.dead.display()));
    }
}

/// Tries the kept session of a dead run again once its time came; it goes once ended or after [`DEAD_TRIES`].
fn retry_dead_session(home: &Path, workspace: &str, id: &str, live: &Paths, now: i64, host: &mut dyn Host) {
    let Ok(bytes) = fs::read(&live.dead) else {
        return;
    };
    let Ok(mut dead) = serde_json::from_slice::<DeadSession>(&bytes) else {
        log(home, &format!("{workspace}/{id}: {} is not readable; removed", live.dead.display()));
        let _ = fs::remove_file(&live.dead);
        return;
    };
    if now < dead.next_at {
        return;
    }
    match host.end_dead_session(home, &dead.session) {
        Ok(()) => {
            log(home, &format!("{workspace}/{id} {}: its session {} is ended", dead.run, dead.session.name));
            let _ = fs::remove_file(&live.dead);
        }
        Err(e) if dead.tries + 1 >= DEAD_TRIES => {
            log(home, &format!("{workspace}/{id} {}: given up on its session {} after {} tries: {e}", dead.run, dead.session.name, dead.tries + 1));
            let _ = fs::remove_file(&live.dead);
        }
        Err(e) => {
            dead.tries += 1;
            dead.next_at = now + DEAD_RETRY_MS * i64::from(dead.tries);
            log(home, &format!("{workspace}/{id} {}: try {} for its session {}: {e}", dead.run, dead.tries, dead.session.name));
            keep_dead(home, live, &dead);
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
    if let Some(lock) = supraman_core::fs::try_lock_file(&live.lock).map_err(|e| io(&live.lock, e))? {
        return Ok(lock);
    }
    let in_flight = || {
        let run = fs::read(&live.json).ok().and_then(|b| serde_json::from_slice::<Live>(&b).ok()).map(|l| l.run);
        format!("run {} is in flight", run.as_deref().unwrap_or("of this schedule"))
    };
    match policy {
        Overlap::Skip => Err(Overlapped::Skip(format!("overlap: {}", in_flight()))),
        Overlap::Queue => {
            let Some(_waiting) = supraman_core::fs::try_lock_file(&live.queued).map_err(|e| io(&live.queued, e))? else {
                return Err(Overlapped::Skip(format!("overlap: {} and another run waits for it", in_flight())));
            };
            supraman_core::fs::lock_file(&live.lock).map_err(|e| io(&live.lock, e))
        }
        Overlap::Restart => {
            fs::write(&live.restart, b"").map_err(|e| Overlapped::Io(format!("cannot write {}: {e}", live.restart.display())))?;
            supraman_core::fs::lock_file(&live.lock).map_err(|e| io(&live.lock, e))
        }
    }
}

/// The app's recovery (scheduler.md, step 11): a live file whose sm is gone, while no fire holds the overlap
/// lock, is recovered as [`recover_dead_run`] does; the run id when it was.
pub fn recover_if_dead(home: &Path, workspace: &str, id: &str) -> Option<String> {
    let live = Paths::new(schedule::live_dir(home, workspace).ok()?, id);
    if !live.json.exists() && !live.dead.exists() {
        return None;
    }
    recover_if_dead_with(home, workspace, id, supraman_core::time::now_ms(), &mut RealHost::new(home))
}

/// [`recover_if_dead`] for each schedule of `workspace` with a live or a dead file; the runs recovered.
pub fn recover_workspace(home: &Path, workspace: &str) -> Vec<String> {
    let ids = live_ids(home, workspace);
    if ids.is_empty() {
        return Vec::new();
    }
    recover_ids(home, workspace, ids, supraman_core::time::now_ms(), &mut RealHost::new(home))
}

/// [`recover_workspace`] at `now` through `host`.
pub fn recover_workspace_with(home: &Path, workspace: &str, now: i64, host: &mut dyn Host) -> Vec<String> {
    recover_ids(home, workspace, live_ids(home, workspace), now, host)
}

fn recover_ids(home: &Path, workspace: &str, ids: std::collections::BTreeSet<String>, now: i64, host: &mut dyn Host) -> Vec<String> {
    ids.iter().filter_map(|id| recover_if_dead_with(home, workspace, id, now, host)).collect()
}

/// The schedule ids of `<id>.json` and `<id>.dead.json` in `workspace`'s live folder, each once.
fn live_ids(home: &Path, workspace: &str) -> std::collections::BTreeSet<String> {
    let Some(entries) = schedule::live_dir(home, workspace).ok().and_then(|d| fs::read_dir(d).ok()) else {
        return Default::default();
    };
    entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let stem = name.strip_suffix(".json")?;
            Some(stem.strip_suffix(".dead").unwrap_or(stem).to_string())
        })
        .collect()
}

/// [`recover_if_dead`] at `now` through `host`; it also tries a dead run's kept session again.
pub fn recover_if_dead_with(home: &Path, workspace: &str, id: &str, now: i64, host: &mut dyn Host) -> Option<String> {
    let live = Paths::new(schedule::live_dir(home, workspace).ok()?, id);
    if live.dead.exists()
        && let Ok(Some(_lock)) = supraman_core::fs::try_lock_file(&live.lock)
    {
        retry_dead_session(home, workspace, id, &live, now, host);
    }
    let dead: Live = serde_json::from_slice(&fs::read(&live.json).ok()?).ok()?;
    if sm_alive(&dead) {
        return None;
    }
    let _lock = supraman_core::fs::try_lock_file(&live.lock).ok()??;
    let still: Live = serde_json::from_slice(&fs::read(&live.json).ok()?).ok()?;
    if still.run != dead.run {
        return None;
    }
    recover_dead_run(home, workspace, id, &live, now, host);
    Some(dead.run)
}

/// The live file's sm runs: its pid is up with the start time it recorded.
pub fn sm_alive(live: &Live) -> bool {
    match (supraman_core::platform::process::started(live.fm_pid), live.fm_started) {
        (Some(now), Some(then)) => now == then,
        (running, _) => running.is_some(),
    }
}

/// A live file left under a free lock is a run whose sm died: its session is stopped, and it
/// ends as `interrupted` unless it ended before.
fn recover_dead_run(home: &Path, workspace: &str, id: &str, live: &Paths, now: i64, host: &mut dyn Host) {
    let Ok(bytes) = fs::read(&live.json) else {
        return;
    };
    match serde_json::from_slice::<Live>(&bytes) {
        Ok(dead) => {
            let mut why = format!("sm (pid {}) ended without finishing the run", dead.fm_pid);
            if let Some(session) = &dead.session
                && let Err(e) = host.end_dead_session(home, session)
            {
                log(home, &format!("{workspace}/{id} {}: {e}; tried again later", dead.run));
                why.push_str(&format!("; {e}"));
                keep_dead(home, live, &DeadSession { run: dead.run.clone(), session: session.clone(), tries: 1, next_at: now + DEAD_RETRY_MS });
            }
            let ended = schedule::runs(home, workspace).is_ok_and(|runs| runs.iter().any(|r| r.run == dead.run && r.ended.is_some()));
            if !ended {
                let ms = u64::try_from(now - dead.started_at).unwrap_or(0);
                let run = dead.run.clone();
                let record = RunRecord { ts: now, run: dead.run, schedule: id.to_string(), event: RunEvent::Ended { outcome: Outcome::Interrupted, why, tokens: None, ms } };
                if let Err(e) = schedule::append_run(home, workspace, &record) {
                    log(home, &format!("{workspace}/{id} {run}: cannot record it as interrupted: {e}"));
                }
            }
        }
        Err(e) => log(home, &format!("{workspace}/{id}: the live file {} is not readable ({e}); removed", live.json.display())),
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
                return Err(format!("secret {name} is not set: sm schedule secret set {workspace} {} {name}", s.id));
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
    let mut cmd = script_command(env.folder, script, &s.action.script_args);
    cmd.current_dir(env.folder).envs(supraman_core::home::home_env(env.home));
    let vars: [(&str, std::ffi::OsString); 7] = [
        ("WORKSPACE", env.workspace.into()),
        ("SCHEDULE", env.id.into()),
        ("RUN", env.run.into()),
        ("FOLDER", env.folder.into()),
        ("LAST_RUN", env.last.map(|(ts, _)| ts.to_string()).unwrap_or_default().into()),
        ("LAST_OUTCOME", env.last.and_then(|(_, o)| o).map(outcome_name).unwrap_or_default().into()),
        ("STATE_DIR", state.clone().into()),
    ];
    for (name, value) in &vars {
        // FOREMAN_* keeps scripts approved before the rename working; removed in 0.2.0.
        cmd.env(format!("SUPRAMAN_{name}"), value).env(format!("FOREMAN_{name}"), value);
    }
    cmd.envs(secrets.iter().map(|(k, v)| (k, v)));
    let ran = process::run_watched(&mut cmd, timeout, Kill::Tree, &mut stop);
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
fn script_command(folder: &Path, script: &str, args: &[String]) -> Command {
    let path = folder.join(script);
    match extension(script).as_str() {
        "ps1" => {
            let mut cmd = process::command(supraman_core::paths::script_powershell().path);
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
