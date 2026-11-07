//! The session stage of `sm schedule fire` against a fake `claude` (scripted rows, a clock that moves only when
//! fire sleeps), `FakeWorkman` with its OAuth, a temp home and a temp git repo.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use sm::fire::{self, Host, Report, Request, RunCtx};
use sm::run::{Agents, RunLog, SessionEnd, SessionHost};
use supraman_claude::{Kind, LaunchSpec, Session, SessionState, Status};
use supraman_core::schedule::{self, Action, ActionKind, Fired, LiveSession, Outcome, Profile, Prompt, RunEvent, Schedule, Span, Stage, Status as Sched, Trigger};
use supraman_core::secret::{MemoryStore, SecretStore};
use supraman_core::{Folder, Registry, Settings, Workspace};
use supraman_supervisor::{AppTokens, HttpMinter, Launcher, Minter, TokenProvider};
use supraman_workman::fake::{Answer, FakeWorkman};
use supraman_workman::{Auth, auth::http_browser};

mod common;

const T0: i64 = 1_793_595_600_000;
const UUID: &str = "aaaa0001-0000-4000-8000-000000000001";
const MINTED: &str = "wss_run_token_7f3c";

struct Fixture {
    _tmp: tempfile::TempDir,
    home: PathBuf,
    alpha: PathBuf,
    claude_dir: PathBuf,
    store: MemoryStore,
    fake: FakeWorkman,
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = supraman_core::process::git(dir, args).unwrap();
    assert!(out.success(), "git {args:?}: {}", out.stderr);
    out.stdout.trim().to_string()
}

