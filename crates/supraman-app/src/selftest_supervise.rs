//! `--selftest-supervise`: the M2 gate; every session is `fm-selftest-<hex>-…`, and cleanup acts only on those (`selftest::cleanup`).

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use anyhow::{Context as _, anyhow, bail};
use supraman_claude::{Claude, Health, LaunchSpec, Removal, Route, Sender, Session, SessionState, check_supported, list_sessions, now_ms, peer, send_from_human};
use supraman_core::{
    DEFAULT_MAX_WORKING, EventReader, RelayDrop, Folder, FolderKind, SENT_DIR, Workspace, Registry, Settings, find_plugin_dir, inbox_dir, outbox_dir, read_answered, read_outbox,
    relay_ready, role_tag,
};
use supraman_supervisor::{
    DesiredState, NeedsKind, WORKSPACE_ROLE, Remedy, WantedSession, is_live, mid_turn, relay_session, repo_session, repo_session_name, relay_session_name, workspace_session_name,
    render_role, role_prompt,
};
use gpui::{AsyncWindowContext, Entity, KeyDownEvent, Keystroke, Modifiers, PlatformInput, px, size};

use crate::selftest::{
    Scratch, click, double_click, hex6, install_guards, lock, raw, remove_dir_retrying, scrolled_into_view, sleep,
    wait_for_tab_terminal, cargo_root,
};
use crate::view::FleetView;
use crate::{Mode, supervise};

const PROMPT_TARGET: &str = "This is an automated Supraman selftest. Messages will arrive that contain nonces like \
FOREMAN-DIRECT-1a2b3c01 or FOREMAN-RELAY-1a2b3c. Each time you get one or more of them, run exactly one Bash command: \
sm tell \"<every nonce you got since your last reply, space-separated>\" and then wait for more. Never use SendMessage. \
Do nothing else.";
/// One a second: half the receiver's burst, none paced.
const DIRECT_SENDS: usize = 30;
const REPO: &str = "Repo";
/// Long enough for the relay to launch, arm and send.
const RELAY_IDLE_SECS: u64 = 60;
const PROMPT_PARK: &str = "This is an automated Supraman selftest. Run exactly this one Bash command and nothing \
else: `echo parked > parked.txt`. Then stop.";

struct Testbed {
    hex: String,
    home: PathBuf,
    /// testbed, untrusted, budget 1, budget 2: (workspace name, root)
    a: (String, PathBuf),
    u: (String, PathBuf),
    c1: (String, PathBuf),
    c2: (String, PathBuf),
}

static TESTBED: OnceLock<Testbed> = OnceLock::new();

fn testbed() -> anyhow::Result<&'static Testbed> {
    TESTBED.get().ok_or_else(|| anyhow!("the testbed was not prepared"))
}

impl Testbed {
    fn park(&self) -> String {
        format!("fm-selftest-{}-park", self.hex)
    }

    fn target(&self) -> String {
        format!("fm-selftest-{}-target", self.hex)
    }
}

pub fn prepare() -> anyhow::Result<()> {
    let target = cargo_root().join("target");
    let home = crate::selftest::home();
    remove_dir_retrying(&home).with_context(|| format!("cannot clear {}", home.display()))?;
    let hex = hex6();
    let workspace = |suffix: &str| {
        let name = format!("selftest-{hex}{suffix}");
        let root = target.join(&name);
        (name, root)
    };
    let tb = Testbed {
        a: workspace(""),
        u: workspace("-u"),
        c1: workspace("-c1"),
        c2: workspace("-c2"),
        home: home.clone(),
        hex: hex.clone(),
    };
    let roots = [&tb.a.1, &tb.u.1, &tb.c1.1, &tb.c2.1];
    *lock() = Some(Scratch {
        dirs: std::iter::once(home.clone()).chain(roots.iter().map(|r| (*r).clone())).collect(),
        session_name: None,
        name_prefix: Some(format!("fm-selftest-{hex}")),
        kill: Vec::new(),
        transcripts: Vec::new(),
    });
    install_guards(Mode::SelftestSupervise);
    // Still single-threaded apart from the watchdog, which never reads the environment.
    unsafe { std::env::set_var(supraman_core::HOME_VAR, &home) };

    let version = check_supported()?;
    for root in roots {
        std::fs::create_dir_all(root).with_context(|| format!("create {}", root.display()))?;
    }
    let git = supraman_core::process::git(&tb.u.1, &["init", "-q"]).context("git init")?;
    if !git.success() {
        bail!("git init {}: {}", tb.u.1.display(), git.stderr);
    }
    let registry = Registry::at(&home);
    let repo = tb.a.1.join(REPO);
    std::fs::create_dir_all(&repo).with_context(|| format!("create {}", repo.display()))?;
    for (name, root) in [&tb.a, &tb.u, &tb.c1, &tb.c2] {
        let repos = if name == &tb.a.0 {
            vec![Folder::new(REPO, repo.clone())]
        } else {
            Vec::new()
        };
        registry.save(&Workspace::new(name.as_str(), root.as_path(), repos), true)?;
    }
    let plugin = plugin_dir()?;
    let mut desired = DesiredState::workspace_controller(&tb.a.0, &tb.a.1, role_prompt(&plugin, WORKSPACE_ROLE, &tb.a.0)?);
    desired.sessions.push(WantedSession {
        name: tb.park(),
        role: "probe".into(),
        cwd: tb.a.1.clone(),
        repo: None,
        prompt: PROMPT_PARK.into(),
        permission_mode: "default".into(),
        add_dirs: Vec::new(),
        resume_prompt: None,
        fork_from: None,
    });
    // as a legacy desired state has it: the relay first
    let relay_dir = supraman_core::ensure_relay_dirs(&home, &tb.a.0)?;
    desired.sessions.insert(0, relay_session(&tb.a.0, &tb.a.1, relay_dir, "relay"));
    desired.save(&home)?;
    // no sign-in, and a closed loopback port as the server: the status check never reaches a real WorkState
    let closed = std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
    let mut settings = Settings::default();
    settings.sessions.relay_idle_secs = Some(RELAY_IDLE_SECS);
    settings.workstate.url = Some(format!("http://127.0.0.1:{closed}"));
    settings.save(&home)?;
    println!(
        "step a: claude {} | SUPRAMAN_HOME={} | workspaces {} (testbed, plain dir), {} (git init: untrusted), {}, {} | {} wants {} and {} (permission mode default)",
        version.raw,
        home.display(),
        tb.a.0,
        tb.u.0,
        tb.c1.0,
        tb.c2.0,
        tb.a.0,
        workspace_session_name(&tb.a.0),
        tb.park()
    );
    if let Some(warning) = version.untested() {
        println!("WARNING {warning}");
    }
    TESTBED.set(tb).map_err(|_| anyhow!("prepared twice"))?;
    Ok(())
}

