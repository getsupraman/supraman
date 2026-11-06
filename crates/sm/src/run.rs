//! The session stage of `sm schedule fire` (docs/research/scheduler.md, "The life of `sm schedule fire`", steps
//! 8 to 10): the run's worktree, its minted Workman token, `claude --bg`, the watch until the run ends, and
//! the stop (never rm). The token, its config and its record stay: a deleted run session revokes it.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use supraman_claude::{Claude, Health, LaunchSpec, Session};
use supraman_core::EventReader;
use supraman_core::schedule::{self, LiveSession, Outcome, Reported, RunEvent, Stage, Tokens};
use supraman_supervisor::{Bearer, Keeper, Launcher, MintRequest, MintedSession, MintedSessions, Minter, TokenProvider, api_base};

use crate::fire::RunCtx;

/// Between two looks at the run's session.
pub const POLL: Duration = Duration::from_secs(15);
/// An idle session after work ends the run as `done` once it has been idle this long.
pub const IDLE_AFTER: Duration = Duration::from_secs(120);
/// How long the run's row may take to show after `--bg`.
pub const FIND_WITHIN: Duration = Duration::from_secs(60);
const FIND_POLL: Duration = Duration::from_secs(1);
/// Polls in a row without a live row, and without a report, that end the run as `failed`.
const GONE_POLLS: u32 = 2;
/// The script's stdout in the prompt is cut here.
pub const GATE_CAP: usize = 32 * 1024;
const WORKMAN_EXTERNAL_ROLE: &str = "schedule";

/// What `claude` the session stage needs: the supervisor's [`Launcher`] and the session list.
pub trait Agents: Launcher {
    fn list(&self) -> supraman_claude::Result<Vec<Session>>;
}

impl Agents for Claude {
    fn list(&self) -> supraman_claude::Result<Vec<Session>> {
        self.list_sessions()
    }
}

/// What the session stage takes from the machine, so tests can stand in for it.
pub trait SessionHost {
    fn claude(&self) -> &dyn Agents;
    fn tokens(&self) -> &dyn TokenProvider;
    fn minter(&self) -> &dyn Minter;
    /// claude's config dir, where the run's transcript is.
    fn claude_dir(&self) -> Option<PathBuf>;
    fn plugin_dir(&self) -> Option<PathBuf>;
    fn now_ms(&mut self) -> i64;
    fn sleep(&mut self, d: Duration);
}