fn fixture(signed_in: bool) -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let alpha = tmp.path().join("ws").join("Alpha");
    fs::create_dir_all(alpha.join("scripts")).unwrap();
    fs::create_dir_all(&home).unwrap();
    fs::write(alpha.join("scripts").join("gate.cmd"), "@echo two bugs found\r\n@exit /b 10\r\n").unwrap();
    git(&alpha, &["init", "-q", "-b", "main"]);
    git(&alpha, &["-c", "user.name=t", "-c", "user.email=t@t", "add", "-A"]);
    git(&alpha, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "init"]);
    let w = Workspace::new("demo", tmp.path().join("ws"), vec![Folder::new("Alpha", &alpha)]);
    Registry::at(&home).save(&w, false).unwrap();
    let fake = FakeWorkman::start(vec![
        (
            "POST /api/agent-sessions".into(),
            Answer::status(201, format!(r#"{{"sessionId":"91","name":"x","role":"orchestrator","token":"{MINTED}"}}"#)),
        ),
        ("PATCH /api/agent-sessions/91".into(), Answer::status(204, "")),
        ("DELETE /api/agent-sessions/91".into(), Answer::status(204, "")),
    ]);
    fake.enable_oauth();
    if signed_in {
        let mut settings = Settings::load(&home).unwrap_or_default();
        settings.workman.url = Some(fake.base().to_string());
        settings.save(&home).unwrap();
    }
    let claude_dir = tmp.path().join("claude");
    Fixture { _tmp: tmp, home, alpha, claude_dir, store: MemoryStore::default(), fake }
}

fn session_schedule(id: &str, kind: ActionKind, profile: Profile) -> Schedule {
    Schedule {
        id: id.into(),
        name: id.into(),
        description: String::new(),
        folder: "Alpha".into(),
        status: Sched::Pending,
        created_by: schedule::CreatedBy::Fm,
        created_at: 0,
        trigger: Trigger::Daily { at: "09:00".parse().unwrap() },
        action: Action {
            kind,
            prompt: Some(Prompt::Text("Fix the bugs.".into())),
            script: (kind == ActionKind::ScriptThenSession).then(|| "scripts/gate.cmd".to_string()),
            script_args: Vec::new(),
            script_timeout: Span(60),
            worktree: profile != Profile::Read,
            model: String::new(),
        },
        permissions: schedule::Permissions { profile, ..Default::default() },
        caps: Default::default(),
        policy: Default::default(),
        approval: None,
    }
}

impl Fixture {
    fn workspace(&self) -> Workspace {
        Registry::at(&self.home).load("demo").unwrap()
    }

    /// Adds `s` and stands in for René's Approve in the window (C100: sm itself never approves).
    fn add_approved(&self, s: Schedule) {
        let id = s.id.clone();
        schedule::edit(&self.home, &self.workspace(), |all| {
            all.push(s);
            Ok(())
        })
        .unwrap();
        schedule::approve(&self.home, &self.store, &self.workspace(), &id, 1).unwrap();
    }

    fn records(&self, run: &str) -> Vec<RunEvent> {
        schedule::read_runs(&self.home, "demo").unwrap().into_iter().filter(|r| r.run == run).map(|r| r.event).collect()
    }

    fn auth(&self) -> Arc<Auth> {
        let auth = Arc::new(Auth::with_store(&self.home, Arc::new(supraman_workman::MemoryStore::default())));
        if Settings::load(&self.home).unwrap().workman.url.is_some() {
            auth.sign_in(self.fake.base(), &http_browser()).unwrap();
        }
        auth
    }

    fn api(&self, method: &str) -> Vec<supraman_workman::fake::Seen> {
        self.fake.seen().into_iter().filter(|s| s.method == method && s.target.starts_with("/api/agent-sessions")).collect()
    }
}

/// How the fake session looks on each `claude agents` after the launch; the last one repeats.
#[derive(Clone, Copy, Debug)]
enum Look {
    Working,
    Idle,
    Blocked,
    Gone,
}

#[derive(Default)]
struct FakeClaude {
    calls: RefCell<Vec<String>>,
    launched: RefCell<Vec<LaunchSpec>>,
    /// The mcp config's text as it was at launch.
    config_at_launch: RefCell<Option<String>>,
    looks: RefCell<VecDeque<Look>>,
    now: RefCell<i64>,
}

impl Launcher for FakeClaude {
    fn launch(&self, spec: &LaunchSpec) -> supraman_claude::Result<String> {
        self.calls.borrow_mut().push(format!("launch {}", spec.name));
        *self.config_at_launch.borrow_mut() = spec.mcp_config.as_ref().map(|p| fs::read_to_string(p).unwrap());
        self.launched.borrow_mut().push(spec.clone());
        Ok(format!("backgrounded · aaaa0001 · {}", spec.name))
    }

    fn resume(&self, _: &str, _: &Path, _: Option<&str>) -> supraman_claude::Result<String> {
        unreachable!("a run never resumes")
    }

    fn stop(&self, id: &str) -> supraman_claude::Result<String> {
        self.calls.borrow_mut().push(format!("stop {id}"));
        Ok(format!("stopped {id}"))
    }
}

impl Agents for FakeClaude {
    fn list(&self) -> supraman_claude::Result<Vec<Session>> {
        let Some(spec) = self.launched.borrow().last().cloned() else { return Ok(Vec::new()) };
        let mut looks = self.looks.borrow_mut();
        let look = if looks.len() > 1 { looks.pop_front().unwrap() } else { looks.front().copied().unwrap_or(Look::Working) };
        let (state, status, pid, waiting_for) = match look {
            Look::Working => (SessionState::Working, Some(Status::Busy), Some(7), None),
            Look::Idle => (SessionState::Done, Some(Status::Idle), Some(7), None),
            Look::Blocked => (SessionState::Blocked, Some(Status::Waiting), Some(7), Some("permission prompt".to_string())),
            Look::Gone => (SessionState::Stopped, None, None, None),
        };
        Ok(vec![Session {
            id: "aaaa0001".into(),
            session_id: UUID.into(),
            name: spec.name.clone(),
            cwd: spec.cwd.clone(),
            kind: Kind::Background,
            state: Some(state),
            status,
            waiting_for,
            pid,
            started_at: Some(*self.now.borrow() - 1_000),
        }])
    }
}

type OnSleep = Box<dyn FnMut(i64, &Path)>;

/// fire's host and the session stage's, with a clock that moves only when they sleep.
struct TestHost<'a> {
    f: &'a Fixture,
    claude: FakeClaude,
    tokens: AppTokens,
    minter: HttpMinter,
    now: i64,
    on_sleep: Option<OnSleep>,
    dead_ended: Vec<LiveSession>,
}

