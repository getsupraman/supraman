//! `--selftest-supervise`: the M2 gate, through the real UI (clicks on painted hit targets) and the
//! real `claude` CLI. `FOREMAN_HOME` = `<workspace>/target/selftest-home`; four throwaway platforms
//! rooted at fresh dirs under `<workspace>/target/` (never Nomos, never the user's registry); every
//! session is `fm-selftest-<hex>-…`. One line per step:
//!
//! - a. platforms registered: `selftest-<hex>` (testbed, plain dir: trusted, inside the checkout),
//!   `-u` (`git init`ed: untrusted by construction), `-c1`/`-c2` (plain, for the budget). The testbed's
//!   desired state also wants `fm-selftest-<hex>-park` (`--permission-mode default`, told to run one
//!   Bash command): the platform session itself is `auto`, so the park is a separate wanted session.
//! - b. Start (click) → `fm-selftest-<hex>-platform` live; its event log has SessionStart(startup) and
//!   its transcript the role tag the hook injected.
//! - c. its turn ended → its pid is killed (`taskkill /PID`, that pid only) → the row drops the pid,
//!   and the supervisor resumes it: same sessionId, new pid.
//! - d. Start `-u` → Inbox TrustNeeded with Run in terminal; the click opens a pwsh tab there with
//!   `claude` typed and NOT run; the tab is closed. Nothing is accepted.
//! - e. the park session → Inbox Blocked with Attach.
//! - f. busy limit set to 1 with the stepper, Start `-c1` and `-c2` → never more than 1 of this run's
//!   sessions `working`, and both get launched.
//! - g. Stop (click) → the testbed's sessions stopped, rows kept.
//!
//! Cleanup (every exit path, see `selftest::cleanup`): supervisor off, then stop + rm every row named
//! `fm-selftest-<hex>…`, then the dirs.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use anyhow::{Context as _, anyhow, bail};
use foreman_claude::{Health, Session, SessionState, check_supported, list_sessions, now_ms};
use foreman_core::{EventReader, Platform, Registry, Settings, role_tag};
use foreman_supervisor::{
    DesiredState, NeedsKind, PLATFORM_ROLE, Remedy, WantedSession, is_live, platform_session_name, render_role,
    role_prompt,
};
use gpui::{AsyncWindowContext, Entity};

use crate::selftest::{
    Scratch, claude, click, hex6, install_guards, lock, raw, remove_dir_retrying, sleep, wait_for_tab_terminal,
    workspace_root,
};
use crate::view::{FleetView, process_alive};
use crate::{Mode, supervise};

const PROMPT_PARK: &str = "This is an automated Foreman selftest. Run exactly this one Bash command and nothing \
else: `echo parked > parked.txt`. Then stop.";

struct Testbed {
    hex: String,
    home: PathBuf,
    /// testbed, untrusted, budget 1, budget 2: (platform name, root)
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
}