/// True when none is left; with `(sessionId, cwd)` of each, for their transcripts.
pub fn remove_own_rows(prefix: &str) -> (bool, Vec<(String, std::path::PathBuf)>) {
    supervise::disable_and_wait();
    let rows = match list_sessions() {
        Ok(rows) => rows,
        Err(e) => {
            println!("cleanup: cannot list sessions: {e}");
            return (false, Vec::new());
        }
    };
    let own: Vec<Session> = rows.into_iter().filter(|s| s.name.starts_with(prefix)).collect();
    let transcripts = own.iter().map(|s| (s.session_id.clone(), s.cwd.clone())).collect();
    let claude = Claude::default();
    for row in &own {
        if row.pid.is_some() {
            match claude.stop(&row.id) {
                Ok(out) => println!("cleanup: claude stop {} ({}): {}", row.id, row.name, out.trim()),
                Err(e) => println!("cleanup: claude stop {}: {e}", row.id),
            }
        }
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline
        && list_sessions().is_ok_and(|rows| rows.iter().any(|s| s.name.starts_with(prefix) && s.pid.is_some()))
    {
        std::thread::sleep(Duration::from_millis(500));
    }
    for row in &own {
        match claude.remove(&row.id, None) {
            Ok(Removal::Removed(out)) => println!("cleanup: claude rm {} ({}): {out}", row.id, row.name),
            Ok(Removal::Kept(refusal)) => match refusal.discard.as_deref() {
                Some(discard) => match claude.remove(&row.id, Some(discard)) {
                    Ok(Removal::Removed(out)) => println!("cleanup: claude rm {} --discard-unpushed: {out}", row.id),
                    Ok(Removal::Kept(again)) => println!("cleanup: claude rm {} --discard-unpushed kept it: {}", row.id, again.text),
                    Err(e) => println!("cleanup: claude rm {} --discard-unpushed: {e}", row.id),
                },
                None => println!("cleanup: claude rm {} kept it: {}", row.id, refusal.text),
            },
            Err(e) => println!("cleanup: claude rm {}: {e}", row.id),
        }
    }
    match list_sessions() {
        Ok(rows) => {
            let left: Vec<String> = rows.iter().filter(|s| s.name.starts_with(prefix)).map(|s| s.name.clone()).collect();
            println!("cleanup: {} {prefix}* row(s) removed, {} left {left:?}", own.len(), left.len());
            (left.is_empty(), transcripts)
        }
        Err(e) => {
            println!("cleanup: cannot verify removal: {e}");
            (false, transcripts)
        }
    }
}

async fn rows(cx: &mut AsyncWindowContext) -> anyhow::Result<Vec<Session>> {
    Ok(cx.background_executor().spawn(async { list_sessions() }).await?)
}

async fn row(name: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<Option<Session>> {
    Ok(rows(cx).await?.into_iter().find(|s| s.name == name))
}

async fn wait_for<T>(
    what: &str,
    within: Duration,
    cx: &mut AsyncWindowContext,
    mut done: impl FnMut(&[Session]) -> Option<T>,
) -> anyhow::Result<T> {
    let started = Instant::now();
    loop {
        if let Some(t) = done(&rows(cx).await?) {
            return Ok(t);
        }
        if started.elapsed() > within {
            bail!("{what}: not within {} s", within.as_secs());
        }
        sleep(cx, Duration::from_millis(500)).await;
    }
}

async fn wait_painted(view: &Entity<FleetView>, key: &str, within: Duration, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let started = Instant::now();
    while !view.read_with(cx, |v, _| v.hits.borrow().contains_key(key)) {
        if started.elapsed() > within {
            bail!("nothing painted under {key} within {} s", within.as_secs());
        }
        sleep(cx, Duration::from_millis(100)).await;
    }
    Ok(())
}

/// In a release folder that is its own `plugins/supraman`, not the repo's.
fn plugin_dir() -> anyhow::Result<PathBuf> {
    find_plugin_dir(&std::env::current_exe()?).map_err(anyhow::Error::msg)
}

fn secs(d: Duration) -> String {
    format!("{:.1} s", d.as_secs_f64())
}

pub async fn run(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let tb = testbed()?;
    let ctrl = workspace_session_name(&tb.a.0);

    wait_painted(view, &format!("start-{}", tb.a.0), Duration::from_secs(30), cx).await.context("step b")?;
    let notices = view.read_with(cx, |v, _| v.setup_notices.clone());
    if !notices.is_empty() {
        bail!("step b: setup notices {notices:?}");
    }
    click(view, &format!("start-{}", tb.a.0), cx).await?;
    let clicked = Instant::now();
    let relay_name = relay_session_name(&tb.a.0);
    let launched = wait_for("step b: workspace session live", Duration::from_secs(60), cx, |rows| {
        rows.iter().find(|s| s.name == ctrl && s.pid.is_some()).cloned()
    })
    .await?;
    let live_after = clicked.elapsed();
    let relay_upgrade = no_relay(tb, &relay_name, view, cx).await.context("step b")?;
    let template = std::fs::read_to_string(tb.home.join("roles").join("workspace.md")).context("step b: role file")?;
    let tag = role_tag(WORKSPACE_ROLE, &render_role(&template, &tb.a.0));
    let mut events = EventReader::new(&tb.home);
    let mut seen = Vec::new();
    let mut transcript = None;
    let started = Instant::now();
    let tagged = loop {
        seen.extend(events.poll_session(&launched.session_id));
        let startup = seen.iter().find(|e| e.event == "SessionStart" && e.source.as_deref() == Some("startup"));
        if let Some(e) = startup {
            transcript = e.transcript_path.clone().or(transcript);
        }
        if let Some(path) = &transcript
            && std::fs::read_to_string(path).is_ok_and(|t| t.contains(&tag))
        {
            break true;
        }
        if started.elapsed() > Duration::from_secs(90) {
            break false;
        }
        sleep(cx, Duration::from_millis(500)).await;
    };
    let shown = Instant::now();
    let in_panel = loop {
        let in_panel = view.read_with(cx, |v, _| {
            v.tree().workspaces.iter().any(|p| p.name == tb.a.0 && p.own.as_ref().is_some_and(|r| r.session.name == ctrl))
        });
        if in_panel || shown.elapsed() > Duration::from_secs(10) {
            break in_panel;
        }
        sleep(cx, Duration::from_millis(250)).await;
    };
    if !in_panel {
        let state = view.read_with(cx, |v, _| {
            format!(
                "refreshes {} ticks {} desired {:?} rows named {} in the view: {} errors {:?}",
                v.refreshes,
                v.ticks,
                v.desired,
                ctrl,
                v.sessions.iter().filter(|s| s.name == ctrl).count(),
                v.supervise_errors
            )
        });
        println!("step b: panel state: {state}");
    }
    let events_line = seen.iter().map(|e| e.event.clone() + &e.source.as_deref().map(|s| format!("({s})")).unwrap_or_default()).collect::<Vec<_>>().join(", ");
    if !tagged || !in_panel {
        bail!(
            "step b: {ctrl} live, but SessionStart(startup) with the role tag {tag} in its transcript={tagged}, row under its workspace={in_panel}; events: {events_line}"
        );
    }
    let row_is_session = workspace_row_step(tb, &ctrl, &launched, view, cx).await.context("step b")?;
    let transcript = transcript.unwrap_or_default();
    let listed = skills_listed(&std::fs::read_to_string(&transcript).unwrap_or_default());
    if !template.contains(&format!("/{WORKSPACE_SKILL}")) || ![WORKSPACE_SKILL, REPO_SKILL].iter().all(|s| listed.iter().any(|l| l == s)) {
        bail!("step b: the role must name /{WORKSPACE_SKILL} and the session list it and {REPO_SKILL}; listed {listed:?}");
    }
    println!(
        "step b: Start clicked → {ctrl} ({}) live after {}, {relay_upgrade} | events: {events_line} | transcript has `{tag}` naming /{WORKSPACE_SKILL}, skills listed {listed:?} | {row_is_session}",
        launched.id,
        secs(live_after),
    );

    // right after b: a prompt left open for the whole run is one anything else may answer before a later step looks
    park_step(tb, view, cx).await.context("step e")?;
    let foreign = crate::selftest_left::foreign_row(view, &tb.a.0, &tb.a.1.join(REPO), &tb.park(), &tb.hex, cx).await.context("step e2")?;
    println!("step e2: {foreign}");

    relay_step(tb, &relay_name, view, cx).await.context("step r")?;

    // `claude stop`, not a kill: the daemon respawns a session killed while `state=working`/`blocked` (R13), never a stopped one.
    let idle = wait_for("step c: workspace session idle", Duration::from_secs(120), cx, |rows| {
        rows.iter().find(|s| s.name == ctrl && s.health(now_ms()) == Health::Idle && s.pid.is_some()).cloned()
    })
    .await?;
    // what the skill made of its first turn: reported, not asserted (a model's choice, not Foreman's)
    let (_, tools) = transcript_turn(&transcript);
    let ready = read_outbox(&tb.home, &tb.a.0)?.into_iter().find(|m| m.from_name == ctrl).map(|m| m.body);
    println!("step b: {ctrl}'s first turn: tools {tools:?} | its first message: {ready:?}");
    let hidden = hide(view, cx).await.context("step h")?;
    let pid = idle.pid.unwrap_or_default();
    let killed_at = now_ms();
    let kill = Instant::now();
    let stop = Claude::default().stop(&idle.id).context("step c: claude stop")?;
    let stop = stop.trim();
    let mut dropped = None;
    let mut lingered = 0u32;
    let resumed = loop {
        let row = row(&ctrl, cx).await?;
        match row {
            Some(r) if r.pid == Some(pid) => lingered += 1,
            Some(r) if r.pid.is_none() => {
                dropped.get_or_insert((kill.elapsed(), raw(&r)));
            }
            Some(r) if r.pid.is_some() && r.id == idle.id => break r,
            Some(r) => bail!("step c: {ctrl} came back as another row: {} / {} ({})", r.id, r.session_id, raw(&r)),
            None => bail!("step c: {ctrl} is no longer listed"),
        }
        if cx.update(|window, _| crate::tray::is_shown(window))? {
            bail!("step h: the window was shown again while step c waited for the resume");
        }
        if kill.elapsed() > Duration::from_secs(120) {
            bail!("step c: {ctrl} not resumed within 120 s of `claude stop` (pid dropped: {dropped:?})");
        }
        sleep(cx, Duration::from_millis(250)).await;
    };
    let resumed_after = kill.elapsed();
    sleep(cx, Duration::from_secs(3)).await;
    let attempt = view.read_with(cx, |v, _| v.last_attempt.get(&ctrl).copied());
    if !attempt.is_some_and(|t| t >= killed_at) {
        bail!(
            "step c: {ctrl} came back with pid {:?}, but not by the supervisor (its last attempt {attempt:?}; stopped as [{}])",
            resumed.pid,
            raw(&idle)
        );
    }
    if supraman_core::process::alive(pid) {
        bail!("step c: the stopped pid {pid} is still running");
    }
    // Right after `--bg --resume` the row shows a transient sessionId for a few seconds.
    let transient = (resumed.session_id != idle.session_id).then(|| resumed.session_id.clone());
    let settled = wait_for("step c: sessionId back to the original", Duration::from_secs(30), cx, |rows| {
        rows.iter()
            .find(|s| s.name == ctrl && s.id == idle.id && s.session_id == idle.session_id && s.pid.is_some())
            .cloned()
    })
    .await?;
    if rows(cx).await?.iter().filter(|s| s.name == ctrl).count() != 1 {
        bail!("step c: more than one row named {ctrl} after the resume");
    }
    let (dropped_after, dropped_row) = dropped.map_or(("never seen".into(), String::new()), |(d, r)| (secs(d), r));
    println!(
        "step c: `claude stop {}` ({}) on pid {pid} (turn ended: {}) → row without pid after {dropped_after} [{dropped_row}], old pid seen in {lingered} poll(s) | resumed by the supervisor after {}: same id {}, same sessionId {}{}, pid {pid} → {} [{}], one row",
        idle.id,
        stop.lines().next().unwrap_or_default(),
        raw(&idle),
        secs(resumed_after),
        settled.id,
        settled.session_id,
        transient.map(|t| format!(" (transient {t} right after the resume)")).unwrap_or_default(),
        settled.pid.unwrap_or_default(),
        raw(&settled)
    );

    reopen(&tb.a.0, &ctrl, &settled, hidden, view, cx).await.context("step h")?;

    // after c, not right after r: the drops give the workspace session turns of its own, and c's stop must not land mid-turn
    task_box_step(tb, &transcript, view, cx).await.context("step t")?;

    want_step(tb, view, cx).await.context("step w")?;

    let u_ctrl = workspace_session_name(&tb.u.0);
    wait_painted(view, &format!("start-{}", tb.u.0), Duration::from_secs(10), cx).await?;
    click(view, &format!("start-{}", tb.u.0), cx).await?;
    let clicked = Instant::now();
    let run_key = format!("inbox-run-{u_ctrl}");
    let need = loop {
        let need = view.read_with(cx, |v, _| v.needs.iter().find(|n| n.name == u_ctrl && n.kind == NeedsKind::TrustNeeded).cloned());
        if let Some(need) = need
            && view.read_with(cx, |v, _| v.hits.borrow().contains_key(&run_key))
        {
            break need;
        }
        if clicked.elapsed() > Duration::from_secs(30) {
            bail!("step d: no TrustNeeded Inbox row for {u_ctrl} within 30 s (needs: {:?})", view.read_with(cx, |v, _| v.needs.clone()));
        }
        sleep(cx, Duration::from_millis(100)).await;
    };
    let Some(Remedy::RunInTerminal { cwd, command }) = need.action.clone() else {
        bail!("step d: TrustNeeded without RunInTerminal: {need:?}");
    };
    if row(&u_ctrl, cx).await?.is_some() {
        bail!("step d: a row named {u_ctrl} exists although the launch was refused");
    }
    let tabs_before = view.read_with(cx, |v, _| v.tabs.iter().map(|t| t.id).max());
    click(view, &run_key, cx).await?;
    let tab = view
        .read_with(cx, |v, _| v.tabs.iter().map(|t| t.id).max())
        .filter(|id| Some(*id) != tabs_before)
        .ok_or_else(|| anyhow!("step d: Run in terminal opened no tab"))?;
    // The tab is closed whatever happened, so its shell lets go of the dir.
    let typed: anyhow::Result<String> = async {
        let terminal = wait_for_tab_terminal(view, tab, Duration::from_secs(10), cx).await?;
        // PSReadLine may draw a history prediction after the typed text; it is display only.
        let typed_at = format!(" {command}");
        let screen = |cx: &mut AsyncWindowContext| terminal.read_with(cx, |t, _| t.get_content());
        let last_line = |content: &str| content.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or_default().trim().to_string();
        let started = Instant::now();
        let before = loop {
            let content = screen(cx);
            if last_line(&content).contains(&typed_at) {
                break content;
            }
            if started.elapsed() > Duration::from_secs(25) {
                bail!("`{command}` was not typed at the prompt within 25 s; last line {:?}", last_line(&content));
            }
            sleep(cx, Duration::from_millis(100)).await;
        };
        sleep(cx, Duration::from_secs(3)).await;
        let after = screen(cx);
        if after != before {
            bail!("the screen changed after typing (was it run?): last line {:?}", last_line(&after));
        }
        Ok(last_line(&after))
    }
    .await;
    click(view, &format!("close-tab-{tab}"), cx).await?;
    sleep(cx, Duration::from_millis(500)).await;
    let closed = view.read_with(cx, |v, _| v.tab(tab).is_none());
    let line = typed.context("step d")?;
    println!(
        "step d: Start {} → Inbox `trust needed` after {}: {} | Run in terminal opened a pwsh tab in {} with `{command}` typed at the prompt and not run (screen unchanged 3 s; prompt line {line:?}, any text after `{command}` is PSReadLine's prediction); tab closed={closed}; no row named {u_ctrl}",
        tb.u.0,
        secs(clicked.elapsed()),
        need.detail,
        cwd.display()
    );
    if !closed {
        bail!("step d: the Run in terminal tab did not close");
    }

    view.update(cx, |v, cx| v.set_max_working(1, cx));
    let saved = Settings::load(&tb.home)?.sessions.max_working;
    if saved != 1 {
        bail!("step f: settings.toml has [sessions] max_working {saved} after the stepper showed 1");
    }
    let prefix = format!("fm-selftest-{}", tb.hex);
    let (c1, c2) = (workspace_session_name(&tb.c1.0), workspace_session_name(&tb.c2.0));
    let hit = |key: String, cx: &mut AsyncWindowContext| view.read_with(cx, |v, _| v.hits.borrow().get(&key).copied());
    let mut clicked_at = Vec::new();
    let mut folds = Vec::new();
    for (name, _) in [&tb.c1, &tb.c2] {
        let key = format!("start-{name}");
        wait_painted(view, &key, Duration::from_secs(10), cx).await?;
        let before = hit(key.clone(), cx);
        click(view, &key, cx).await?;
        let at = hit(key.clone(), cx);
        if let (Some(b), Some(a)) = (before, at)
            && !b.fully_visible()
        {
            folds.push(format!(
                "{key} below the fold at y {:.0} (visible to {:.0}), clicked at y {:.0}",
                f32::from(b.bounds.top()),
                f32::from(b.visible.bottom()),
                f32::from(a.bounds.top())
            ));
        }
        clicked_at.push(at);
    }
    sleep(cx, Duration::from_secs(3)).await;
    for ((name, _), at) in [&tb.c1, &tb.c2].into_iter().zip(&clicked_at) {
        if !DesiredState::load(&tb.home, name)?.is_some_and(|d| d.started) {
            bail!(
                "step f: Start {name} clicked at {at:?}, but its desired state is not started 3 s later; start-{name} is now painted at {:?}",
                hit(format!("start-{name}"), cx)
            );
        }
    }
    let started = Instant::now();
    let mut max_seen = 0;
    let mut polls = 0;
    let mut first_live: [Option<Duration>; 2] = [None, None];
    let mut first_idle: [Option<Duration>; 2] = [None, None];
    while first_live.iter().any(Option::is_none) {
        let rows = rows(cx).await?;
        let now = now_ms();
        // relays are outside the budget (an armed one may show `working` throughout)
        let working: Vec<&Session> = rows
            .iter()
            .filter(|s| s.name.starts_with(&prefix) && !s.name.ends_with("-relay") && mid_turn(s, now))
            .collect();
        polls += 1;
        if working.len() > max_seen {
            max_seen = working.len();
        }
        if working.len() > 1 {
            let names: Vec<String> = working.iter().map(|s| format!("{} [{}]", s.name, raw(s))).collect();
            bail!("step f: {} sessions working at once with the limit at 1: {names:?}", working.len());
        }
        for (i, name) in [&c1, &c2].into_iter().enumerate() {
            if let Some(r) = rows.iter().find(|s| &s.name == name) {
                if is_live(r, now) {
                    first_live[i].get_or_insert(started.elapsed());
                }
                if r.health(now) == Health::Idle {
                    first_idle[i].get_or_insert(started.elapsed());
                }
            }
        }
        if started.elapsed() > Duration::from_secs(180) {
            bail!("step f: not both launched within 180 s (first live {first_live:?})");
        }
        sleep(cx, Duration::from_millis(500)).await;
    }
    let fmt = |d: Option<Duration>| d.map_or("-".to_string(), secs);
    let (first, second) = if first_live[0] <= first_live[1] { (0, 1) } else { (1, 0) };
    if !first_idle[first].is_some_and(|idle| first_live[second].is_some_and(|live| live >= idle)) {
        bail!(
            "step f: the second workspace went live at {} before the first one's turn ended ({})",
            fmt(first_live[second]),
            fmt(first_idle[first])
        );
    }
    println!(
        "step f: limit {DEFAULT_MAX_WORKING} → 1 via set_max_working (settings.toml [sessions] max_working = 1) | Start {} and {} ({}): max {max_seen} working over {polls} polls | first live {} / {}, first idle {} / {}: the second went only after the first one's turn ended",
        tb.c1.0,
        tb.c2.0,
        if folds.is_empty() { "both in view".to_string() } else { folds.join("; ") },
        fmt(first_live[0]),
        fmt(first_live[1]),
        fmt(first_idle[0]),
        fmt(first_idle[1])
    );

    scroll_step(tb, view, cx).await.context("step s")?;
    virtual_step(tb, view, cx).await.context("step v")?;
    edit_step(tb, view, cx).await.context("step x")?;
    adopt_step(tb, view, cx).await.context("step y")?;

    click(view, &format!("stop-{}", tb.a.0), cx).await?;
    let clicked = Instant::now();
    // the relay was retired in step r: stopped already, and no longer the workspace's
    let names = [ctrl.clone(), tb.park(), relay_name.clone()];
    let stopped = wait_for("step g: testbed sessions stopped", Duration::from_secs(60), cx, |rows| {
        let ours: Vec<&Session> = rows.iter().filter(|s| names.contains(&s.name)).collect();
        (ours.len() == 3 && ours.iter().all(|s| s.pid.is_none())).then(|| ours.iter().map(|s| format!("{} [{}]", s.name, raw(s))).collect::<Vec<_>>())
    })
    .await?;
    sleep(cx, Duration::from_secs(3)).await;
    let (shown, own) = view.read_with(cx, |v, _| {
        v.tree().workspaces.iter().find(|p| p.name == tb.a.0).map(|p| (p.supervised.len(), p.own.as_ref().map(|r| r.session.name.clone()))).unwrap_or_default()
    });
    let still = rows(cx).await?.into_iter().filter(|s| names.contains(&s.name) && s.pid.is_some()).count();
    println!(
        "step g: Stop clicked → stopped after {}: {} | still listed: the workspace row's session {own:?}, {shown} row(s) shown under {} | restarted since: {still}",
        secs(clicked.elapsed()),
        stopped.join(", "),
        tb.a.0
    );
    if shown != 1 || own.as_deref() != Some(ctrl.as_str()) || still != 0 {
        bail!("step g: expected {ctrl} on the workspace row, 1 row shown (park) and none restarted, got {own:?}, {shown} shown, {still} live");
    }
    stop_all_step(tb, view, cx).await.context("step q")
}

struct Hidden {
    at: Instant,
    ticks: u64,
}

/// WM_CLOSE to this process's windows, as the caption button and Alt+F4 send it: the window hides.
async fn hide(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<Hidden> {
    let posted = crate::selftest::post_close_to_own_windows();
    let at = Instant::now();
    while cx.update(|window, _| crate::tray::is_shown(window))? {
        if at.elapsed() > Duration::from_secs(5) {
            bail!("the window is still shown 5 s after WM_CLOSE (posted to {posted} window(s))");
        }
        sleep(cx, Duration::from_millis(50)).await;
    }
    let ticks = view.read_with(cx, |v, _| v.ticks);
    println!("step h: WM_CLOSE posted to {posted} window(s) → hidden after {} ms, tick {ticks}", at.elapsed().as_millis());
    Ok(Hidden { at, ticks })
}

async fn reopen(
    workspace: &str,
    ctrl: &str,
    settled: &Session,
    hidden: Hidden,
    view: &Entity<FleetView>,
    cx: &mut AsyncWindowContext,
) -> anyhow::Result<()> {
    if cx.update(|window, _| crate::tray::is_shown(window))? {
        bail!("the window was shown before Open");
    }
    let hidden_for = hidden.at.elapsed();
    let ticks = view.read_with(cx, |v, _| v.ticks) - hidden.ticks;
    if ticks == 0 {
        bail!("no supervisor tick in the {} the window was hidden", secs(hidden_for));
    }
    // painted again after Open, not left over from before the hide
    let key = format!("stop-{workspace}");
    view.read_with(cx, |v, _| v.hits.borrow_mut().remove(&key));
    let fleet = view.clone();
    let opened = Instant::now();
    let opened_ms = now_ms();
    cx.update(|window, cx| crate::tray::apply(crate::tray::Command::Open, &fleet, window, cx))?;
    while !cx.update(|window, _| crate::tray::is_shown(window))? {
        if opened.elapsed() > Duration::from_secs(5) {
            bail!("the window is not shown 5 s after Open");
        }
        sleep(cx, Duration::from_millis(50)).await;
    }
    let shown_after = opened.elapsed();
    wait_painted(view, &key, Duration::from_secs(10), cx).await?;
    let painted_after = opened.elapsed();
    loop {
        let (polled_at, pid, needs) = view.read_with(cx, |v, _| {
            (
                v.polled_at,
                v.sessions.iter().find(|s| s.name == ctrl).and_then(|s| s.pid),
                v.needs.iter().filter(|n| n.name == ctrl).map(|n| format!("{:?}", n.kind)).collect::<Vec<_>>(),
            )
        });
        if polled_at > opened_ms && pid == settled.pid && needs.is_empty() {
            break;
        }
        if opened.elapsed() > Duration::from_secs(15) {
            bail!("after Open the panel shows {ctrl} with pid {pid:?} (resumed as {:?}), Inbox rows {needs:?}", settled.pid);
        }
        sleep(cx, Duration::from_millis(100)).await;
    }
    println!(
        "step h: hidden {} with {ticks} supervisor tick(s), {ctrl} resumed meanwhile | Open → shown after {} ms, `{key}` repainted after {} ms, a poll after Open shows {ctrl} with the resumed pid {} and no Inbox row for it",
        secs(hidden_for),
        shown_after.as_millis(),
        painted_after.as_millis(),
        settled.pid.unwrap_or_default()
    );
    Ok(())
}

async fn stop_all_step(tb: &Testbed, view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let others = [&tb.u.0, &tb.c1.0, &tb.c2.0];
    let started = |w: &str| DesiredState::load(&tb.home, w).ok().flatten().is_some_and(|d| d.started);
    let before: Vec<&String> = others.iter().copied().filter(|w| started(w)).collect();
    if before.len() != others.len() {
        bail!("before Stop all only {before:?} of {others:?} are started");
    }
    let fleet = view.clone();
    let asked = Instant::now();
    cx.update(|window, cx| crate::tray::apply(crate::tray::Command::StopAll, &fleet, window, cx))?;
    // the save runs off the UI thread, one write after another
    while let Some(w) = others.iter().find(|w| started(w)) {
        if asked.elapsed() > Duration::from_secs(5) {
            bail!("{w} is still saved as started 5 s after Stop all");
        }
        sleep(cx, Duration::from_millis(20)).await;
    }
    let saved_after = asked.elapsed();
    let names: Vec<String> = [&tb.c1.0, &tb.c2.0].iter().map(|w| workspace_session_name(w)).collect();
    let stopped = wait_for("step q: the budget workspaces' sessions stopped", Duration::from_secs(60), cx, |rows| {
        let ours: Vec<&Session> = rows.iter().filter(|s| names.contains(&s.name)).collect();
        (ours.len() == names.len() && ours.iter().all(|s| s.pid.is_none()))
            .then(|| ours.iter().map(|s| format!("{} [{}]", s.name, raw(s))).collect::<Vec<_>>())
    })
    .await?;
    println!(
        "step q: Stop all → {} saved as not started after {} ms, sessions stopped after {}: {}",
        before.iter().map(|w| w.as_str()).collect::<Vec<_>>().join(", "),
        saved_after.as_millis(),
        secs(asked.elapsed()),
        stopped.join(", ")
    );
    Ok(())
}

/// Fails, rather than waits, when the prompt was answered before (by whom this run cannot know).
async fn park_step(tb: &Testbed, view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let park = tb.park();
    let attach_key = format!("inbox-attach-{park}");
    let started = Instant::now();
    let mut events = EventReader::new(&tb.home);
    let mut seen = Vec::new();
    let (park_row, prompt, need) = loop {
        if let Some(r) = row(&park, cx).await? {
            seen.extend(events.poll_session(&r.session_id));
            let prompt = seen
                .iter()
                .find(|e| e.event == "Notification" && e.notification_type.as_deref() == Some("permission_prompt"))
                .cloned();
            let transcript = seen.iter().find_map(|e| e.transcript_path.clone());
            if let Some(path) = &transcript
                && let Some(result) = tool_result(path, |t| t.starts_with("Bash: ") && t.contains("parked"))
            {
                bail!("{park}'s command was answered before this step saw it parked: {result:?} [{}]", raw(&r));
            }
            let need = view.read_with(cx, |v, _| v.needs.iter().find(|n| n.name == park && n.kind == NeedsKind::Blocked).cloned());
            if let (Some(prompt), Some(need)) = (prompt, need)
                && view.read_with(cx, |v, _| v.hits.borrow().contains_key(&attach_key))
            {
                break (r, prompt, need);
            }
        }
        if started.elapsed() > Duration::from_secs(180) {
            let events_line = seen.iter().map(|e| e.event.clone() + &e.notification_type.as_deref().map(|s| format!("({s})")).unwrap_or_default()).collect::<Vec<_>>().join(", ");
            bail!(
                "no permission_prompt + Blocked Inbox row with Attach for {park} within 180 s (row: {:?}; hook events: {events_line})",
                row(&park, cx).await?.map(|r| raw(&r))
            );
        }
        sleep(cx, Duration::from_millis(250)).await;
    };
    let Some(Remedy::Attach { id }) = &need.action else {
        bail!("Blocked without Attach: {need:?}");
    };
    if &park_row.id != id {
        bail!("Attach targets {id}, the row is {}", park_row.id);
    }
    println!(
        "step e: {park} parked → hook Notification(permission_prompt) {} after its launch, Inbox `blocked` ({}) with Attach {id} seen {} into the step | row [{}]",
        secs(Duration::from_millis(park_row.started_at.map_or(0, |t| (prompt.ts - t).max(0)) as u64)),
        need.detail,
        secs(started.elapsed()),
        raw(&park_row),
    );
    Ok(())
}

async fn scroll_step(tb: &Testbed, view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let original = cx.update(|window, _| window.viewport_size())?;
    cx.update(|window, _| window.resize(size(original.width, px(480.))))?;
    sleep(cx, Duration::from_secs(1)).await;
    let result = lowest_row_scrolled_into_view(tb, view, cx).await;
    cx.update(|window, _| window.resize(original))?;
    sleep(cx, Duration::from_secs(1)).await;
    println!("step s: {}", result?);
    Ok(())
}

async fn lowest_row_scrolled_into_view(tb: &Testbed, view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let anchor = format!("stop-{}", tb.a.0);
    let tree = view
        .read_with(cx, |v, _| v.hits.borrow().get(&anchor).copied())
        .ok_or_else(|| anyhow!("nothing painted under {anchor}"))?
        .visible;
    let (key, before) = view
        .read_with(cx, |v, _| {
            v.hits
                .borrow()
                .iter()
                .filter(|(_, h)| h.visible == tree)
                .max_by(|a, b| f32::from(a.1.bounds.bottom()).total_cmp(&f32::from(b.1.bounds.bottom())))
                .map(|(k, h)| (k.clone(), *h))
        })
        .ok_or_else(|| anyhow!("no hit target in the tree"))?;
    if before.fully_visible() {
        bail!("the tree does not overflow a 480 px window: its lowest row {key} is in view at {:?} (tree {tree:?})", before.bounds);
    }
    let after = scrolled_into_view(view, &key, cx).await?;
    let now = view.read_with(cx, |v, _| v.hits.borrow().get(&key).copied()).ok_or_else(|| anyhow!("{key} gone"))?;
    if !now.fully_visible() {
        bail!("{key} at {:?} is not wholly inside the tree {:?} after scrolling", now.bounds, now.visible);
    }
    Ok(format!(
        "window 480 px tall: the tree shows y {:.0}..{:.0}; its lowest row {key} at y {:.0} (below the fold) → scrolled to y {:.0}, wholly visible; size restored",
        f32::from(tree.top()),
        f32::from(tree.bottom()),
        f32::from(before.bounds.top()),
        f32::from(after.top())
    ))
}

const PROMPT_OK: &str = "This is an automated Supraman selftest. Reply with the single word ok, use no tool, and stop.";

/// A workspace of this checkout (a git folder: claude asks trust for any folder holding a `.git`, so a git
/// folder made here could not launch) and a plain folder in the testbed's home: each folder's wanted session
/// runs and sits first under its folder, and the plain one's not under the testbed, whose home holds it but
/// which does not enable it; then both stop.
async fn virtual_step(tb: &Testbed, view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let name = format!("{}-v", tb.a.0);
    let (git_dir, plain_dir) = (cargo_root(), tb.a.1.join("VPlain"));
    std::fs::create_dir_all(&plain_dir)?;
    let mut ws = Workspace::new(name.as_str(), &git_dir, Vec::new());
    ws.home = None;
    ws.add_folder(&git_dir);
    ws.add_folder(&plain_dir);
    ws.folders[0].name = "VGit".into();
    let kinds: Vec<FolderKind> = ws.folders.iter().map(|f| f.kind).collect();
    if kinds != [FolderKind::Git, FolderKind::Plain] || ws.folders[0].branch.is_none() || ws.folders[1].branch.is_some() {
        bail!("the folders read as {:?}", ws.folders);
    }
    let folders: Vec<String> = ws.folders.iter().map(|f| f.name.clone()).collect();
    Registry::at(&tb.home).save(&ws, true)?;
    let wanted: Vec<WantedSession> =
        ws.folders.iter().map(|f| repo_session(&name, &f.name, &f.path, PROMPT_OK).ok_or_else(|| anyhow!("no name for {}", f.name))).collect::<anyhow::Result<_>>()?;
    let names: Vec<String> = wanted.iter().map(|w| w.name.clone()).collect();
    let started = Instant::now();
    DesiredState::update(&tb.home, &name, |d| {
        for w in &wanted {
            d.want(w.clone());
        }
        d.started = true;
    })?;
    let live = wait_for("both folders' sessions live", Duration::from_secs(180), cx, |rows| {
        let ours: Vec<Session> = names.iter().filter_map(|n| rows.iter().find(|s| &s.name == n && s.pid.is_some()).cloned()).collect();
        (ours.len() == names.len()).then_some(ours)
    })
    .await?;
    let live_after = started.elapsed();
    let placed = Instant::now();
    loop {
        let tree = view.read_with(cx, |v, _| {
            let t = v.tree();
            let node = |w: &str| t.workspaces.iter().find(|n| n.name == w).cloned();
            (node(&name), node(&tb.a.0))
        });
        let ok = |t: &(Option<crate::fleet::WorkspaceNode>, Option<crate::fleet::WorkspaceNode>)| {
            let (Some(v), Some(a)) = t else { return false };
            let firsts: Vec<(&str, FolderKind, Option<&str>, Option<&str>)> = v
                .repos
                .iter()
                .map(|r| (r.name.as_str(), r.kind, r.sessions.first().map(|s| s.session.name.as_str()), r.sessions.first().and_then(|s| s.also_in.as_deref())))
                .collect();
            let in_a = |n: &str| a.rows().any(|s| s.session.name == n);
            firsts == [
                (folders[0].as_str(), FolderKind::Git, Some(names[0].as_str()), None),
                (folders[1].as_str(), FolderKind::Plain, Some(names[1].as_str()), None),
            ] && !in_a(&names[1])
        };
        if ok(&tree) {
            break;
        }
        if placed.elapsed() > Duration::from_secs(15) {
            bail!("the tree does not place them: {name} {:?} / {} {:?}", tree.0.map(|n| n.repos), tb.a.0, tree.1.map(|n| n.repos.iter().map(|r| r.name.clone()).collect::<Vec<_>>()));
        }
        sleep(cx, Duration::from_millis(250)).await;
    }
    for s in &live {
        wait_painted(view, &format!("session-row-{}", s.session_id), Duration::from_secs(10), cx).await?;
    }
    let scoped = format!("session-row-{}@{}", live[1].session_id, tb.a.0);
    if view.read_with(cx, |v, _| v.hits.borrow().contains_key(&scoped)) {
        bail!("{scoped} painted: {} enables {}, so its session shows only there", name, folders[1]);
    }
    DesiredState::update(&tb.home, &name, |d| d.started = false)?;
    let stop_at = Instant::now();
    let stopped = wait_for("both stopped", Duration::from_secs(60), cx, |rows| {
        let ours: Vec<&Session> = rows.iter().filter(|s| names.contains(&s.name)).collect();
        (ours.len() == names.len() && ours.iter().all(|s| s.pid.is_none())).then(|| ours.iter().map(|s| format!("{} [{}]", s.name, raw(s))).collect::<Vec<_>>())
    })
    .await?;
    let refused = unwanted_double_click(view, &live[0], cx).await?;
    println!(
        "step v: workspace {name} = {} (git, this checkout) + {} (plain, inside {}), home the first folder | wanted {} live after {} | each first under its folder in {name}, neither under {} (no session-row-<id>@{}) | not started → stopped (still listed) after {}: {} | {refused}",
        folders[0],
        folders[1],
        tb.a.0,
        names.join(", "),
        secs(live_after),
        tb.a.0,
        tb.a.0,
        secs(stop_at.elapsed()),
        stopped.join(", ")
    );
    Ok(())
}

/// A stopped `fm-` session is the supervisor's to run: a double-click on its row opens nothing and says so.
async fn unwanted_double_click(view: &Entity<FleetView>, session: &Session, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let shown = view.update(cx, |v, cx| {
        cx.notify();
        std::mem::replace(&mut v.show_stopped, true)
    });
    let result = async {
        let key = format!("session-row-{}", session.session_id);
        let started = Instant::now();
        while !view.read_with(cx, |v, _| {
            v.hits.borrow().contains_key(&key) && v.tree().workspaces.iter().flat_map(|w| w.rows()).any(|r| r.session.session_id == session.session_id && !crate::fleet::is_alive(&r.health))
        }) {
            if started.elapsed() > Duration::from_secs(20) {
                bail!("{key} not painted as stopped within 20 s");
            }
            sleep(cx, Duration::from_millis(100)).await;
        }
        let tabs = view.read_with(cx, |v, _| v.tabs.len());
        double_click(view, &key, cx).await?;
        sleep(cx, Duration::from_millis(500)).await;
        let (after, toast) = view.read_with(cx, |v, _| (v.tabs.len(), v.toast.text().map(String::from)));
        if after != tabs {
            bail!("a double-click on stopped {}'s row opened {} tab(s)", session.name, after - tabs);
        }
        let toast = toast.unwrap_or_default();
        if !toast.contains("Start the workspace from its menu") {
            bail!("a double-click on stopped {}'s row showed no hint to Start the workspace: toast {toast:?}", session.name);
        }
        Ok(format!("a double-click on stopped {} opens nothing, toast \"{toast}\"", session.name))
    }
    .await;
    view.update(cx, |v, cx| {
        v.show_stopped = shown;
        cx.notify();
    });
    result
}

/// Through the window: Add workspace → Empty, two sibling folders of the testbed added by Edit folders… (a git
/// one, never launched, see [`virtual_step`], and a plain one), the testbed as home; the plain folder's
/// wanted session runs under it; the git folder disabled leaves the tree and search finds it `disabled`; the
/// plain folder removed after the question stops its session (stop, never rm). The file follows each change.
async fn edit_step(tb: &Testbed, view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let name = format!("{}-x", tb.a.0);
    let (git_dir, plain_dir) = (tb.a.1.join("XGit"), tb.a.1.join("XPlain"));
    std::fs::create_dir_all(git_dir.join(".git"))?;
    std::fs::write(git_dir.join(".git").join("HEAD"), "ref: refs/heads/main
")?;
    std::fs::create_dir_all(&plain_dir)?;
    let registry = Registry::at(&tb.home);
    let within = Duration::from_secs(10);

    click(view, "add-workspace", cx).await?;
    click(view, "menu-add-empty", cx).await?;
    view.update(cx, |v, cx| v.adding.as_ref().map(|d| d.name.update(cx, |n, cx| n.set_text(&name, cx))));
    click(view, "add-save", cx).await?;
    let made = saved(&registry, &name, "saved empty", |w| w.folders.is_empty() && w.home.is_none() && w.scans.is_empty(), cx).await?;
    wait_painted(view, "edit-folders", within, cx).await?;

    for dir in [&git_dir, &plain_dir] {
        view.update(cx, |v, _| v.pick_folder = Some(dir.clone()));
        click(view, "edit-add-folder", cx).await?;
    }
    let kinds = |w: &Workspace| w.folders.iter().map(|f| (f.name.clone(), f.kind, f.enabled)).collect::<Vec<_>>();
    let both = vec![("XGit".to_string(), FolderKind::Git, true), ("XPlain".to_string(), FolderKind::Plain, true)];
    saved(&registry, &name, "with XGit and XPlain", |w| kinds(w) == both, cx).await?;
    let home = tb.a.1.clone();
    view.update(cx, |v, cx| v.editing.as_ref().map(|e| e.path.update(cx, |p, cx| p.set_text(&home.display().to_string(), cx))));
    click(view, "edit-set-home", cx).await?;
    saved(&registry, &name, "homed in the testbed", |w| w.home.as_deref() == Some(home.as_path()), cx).await?;

    let wanted = repo_session(&name, "XPlain", &plain_dir, PROMPT_OK).ok_or_else(|| anyhow!("no name for XPlain"))?;
    let session_name = wanted.name.clone();
    let started = Instant::now();
    DesiredState::update(&tb.home, &name, |d| {
        d.want(wanted.clone());
        d.started = true;
    })?;
    let live = wait_for("XPlain's session live", Duration::from_secs(180), cx, |rows| rows.iter().find(|s| s.name == session_name && s.pid.is_some()).cloned()).await?;
    let live_after = started.elapsed();
    // the testbed's own sessions in the home show as its unlisted folders
    let folders_of = |v: &FleetView| {
        let node = v.tree().workspaces.into_iter().find(|n| n.name == name)?;
        Some(node.repos.iter().filter(|r| r.listed).map(|r| (r.name.clone(), r.sessions.first().map(|s| s.session.name.clone()))).collect::<Vec<_>>())
    };
    let placed = vec![("XGit".to_string(), None), ("XPlain".to_string(), Some(session_name.clone()))];
    crate::selftest::until(view, cx, within, "XGit and XPlain in the tree, the session first under XPlain", |v| (folders_of(v).as_ref() == Some(&placed)).then_some(()))
        .await
        .map_err(|e| e.context(format!("listed folders {:?}", view.read_with(cx, |v, _| folders_of(v)))))?;
    wait_painted(view, &format!("session-row-{}", live.session_id), within, cx).await?;

    click(view, "edit-enable-XGit", cx).await?;
    saved(&registry, &name, "with XGit disabled", |w| w.folder("XGit").is_some_and(|f| !f.enabled), cx).await?;
    let only_plain = vec![("XPlain".to_string(), Some(session_name.clone()))];
    crate::selftest::until(view, cx, within, "the disabled XGit out of the tree", |v| (folders_of(v).as_ref() == Some(&only_plain)).then_some(())).await?;
    view.update(cx, |v, cx| v.search.update(cx, |s, cx| s.set_text("XGit", cx)));
    wait_painted(view, &format!("result-disabled-{name}/XGit"), within, cx).await?;
    view.update(cx, |v, cx| v.clear_search(cx));

    click(view, "edit-remove-XPlain", cx).await?;
    wait_painted(view, "confirm-yes", within, cx).await?;
    let question = view.read_with(cx, |v, _| v.confirm.as_ref().map(|c| c.text.clone())).unwrap_or_default();
    if !question.contains(&session_name) || !question.contains("stop, never rm") {
        bail!("the question before removing XPlain does not name its stop: {question:?}");
    }
    click(view, "confirm-yes", cx).await?;
    let stop_at = Instant::now();
    saved(&registry, &name, "without XPlain", |w| w.folders.iter().map(|f| f.name.as_str()).collect::<Vec<_>>() == ["XGit"], cx).await?;
    let stopped = wait_for("XPlain's session stopped", Duration::from_secs(60), cx, |rows| {
        rows.iter().find(|s| s.name == session_name && s.pid.is_none()).map(|s| format!("{} [{}]", s.name, raw(s)))
    })
    .await?;
    let desired = DesiredState::load(&tb.home, &name)?.ok_or_else(|| anyhow!("no desired state for {name}"))?;
    if desired.sessions.iter().any(|s| s.name == session_name) || !desired.retired.contains(&session_name) {
        bail!("{session_name} is still wanted or not retired: {desired:?}");
    }
    DesiredState::update(&tb.home, &name, |d| d.started = false)?;
    click(view, "edit-done", cx).await?;
    println!(
        "step x: Add workspace → Empty → {name} saved with no folder (created {}) | Edit folders…: XGit (git) and XPlain (plain), siblings in {}, added by the picker, home set to it | {session_name} live after {}, first under XPlain | XGit disabled → out of the tree, search shows it `disabled` | Remove XPlain asked {question:?} → removed from the file, stopped after {} (still listed, never rm): {stopped}",
        made.created_at,
        tb.a.0,
        secs(live_after),
        secs(stop_at.elapsed()),
    );
    Ok(())
}

/// m5/61, through the window: a background session this run launched itself, unmanaged, in a folder of a started
/// workspace is adopted from its menu. The question names the copy; the original is stopped (stop, never rm) and
/// stays listed; `fm-<ws>-<folder>` runs as a copy under a new id whose transcript carries the original's turn,
/// and the desired state forks no more once that row shows.
async fn adopt_step(tb: &Testbed, view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let name = format!("{}-y", tb.a.0);
    let dir = tb.a.1.join("YPlain");
    std::fs::create_dir_all(&dir)?;
    let mut ws = Workspace::new(name.as_str(), &dir, Vec::new());
    ws.home = None;
    ws.add_folder(&dir);
    let folder = ws.folders.first().map(|f| f.name.clone()).ok_or_else(|| anyhow!("no folder"))?;
    Registry::at(&tb.home).save(&ws, true)?;
    DesiredState::update(&tb.home, &name, |d| d.started = true)?;
    let supervised = repo_session_name(&name, &folder).ok_or_else(|| anyhow!("no name for {folder}"))?;

    let adoptee = format!("selftest-{}-adoptee", tb.hex);
    if let Some(scratch) = lock().as_mut() {
        scratch.session_name = Some(adoptee.clone());
    }
    let word = format!("PELICAN{}", tb.hex.to_uppercase());
    let mut spec = LaunchSpec::new(&adoptee, &dir, format!("This is an automated Supraman selftest. Remember the word {word}. Reply with exactly: OK"));
    spec.permission_mode = "default".into();
    spec.strict_mcp_config = true;
    let launched = Instant::now();
    Claude::default().launch(&spec).context("launch the unmanaged session")?;
    let original = wait_for("the unmanaged session's first turn ended", Duration::from_secs(120), cx, |rows| {
        rows.iter().find(|s| s.name == adoptee && s.pid.is_some() && s.state == Some(SessionState::Done)).cloned()
    })
    .await?;
    let turn_after = launched.elapsed();

    // step v's folder is this checkout, which holds YPlain too: the row under YPlain may be its second occurrence
    let row_key = crate::selftest::until(view, cx, Duration::from_secs(15), "the unmanaged session under YPlain of the new workspace", |v| {
        let tree = v.tree();
        let node = tree.workspaces.iter().find(|n| n.name == name)?;
        let repo = node.repos.iter().find(|r| r.name == folder)?;
        let row = repo.sessions.iter().find(|r| r.session.session_id == original.session_id)?;
        Some(format!("session-row-{}{}", original.session_id, row.scope(&name)))
    })
    .await
    .map_err(|e| {
        let at = view.read_with(cx, |v, _| {
            let tree = v.tree();
            tree.workspaces
                .iter()
                .flat_map(|n| n.repos.iter().flat_map(move |r| r.sessions.iter().map(move |s| (n.name.clone(), r.name.clone(), s))))
                .filter(|(_, _, s)| s.session.session_id == original.session_id)
                .map(|(w, r, s)| format!("{w}/{r} also_in={:?}", s.also_in))
                .collect::<Vec<_>>()
        });
        e.context(format!("it sits at {at:?}"))
    })?;
    crate::selftest_manage::press_painted(view, &row_key, gpui::MouseButton::Right, cx).await?;
    wait_painted(view, "menu-adopt", Duration::from_secs(10), cx).await?;
    click(view, "menu-adopt", cx).await?;
    wait_painted(view, "confirm-yes", Duration::from_secs(10), cx).await?;
    let question = view.read_with(cx, |v, _| v.confirm.as_ref().map(|c| c.text.clone())).unwrap_or_default();
    for part in [supervised.as_str(), "stop, never rm", "a copy under a new id", "The original stays stopped and listed"] {
        if !question.contains(part) {
            bail!("the Adopt question does not say {part:?}: {question:?}");
        }
    }
    click(view, "confirm-yes", cx).await?;
    let adopted = Instant::now();
    let (copy, stopped) = wait_for("the copy live as the supervised name, the original stopped", Duration::from_secs(180), cx, |rows| {
        let copy = rows.iter().find(|s| s.name == supervised && s.pid.is_some() && s.session_id != original.session_id)?;
        let stopped = rows.iter().find(|s| s.session_id == original.session_id && s.pid.is_none())?;
        Some((copy.clone(), stopped.clone()))
    })
    .await?;
    let copy_after = adopted.elapsed();
    if stopped.name != adoptee || stopped.id != original.id {
        bail!("the original changed: {} ({}) → {} ({})", original.name, original.id, stopped.name, stopped.id);
    }
    crate::selftest::until(view, cx, Duration::from_secs(30), "fork_from cleared once the copy's row shows", |_| {
        DesiredState::load(&tb.home, &name)
            .ok()
            .flatten()
            .and_then(|d| d.sessions.into_iter().find(|w| w.name == supervised))
            .filter(|w| w.fork_from.is_none())
            .map(drop)
    })
    .await?;
    let claude_dir = supraman_core::paths::claude_dir().ok_or_else(|| anyhow!("no claude dir"))?;
    let carried = crate::selftest::until(view, cx, Duration::from_secs(60), "the copy's transcript carries the original's turn and the adopt prompt", |_| {
        let path = supraman_claude::find_transcript(&claude_dir, &copy.session_id, Some(&copy.cwd))?;
        let text = std::fs::read_to_string(&path).ok()?;
        (text.contains(&word) && text.contains("Supraman adopted this conversation")).then_some(path)
    })
    .await?;
    let no_second = view.read_with(cx, |v, _| {
        // stopped, its row is hidden unless stopped sessions are shown
        let session = v.sessions.iter().find(|s| s.session_id == original.session_id)?.clone();
        let health = session.health(now_ms());
        v.adoption(&name, &folder, &crate::fleet::SessionRow::new(session, health))
    });
    if !matches!(&no_second, Some(crate::fleet::Adoption::No(why)) if why.contains(&supervised)) {
        bail!("the stopped original still offers Adopt or says nothing: {no_second:?}");
    }
    DesiredState::update(&tb.home, &name, |d| d.started = false)?;
    wait_for("the copy stopped", Duration::from_secs(60), cx, |rows| rows.iter().any(|s| s.name == supervised && s.pid.is_none()).then_some(())).await?;
    println!(
        "step y: unmanaged {adoptee} ({}, {}) launched in {folder} of {name}, first turn ended after {} | menu Adopt… asked {question:?} → original stopped and listed [{}] | {supervised} live after {} as a copy {} ({}), fork_from cleared | its transcript {} carries {word} and the adopt prompt | the original's card: {no_second:?}",
        original.id,
        original.session_id,
        secs(turn_after),
        raw(&stopped),
        secs(copy_after),
        copy.id,
        copy.session_id,
        carried.display(),
    );
    Ok(())
}

/// The workspace's file once `ok` holds for it.
async fn saved(registry: &Registry, name: &str, what: &str, ok: impl Fn(&Workspace) -> bool, cx: &mut AsyncWindowContext) -> anyhow::Result<Workspace> {
    let started = Instant::now();
    loop {
        if let Ok(w) = registry.load(name)
            && ok(&w)
        {
            return Ok(w);
        }
        if started.elapsed() > Duration::from_secs(10) {
            bail!("{name}.toml: not {what}: {:?}", registry.load(name).ok());
        }
        sleep(cx, Duration::from_millis(100)).await;
    }
}

const FM_WITHIN: Duration = Duration::from_secs(60);

/// `sm.exe` built beside this `supraman.exe`, run with this process's home (the testbed's).
fn sm(args: &[&str]) -> anyhow::Result<String> {
    let exe = std::env::current_exe()?.with_file_name(supraman_core::HOOK_EXE);
    let out = supraman_core::process::run(supraman_core::process::command(&exe).args(args), FM_WITHIN, supraman_core::process::Kill::Tree)
        .with_context(|| format!("run {}", exe.display()))?;
    let text = format!("{}{}", out.stdout, out.stderr);
    if !out.success() {
        bail!("sm {}: {}", args.join(" "), text.trim());
    }
    Ok(text)
}

async fn want_step(tb: &Testbed, view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let w = tb.a.0.as_str();
    let name = supraman_supervisor::repo_session_name(w, REPO).ok_or_else(|| anyhow!("no session name for {REPO}"))?;
    let task = tb.home.join("want-task.md");
    std::fs::write(&task, format!("Run `sm roster {w}` with the Bash tool and reply with its output verbatim. Then stop."))?;
    let started = Instant::now();
    let wanted = sm(&["want", w, REPO, "--prompt-file", &task.display().to_string()])?;
    let live = wait_for("repo session live", Duration::from_secs(60), cx, |rows| {
        rows.iter().find(|s| s.name == name && s.pid.is_some()).cloned()
    })
    .await?;
    let live_after = started.elapsed();
    let attempt = view.read_with(cx, |v, _| v.last_attempt.get(&name).copied());
    if attempt.is_none() {
        bail!("{name} is live, but the supervisor never launched it");
    }
    // Asserted on the command's result in the transcript, not on the reply: what the model writes around it is its own.
    let mut events = EventReader::new(&tb.home);
    let mut transcript = None;
    let ran = |t: &str| t.starts_with("Bash: ") && t.contains("sm roster");
    let result = loop {
        for e in events.poll_session(&live.session_id) {
            transcript = e.transcript_path.clone().or(transcript);
        }
        if let Some(path) = &transcript
            && let Some(result) = tool_result(path, ran)
        {
            break result;
        }
        if let Some(r) = row(&name, cx).await?
            && let Health::Blocked { waiting_for } = r.health(now_ms())
        {
            bail!("{name} parked running sm ({waiting_for:?}): [{}]", raw(&r));
        }
        if started.elapsed() > Duration::from_secs(150) {
            let tools = transcript.as_deref().map(|p| transcript_turn(p).1).unwrap_or_default();
            bail!("{name} has no result of a Bash `sm roster` within 150 s: tools {tools:?}");
        }
        sleep(cx, Duration::from_millis(500)).await;
    };
    let tools = transcript.as_deref().map(|p| transcript_turn(p).1).unwrap_or_default();
    let own = format!("{name}\trepo\t");
    if !result.lines().any(|l| l.starts_with(&own)) {
        bail!("{name} ran {tools:?}; its `sm roster` result does not list it as repo: {result:?}");
    }
    let roster = sm(&["roster", w])?;
    let line = roster
        .lines()
        .find(|l| l.starts_with(&format!("{name}\trepo\t")))
        .ok_or_else(|| anyhow!("sm roster {w} does not list {name} as repo:\n{roster}"))?
        .to_string();
    let unwanted = sm(&["unwant", w, REPO])?;
    let unwant_at = Instant::now();
    let stopped = wait_for("repo session stopped after unwant", Duration::from_secs(60), cx, |rows| {
        rows.iter().find(|s| s.name == name && s.pid.is_none()).cloned()
    })
    .await?;
    let stopped_after = unwant_at.elapsed();
    sleep(cx, Duration::from_secs(5)).await;
    if row(&name, cx).await?.is_some_and(|r| r.pid.is_some()) {
        bail!("{name} came back after sm unwant");
    }
    println!(
        "step w: `sm want {w} {REPO}` ({}) → {name} ({}) launched by the supervisor, live after {} | its first turn ran {tools:?} itself, no park, the result lists it ({} lines) | `sm roster {w}`: {line:?} | `sm unwant` ({}) → stopped after {} [{}], not resumed 5 s later",
        wanted.lines().next().unwrap_or_default(),
        live.id,
        secs(live_after),
        result.lines().count(),
        unwanted.lines().next().unwrap_or_default(),
        secs(stopped_after),
        raw(&stopped),
    );
    Ok(())
}

const WORKSPACE_SKILL: &str = "supraman:workspace-controller";
const REPO_SKILL: &str = "supraman:repo-controller";

/// `attachment.names`, else the `- <name>: <description>` lines of `attachment.content` (CLI 2.1.309 writes both).
fn skills_listed(transcript: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in transcript.lines().filter(|l| l.contains("\"skill_listing\"")) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        let a = &v["attachment"];
        let names: Vec<String> = match a["names"].as_array() {
            Some(names) => names.iter().filter_map(|n| n.as_str().map(String::from)).collect(),
            None => a["content"]
                .as_str()
                .unwrap_or_default()
                .lines()
                .filter_map(|l| l.strip_prefix("- "))
                .filter_map(|l| l.split_once(": ").map(|(name, _)| name.to_string()))
                .collect(),
        };
        for name in names {
            if name.starts_with("supraman:") && !out.contains(&name) {
                out.push(name);
            }
        }
    }
    out.sort();
    out
}

fn transcript_turn(path: &str) -> (Option<String>, Vec<String>) {
    let Ok(text) = std::fs::read_to_string(path) else { return (None, Vec::new()) };
    let mut last = None;
    let mut tools = Vec::new();
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        if v["type"] != "assistant" {
            continue;
        }
        for c in v["message"]["content"].as_array().into_iter().flatten() {
            match (c["type"].as_str(), c["text"].as_str()) {
                (Some("text"), Some(t)) => last = Some(t.to_string()),
                (Some("tool_use"), _) => tools.push(format!(
                    "{}: {}",
                    c["name"].as_str().unwrap_or("?"),
                    c["input"]["command"].as_str().unwrap_or("")
                )),
                _ => {}
            }
        }
    }
    (last, tools)
}

fn tool_result(path: &str, matches: impl Fn(&str) -> bool) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut id = None;
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        for c in v["message"]["content"].as_array().into_iter().flatten() {
            match c["type"].as_str() {
                Some("tool_use") if id.is_none() => {
                    let call = format!("{}: {}", c["name"].as_str().unwrap_or("?"), c["input"]["command"].as_str().unwrap_or(""));
                    if matches(&call) {
                        id = c["id"].as_str().map(String::from);
                    }
                }
                Some("tool_result") if id.is_some() && c["tool_use_id"].as_str() == id.as_deref() => {
                    return Some(match &c["content"] {
                        serde_json::Value::String(s) => s.clone(),
                        serde_json::Value::Array(parts) => parts.iter().filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join("\n"),
                        other => other.to_string(),
                    });
                }
                _ => {}
            }
        }
    }
    None
}