impl<'a> TestHost<'a> {
    fn new(f: &'a Fixture, looks: &[Look]) -> Self {
        let claude = FakeClaude { looks: RefCell::new(looks.iter().copied().collect()), now: RefCell::new(T0), ..FakeClaude::default() };
        Self { f, claude, tokens: AppTokens { auth: f.auth() }, minter: HttpMinter::default(), now: T0, on_sleep: None, dead_ended: Vec::new() }
    }

    fn calls(&self) -> Vec<String> {
        self.claude.calls.borrow().clone()
    }

    fn spec(&self) -> LaunchSpec {
        let launched = self.claude.launched.borrow();
        assert_eq!(launched.len(), 1, "one session per run");
        launched[0].clone()
    }
}

impl Host for TestHost<'_> {
    fn store(&self) -> &dyn SecretStore {
        &self.f.store
    }

    fn busy(&mut self, _home: &Path) -> Result<bool, String> {
        Ok(false)
    }

    fn sleep(&mut self, d: Duration) {
        SessionHost::sleep(self, d);
    }

    fn now_ms(&mut self) -> i64 {
        self.now
    }

    fn session(&mut self, ctx: &RunCtx, gate: Option<&str>, log: &mut dyn RunLog) -> SessionEnd {
        sm::run::session_stage(ctx, gate, self, log)
    }

    fn end_dead_session(&mut self, home: &Path, session: &LiveSession) -> Result<(), String> {
        self.dead_ended.push(session.clone());
        fire::end_dead_session(self, home, session)
    }
}

impl SessionHost for TestHost<'_> {
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
        Some(self.f.claude_dir.clone())
    }

    fn plugin_dir(&self) -> Option<PathBuf> {
        None
    }

    fn now_ms(&mut self) -> i64 {
        self.now
    }

    fn sleep(&mut self, d: Duration) {
        self.now += d.as_millis() as i64;
        *self.claude.now.borrow_mut() = self.now;
        if let Some(hook) = &mut self.on_sleep {
            hook(self.now - T0, &self.f.home);
        }
    }
}

fn run_now(f: &Fixture, id: &str, host: &mut TestHost) -> Report {
    fire::fire(&Request { home: &f.home, workspace: "demo", id, trigger: Fired::RunNow, fingerprint: None }, host)
}

fn ended(events: &[RunEvent]) -> Vec<(Outcome, String, Option<schedule::Tokens>)> {
    events
        .iter()
        .filter_map(|e| match e {
            RunEvent::Ended { outcome, why, tokens, .. } => Some((*outcome, why.clone(), *tokens)),
            _ => None,
        })
        .collect()
}

fn session_stage(events: &[RunEvent]) -> Stage {
    events
        .iter()
        .find_map(|e| match e {
            RunEvent::Stage(s @ Stage::Session { .. }) => Some(s.clone()),
            _ => None,
        })
        .expect("a session stage line")
}

fn flag<'s>(spec: &'s LaunchSpec, name: &str) -> Vec<&'s str> {
    let args: Vec<&str> = spec.extra_args.iter().map(String::as_str).collect();
    let Some(at) = args.iter().position(|a| *a == name) else { return Vec::new() };
    args[at + 1..].iter().take_while(|a| !a.starts_with("--")).copied().collect()
}