pub fn prepare() -> anyhow::Result<()> {
    let target = workspace_root().join("target");
    let home = target.join("selftest-home");
    remove_dir_retrying(&home).with_context(|| format!("cannot clear {}", home.display()))?;
    let hex = hex6();
    let platform = |suffix: &str| {
        let name = format!("selftest-{hex}{suffix}");
        let root = target.join(&name);
        (name, root)
    };
    let tb = Testbed {
        a: platform(""),
        u: platform("-u"),
        c1: platform("-c1"),
        c2: platform("-c2"),
        home: home.clone(),
        hex: hex.clone(),
    };
    let roots = [&tb.a.1, &tb.u.1, &tb.c1.1, &tb.c2.1];
    *lock() = Some(Scratch {
        dirs: std::iter::once(home.clone()).chain(roots.iter().map(|r| (*r).clone())).collect(),
        session_name: None,
        name_prefix: Some(format!("fm-selftest-{hex}")),
    });
    install_guards(Mode::SelftestSupervise);
    // Still single-threaded apart from the watchdog, which never reads the environment.
    unsafe { std::env::set_var("FOREMAN_HOME", &home) };

    let version = check_supported()?;
    for root in roots {
        std::fs::create_dir_all(root).with_context(|| format!("create {}", root.display()))?;
    }
    let git = Command::new("git")
        .args(["init", "-q"])
        .current_dir(&tb.u.1)
        .stdin(Stdio::null())
        .output()
        .context("git init")?;
    if !git.status.success() {
        bail!("git init {}: {}", tb.u.1.display(), String::from_utf8_lossy(&git.stderr));
    }
    let registry = Registry::at(&home);
    for (name, root) in [&tb.a, &tb.u, &tb.c1, &tb.c2] {
        registry.save(&Platform::new(name.as_str(), root.as_path(), Vec::new()), true)?;
    }
    let plugin = workspace_root().join("plugins").join("foreman");
    let mut desired = DesiredState::platform_controller(&tb.a.0, &tb.a.1, role_prompt(&plugin, PLATFORM_ROLE, &tb.a.0)?);
    desired.sessions.push(WantedSession {
        name: tb.park(),
        role: "probe".into(),
        cwd: tb.a.1.clone(),
        repo: None,
        prompt: PROMPT_PARK.into(),
        permission_mode: "default".into(),
    });
    desired.save(&home)?;
    println!(
        "step a: claude {} | FOREMAN_HOME={} | platforms {} (testbed, plain dir), {} (git init: untrusted), {}, {} | {} wants {} and {} (permission mode default)",
        version.raw,
        home.display(),
        tb.a.0,
        tb.u.0,
        tb.c1.0,
        tb.c2.0,
        tb.a.0,
        platform_session_name(&tb.a.0),
        tb.park()
    );
    TESTBED.set(tb).map_err(|_| anyhow!("prepared twice"))?;
    Ok(())
}