/// Where the session stage writes as it goes: run records, and the live file's session.
pub trait RunLog {
    fn event(&mut self, ts: i64, event: RunEvent);
    fn session(&mut self, session: &LiveSession);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionEnd {
    pub outcome: Outcome,
    pub why: String,
    pub tokens: Option<Tokens>,
    /// The `ended` line is written already: a `needs_you` run recorded when it parked.
    pub recorded: bool,
}

impl SessionEnd {
    fn failed(why: impl Into<String>) -> Self {
        Self { outcome: Outcome::Failed, why: why.into(), tokens: None, recorded: false }
    }
}

/// `schedules/live/<ws>/<id>.report`, where `sm schedule report` leaves the run's outcome.
pub fn report_path(live_dir: &Path, id: &str) -> PathBuf {
    live_dir.join(format!("{id}.report"))
}

/// Steps 8 to 10 for the run in `ctx`; `gate` is the script's stdout when it exited 10.
pub fn session_stage(ctx: &RunCtx, gate: Option<&str>, host: &mut dyn SessionHost, log: &mut dyn RunLog) -> SessionEnd {
    let (w, s) = (ctx.workspace, ctx.schedule);
    let Some(folder) = w.folder(&s.folder) else {
        return SessionEnd::failed(format!("workspace {} has no folder {}", w.name, s.folder));
    };
    let live_dir = match schedule::live_dir(ctx.home, &w.name) {
        Ok(dir) => dir,
        Err(e) => return SessionEnd::failed(e.to_string()),
    };
    let report = report_path(&live_dir, &s.id);
    let _ = fs::remove_file(&report);

    if let Err(why) = git_accepts(&folder.path, s.action.worktree) {
        return SessionEnd::failed(why);
    }
    let worktree = if s.action.worktree {
        let path = schedule::run_worktree(&folder.path, &s.id, ctx.stamp);
        if let Err(why) = add_worktree(&folder.path, &path, &schedule::run_branch(&s.id, ctx.stamp)) {
            return SessionEnd::failed(why);
        }
        Some(path)
    } else {
        None
    };

    let mut spec = match crate::schedule::run_spec(ctx.home, w, s, ctx.stamp, host.plugin_dir().as_deref()) {
        Ok(spec) => spec,
        Err(e) => return SessionEnd::failed(crate::failure_text(e)),
    };
    if let Some(stdout) = gate {
        spec.prompt.push_str(&format!("\n\n<gate-output>\n{}\n</gate-output>", capped(stdout)));
    }
    let keeper = Keeper::open(ctx.home);
    let mut mint = match mint(ctx, &spec, host) {
        Ok(m) => m,
        Err(why) => return SessionEnd::failed(why),
    };
    spec.mcp_config = None;
    let mut without_workstate = None;
    match &mint {
        Some(m) => match keeper.launch(spec.clone(), &m.url, &m.token, m.expires_at.as_deref()) {
            Ok(with) => spec = with,
            Err(why) => {
                end_mint(host, &keeper, ctx.home, &spec.name, Some(m));
                return SessionEnd::failed(why);
            }
        },
        None => without_workstate = Some("Supraman is not signed in to Workman: the run has no Workman".to_string()),
    }

    if let Err(e) = host.claude().launch(&spec) {
        end_mint(host, &keeper, ctx.home, &spec.name, mint.as_ref());
        return SessionEnd::failed(format!("cannot launch {}: {e}", spec.name));
    }
    let row = match find_row(host, &spec.name) {
        Some(row) => row,
        None => {
            end_mint(host, &keeper, ctx.home, &spec.name, mint.as_ref());
            return SessionEnd::failed(format!("{} launched, but no session of that name showed within {}s", spec.name, FIND_WITHIN.as_secs()));
        }
    };
    if let Some(m) = &mut mint
        && !row.session_id.is_empty()
        && let Err(e) = m.bind(&spec.name, &row.session_id)
    {
        crate::fire::log(ctx.home, &format!("{}/{} {}: {e}", w.name, s.id, ctx.run));
    }
    let workstate_session = mint.as_ref().map(|m| m.session_id.clone());
    log.session(&LiveSession {
        name: spec.name.clone(),
        id: row.id.clone(),
        workstate_session: workstate_session.clone(),
        origin: mint.as_ref().map(|m| api_base(&m.url)),
    });
    let ts = host.now_ms();
    log.event(
        ts,
        RunEvent::Stage(Stage::Session {
            name: spec.name.clone(),
            id: row.id.clone(),
            session_id: row.session_id.clone(),
            worktree,
            workstate_session,
            without_workstate,
        }),
    );
    if let Some(m) = &mint
        && !row.session_id.is_empty()
        && let Ok(Some(t)) = host.tokens().token(crate::schedule::RUN_ROLE)
        && api_base(&t.url) == api_base(&m.url)
        && let Err(e) = host.minter().bind(&api_base(&t.url), &t.token, &m.session_id, &row.session_id)
    {
        crate::fire::log(ctx.home, &format!("{}/{} {}: cannot tell Workman the claude session: {e}", w.name, s.id, ctx.run));
    }

    let mut end = watch(ctx, &spec, &row, &report, host, log);

    if let Err(e) = host.claude().stop(&row.id) {
        crate::fire::log(ctx.home, &format!("{}/{} {}: claude stop {}: {e}", w.name, s.id, ctx.run, row.id));
    }
    let _ = fs::remove_file(&report);
    if end.outcome == Outcome::Done && end.why.is_empty() {
        end.why = "the session finished".into();
    }
    end
}

/// git's refusal of a folder owned by another user, with the one command that fixes it (scheduler.md, probe (c)).
fn git_accepts(folder: &Path, needs_repo: bool) -> Result<(), String> {
    let out = supraman_core::process::git(folder, &["rev-parse", "--git-dir"]).map_err(|e| format!("cannot run git in {}: {e}", folder.display()))?;
    if out.success() {
        return Ok(());
    }
    if out.stderr.contains("dubious ownership") {
        let shown = folder.display().to_string().replace('\\', "/");
        return Err(format!("git refuses {shown} (dubious ownership); fix once: git config --global --add safe.directory {shown}"));
    }
    if needs_repo {
        return Err(format!("{} is not a git repo, and the profile runs in a worktree: {}", folder.display(), first_line(&out.stderr)));
    }
    Ok(())
}

fn add_worktree(folder: &Path, path: &Path, branch: &str) -> Result<(), String> {
    let at = path.to_string_lossy();
    let out = supraman_core::process::git(folder, &["worktree", "add", "-b", branch, &at, "HEAD"])
        .map_err(|e| format!("cannot run git worktree add in {}: {e}", folder.display()))?;
    if !out.success() {
        return Err(format!("git worktree add {at} failed: {}", first_line(&out.stderr)));
    }
    Ok(())
}

fn first_line(text: &str) -> &str {
    text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("")
}

fn capped(stdout: &str) -> String {
    if stdout.len() <= GATE_CAP {
        return stdout.to_string();
    }
    let mut at = GATE_CAP;
    while !stdout.is_char_boundary(at) {
        at -= 1;
    }
    format!("{}\n[… cut at 32 KiB]", &stdout[..at])
}

struct Mint {
    url: String,
    token: String,
    expires_at: Option<String>,
    session_id: String,
    /// The run's record, at [`MintedSessions::run_path`]: only this sm writes it.
    record: MintedSessions,
}

impl Mint {
    /// The run's claude session on its record, so deleting that session revokes the token.
    fn bind(&mut self, name: &str, claude_session_id: &str) -> Result<(), String> {
        if let Some(m) = self.record.get_mut(name) {
            m.claude_session_id = Some(claude_session_id.to_string());
        }
        self.record.save().map_err(|e| format!("cannot record the claude session of {name}: {e}"))
    }
}

/// The run's own token, minted with the signed-in bearer as the supervisor mints and recorded at
/// [`MintedSessions::run_path`]; `None` when not signed in.
fn mint(ctx: &RunCtx, spec: &LaunchSpec, host: &mut dyn SessionHost) -> Result<Option<Mint>, String> {
    let Some(t) = host.tokens().token(crate::schedule::RUN_ROLE).map_err(|e| format!("cannot mint a Workman token for {}: {e}", spec.name))? else {
        return Ok(None);
    };
    let origin = format!("foreman/{}/schedule/{}", ctx.workspace.name, ctx.schedule.id);
    let request = MintRequest {
        name: spec.name.clone(),
        role: t.role.clone(),
        launched_by: origin.clone(),
        origin,
        cwd: spec.cwd.display().to_string(),
        external_role: WORKMAN_EXTERNAL_ROLE.into(),
        replace: false,
        repo: supraman_supervisor::git_repo(&spec.cwd),
        claude_session_id: None,
    };
    let minted = host.minter().mint(&api_base(&t.url), &t.token, &request).map_err(|e| format!("cannot mint a Workman token for {}: {e}", spec.name))?;
    let mut record = MintedSessions::empty(Some(MintedSessions::run_path(ctx.home, &spec.name)));
    let session = MintedSession {
        session_id: minted.session_id.clone(),
        role: crate::schedule::RUN_ROLE.into(),
        claude_session_id: None,
        last_active: None,
        origin: Some(api_base(&t.url)),
    };
    record.insert_minted(&spec.name, session, &request);
    record.save().map_err(|e| format!("cannot record the Workman session of {}: {e}", spec.name))?;
    Ok(Some(Mint { url: t.url, token: minted.token, expires_at: minted.expires_at, session_id: minted.session_id, record }))
}

/// A run that never got going: its session cannot come back, so its token is revoked ([`supraman_supervisor::revoke`])
/// and its record goes; `Some` says what failed.
fn end_mint(host: &mut dyn SessionHost, keeper: &Keeper, home: &Path, name: &str, mint: Option<&Mint>) -> Option<String> {
    let mut failed = Vec::new();
    if let Some(session) = mint.and_then(|m| m.record.get(name)) {
        let bearer = Bearer::of(host.tokens().token(crate::schedule::RUN_ROLE).map(|t| t.map(|t| (t.url, t.token))));
        failed.extend(supraman_supervisor::revoke(keeper, host.minter(), &bearer, name, session).notices);
        if let Err(e) = fs::remove_file(MintedSessions::run_path(home, name))
            && e.kind() != std::io::ErrorKind::NotFound
        {
            failed.push(format!("cannot delete the Workman record of {name}: {e}"));
        }
    } else if let Err(e) = supraman_supervisor::remove_mcp_config(keeper.mcp_dir(), name) {
        failed.push(e);
    }
    (!failed.is_empty()).then(|| failed.join("; "))
}

fn find_row(host: &mut dyn SessionHost, name: &str) -> Option<Session> {
    let deadline = host.now_ms() + FIND_WITHIN.as_millis() as i64;
    loop {
        if let Ok(rows) = host.claude().list()
            && let Some(row) = rows.into_iter().filter(|r| r.name == name).max_by_key(|r| r.started_at)
        {
            return Some(row);
        }
        if host.now_ms() >= deadline {
            return None;
        }
        host.sleep(FIND_POLL);
    }
}

/// Step 9: looks every [`POLL`] until the run ends.
fn watch(ctx: &RunCtx, spec: &LaunchSpec, row: &Session, report: &Path, host: &mut dyn SessionHost, log: &mut dyn RunLog) -> SessionEnd {
    let s = ctx.schedule;
    let started = host.now_ms();
    let max_runtime = s.caps.max_runtime.duration().as_millis() as i64;
    let restart = report.with_extension("restart");
    let mut usage = supraman_claude::Usage::default();
    let mut transcript: Option<PathBuf> = None;
    let mut events = EventReader::new(ctx.home);
    let (mut worked, mut idle_since, mut gone) = (false, None::<i64>, 0u32);
    let mut parked: Option<String> = None;
    loop {
        let now = host.now_ms();
        if transcript.is_none() {
            transcript = host.claude_dir().and_then(|dir| supraman_claude::find_transcript(&dir, &row.session_id, Some(&spec.cwd)));
        }
        if let Some(path) = &transcript {
            let _ = usage.read(path);
        }
        let tokens = tokens_of(&usage);
        let end = |outcome, why: String| SessionEnd { outcome, why, tokens: Some(tokens), recorded: false };

        if let Some(r) = reported(report, ctx.run) {
            match r.outcome {
                Outcome::Done => return end(Outcome::Done, r.why),
                _ if parked.is_none() => {
                    parked = Some(r.why.clone());
                    log.event(now, ended(Outcome::NeedsYou, &r.why, tokens, now - started));
                }
                _ => {}
            }
            let _ = fs::remove_file(report);
        }
        if parked.is_none() && restart.exists() {
            return end(Outcome::Interrupted, "restarted".into());
        }
        if tokens.capped() > s.caps.max_tokens {
            let why = format!("{} tokens used, over max_tokens {}", tokens.capped(), s.caps.max_tokens);
            return match parked.take() {
                Some(why_parked) => SessionEnd { outcome: Outcome::NeedsYou, why: why_parked, tokens: Some(tokens), recorded: true },
                None => end(Outcome::CappedTokens, why),
            };
        }
        if now - started >= max_runtime {
            return match parked.take() {
                Some(why_parked) => SessionEnd { outcome: Outcome::NeedsYou, why: why_parked, tokens: Some(tokens), recorded: true },
                None => end(Outcome::CappedRuntime, format!("still running after max_runtime {}", s.caps.max_runtime)),
            };
        }
        if events.poll_session(&row.session_id).iter().any(|e| e.event == "Stop") {
            worked = true;
        }

        let rows = host.claude().list().unwrap_or_default();
        let mine = rows.iter().find(|r| r.id == row.id).filter(|r| supraman_supervisor::is_live(r, now));
        match mine.map(|r| (r.health(now), r)) {
            None => {
                gone += 1;
                if gone >= GONE_POLLS {
                    return match parked.take() {
                        Some(why_parked) => SessionEnd { outcome: Outcome::NeedsYou, why: why_parked, tokens: Some(tokens), recorded: true },
                        None => end(Outcome::Failed, "the session ended without a report".into()),
                    };
                }
            }
            Some((health, r)) => {
                gone = 0;
                match health {
                    Health::Blocked { waiting_for } if parked.is_none() => {
                        let why = match waiting_for {
                            Some(w) => format!("the session waits for {w}; attach to {} to answer", r.name),
                            None => format!("the session is blocked; attach to {} to answer", r.name),
                        };
                        log.event(now, ended(Outcome::NeedsYou, &why, tokens, now - started));
                        parked = Some(why);
                    }
                    Health::Running => {
                        worked = true;
                        idle_since = None;
                    }
                    Health::Idle if worked && parked.is_none() => {
                        let since = *idle_since.get_or_insert(now);
                        if now - since >= IDLE_AFTER.as_millis() as i64 {
                            return end(Outcome::Done, "idle after its work, without a report".into());
                        }
                    }
                    _ => {}
                }
            }
        }
        host.sleep(POLL);
    }
}

fn tokens_of(usage: &supraman_claude::Usage) -> Tokens {
    let t = usage.total();
    Tokens { input: t.input, output: t.output, cache_creation: t.cache_creation, cache_read: t.cache_read }
}

fn ended(outcome: Outcome, why: &str, tokens: Tokens, ms: i64) -> RunEvent {
    RunEvent::Ended { outcome, why: why.to_string(), tokens: Some(tokens), ms: u64::try_from(ms).unwrap_or(0) }
}

fn reported(path: &Path, run: &str) -> Option<Reported> {
    let r: Reported = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    (r.run == run).then_some(r)
}