fn fm_report(home: &Path, args: &[&str]) -> std::process::Output {
    supraman_core::process::command(env!("CARGO_BIN_EXE_sm"))
        .args(["schedule", "report"])
        .args(args)
        .env("SUPRAMAN_HOME", home)
        .env_remove("APPDATA")
        .env_remove("SUPRAMAN_WORKSPACE")
        .output()
        .unwrap()
}

/// The session reports done through the real `sm schedule report` after `after_ms`.
fn report_done_after(after_ms: i64) -> OnSleep {
    let mut sent = false;
    Box::new(move |elapsed, home| {
        if !sent && elapsed >= after_ms {
            sent = true;
            let live = schedule::live_dir(home, "demo").unwrap();
            let entry = fs::read_dir(&live).unwrap().flatten().find(|e| e.path().extension().is_some_and(|x| x == "json")).unwrap();
            let run = serde_json::from_slice::<schedule::Live>(&fs::read(entry.path()).unwrap()).unwrap().run;
            let out = fm_report(home, &[&run, "done", "fixed 2 bugs, branch pushed"]);
            assert!(out.status.success(), "{out:?}");
        }
    })
}

/// A session-only schedule the earlier release saved with `worktree = true` starts one `fm-<ws>-sched-*`
/// session in a fresh worktree, with the profile, the prompt and a minted token; DONE ends it, and the end stops
/// (never rm) and keeps its token.
#[test]
fn fire_session_only_starts_one_session_with_the_profile_the_prompt_and_a_mint() {
    let f = fixture(true);
    f.add_approved(session_schedule("fix", ActionKind::Session, Profile::Edit));
    let mut host = TestHost::new(&f, &[Look::Working]);
    host.on_sleep = Some(report_done_after(30_000));
    let report = run_now(&f, "fix", &mut host);
    assert_eq!((report.outcome, report.why.as_str(), report.exit), (Some(Outcome::Done), "fixed 2 bugs, branch pushed", 0), "{report:?}");

    let run = report.run.unwrap();
    let stamp = run.strip_prefix("fix-").unwrap().to_string();
    let spec = host.spec();
    assert_eq!(spec.name, format!("fm-demo-sched-fix-{stamp}"));
    let worktree = schedule::run_worktree(&f.alpha, "fix", &stamp);
    assert_eq!(spec.cwd, worktree);
    assert_eq!(git(&worktree, &["rev-parse", "--abbrev-ref", "HEAD"]), format!("fm-sched/fix/{stamp}"), "a fresh worktree on the run's branch");
    assert_eq!(spec.permission_mode, "acceptEdits");
    let preamble = schedule::preamble(&run, "sm", &worktree, Some(&format!("fm-sched/fix/{stamp}")));
    assert!(spec.prompt.starts_with(&preamble) && spec.prompt.ends_with("Fix the bugs."), "{}", spec.prompt);
    let allowed = flag(&spec, "--allowedTools");
    for rule in ["Bash(sm:*)", "Edit", "Bash(git commit:*)", &format!("Bash(git push -u origin fm-sched/fix/{stamp})")] {
        assert!(allowed.contains(&rule), "{rule} in {allowed:?}");
    }
    assert!(flag(&spec, "--disallowedTools").contains(&"Bash(claude rm:*)"));
    let settings: serde_json::Value = serde_json::from_str(flag(&spec, "--settings")[0]).unwrap();
    assert_eq!((&settings["env"]["SUPRAMAN_SCHEDULE"], &settings["env"]["SUPRAMAN_RUN"]), (&"fix".into(), &run.clone().into()));
    assert_eq!(settings["worktree"]["bgIsolation"], "none");
    let first = settings["env"]["PATH"].as_str().expect("the run's PATH").split(';').next().unwrap().to_string();
    assert_eq!(first, supraman_core::install::exe_dir().unwrap().display().to_string(), "the run's sm is the one beside its sm");

    let config = host.claude.config_at_launch.borrow().clone().expect("the run's mcp config");
    assert!(config.contains("headersHelper") && config.contains(" mcp-header --home ") && !config.contains("Bearer") && !config.contains("wss_"), "{config}");
    assert_eq!(supraman_supervisor::Keeper::open(&f.home).kept(&spec.name).as_deref(), Some(MINTED), "the run's token is the kept one");
    let posts = f.api("POST");
    let [post] = posts.as_slice() else { panic!("{posts:?}") };
    let body: serde_json::Value = serde_json::from_str(&post.body).unwrap();
    assert_eq!(body["role"], "orchestrator");
    assert_eq!((&body["launchedBy"], &body["origin"]), (&"supraman/demo/schedule/fix".into(), &"supraman/demo/schedule/fix".into()));
    assert_eq!((&body["externalRole"], &body["replace"], &body["name"]), (&"schedule".into(), &false.into(), &spec.name.clone().into()));
    assert_eq!(f.api("PATCH").len(), 1, "the claude session is bound");

    assert_eq!(host.calls(), [format!("launch {}", spec.name), "stop aaaa0001".to_string()], "stopped, never rm'd");
    assert_eq!(f.api("DELETE"), [], "the end revokes nothing: the stopped session can be resumed");
    assert!(spec.mcp_config.unwrap().exists(), "the config is kept");
    assert_eq!(supraman_supervisor::Keeper::open(&f.home).kept(&spec.name).as_deref(), Some(MINTED), "and the kept token");
    let (record, _) = supraman_supervisor::MintedSessions::find(&f.home, &spec.name).expect("the run's record stays");
    assert_eq!((record.session_id.as_str(), record.claude_session_id.as_deref()), ("91", Some(UUID)), "bound, so deleting the session revokes");

    let events = f.records(&run);
    let Stage::Session { name, id, session_id, worktree: wt, workstate_session, without_workstate } = session_stage(&events) else { unreachable!() };
    assert_eq!((name.as_str(), id.as_str(), session_id.as_str()), (spec.name.as_str(), "aaaa0001", UUID));
    assert_eq!((wt, workstate_session.as_deref(), without_workstate), (Some(worktree), Some("91"), None));
    assert_eq!(ended(&events).len(), 1);
    assert!(!schedule::live_dir(&f.home, "demo").unwrap().join("fix.json").exists());
    let all = fs::read_to_string(schedule::runs_path(&f.home, "demo").unwrap()).unwrap();
    assert!(!all.contains(MINTED), "never a token in a run record");
}