async fn relay_step(
    tb: &Testbed,
    relay_name: &str,
    view: &Entity<FleetView>,
    cx: &mut AsyncWindowContext,
) -> anyhow::Result<()> {
    let w = tb.a.0.as_str();
    steady_step(tb, view, cx).await.context("step t0")?;

    let target = tb.target();
    let plugin = plugin_dir()?;
    let mut spec = LaunchSpec::new(&target, &tb.a.1, PROMPT_TARGET);
    spec.strict_mcp_config = true;
    spec.plugin_dirs = vec![plugin.clone()];
    let env = serde_json::json!({
        "env": { "SUPRAMAN_HOME": tb.home.display().to_string(), "FOREMAN_WORKSPACE": w, "FOREMAN_SESSION": target },
        "worktree": { "bgIsolation": "none" },
    });
    spec.extra_args = vec!["--settings".into(), env.to_string(), "--allowedTools".into()];
    spec.extra_args.extend(supervise::sm_allowed_tools(&plugin));
    let launch = cx.background_executor().spawn(async move { Claude::default().launch(&spec) }).await?;
    let target_row = wait_for("target live", Duration::from_secs(60), cx, |rows| {
        rows.iter().find(|s| s.name == target && s.pid.is_some()).cloned()
    })
    .await?;
    // the pipe is registered once the session is up; only ever the one this run launched
    let registered = Instant::now();
    while let Err(e) = own_peer(&target, &tb.a.1) {
        if registered.elapsed() > Duration::from_secs(60) {
            bail!("{target}: {e:#}");
        }
        sleep(cx, Duration::from_millis(250)).await;
    }

    let tag = crate::selftest::hex6();
    let nonces: Vec<String> = (1..=DIRECT_SENDS).map(|i| format!("FOREMAN-DIRECT-{tag}{i:02}")).collect();
    let direct_started = Instant::now();
    let mut not_direct = Vec::new();
    for (i, nonce) in nonces.iter().enumerate() {
        let (home, ws, to) = (tb.home.clone(), w.to_string(), target.clone());
        let body = format!("{nonce} direct delivery {}/{DIRECT_SENDS} from the selftest", i + 1);
        // launched above with LaunchSpec's default mode, auto
        let sent = cx.background_executor().spawn(async move { send_from_human(&home, &ws, &to, Some("auto"), &body, None) }).await?;
        if !sent.is_direct() {
            not_direct.push(sent.label());
        }
        sleep(cx, Duration::from_secs(1)).await;
    }
    if !not_direct.is_empty() {
        bail!("{} of {DIRECT_SENDS} sends did not go direct: {not_direct:?}", not_direct.len());
    }
    let sent_in = direct_started.elapsed();
    let answered = loop {
        let bodies: Vec<String> =
            read_outbox(&tb.home, w)?.into_iter().filter(|m| m.from_name == target && !m.is_notice()).map(|m| m.body).collect();
        let answered: Vec<&String> = nonces.iter().filter(|n| bodies.iter().any(|b| b.contains(n.as_str()))).collect();
        if answered.len() == nonces.len() {
            break answered.len();
        }
        if direct_started.elapsed() > Duration::from_secs(360) {
            let missing: Vec<&String> = nonces.iter().filter(|n| !answered.contains(n)).collect();
            bail!("{target} answered {} of {DIRECT_SENDS} direct nonces within 360 s; missing (held or dropped) {missing:?}", answered.len());
        }
        sleep(cx, Duration::from_millis(250)).await;
    };
    let direct_round_trip = direct_started.elapsed();
    let still_direct = no_relay(tb, relay_name, view, cx).await.context("after the direct sends")?;

    // forced fallback: a registry dir with no entry, so the relay carries it
    let empty = tb.home.join("no-sessions");
    std::fs::create_dir_all(&empty)?;
    let nonce = format!("FOREMAN-RELAY-{}", crate::selftest::hex6());
    let sent_at = Instant::now();
    let fallback =
        Sender { sessions: Some(empty), ..Sender::default() }.send(&tb.home, w, &target, Some("auto"), &format!("{nonce} ping from the selftest\nsecond line"))?;
    let Route::Relay { reason, drop } = &fallback.route else {
        bail!("the forced fallback went {:?}", fallback.route);
    };
    if !reason.starts_with("no session named") {
        bail!("the forced fallback's reason is {reason:?}");
    }
    let label = |cx: &mut AsyncWindowContext| view.read_with(cx, |v, _| v.relays.get(w).and_then(|r| r.label(now_ms())).map(|(t, _)| t));
    let mut starting_after = None;
    let mut ready_after = None;
    let mut labels = Vec::new();
    while ready_after.is_none() {
        let now = label(cx);
        if labels.last() != Some(&now) {
            labels.push(now.clone());
        }
        match now.as_deref() {
            Some("relay starting") => {
                starting_after.get_or_insert(sent_at.elapsed());
            }
            Some("relay ready") if relay_ready(&tb.home, w).is_some() => ready_after = Some(sent_at.elapsed()),
            _ => {}
        }
        if sent_at.elapsed() > Duration::from_secs(120) {
            bail!("the relay was not ready within 120 s of the drop: row labels {labels:?}, relay row {:?}", row(relay_name, cx).await?.map(|r| raw(&r)));
        }
        sleep(cx, Duration::from_millis(100)).await;
    }
    let Some(starting_after) = starting_after else {
        bail!("the row never showed `relay starting` between the drop and `relay ready`: {labels:?}");
    };
    let relay_row = row(relay_name, cx).await?.ok_or_else(|| anyhow!("{relay_name} is ready but not listed"))?;
    let glyph = format!("relay-{w}");
    wait_painted(view, &glyph, Duration::from_secs(5), cx).await.context("the relay glyph")?;
    let relay_rows: Vec<String> = view.read_with(cx, |v, _| v.hits.borrow().keys().filter(|k| k.starts_with(&format!("session-row-{}", relay_row.session_id))).cloned().collect());
    if !relay_rows.is_empty() {
        bail!("{relay_name} is ready and has a tree row {relay_rows:?}: the relay is only the workspace row's glyph");
    }
    let reply = loop {
        let found = read_outbox(&tb.home, w)?
            .into_iter()
            .find(|m| m.body.contains(&nonce) && !m.is_notice());
        if let Some(m) = found {
            break m;
        }
        if sent_at.elapsed() > Duration::from_secs(120) {
            let notices: Vec<String> = read_outbox(&tb.home, w)?.into_iter().filter(|m| m.is_notice()).map(|m| m.body).collect();
            bail!("no outbox message with {nonce} within 120 s of the relay drop (notices: {notices:?}; relay row {:?})", row(relay_name, cx).await?.map(|r| raw(&r)));
        }
        sleep(cx, Duration::from_millis(100)).await;
    };
    let round_trip = sent_at.elapsed();
    let shown = loop {
        let shown = view.read_with(cx, |v, _| v.messages.get(w).is_some_and(|m| m.iter().any(|m| m.file == reply.file)));
        if shown || sent_at.elapsed() > round_trip + Duration::from_secs(10) {
            break shown;
        }
        sleep(cx, Duration::from_millis(100)).await;
    };
    let shown_after = sent_at.elapsed();
    if !shown {
        bail!("{} is in the outbox but not in the app's Messages", reply.file);
    }
    if reply.from_name != target {
        bail!("{} came from {:?}, not {target}", reply.file, reply.from_name);
    }
    let attach_key = format!("msg-attach-{w}/{}", reply.file);
    wait_painted(view, &attach_key, Duration::from_secs(5), cx).await?;
    // An idle target can keep `state=working`, which the busy budget counts: step f's limit of 1 would wait on it for ever.
    Claude::default().stop(&target_row.id).context("stop the target")?;

    let retired = loop {
        let desired = DesiredState::load(&tb.home, w)?.ok_or_else(|| anyhow!("{w} has no desired state"))?;
        let stopped = row(relay_name, cx).await?.is_some_and(|r| r.pid.is_none());
        if !desired.wants_relay() && desired.retired.iter().any(|n| n == relay_name) && stopped && label(cx).as_deref() == Some("direct") {
            break sent_at.elapsed();
        }
        if sent_at.elapsed() > Duration::from_secs(RELAY_IDLE_SECS + 90) {
            bail!(
                "{relay_name} not retired and stopped within {} s of the drop (idle timeout {RELAY_IDLE_SECS} s): wanted {}, row {:?}, label {:?}",
                RELAY_IDLE_SECS + 90,
                desired.wants_relay(),
                row(relay_name, cx).await?.map(|r| raw(&r)),
                label(cx)
            );
        }
        sleep(cx, Duration::from_millis(500)).await;
    };
    if Duration::from_secs(RELAY_IDLE_SECS) > retired {
        bail!("{relay_name} retired {} after the drop, before the idle timeout of {RELAY_IDLE_SECS} s", secs(retired));
    }
    println!(
        "step r: {still_direct} | launched {target} ({}, `{}`) | direct: {DIRECT_SENDS} sends into its pipe in {}, all `sent direct`, {answered} of {DIRECT_SENDS} nonces answered with sm tell after {} (none held or dropped) | fallback (empty registry dir): `{}` → drop {} → row `relay starting` after {}, {relay_name} ({}) armed and `relay ready` after {} (labels {labels:?}), glyph {glyph} painted, no row of its own → outbox {} from {} with {nonce} after {} (round trip), in the app's Messages after {} with Attach | idle {RELAY_IDLE_SECS} s → retired, stopped, row `direct` {} after the drop",
        target_row.id,
        launch.lines().next().unwrap_or_default(),
        secs(sent_in),
        secs(direct_round_trip),
        fallback.label(),
        drop.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default(),
        secs(starting_after),
        relay_row.id,
        secs(ready_after.unwrap_or_default()),
        reply.file,
        reply.from_name,
        secs(round_trip),
        secs(shown_after),
        secs(retired),
    );
    Ok(())
}