/// Stops and removes every row whose name starts with `prefix` (this run's own), after turning the
/// app's supervisor off so it cannot resume them meanwhile. A `--discard-unpushed` refusal is
/// answered for these rows only (m2/2). True when none is left.
pub fn remove_own_rows(prefix: &str) -> bool {
    supervise::disable_and_wait();
    let rows = match list_sessions() {
        Ok(rows) => rows,
        Err(e) => {
            println!("cleanup: cannot list sessions: {e}");
            return false;
        }
    };
    let own: Vec<Session> = rows.into_iter().filter(|s| s.name.starts_with(prefix)).collect();
    for row in &own {
        if row.pid.is_some() {
            match claude(&["stop", &row.id], None) {
                Ok(out) => println!("cleanup: claude stop {} ({}): {out}", row.id, row.name),
                Err(e) => println!("cleanup: {e:#}"),
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
        match claude(&["rm", &row.id], None) {
            Ok(out) => println!("cleanup: claude rm {} ({}): {out}", row.id, row.name),
            Err(e) => {
                let refusal = format!("{e:#}");
                match discard_token(&refusal) {
                    Some(token) => match claude(&["rm", &row.id, "--discard-unpushed", token], None) {
                        Ok(out) => println!("cleanup: claude rm {} --discard-unpushed: {out}", row.id),
                        Err(e) => println!("cleanup: {e:#}"),
                    },
                    None => println!("cleanup: {refusal}"),
                }
            }
        }
    }
    match list_sessions() {
        Ok(rows) => {
            let left: Vec<String> = rows.iter().filter(|s| s.name.starts_with(prefix)).map(|s| s.name.clone()).collect();
            println!("cleanup: {} {prefix}* row(s) removed, {} left {left:?}", own.len(), left.len());
            left.is_empty()
        }
        Err(e) => {
            println!("cleanup: cannot verify removal: {e}");
            false
        }
    }
}

/// `<sha>@<token>` from rm's "… claude rm <id> --discard-unpushed <sha>@<token>".
fn discard_token(refusal: &str) -> Option<&str> {
    let rest = refusal.split_once("--discard-unpushed ")?.1;
    rest.split_whitespace().next().filter(|t| t.contains('@'))
}

async fn rows(cx: &mut AsyncWindowContext) -> anyhow::Result<Vec<Session>> {
    Ok(cx.background_executor().spawn(async { list_sessions() }).await?)
}

async fn row(name: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<Option<Session>> {
    Ok(rows(cx).await?.into_iter().find(|s| s.name == name))
}

/// Polls `claude agents --json --all` every 500 ms until `done` returns a value.
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

fn secs(d: Duration) -> String {
    format!("{:.1} s", d.as_secs_f64())
}

pub async fn run(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let tb = testbed()?;
    let ctrl = platform_session_name(&tb.a.0);

    // b. Start through the UI
    wait_painted(view, &format!("start-{}", tb.a.0), Duration::from_secs(30), cx).await.context("step b")?;
    let notices = view.read_with(cx, |v, _| v.setup_notices.clone());
    if !notices.is_empty() {
        bail!("step b: setup notices {notices:?}");
    }
    click(view, &format!("start-{}", tb.a.0), cx).await?;
    let clicked = Instant::now();
    let launched = wait_for("step b: platform session live", Duration::from_secs(60), cx, |rows| {
        rows.iter().find(|s| s.name == ctrl && s.pid.is_some()).cloned()
    })
    .await?;
    let live_after = clicked.elapsed();
    let template = std::fs::read_to_string(tb.home.join("roles").join("platform.md")).context("step b: role file")?;
    let tag = role_tag(PLATFORM_ROLE, &render_role(&template, &tb.a.0));
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
    let in_panel = view.read_with(cx, |v, _| {
        v.tree().platforms.iter().any(|p| p.name == tb.a.0 && p.supervised.iter().any(|r| r.session.name == ctrl))
    });
    let events_line = seen.iter().map(|e| e.event.clone() + &e.source.as_deref().map(|s| format!("({s})")).unwrap_or_default()).collect::<Vec<_>>().join(", ");
    if !tagged || !in_panel {
        bail!(
            "step b: {ctrl} live, but SessionStart(startup) with the role tag {tag} in its transcript={tagged}, row under its platform={in_panel}; events: {events_line}"
        );
    }
    println!(
        "step b: Start clicked → {ctrl} ({}) live with pid {} after {} | events: {events_line} | transcript has `{tag}` | row under {} in the panel",
        launched.id,
        launched.pid.unwrap_or_default(),
        secs(live_after),
        tb.a.0
    );

    // c. kill its process once its turn ended; the supervisor resumes it
    let idle = wait_for("step c: platform session idle", Duration::from_secs(120), cx, |rows| {
        rows.iter().find(|s| s.name == ctrl && s.health(now_ms()) == Health::Idle && s.pid.is_some()).cloned()
    })
    .await?;
    let pid = idle.pid.unwrap_or_default();
    let killed_at = now_ms();
    let kill = Instant::now();
    let out = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .stdin(Stdio::null())
        .output()
        .context("taskkill")?;
    if !out.status.success() {
        bail!("step c: taskkill /PID {pid}: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    let mut dropped = None;
    let mut lingered = 0u32;
    let resumed = loop {
        let row = row(&ctrl, cx).await?;
        match row {
            Some(r) if r.pid == Some(pid) => lingered += 1,
            Some(r) if r.pid.is_none() => {
                dropped.get_or_insert((kill.elapsed(), raw(&r)));
            }
            Some(r) if r.pid.is_some() && r.session_id == idle.session_id => break r,
            Some(r) => bail!("step c: {ctrl} came back as another session: {} ({})", r.session_id, raw(&r)),
            None => bail!("step c: {ctrl} is no longer listed"),
        }
        if kill.elapsed() > Duration::from_secs(120) {
            bail!("step c: {ctrl} not resumed within 120 s of the kill (pid dropped: {dropped:?})");
        }
        sleep(cx, Duration::from_millis(250)).await;
    };
    let resumed_after = kill.elapsed();
    sleep(cx, Duration::from_secs(3)).await;
    let attempt = view.read_with(cx, |v, _| v.last_attempt.get(&ctrl).copied());
    if !attempt.is_some_and(|t| t >= killed_at) {
        bail!("step c: {ctrl} came back with pid {:?}, but not by the supervisor (its last attempt {attempt:?})", resumed.pid);
    }
    if process_alive(pid) {
        bail!("step c: the killed pid {pid} is still running");
    }
    let (dropped_after, dropped_row) = dropped.map_or(("never seen".into(), String::new()), |(d, r)| (secs(d), r));
    println!(
        "step c: killed pid {pid} (turn ended: {}) → row without pid after {dropped_after} [{dropped_row}], dead pid seen in {lingered} poll(s) | resumed by the supervisor after {}: same sessionId {}, pid {pid} → {}",
        raw(&idle),
        secs(resumed_after),
        resumed.session_id,
        resumed.pid.unwrap_or_default()
    );

    // d. untrusted platform → TrustNeeded with Run in terminal; nothing is run or accepted
    let u_ctrl = platform_session_name(&tb.u.0);
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
    let terminal = wait_for_tab_terminal(view, tab, Duration::from_secs(10), cx).await?;
    let typed = Instant::now();
    let line = loop {
        let content = terminal.read_with(cx, |t, _| t.get_content());
        if let Some(line) = content.lines().rev().find(|l| l.contains("PS ") && l.trim_end().ends_with(&command)) {
            break line.trim().to_string();
        }
        if typed.elapsed() > Duration::from_secs(20) {
            bail!("step d: `{command}` was not typed at a prompt within 20 s: {:?}", content.lines().rev().find(|l| !l.trim().is_empty()));
        }
        sleep(cx, Duration::from_millis(100)).await;
    };
    click(view, &format!("close-tab-{tab}"), cx).await?;
    sleep(cx, Duration::from_millis(500)).await;
    let closed = view.read_with(cx, |v, _| v.tab(tab).is_none());
    println!(
        "step d: Start {} → Inbox `trust needed` after {}: {} | Run in terminal opened a pwsh tab in {} with `{line}` typed, not run; tab closed={closed}; no row named {u_ctrl}",
        tb.u.0,
        secs(clicked.elapsed()),
        need.detail,
        cwd.display()
    );
    if !closed {
        bail!("step d: the Run in terminal tab did not close");
    }

    // e. the park session → Blocked with Attach
    let park = tb.park();
    let attach_key = format!("inbox-attach-{park}");
    let started = Instant::now();
    let need = loop {
        let need = view.read_with(cx, |v, _| v.needs.iter().find(|n| n.name == park && n.kind == NeedsKind::Blocked).cloned());
        if let Some(need) = need
            && view.read_with(cx, |v, _| v.hits.borrow().contains_key(&attach_key))
        {
            break need;
        }
        if started.elapsed() > Duration::from_secs(180) {
            bail!("step e: no Blocked Inbox row for {park} within 180 s (row: {:?})", row(&park, cx).await?.map(|r| raw(&r)));
        }
        sleep(cx, Duration::from_millis(250)).await;
    };
    let Some(Remedy::Attach { id }) = &need.action else {
        bail!("step e: Blocked without Attach: {need:?}");
    };
    let park_row = row(&park, cx).await?.ok_or_else(|| anyhow!("step e: {park} not listed"))?;
    if &park_row.id != id {
        bail!("step e: Attach targets {id}, the row is {}", park_row.id);
    }
    let hook = view.read_with(cx, |v, _| v.hooks.get(&park_row.session_id).map(|h| h.label(now_ms())));
    println!(
        "step e: {park} parked → Inbox `blocked` ({}) with Attach {id} | row [{}] | last hook: {}",
        need.detail,
        raw(&park_row),
        hook.unwrap_or_else(|| "none".into())
    );

    // f. busy limit 1 (stepper), two more platforms started: never more than one working
    let mut clicks = 0;
    while view.read_with(cx, |v, _| v.max_working) > 1 {
        click(view, "budget-dec", cx).await?;
        clicks += 1;
        sleep(cx, Duration::from_millis(100)).await;
        if clicks > 30 {
            bail!("step f: the stepper did not bring the limit to 1");
        }
    }
    let saved = Settings::load(&tb.home)?.max_working;
    if saved != 1 {
        bail!("step f: settings.toml has max_working {saved} after the stepper showed 1");
    }
    let prefix = format!("fm-selftest-{}", tb.hex);
    let (c1, c2) = (platform_session_name(&tb.c1.0), platform_session_name(&tb.c2.0));
    for (name, _) in [&tb.c1, &tb.c2] {
        wait_painted(view, &format!("start-{name}"), Duration::from_secs(10), cx).await?;
        click(view, &format!("start-{name}"), cx).await?;
    }
    let started = Instant::now();
    let mut max_seen = 0;
    let mut polls = 0;
    let mut first_live: [Option<Duration>; 2] = [None, None];
    let mut first_idle: [Option<Duration>; 2] = [None, None];
    while first_live.iter().any(Option::is_none) {
        let rows = rows(cx).await?;
        let now = now_ms();
        let working: Vec<&Session> = rows
            .iter()
            .filter(|s| s.name.starts_with(&prefix) && s.state == Some(SessionState::Working) && is_live(s, now))
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
    println!(
        "step f: limit 3 → 1 by {clicks} stepper clicks (settings.toml max_working = 1) | Start {} and {}: max {max_seen} working over {polls} polls | first live {} / {}, first idle {} / {}",
        tb.c1.0,
        tb.c2.0,
        fmt(first_live[0]),
        fmt(first_live[1]),
        fmt(first_idle[0]),
        fmt(first_idle[1])
    );

    // g. Stop through the UI: stopped, rows and transcripts kept
    click(view, &format!("stop-{}", tb.a.0), cx).await?;
    let clicked = Instant::now();
    let names = [ctrl.clone(), park.clone()];
    let stopped = wait_for("step g: testbed sessions stopped", Duration::from_secs(60), cx, |rows| {
        let ours: Vec<&Session> = rows.iter().filter(|s| names.contains(&s.name)).collect();
        (ours.len() == 2 && ours.iter().all(|s| s.pid.is_none())).then(|| ours.iter().map(|s| format!("{} [{}]", s.name, raw(s))).collect::<Vec<_>>())
    })
    .await?;
    sleep(cx, Duration::from_secs(3)).await;
    let shown = view.read_with(cx, |v, _| {
        v.tree().platforms.iter().find(|p| p.name == tb.a.0).map(|p| p.supervised.len()).unwrap_or(0)
    });
    let still = rows(cx).await?.into_iter().filter(|s| names.contains(&s.name) && s.pid.is_some()).count();
    println!(
        "step g: Stop clicked → stopped after {}: {} | still listed, {shown} row(s) shown under {} | restarted since: {still}",
        secs(clicked.elapsed()),
        stopped.join(", "),
        tb.a.0
    );
    if shown != 2 || still != 0 {
        bail!("step g: expected 2 rows shown and none restarted, got {shown} shown, {still} live");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_discard_token() {
        let refusal = "claude rm 202c9998 failed (exit 1): kept 202c9998 … discard the worktree and its commits: claude rm 202c9998 --discard-unpushed 3ccd48a1@d71f54d8";
        assert_eq!(discard_token(refusal), Some("3ccd48a1@d71f54d8"));
        assert_eq!(discard_token("removed 202c9998"), None);
    }
}