/// A script that exits 10 hands its stdout to one session's prompt.
#[test]
fn fire_session_after_a_script_gets_its_stdout_in_the_prompt() {
    let f = fixture(true);
    f.add_approved(session_schedule("gate", ActionKind::ScriptThenSession, Profile::Read));
    let mut host = TestHost::new(&f, &[Look::Working]);
    host.on_sleep = Some(report_done_after(15_000));
    let report = run_now(&f, "gate", &mut host);
    assert_eq!(report.outcome, Some(Outcome::Done), "{report:?}");
    let spec = host.spec();
    assert_eq!((spec.cwd.as_path(), spec.permission_mode.as_str()), (f.alpha.as_path(), "default"), "read runs in the main checkout");
    assert!(spec.prompt.contains("Fix the bugs.\n\n<gate-output>\ntwo bugs found"), "{}", spec.prompt);
    assert!(!flag(&spec, "--allowedTools").iter().any(|r| r.contains("push") || *r == "Edit"));
    assert_eq!(f.api("POST").len(), 1, "minted");
    let events = f.records(&report.run.unwrap());
    assert!(matches!(events.iter().find_map(|e| match e { RunEvent::Stage(Stage::Script { code, .. }) => Some(*code), _ => None }), Some(Some(10))));
}

/// max_runtime stops the session (stop, never rm) and keeps its token; the record says why.
#[test]
fn fire_session_max_runtime_stops_and_keeps_the_token() {
    let f = fixture(true);
    let mut s = session_schedule("slow", ActionKind::Session, Profile::Read);
    s.caps.max_runtime = Span(5 * 60);
    f.add_approved(s);
    let mut host = TestHost::new(&f, &[Look::Working]);
    let report = run_now(&f, "slow", &mut host);
    assert_eq!((report.outcome, report.exit), (Some(Outcome::CappedRuntime), 1), "{report:?}");
    assert!(report.why.contains("max_runtime 5m"), "{}", report.why);
    assert_eq!(host.calls().last().unwrap(), "stop aaaa0001");
    assert!(host.calls().iter().all(|c| !c.starts_with("rm")));
    assert_eq!(f.api("DELETE").len(), 0, "the end keeps the token");
    let ends = ended(&f.records(&report.run.unwrap()));
    assert_eq!(ends.len(), 1);
    assert_eq!(ends[0].0, Outcome::CappedRuntime);
    assert!(host.now - T0 >= 5 * 60 * 1000 && host.now - T0 < 6 * 60 * 1000, "ended within a poll of max_runtime");
}