/// The workspace row is the workspace session's: its Attach icon, and its menu holding the session's items.
async fn workspace_row_step(tb: &Testbed, ctrl: &str, own: &Session, view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let w = tb.a.0.as_str();
    let attach = format!("attach-ws-{w}");
    wait_painted(view, &attach, Duration::from_secs(10), cx).await?;
    let own_rows: Vec<String> = view.read_with(cx, |v, _| v.hits.borrow().keys().filter(|k| k.starts_with(&format!("session-row-{}", own.session_id))).cloned().collect());
    if !own_rows.is_empty() {
        bail!("{ctrl} has a tree row {own_rows:?}: the workspace row is its row");
    }
    crate::selftest_manage::press_painted(view, &format!("ws-row-{w}"), gpui::MouseButton::Right, cx).await?;
    let keys = crate::selftest_manage::menu_open(view, w, cx).await?;
    let caption = view.read_with(cx, |v, _| v.menu.as_ref().and_then(|m| m.items.iter().find_map(|i| i.section.clone())));
    keystroke(cx, "escape", None, Modifiers::default())?;
    let closed = Instant::now();
    while view.read_with(cx, |v, _| v.menu.is_some()) {
        if closed.elapsed() > Duration::from_secs(5) {
            bail!("the workspace menu stayed open after Esc");
        }
        sleep(cx, Duration::from_millis(50)).await;
    }
    for want in ["stop", "work", "task", "session-attach", "session-stop", "session-copy-name", "session-copy-id", "session-star"] {
        if !keys.contains(&want) {
            bail!("the menu of {w} has {keys:?}, no {want}");
        }
    }
    if caption.as_deref() != Some(format!("workspace session · {ctrl}").as_str()) {
        bail!("the menu's first section is {caption:?}, not the workspace session {ctrl}");
    }
    Ok(format!("no {ctrl} row; the workspace row paints {attach} and its menu {keys:?}"))
}