/// The transcript's input + output + cache-creation tokens over max_tokens stop the session; its token stays.
#[test]
fn fire_session_token_cap_stops_with_the_totals_and_keeps_the_token() {
    let f = fixture(true);
    let mut s = session_schedule("big", ActionKind::Session, Profile::Read);
    s.caps.max_tokens = 1_000;
    f.add_approved(s);
    let project = f.claude_dir.join("projects").join("x");
    fs::create_dir_all(&project).unwrap();
    let line = |id: &str| {
        format!(
            r#"{{"type":"assistant","message":{{"id":"{id}","usage":{{"input_tokens":300,"output_tokens":100,"cache_creation_input_tokens":200,"cache_read_input_tokens":90000}}}}}}"#
        ) + "\n"
    };
    fs::write(project.join(format!("{UUID}.jsonl")), line("m1")).unwrap();
    let mut host = TestHost::new(&f, &[Look::Working]);
    let transcript = project.join(format!("{UUID}.jsonl"));
    host.on_sleep = Some(Box::new(move |elapsed, _| {
        if elapsed == 15_000 {
            let mut text = fs::read_to_string(&transcript).unwrap();
            text.push_str(&line("m2"));
            fs::write(&transcript, text).unwrap();
        }
    }));
    let report = run_now(&f, "big", &mut host);
    assert_eq!(report.outcome, Some(Outcome::CappedTokens), "{report:?}");
    assert!(report.why.contains("1200 tokens") && report.why.contains("max_tokens 1000"), "{}", report.why);
    assert_eq!(host.calls().last().unwrap(), "stop aaaa0001");
    assert_eq!(f.api("DELETE").len(), 0, "the end keeps the token");
    let ends = ended(&f.records(&report.run.unwrap()));
    assert_eq!(ends[0].2, Some(schedule::Tokens { input: 600, output: 200, cache_creation: 400, cache_read: 180_000 }), "cache reads recorded, not capped");
}

/// Idle after work, with no report, ends the run as done.
#[test]
fn fire_session_idle_after_work_ends_the_run_done() {
    let f = fixture(true);
    f.add_approved(session_schedule("tidy", ActionKind::Session, Profile::Read));
    let mut host = TestHost::new(&f, &[Look::Idle, Look::Working, Look::Working, Look::Idle]);
    let report = run_now(&f, "tidy", &mut host);
    assert_eq!((report.outcome, report.why.as_str()), (Some(Outcome::Done), "idle after its work, without a report"), "{report:?}");
    assert_eq!(host.calls().last().unwrap(), "stop aaaa0001");
    assert_eq!(f.api("DELETE").len(), 0, "the end keeps the token");
}