async fn no_relay(tb: &Testbed, relay_name: &str, view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let w = tb.a.0.as_str();
    let started = Instant::now();
    let label = loop {
        let label = view.read_with(cx, |v, _| v.relays.get(w).and_then(|r| r.label(now_ms())));
        if label.as_ref().is_some_and(|(t, _)| t == "direct") || started.elapsed() > Duration::from_secs(10) {
            break label;
        }
        sleep(cx, Duration::from_millis(250)).await;
    };
    if let Some(r) = row(relay_name, cx).await? {
        bail!("{relay_name} has a row although no message fell back: {}", raw(&r));
    }
    let desired = DesiredState::load(&tb.home, w)?.ok_or_else(|| anyhow!("{w} has no desired state"))?;
    if desired.wants_relay() || !desired.retired.iter().any(|n| n == relay_name) {
        bail!("the desired state wants the relay ({}) or did not retire it: retired {:?}", desired.wants_relay(), desired.retired);
    }
    if label.as_ref().map(|(t, _)| t.as_str()) != Some("direct") {
        bail!("the workspace row shows {label:?}, not `direct`");
    }
    Ok(format!("no {relay_name} row, the old desired state's relay retired, row `direct`"))
}

/// As gpui_windows makes them: `enter` for a new line, `space`, else the character itself.
fn keystroke(cx: &mut AsyncWindowContext, key: &str, key_char: Option<&str>, modifiers: Modifiers) -> anyhow::Result<()> {
    let event = KeyDownEvent {
        keystroke: Keystroke { modifiers, key: key.into(), key_char: key_char.map(String::from) },
        is_held: false,
        prefer_character_input: false,
    };
    cx.update(|window, cx| {
        window.dispatch_event(PlatformInput::KeyDown(event), cx);
    })?;
    Ok(())
}

/// Each character a keystroke that no binding takes, which gpui then hands to the input's handler as a real key's WM_CHAR is.
fn type_text(cx: &mut AsyncWindowContext, text: &str) -> anyhow::Result<()> {
    for ch in text.chars() {
        let c = ch.to_string();
        let keystroke = match ch {
            '\n' => {
                keystroke(cx, "enter", None, Modifiers::default())?;
                continue;
            }
            ' ' => Keystroke { modifiers: Modifiers::default(), key: "space".into(), key_char: Some(c) },
            _ => Keystroke { modifiers: Modifiers { shift: ch.is_uppercase(), ..Default::default() }, key: c.to_lowercase(), key_char: Some(c) },
        };
        cx.update(|window, cx| window.dispatch_keystroke(keystroke, cx))?;
    }
    Ok(())
}

fn compose_state(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> Option<(String, std::ops::Range<usize>)> {
    view.read_with(cx, |v, cx| v.compose.as_ref().map(|c| (c.text(cx), c.input.read(cx).line.selection())))
}

/// Only once a frame painted it focused is its input handler the window's, which typed characters go to.
async fn until_compose_focused(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let started = Instant::now();
    loop {
        let ready = cx.update(|window, cx| {
            view.read(cx).compose.as_ref().is_some_and(|c| {
                let input = c.input.read(cx);
                input.is_focused(window) && input.caret_painted.get()
            })
        })?;
        if ready {
            return Ok(());
        }
        if started.elapsed() > Duration::from_secs(5) {
            bail!("the task box's input was not painted focused within 5 s");
        }
        sleep(cx, Duration::from_millis(50)).await;
    }
}

/// The drops in the inbox and in its `sent/` (where the relay's watcher moves each one it sent).
fn drops(home: &std::path::Path, w: &str) -> anyhow::Result<Vec<(String, RelayDrop)>> {
    let inbox = inbox_dir(home, w)?;
    let mut out = Vec::new();
    for dir in [inbox.clone(), inbox.join(SENT_DIR)] {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if let Some(d) = name
                .ends_with(".json")
                .then(|| std::fs::read_to_string(e.path()).ok())
                .flatten()
                .and_then(|t| serde_json::from_str::<RelayDrop>(&t).ok())
            {
                out.push((name, d));
            }
        }
    }
    Ok(out)
}