/// A blocked run is recorded `needs_you` at once, stays parked until max_runtime, then is stopped.
#[test]
fn fire_session_blocked_is_needs_you_at_once_and_parked_until_max_runtime() {
    let f = fixture(true);
    let mut s = session_schedule("ask", ActionKind::Session, Profile::Read);
    s.caps.max_runtime = Span(10 * 60);
    f.add_approved(s);
    let mut host = TestHost::new(&f, &[Look::Working, Look::Blocked]);
    let seen_early: Arc<std::sync::Mutex<Option<Vec<RunEvent>>>> = Arc::default();
    let early = seen_early.clone();
    host.on_sleep = Some(Box::new(move |elapsed, home| {
        if elapsed == 60_000 {
            let records = schedule::read_runs(home, "demo").unwrap().into_iter().map(|r| r.event).collect();
            *early.lock().unwrap() = Some(records);
        }
    }));
    let report = run_now(&f, "ask", &mut host);
    assert_eq!((report.outcome, report.exit), (Some(Outcome::NeedsYou), 1), "{report:?}");
    assert!(report.why.contains("permission prompt"), "{}", report.why);
    let early = seen_early.lock().unwrap().clone().unwrap();
    assert_eq!(ended(&early).first().map(|e| e.0), Some(Outcome::NeedsYou), "recorded at once, for the Inbox");
    assert_eq!(host.calls().iter().filter(|c| c.starts_with("stop")).count(), 1, "parked, then stopped once");
    assert!(host.now - T0 >= 10 * 60 * 1000, "parked until max_runtime");
    assert_eq!(ended(&f.records(&report.run.unwrap())).len(), 1, "one ended line");
    assert_eq!(f.api("DELETE").len(), 0, "the end keeps the token");
}

/// The session gone without a report is `failed`; a report names a run in flight.
#[test]
fn fire_session_a_vanished_session_fails_and_a_report_needs_a_run_in_flight() {
    let f = fixture(true);
    f.add_approved(session_schedule("gone", ActionKind::Session, Profile::Read));
    let mut host = TestHost::new(&f, &[Look::Working, Look::Gone]);
    let report = run_now(&f, "gone", &mut host);
    assert_eq!((report.outcome, report.why.as_str()), (Some(Outcome::Failed), "the session ended without a report"), "{report:?}");

    let out = fm_report(&f.home, &["gone-20990101-000000", "done", "x"]);
    assert!(!out.status.success() && String::from_utf8_lossy(&out.stderr).contains("no run gone-20990101-000000 is in flight"), "{out:?}");
}

/// Not signed in: the run launches with its helped config and no token, and its record says so.
#[test]
fn fire_session_without_a_sign_in_launches_with_its_helped_config_and_no_token() {
    let f = fixture(false);
    f.add_approved(session_schedule("plain", ActionKind::Session, Profile::Read));
    let mut host = TestHost::new(&f, &[Look::Working]);
    host.on_sleep = Some(report_done_after(15_000));
    let report = run_now(&f, "plain", &mut host);
    assert_eq!(report.outcome, Some(Outcome::Done), "{report:?}");
    let spec = host.spec();
    let config = spec.mcp_config.clone().expect("its helped config");
    assert_eq!(config, supraman_claude::mcp_config_path(&f.home.join("mcp"), &spec.name).unwrap());
    assert!(spec.strict_mcp_config, "no user-scope server either");
    let text = fs::read_to_string(&config).unwrap();
    let workman = &serde_json::from_str::<serde_json::Value>(&text).unwrap()["mcpServers"]["workman"];
    assert_eq!(workman["url"].as_str(), Some(Settings::load(&f.home).unwrap_or_default().workman.mcp_url().as_str()));
    assert!(workman["headersHelper"].as_str().is_some_and(|h| h.contains(" mcp-header ") && h.ends_with(&spec.name)), "{text}");
    assert!(!text.contains("Bearer") && !text.contains("wss_"), "no token in the config: {text}");
    assert_eq!(supraman_core::secret::default_store(&f.home).get(&supraman_claude::mcp_token_key(&spec.name)).unwrap(), None, "no token kept");
    assert!(f.fake.seen().iter().all(|s| !s.target.starts_with("/api/")));
    let Stage::Session { workstate_session, without_workstate, .. } = session_stage(&f.records(&report.run.unwrap())) else { unreachable!() };
    assert_eq!(workstate_session, None);
    assert!(without_workstate.unwrap().contains("not signed in"));
}

/// A folder that is not a git repo cannot take a worktree: the run fails before anything launches.
#[test]
fn fire_session_needs_a_repo_for_a_worktree_and_launches_nothing() {
    let f = fixture(true);
    f.add_approved(session_schedule("wt", ActionKind::Session, Profile::Edit));
    fs::remove_dir_all(f.alpha.join(".git")).unwrap();
    let mut host = TestHost::new(&f, &[Look::Working]);
    let report = run_now(&f, "wt", &mut host);
    assert_eq!(report.outcome, Some(Outcome::Failed), "{report:?}");
    assert!(report.why.contains("is not a git repo"), "{}", report.why);
    assert!(host.calls().is_empty() && f.api("POST").is_empty());
}

/// A run whose sm died with its session live: the next fire stops that session, keeps its token, and records it interrupted.
#[test]
fn fire_session_of_a_dead_fm_is_stopped_by_the_next_fire_and_keeps_its_token() {
    let f = fixture(true);
    f.add_approved(session_schedule("crash", ActionKind::Session, Profile::Read));
    let live = schedule::live_dir(&f.home, "demo").unwrap();
    fs::create_dir_all(&live).unwrap();
    let dead = schedule::Live {
        run: "crash-20261101-090000".into(),
        fm_pid: common::exited_pid(),
        fm_started: None,
        started_at: T0 - 60_000,
        stage: "session".into(),
        session: Some(LiveSession {
            name: "fm-demo-sched-crash-20261101-090000".into(),
            id: "dead0001".into(),
            workstate_session: Some("91".into()),
            origin: Some(f.fake.base().into()),
        }),
    };
    fs::write(live.join("crash.json"), serde_json::to_vec(&dead).unwrap()).unwrap();
    let mut host = TestHost::new(&f, &[Look::Working]);
    host.on_sleep = Some(report_done_after(15_000));
    run_now(&f, "crash", &mut host);
    assert_eq!(host.dead_ended.len(), 1);
    assert_eq!(host.calls()[0], "stop dead0001");
    assert!(f.api("DELETE").is_empty(), "neither the dead run's token nor the new run's is revoked");
    assert_eq!(ended(&f.records("crash-20261101-090000"))[0].0, Outcome::Interrupted);
}

/// An `edit` run without a worktree starts in the folder's checkout, needs no git repo, makes no worktree and
/// may not push.
#[test]
fn fire_session_of_an_edit_schedule_works_in_the_folders_checkout() {
    let f = fixture(true);
    let mut s = session_schedule("ops", ActionKind::Session, Profile::Edit);
    s.action.worktree = false;
    f.add_approved(s);
    fs::remove_dir_all(f.alpha.join(".git")).unwrap();
    let mut host = TestHost::new(&f, &[Look::Working]);
    host.on_sleep = Some(report_done_after(30_000));
    let report = run_now(&f, "ops", &mut host);
    assert_eq!(report.outcome, Some(Outcome::Done), "{report:?}");
    let spec = host.spec();
    assert_eq!(spec.cwd, f.alpha);
    assert!(spec.prompt.contains("You work in the folder's checkout"), "{}", spec.prompt);
    assert_eq!(spec.permission_mode, "acceptEdits");
    let allowed = flag(&spec, "--allowedTools");
    assert!(allowed.contains(&"Bash(git commit:*)") && !allowed.iter().any(|r| r.contains("git push")), "{allowed:?}");
    assert!(!f.alpha.join(".claude").join("worktrees").exists(), "no run worktree");
    let Stage::Session { worktree, .. } = session_stage(&f.records(&report.run.unwrap())) else { unreachable!() };
    assert_eq!(worktree, None);
}