/// A selftest writes into no other session's pipe.
fn own_peer(name: &str, cwd: &std::path::Path) -> anyhow::Result<peer::PeerTarget> {
    let prefix = format!("fm-selftest-{}-", testbed()?.hex);
    if !name.starts_with(&prefix) {
        bail!("{name} is not this run's session ({prefix}…)");
    }
    let sessions = peer::sessions_dir().ok_or_else(|| anyhow!("no home directory for ~/.claude/sessions"))?;
    let t = peer::find_session(&sessions, name, &peer::System)?;
    let same = |p: &std::path::Path| std::fs::canonicalize(p).ok();
    if t.cwd.as_deref().and_then(same) != same(cwd) {
        bail!("{name} (pid {}) registered cwd {:?}, not {}", t.pid, t.cwd, cwd.display());
    }
    Ok(t)
}

fn direct_id(label: &str) -> anyhow::Result<String> {
    label
        .strip_prefix("sent direct [")
        .and_then(|rest| rest.strip_suffix(']'))
        .map(String::from)
        .ok_or_else(|| anyhow!("{label:?} is not a direct send"))
}

/// JSON-escaped, as the transcript stores a message's content.
fn in_transcript(transcript: &str, w: &str, text: &str, id: &str) -> bool {
    let framed = serde_json::to_string(&supraman_claude::human::framed(w, text, id)).unwrap_or_default();
    std::fs::read_to_string(transcript).is_ok_and(|t| t.contains(framed.trim_matches('"')))
}

fn ledger_count(home: &std::path::Path, w: &str, to: &str) -> anyhow::Result<usize> {
    let path = supraman_core::relay_dir(home, w)?.join(supraman_claude::human::LEDGER_FILE);
    let v: serde_json::Value = std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    Ok(v["sent"].as_array().map_or(0, |a| a.iter().filter(|e| e["to"] == to).count()))
}

async fn wait_delivered(transcript: &str, w: &str, text: &str, id: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<Duration> {
    let started = Instant::now();
    while !in_transcript(transcript, w, text, id) {
        if started.elapsed() > Duration::from_secs(90) {
            bail!("{transcript} has no framed {text:?} within 90 s");
        }
        sleep(cx, Duration::from_millis(250)).await;
    }
    Ok(started.elapsed())
}

async fn steady_step(tb: &Testbed, view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let w = tb.a.0.as_str();
    let task_key = format!("task-{w}");
    wait_painted(view, &task_key, Duration::from_secs(10), cx).await?;
    let outbox = outbox_dir(&tb.home, w)?;
    let (steady, aimed) = crate::selftest_left::steady_click(view, &task_key, &outbox, &workspace_session_name(w), cx).await?;
    println!("step t0: {steady}");
    wait_painted(view, "compose-box", Duration::from_secs(5), cx).await?;
    let table = crate::selftest_left::table_lines(view, &task_key, aimed, cx).await?;
    println!("step t0: {table}");
    view.update(cx, |v, cx| {
        v.compose = None;
        cx.notify();
    });
    Ok(())
}

async fn task_box_step(tb: &Testbed, transcript: &str, view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let w = tb.a.0.as_str();
    let ctrl = workspace_session_name(w);
    let task_key = format!("task-{w}");
    wait_painted(view, &task_key, Duration::from_secs(10), cx).await?;
    click(view, &task_key, cx).await?;
    wait_painted(view, "compose-box", Duration::from_secs(5), cx).await?;
    let open = cx.update(|window, cx| {
        view.read(cx).compose.as_ref().map(|c| (c.workspace.clone(), c.reply_to.clone(), c.input.read(cx).is_focused(window)))
    })?;
    if open != Some((w.to_string(), None, true)) {
        bail!("Task… opened {open:?}, not a focused task box for {w}");
    }
    until_compose_focused(view, cx).await?;
    own_peer(&ctrl, &tb.a.1).context("the workspace session's registry entry")?;
    let nonce = format!("FOREMAN-TASKBOX-{}", crate::selftest::hex6());
    // the workspace session reads this as the human's: it must not start work
    let first = format!("{nonce} Automated Supraman selftest of the task box. This is not a task.");
    let task = format!("{first}\nDo nothing and send nothing.");
    let typed = format!("{first}\nDo nothing and send WRONG.");
    type_text(cx, &typed)?;
    let state = compose_state(view, cx);
    if state.as_ref().map(|(t, s)| (t.as_str(), s.clone())) != Some((typed.as_str(), typed.len()..typed.len())) {
        bail!("the box holds {state:?} after typing {typed:?}");
    }
    sleep(cx, Duration::from_millis(100)).await;
    keystroke(cx, "up", None, Modifiers::default())?;
    let up = compose_state(view, cx).map(|(_, s)| s.end);
    keystroke(cx, "down", None, Modifiers::default())?;
    let down = compose_state(view, cx).map(|(_, s)| s.end);
    if !up.is_some_and(|c| c <= first.len()) || !down.is_some_and(|c| c > first.len()) {
        bail!("Up put the caret at {up:?} (line 1 ends at {}), Down at {down:?}", first.len());
    }
    keystroke(cx, "end", None, Modifiers { control: true, ..Default::default() })?;
    keystroke(cx, "left", None, Modifiers { control: true, shift: true, ..Default::default() })?;
    let selected = compose_state(view, cx).map(|(t, s)| t[s].to_string());
    if selected.as_deref() != Some("WRONG.") {
        bail!("Ctrl+End, Ctrl+Shift+Left selected {selected:?}, not \"WRONG.\"");
    }
    type_text(cx, "nothing.")?;
    let replaced = compose_state(view, cx).map(|(t, _)| t);
    if replaced.as_deref() != Some(task.as_str()) {
        bail!("typing over the selection left {replaced:?}, expected {task:?}");
    }
    println!("step t: two lines typed; Up → {up:?}, Down → {down:?}; Ctrl+Shift+Left selected \"WRONG.\", typed over");
    keystroke(cx, "enter", None, Modifiers::control())?;
    let after = view.read_with(cx, |v, cx| v.compose.as_ref().map(|c| (c.text(cx), c.sent.clone(), c.route.clone(), c.error.clone())));
    let Some((left, Some(sent), Some(route), None)) = after else {
        bail!("after Ctrl+Enter the box is {after:?}, expected open, cleared, `sent` set and no error");
    };
    if !left.is_empty() || !sent.starts_with(&nonce) {
        bail!("after Ctrl+Enter the box holds {left:?}, sent {sent:?} ({route})");
    }
    let task_id = direct_id(&route)?;
    let task_in = wait_delivered(transcript, w, &task, &task_id, cx).await?;

    let file = format!("{:013}-selftest-approve.json", now_ms());
    let body = format!("APPROVE? selftest plan {nonce} — 1 repo, 1 unit, 0 need a decision from you\n(injected by the selftest)");
    let outbox = outbox_dir(&tb.home, w)?;
    let text = serde_json::json!({"from": "uds:selftest", "from_name": ctrl, "body": body}).to_string();
    std::fs::write(outbox.join(format!("{file}.tmp")), text)?;
    std::fs::rename(outbox.join(format!("{file}.tmp")), outbox.join(&file))?;
    let key = format!("{w}/{file}");
    let approve_key = format!("msg-approve-{key}");
    wait_painted(view, &approve_key, Duration::from_secs(10), cx).await?;
    if !view.read_with(cx, |v, _| v.hits.borrow().contains_key(&format!("msg-reply-{key}"))) {
        bail!("the APPROVE? row has Approve but no Reply…");
    }
    crate::selftest_manage::shot("m5-14-messages", cx).await;
    click(view, &approve_key, cx).await?;
    wait_painted(view, &format!("msg-answered-{key}"), Duration::from_secs(5), cx).await?;
    if view.read_with(cx, |v, _| v.hits.borrow().contains_key(&approve_key)) {
        bail!("the answered row still offers Approve");
    }
    let approve_route = view.read_with(cx, |v, _| v.answered_route.get(&key).cloned()).unwrap_or_default();
    let approve_id = direct_id(&approve_route)?;
    let approve_in = wait_delivered(transcript, w, "approve", &approve_id, cx).await?;
    let relayed: Vec<String> = drops(&tb.home, w)?.into_iter().filter(|(_, d)| d.to == ctrl).map(|(f, _)| f).collect();
    if !relayed.is_empty() {
        bail!("direct sends left relay drops for {ctrl}: {relayed:?}");
    }
    let saved = read_answered(&tb.home, w)?;
    if saved.get(&file).map(String::as_str) != Some("approve") {
        bail!("answered.json has {saved:?}, not {file} → approve");
    }

    // The Approve click gave the keys to the window: a click in the box takes them back.
    click(view, "compose-text", cx).await?;
    until_compose_focused(view, cx).await?;
    type_text(cx, "typed, not sent")?;
    let held = compose_state(view, cx).map(|(t, _)| t);
    if held.as_deref() != Some("typed, not sent") {
        bail!("the box holds {held:?} after a click in it and typing \"typed, not sent\"");
    }
    let hold = std::env::var("FOREMAN_SELFTEST_HOLD_SECS").ok().and_then(|s| s.parse::<u64>().ok());
    if let Some(secs) = hold {
        std::fs::write(tb.home.join("hold"), "")?;
        println!("step t: holding {secs} s for a screenshot");
        sleep(cx, Duration::from_secs(secs)).await;
    }
    let before = (drops(&tb.home, w)?.len(), ledger_count(&tb.home, w, &ctrl)?);
    keystroke(cx, "escape", None, Modifiers::default())?;
    sleep(cx, Duration::from_millis(500)).await;
    let closed = view.read_with(cx, |v, _| v.compose.is_none());
    let after = (drops(&tb.home, w)?.len(), ledger_count(&tb.home, w, &ctrl)?);
    if !closed || after != before || before.1 != 2 {
        bail!("Escape: box closed={closed}, (relay drops, direct sends) {before:?} → {after:?}, expected 2 direct sends");
    }
    // A later step's kill between the two prompts is a kill mid-turn, which the daemon resumes instead of the supervisor.
    const SETTLED: Duration = Duration::from_secs(15);
    let started = Instant::now();
    let mut idle_since: Option<Instant> = None;
    loop {
        let delivered = in_transcript(transcript, w, &task, &task_id) && in_transcript(transcript, w, "approve", &approve_id);
        let idle = delivered && row(&ctrl, cx).await?.is_some_and(|r| r.pid.is_some() && r.health(now_ms()) == Health::Idle);
        idle_since = if idle { idle_since.or(Some(Instant::now())) } else { None };
        if idle_since.is_some_and(|t| t.elapsed() >= SETTLED) {
            break;
        }
        if started.elapsed() > Duration::from_secs(180) {
            bail!("{ctrl} not idle for {} s within 180 s of the sends (both in its transcript: {delivered})", SETTLED.as_secs());
        }
        sleep(cx, Duration::from_millis(500)).await;
    }
    println!(
        "step t: Task… → box for {w}; {} chars typed (2 lines), Ctrl+Enter → `sent direct [{task_id}]`, box cleared, in {ctrl}'s transcript framed as the human's after {} | APPROVE? injected as {file} from {ctrl} → Approve + Reply… shown; Approve clicked → `answered: approve · sent direct [{approve_id}]`, answered.json has it, in the transcript after {}, no relay drop | typed + Escape → closed, nothing sent | {ctrl} idle {} s after {}",
        task.chars().count(),
        secs(task_in),
        secs(approve_in),
        SETTLED.as_secs(),
        secs(started.elapsed())
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_result_of_the_matching_tool_call() {
        let dir = std::env::temp_dir().join(format!("fm-tool-result-{}", hex6()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.jsonl");
        let lines = [
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"a","name":"Skill","input":{"skill":"x"}}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"a","content":"Launching skill: x"}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"b","name":"Bash","input":{"command":"sm roster w"}}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"b","content":[{"type":"text","text":"fm-w-repo\trepo\trunning"}]}]}}"#,
        ];
        std::fs::write(&path, lines[..3].join("\n")).unwrap();
        let p = path.to_str().unwrap();
        let roster = |t: &str| t.starts_with("Bash: ") && t.contains("sm roster");
        assert_eq!(tool_result(p, roster), None, "called, no result yet");
        std::fs::write(&path, lines.join("\n")).unwrap();
        assert_eq!(tool_result(p, roster).as_deref(), Some("fm-w-repo\trepo\trunning"));
        assert_eq!(tool_result(p, |t| t.starts_with("Skill: ")).as_deref(), Some("Launching skill: x"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn reads_the_supraman_skills_of_a_skill_listing() {
        // shape of a CLI 2.1.309 transcript line, shortened
        let names = r#"{"attachment":{"type":"skill_listing","content":"- x: y\n","names":["graphify","supraman:workspace-controller","rlarch:orchestrate","supraman:repo-controller"]},"type":"attachment"}"#;
        let content = r#"{"attachment":{"type":"skill_listing","content":"- graphify (graphify-windows): Use for: x\n- supraman:repo-controller: Be the repo-controller: of one repo\n"},"type":"attachment"}"#;
        let other = r#"{"type":"user","message":{"content":"- supraman:fake: not a listing"}}"#;
        assert_eq!(skills_listed(&format!("{other}\n{names}\n")), [REPO_SKILL, WORKSPACE_SKILL]);
        assert_eq!(skills_listed(content), [REPO_SKILL]);
        assert!(skills_listed(other).is_empty());
    }
}
