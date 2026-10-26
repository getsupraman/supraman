//! `--selftest` (the M1 gate), `--selftest-detach <n>`, `--selftest-detach-shutdown` and
//! `--selftest-perf`.
//!
//! Both run against a scratch `FOREMAN_HOME` under `<cargo root>/target/` holding one workspace: a
//! read-only scan of the folder that contains the main repo, found through the nearest `.git` above
//! the cargo workspace (P:/Projects/LarchSys → `larchsys`, from the main checkout or any linked worktree).
//! The gate also launches one `fm-selftest-<hex>` background session with its cwd in
//! `<cargo root>/target/selftest-cwd`: inside the already-trusted tree the exe was built in, so from a
//! worktree `assign` has to map it through the worktree's `.git` file to the main repo. [`cleanup`] stops and removes that session
//! (by the id of the `agents --json --all` row carrying its name) and deletes the scratch dirs on
//! every exit path: normal end, failure, watchdog, panic. The panel also shows the user's real
//! sessions in those repos; the selftest only ever acts on its own row, by name.

use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Mutex, MutexGuard, OnceLock, atomic::Ordering},
    time::{Duration, Instant},
};

use anyhow::{Context as _, anyhow, bail};
use foreman_claude::{Session, check_supported, list_sessions, main_repo, main_repo_walks, now_ms};
use foreman_core::{DEFAULT_DEPTH, Workspace, Registry, scan};
use foreman_term::Terminal;
use foreman_ui::{memory, mib};
use gpui::{
    App, AsyncWindowContext, Bounds, ClipboardItem, Entity, KeyDownEvent, Keystroke, Modifiers, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, PlatformInput, ScrollDelta, ScrollEvent, Window,
    point, px,
};

use crate::{
    EXIT_CODE, Mode,
    fleet::{self, SessionRow},
    view::{DETACH_WITHIN, Detach, FleetView, TabKind, process_alive},
};

pub(crate) mod frames;

pub(crate) use frames::injected_frame_sleep;

pub const WORKSPACE: &str = "larchsys";
const PROMPT: &str = "Reply with the word ready and then wait for input";
const LISTED_WITHIN: Duration = Duration::from_secs(15);
const RENDERED_WITHIN: Duration = Duration::from_secs(10);

pub(crate) struct Scratch {
    pub(crate) dirs: Vec<PathBuf>,
    pub(crate) session_name: Option<String>,
    /// `--selftest-supervise`: every row whose name starts with this is the run's own; cleanup
    /// first turns the app's supervisor off, then stops and removes each of them.
    pub(crate) name_prefix: Option<String>,
}

static SCRATCH: Mutex<Option<Scratch>> = Mutex::new(None);
static SESSION_NAME: Mutex<Option<String>> = Mutex::new(None);
/// The main repo's folder name, the row the gate expects its session under.
static REPO: OnceLock<String> = OnceLock::new();

fn repo() -> &'static str {
    REPO.get().map_or("", String::as_str)
}

pub(crate) fn cargo_root() -> &'static Path {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest.ancestors().nth(2).unwrap_or(manifest)
}

/// The main repo of the tree `from` lies in: the folder of the nearest `.git` directory, or the repo a
/// `.git` file's `gitdir:` points into.
fn find_main_repo(from: &Path) -> anyhow::Result<PathBuf> {
    main_repo(from).ok_or_else(|| {
        anyhow!(
            "no .git directory or worktree .git file (gitdir: <repo>/.git/worktrees/<name>) at or above {}",
            from.display()
        )
    })
}

pub(crate) fn hex6() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!("{:06x}", (nanos as u64 ^ (std::process::id() as u64) << 20) & 0xff_ffff)
}

pub(crate) fn remove_dir_retrying(dir: &Path) -> std::io::Result<()> {
    // A just-stopped session can hold a handle in its cwd for a few seconds (LEARNINGS m0/2 run).
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match std::fs::remove_dir_all(dir) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) if Instant::now() > deadline => return Err(e),
            Err(_) => std::thread::sleep(Duration::from_millis(500)),
        }
    }
}

pub(crate) fn claude(args: &[&str], cwd: Option<&Path>) -> anyhow::Result<String> {
    let mut command = Command::new("claude");
    command.args(args).stdin(Stdio::null());
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let out = command
        .output()
        .with_context(|| format!("cannot run claude {}", args.join(" ")))?;
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() {
        bail!(
            "claude {} failed ({}): {} {}",
            args.join(" "),
            out.status,
            stdout,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(stdout)
}

/// Before the window: scratch dirs, the workspace registered through foreman-core, and (gate only)
/// the throwaway session launched from the scratch cwd.
pub fn prepare(mode: Mode) -> anyhow::Result<()> {
    if mode == Mode::SelftestSupervise {
        return crate::selftest_supervise::prepare();
    }
    let target = cargo_root().join("target");
    let home = target.join("selftest-home");
    let cwd = target.join("selftest-cwd");
    let work = target.join("selftest-work");
    let add = target.join(crate::selftest_manage::FOLDER);
    for dir in [&home, &cwd, &work, &add] {
        remove_dir_retrying(dir).with_context(|| format!("cannot clear {}", dir.display()))?;
    }
    *lock() = Some(Scratch {
        dirs: vec![home.clone(), cwd.clone(), work.clone(), add],
        session_name: None,
        name_prefix: None,
    });
    install_guards(mode);

    // Still single-threaded apart from the watchdog, which never reads the environment.
    unsafe { std::env::set_var("FOREMAN_HOME", &home) };
    std::fs::create_dir_all(&cwd)?;
    // <root>/AutoWork or <root>/AutoWork.worktrees/<unit> → main repo <root>/AutoWork → <root>.
    let main = find_main_repo(cargo_root())?;
    let workspace_root = main
        .parent()
        .ok_or_else(|| anyhow!("main repo {} has no parent", main.display()))?;
    let repo_name = main
        .file_name()
        .ok_or_else(|| anyhow!("main repo {} has no folder name", main.display()))?
        .to_string_lossy()
        .into_owned();
    let registry = Registry::from_env()?;
    let repos = scan(workspace_root, DEFAULT_DEPTH)?;
    if !repos.iter().any(|r| r.name == repo_name) {
        bail!("scan of {} found no {repo_name} repo (main repo {})", workspace_root.display(), main.display());
    }
    REPO.set(repo_name).ok();
    let count = repos.len();
    let saved = registry.save(&Workspace::new(WORKSPACE, workspace_root, repos), true)?;
    println!(
        "step 1: FOREMAN_HOME={} ; workspace {WORKSPACE} = {} ({count} repos, read-only scan) registered at {}",
        home.display(),
        workspace_root.display(),
        saved.display()
    );

    if mode == Mode::Selftest {
        crate::selftest_work::prepare(&home, &work)?;
    }

    if mode.launches_session() {
        let version = check_supported()?;
        println!("claude CLI: {}", version.raw);
        if let Some(warning) = version.untested() {
            println!("WARNING {warning}");
        }
        let name = format!("fm-selftest-{}", hex6());
        // Recorded before launching, so cleanup looks for it even if the launch half-succeeds.
        if let Some(scratch) = lock().as_mut() {
            scratch.session_name = Some(name.clone());
        }
        *SESSION_NAME.lock().unwrap_or_else(|e| e.into_inner()) = Some(name.clone());
        let launched = Instant::now();
        let out = claude(
            &["--bg", "--name", &name, "--permission-mode", "auto", PROMPT],
            Some(&cwd),
        )?;
        println!(
            "step 2: launched {name} from {} in {} ms: {out}",
            cwd.display(),
            launched.elapsed().as_millis()
        );
    }
    Ok(())
}

pub(crate) fn lock() -> MutexGuard<'static, Option<Scratch>> {
    SCRATCH.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn install_guards(mode: Mode) {
    let budget = match mode {
        Mode::Selftest => Duration::from_secs(150),
        Mode::SelftestDetach(cycles) => Duration::from_secs(90 + 15 * u64::from(cycles)),
        Mode::SelftestSupervise => Duration::from_secs(900),
        _ => Duration::from_secs(120),
    };
    std::thread::spawn(move || {
        std::thread::sleep(budget);
        println!("FAIL watchdog: no result after {} s", budget.as_secs());
        EXIT_CODE.store(1, Ordering::SeqCst);
        cleanup();
        std::process::exit(1);
    });
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        previous(info);
        EXIT_CODE.store(1, Ordering::SeqCst);
        // try_lock: a panic inside cleanup itself must not deadlock here.
        if let Ok(mut guard) = SCRATCH.try_lock() {
            run_cleanup(&mut guard);
        }
    }));
}

/// Idempotent; holds the lock throughout, so a second caller waits for the first to finish.
pub fn cleanup() {
    let mut guard = lock();
    run_cleanup(&mut guard);
}

fn run_cleanup(guard: &mut Option<Scratch>) {
    let Some(scratch) = guard.take() else {
        return;
    };
    let mut ok = true;
    if let Some(prefix) = &scratch.name_prefix {
        ok &= crate::selftest_supervise::remove_own_rows(prefix);
    }
    if let Some(name) = &scratch.session_name {
        // The row can trail `--bg` by a moment, so a missing row is retried before giving up.
        let deadline = Instant::now() + Duration::from_secs(10);
        let ids: Vec<String> = loop {
            match list_sessions() {
                Ok(rows) => {
                    let ids: Vec<String> =
                        rows.iter().filter(|s| &s.name == name).map(|s| s.id.clone()).collect();
                    if !ids.is_empty() || Instant::now() > deadline {
                        break ids;
                    }
                }
                Err(e) if Instant::now() > deadline => {
                    println!("cleanup: cannot list sessions: {e}");
                    ok = false;
                    break Vec::new();
                }
                Err(_) => {}
            }
            std::thread::sleep(Duration::from_millis(500));
        };
        if ids.is_empty() {
            println!("cleanup: no row named {name} in claude agents --json --all");
        }
        for id in &ids {
            for verb in ["stop", "rm"] {
                match claude(&[verb, id], None) {
                    Ok(out) => println!("cleanup: claude {verb} {id}: {out}"),
                    Err(e) => {
                        println!("cleanup: {e:#}");
                        ok = false;
                    }
                }
            }
        }
        match list_sessions() {
            Ok(rows) if rows.iter().any(|s| &s.name == name) => {
                println!("cleanup: {name} is still listed after rm");
                ok = false;
            }
            Ok(_) => println!("cleanup: {name} no longer listed"),
            Err(e) => {
                println!("cleanup: cannot verify removal: {e}");
                ok = false;
            }
        }
    }
    for dir in &scratch.dirs {
        match remove_dir_retrying(dir) {
            Ok(()) => println!("cleanup: removed {}", dir.display()),
            Err(e) => {
                println!("cleanup: cannot remove {}: {e}", dir.display());
                ok = false;
            }
        }
    }
    if !ok {
        EXIT_CODE.store(1, Ordering::SeqCst);
    }
}

pub fn start(mode: Mode, view: Entity<FleetView>, window: &mut Window, cx: &mut App) {
    window
        .spawn(cx, async move |cx| {
            let result = match mode {
                Mode::Selftest => gate(&view, cx).await,
                Mode::SelftestDetach(cycles) => detach_cycles(&view, cycles, cx).await,
                Mode::SelftestDetachShutdown => detach_on_shutdown(&view, cx).await,
                Mode::SelftestSupervise => crate::selftest_supervise::run(&view, cx).await,
                _ => perf(&view, cx).await,
            };
            // The window closed itself; `finish` gives the verdict once the event loop has ended.
            if mode == Mode::SelftestDetachShutdown && result.is_ok() {
                return;
            }
            match result {
                Ok(()) => println!("PASS"),
                Err(e) => {
                    println!("FAIL {e:#}");
                    EXIT_CODE.store(1, Ordering::SeqCst);
                }
            }
            // A shell tab left open (a failed step) would hold its cwd, a scratch dir.
            let shells: Vec<u64> = view.read_with(cx, |v, _| {
                v.tabs.iter().filter(|t| matches!(t.kind, TabKind::Shell)).map(|t| t.id).collect()
            });
            view.update(cx, |v, cx| shells.into_iter().for_each(|id| v.close_tab(id, cx)));
            // Off the UI thread: cleanup blocks on `claude` processes and file deletes.
            cx.background_executor().spawn(async { cleanup() }).await;
            cx.update(|_, cx| cx.quit()).ok();
        })
        .detach();
}

pub(crate) async fn sleep(cx: &AsyncWindowContext, duration: Duration) {
    cx.background_executor().timer(duration).await;
}

/// A left click at the centre of the element painted under `key`, after scrolling it fully into view
/// ([`scrolled_into_view`]), through the window's own input dispatch, so it lands only if that element is
/// really there and hit-testable.
pub(crate) async fn click(view: &Entity<FleetView>, key: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    press(view, key, MouseButton::Left, cx).await
}

/// [`click`] with any button: `MouseButton::Right` opens a row's context menu.
pub(crate) async fn press(view: &Entity<FleetView>, key: &str, button: MouseButton, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let position = scrolled_into_view(view, key, cx).await?.center();
    let modifiers = Modifiers::default();
    cx.update(|window, cx| {
        window.dispatch_event(
            PlatformInput::MouseMove(MouseMoveEvent {
                position,
                pressed_button: None,
                modifiers,
            }),
            cx,
        );
        window.dispatch_event(
            PlatformInput::MouseDown(MouseDownEvent {
                button,
                position,
                modifiers,
                click_count: 1,
                first_mouse: false,
            }),
            cx,
        );
        window.dispatch_event(
            PlatformInput::MouseUp(MouseUpEvent {
                button,
                position,
                modifiers,
                click_count: 1,
            }),
            cx,
        );
    })?;
    Ok(())
}

/// The bounds painted under `key`, once **fully** inside its visible area ([`Hit::visible`], the clip of the
/// scroll container around it): an element scrolled out of its container is painted but takes no click, so
/// that container is scrolled with the wheel, from inside its visible area, until the element is in view.
/// Fails with every bounds it saw when it never gets there (m3/3: the tree once could not scroll at all).
pub(crate) async fn scrolled_into_view(view: &Entity<FleetView>, key: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<Bounds<Pixels>> {
    let margin = px(4.);
    let mut seen = Vec::new();
    for _ in 0..20 {
        // a refresh can rebuild the tree and drop the row from a frame or two, more often on a busy machine (m5/15)
        let missing = Instant::now();
        let hit = loop {
            if let Some(hit) = view.read_with(cx, |v, _| v.hits.borrow().get(key).copied()) {
                break hit;
            }
            if missing.elapsed() > Duration::from_secs(2) {
                bail!("nothing painted under {key} for 2 s (seen before: {})", if seen.is_empty() { "nothing".into() } else { seen.join(" → ") });
            }
            sleep(cx, Duration::from_millis(50)).await;
        };
        let (b, v) = (hit.bounds, hit.visible);
        seen.push(format!("{}..{} in {}..{}", f32::from(b.top()), f32::from(b.bottom()), f32::from(v.top()), f32::from(v.bottom())));
        if hit.fully_visible() {
            if seen.len() > 1 {
                println!("click: scrolled {key} into view: {}", seen.join(" → "));
            }
            return Ok(b);
        }
        if b.size.height > v.size.height || b.size.width > v.size.width {
            bail!("{key} ({b:?}) is larger than its visible area {v:?}");
        }
        let dy = if b.bottom() > v.bottom() { v.bottom() - margin - b.bottom() } else { v.top() + margin - b.top() };
        // a scroll goes to the scroll container under the pointer, so it is sent from inside the visible area
        let position = v.center();
        cx.update(|window, cx| {
            window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position, pressed_button: None, modifiers: Modifiers::default() }), cx);
            let event = ScrollEvent { position, delta: ScrollDelta::Pixels(point(px(0.), dy)), ..Default::default() };
            window.dispatch_event(PlatformInput::Scroll(event), cx);
        })?;
        sleep(cx, Duration::from_millis(300)).await;
    }
    bail!("{key} is not fully inside its visible area after scrolling: {}", seen.join(" → "))
}

fn find_row(view: &FleetView, name: &str) -> Option<SessionRow> {
    view.tree()
        .workspaces
        .into_iter()
        .find(|p| p.name == WORKSPACE)?
        .repos
        .into_iter()
        .find(|r| r.name == repo())?
        .sessions
        .into_iter()
        .find(|r| r.session.name == name)
}

/// Non-blank cells in what the painter last laid out (prepaint syncs `last_content`).
fn painted_cells(terminal: &Entity<Terminal>, cx: &mut AsyncWindowContext) -> usize {
    terminal.read_with(cx, |t, _| {
        t.last_content()
            .cells
            .iter()
            .filter(|c| !c.cell.character().is_whitespace())
            .count()
    })
}

fn first_line(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default();
    line.chars().take(80).collect()
}

pub(crate) async fn wait_for_tab_terminal(
    view: &Entity<FleetView>,
    tab: u64,
    within: Duration,
    cx: &mut AsyncWindowContext,
) -> anyhow::Result<Entity<Terminal>> {
    let deadline = Instant::now() + within;
    loop {
        let (terminal, error) = view.read_with(cx, |v, _| {
            v.tab(tab)
                .map(|t| (t.terminal.clone(), t.error.clone()))
                .unwrap_or_default()
        });
        if let Some(error) = error {
            bail!("tab {tab} failed to start its terminal: {error}");
        }
        if let Some(terminal) = terminal {
            return Ok(terminal);
        }
        if Instant::now() > deadline {
            bail!("tab {tab} has no terminal after {} ms", within.as_millis());
        }
        sleep(cx, Duration::from_millis(20)).await;
    }
}

/// The session's row by name, alive by health and by its own pid. `state` describes the turn, not
/// the process, so `done` with a live pid is alive (LEARNINGS m1/7).
fn session_alive_in_cli(name: &str) -> anyhow::Result<Session> {
    let rows = list_sessions()?;
    let row = rows
        .into_iter()
        .find(|s| s.name == name)
        .ok_or_else(|| anyhow!("{name} is no longer listed in claude agents --json --all"))?;
    let health = row.health(now_ms());
    if !fleet::is_alive(&health) {
        bail!("{name} is listed but {health} ({}) — the close stopped it instead of detaching", raw(&row));
    }
    let pid = row.pid.ok_or_else(|| anyhow!("{name} is {health} but has no pid ({})", raw(&row)))?;
    if !process_alive(pid) {
        bail!("{name}'s pid {pid} is not a running process ({})", raw(&row));
    }
    Ok(row)
}

/// The row's own fields, as `claude agents --json --all` gave them.
pub(crate) fn raw(s: &Session) -> String {
    format!(
        "state={} status={} pid={}{}",
        s.state.as_ref().map_or("-", |s| s.as_str()),
        match &s.status {
            None => "-".to_string(),
            Some(status) => format!("{status:?}").to_lowercase(),
        },
        s.pid.map_or("-".to_string(), |p| p.to_string()),
        s.waiting_for.as_deref().map(|w| format!(" waitingFor={w}")).unwrap_or_default()
    )
}

fn session_name() -> anyhow::Result<String> {
    SESSION_NAME
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .ok_or_else(|| anyhow!("no session was launched"))
}

/// Step 3: listed under the scratch workspace/repo with a live health and a painted [attach], and the
/// panel row's id is the one id `claude agents --json --all` has for the name.
async fn wait_attachable(
    view: &Entity<FleetView>,
    name: &str,
    cx: &mut AsyncWindowContext,
) -> anyhow::Result<(SessionRow, Duration)> {
    let opened = Instant::now();
    let row = loop {
        let row = view.read_with(cx, |v, _| find_row(v, name));
        if let Some(row) = &row {
            let key = format!("attach-{}", row.session.id);
            let painted = view.read_with(cx, |v, _| v.hits.borrow().contains_key(&key));
            // A pid means the process is up; the pre-process row (no pid) is too early to attach.
            if row.attachable && row.session.pid.is_some() && painted {
                break row.clone();
            }
        }
        if opened.elapsed() > LISTED_WITHIN {
            let seen = row.map_or("not listed".to_string(), |r| {
                format!("listed as {} ({}, attachable {})", r.health, raw(&r.session), r.attachable)
            });
            bail!("{name} not live under {WORKSPACE}/./{} within {} s: {seen}", repo(), LISTED_WITHIN.as_secs());
        }
        sleep(cx, Duration::from_millis(100)).await;
    };
    let id = row.session.id.clone();
    let name = name.to_string();
    let resolved = cx
        .background_executor()
        .spawn(async move {
            list_sessions().map(|rows| {
                rows.into_iter().filter(|s| s.name == name).map(|s| s.id).collect::<Vec<_>>()
            })
        })
        .await?;
    if resolved != [id.clone()] {
        bail!("the panel row's id {id} is not the one id resolved by name from claude agents --json --all: {resolved:?}");
    }
    Ok((row, opened.elapsed()))
}

pub(crate) struct Attached {
    pub(crate) tab: u64,
    pub(crate) title: String,
    pid: u32,
    rendered: usize,
    from: usize,
    after: Duration,
    first_line: String,
}

/// Step 4: click [attach] (`key`; step 7 clicks the item page's Attach); a tab opens; its painted
/// grid gains output within 10 s.
pub(crate) async fn open_attach(view: &Entity<FleetView>, key: &str, id: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<Attached> {
    let before = view.read_with(cx, |v, _| v.tabs.len());
    let clicked = Instant::now();
    click(view, key, cx).await?;
    let (tab, title) = view
        .read_with(cx, |v, _| {
            v.tabs.iter().find_map(|t| match &t.kind {
                TabKind::Attach { session_id } if session_id == id => Some((t.id, t.title.clone())),
                _ => None,
            })
        })
        .ok_or_else(|| anyhow!("clicking {key} opened no tab for {id}"))?;
    let tabs = view.read_with(cx, |v, _| v.tabs.len());
    if tabs != before + 1 {
        bail!("expected {} tab(s) after the click, found {tabs}", before + 1);
    }
    let terminal = wait_for_tab_terminal(view, tab, RENDERED_WITHIN, cx).await?;
    let pid = terminal
        .read_with(cx, |t, _| t.pid())
        .map(|p| p.as_u32())
        .ok_or_else(|| anyhow!("the attach terminal has no process id"))?;
    let from = painted_cells(&terminal, cx);
    let rendered = loop {
        let cells = painted_cells(&terminal, cx);
        if cells > from {
            break cells;
        }
        if clicked.elapsed() > RENDERED_WITHIN {
            bail!(
                "tab \"{title}\" painted no output within {} s (non-blank cells stayed at {from})",
                RENDERED_WITHIN.as_secs()
            );
        }
        sleep(cx, Duration::from_millis(20)).await;
    };
    let first_line = first_line(&terminal.read_with(cx, |t, _| t.get_content()));
    Ok(Attached {
        tab,
        title,
        pid,
        rendered,
        from,
        after: clicked.elapsed(),
        first_line,
    })
}

async fn session_now(name: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<Session> {
    let name = name.to_string();
    cx.background_executor().spawn(async move { session_alive_in_cli(&name) }).await
}

/// Step 5: click the tab's ×. The tab detaches (Ctrl+Z, then waits for `claude attach` to exit) and
/// goes away; the session is alive afterwards as the same process it was before the close.
pub(crate) async fn close_attach(
    view: &Entity<FleetView>,
    attached: &Attached,
    name: &str,
    cx: &mut AsyncWindowContext,
) -> anyhow::Result<String> {
    let (tab, pid) = (attached.tab, attached.pid);
    let before = session_now(name, cx).await.context("before the close")?;
    let session_pid = before.pid.unwrap_or_default();
    click(view, &format!("close-tab-{tab}"), cx).await?;
    let closed = Instant::now();
    loop {
        match view.read_with(cx, |v, _| v.tab(tab).map(|t| t.detach)) {
            None => break,
            Some(Detach::Failed) => bail!(
                "detach failed: claude attach {pid} did not exit within {} s of Ctrl+Z; tab left open (alive: {})",
                DETACH_WITHIN.as_secs(),
                process_alive(pid)
            ),
            Some(_) if closed.elapsed() > DETACH_WITHIN + Duration::from_secs(1) => {
                bail!("tab {tab} still open {} ms after its close was clicked", closed.elapsed().as_millis())
            }
            Some(_) => sleep(cx, Duration::from_millis(10)).await,
        }
    }
    let gone_after = closed.elapsed();
    if process_alive(pid) {
        bail!("the tab went away but the attach process {pid} is still running");
    }
    sleep(cx, Duration::from_secs(2)).await;
    let after = session_now(name, cx).await.context("after the close")?;
    if after.pid != Some(session_pid) {
        bail!(
            "{name} was pid {session_pid} before the close and is {} after it",
            after.pid.map_or("without a pid".to_string(), |p| format!("pid {p}"))
        );
    }
    Ok(format!(
        "detached in {} ms (attach pid {pid} exited); {name} alive as the same pid {session_pid}: before [{}] after [{}] = {}",
        gone_after.as_millis(),
        raw(&before),
        raw(&after),
        after.health(now_ms())
    ))
}

async fn gate(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let name = session_name()?;
    println!("UI thread priority raised to highest: {}", frames::raise_ui_thread());
    let (row, listed_after) = wait_attachable(view, &name, cx).await.context("step 3")?;
    let id = row.session.id.clone();
    println!(
        "step 3: {name} ({id}) listed under {WORKSPACE} / ./{} as {} after {} ms",
        repo(),
        row.health,
        listed_after.as_millis()
    );

    let attached = open_attach(view, &format!("attach-{id}"), &id, cx).await.context("step 4")?;
    println!(
        "step 4: tab \"{}\" (claude attach, pid {}) painted {} non-blank cells (from {}) {} ms after the click; first line: {}",
        attached.title,
        attached.pid,
        attached.rendered,
        attached.from,
        attached.after.as_millis(),
        attached.first_line
    );

    let closed = close_attach(view, &attached, &name, cx).await.context("step 5")?;
    println!("step 5: tab closed, {closed}");

    let scrolled = wheel_scroll(view, cx).await.context("step 6")?;
    println!("step 6: {scrolled}");

    let work = crate::selftest_work::run(view, &row.session, cx).await.context("step 7")?;
    println!("step 7: {work}");

    let managed = crate::selftest_manage::run(view, &row.session, cx).await.context("step 7b")?;
    println!("step 7b: {managed}");

    // 8. Idle memory and frame times while the panel keeps refreshing.
    let (idle_private, idle_ws) = memory();
    let refreshes_before = view.read_with(cx, |v, _| v.refreshes);
    let before = frames::baseline(view, cx).await;
    let walks_before = main_repo_walks();
    let idle = frames::record(view, Duration::from_secs(6), cx).await;
    let walks = main_repo_walks() - walks_before;
    let stats = idle.stats;
    let (refreshes, sessions) = view.read_with(cx, |v, _| (v.refreshes - refreshes_before, v.sessions.len() as u64));
    println!("idle private bytes: {} (working set {})", mib(idle_private), mib(idle_ws));
    let judged = frames::judge("step 8", &before, &idle, view, cx).await;
    println!("frame time over 6 s with {refreshes} panel refreshes: {}", judged.as_ref().map_or_else(|e| format!("{e:#}"), String::clone));
    judged?;
    if refreshes == 0 {
        bail!("step 8: the panel did not refresh during the 6 s frame window");
    }
    // A poll walks each session cwd once (one may be in flight at either edge); rendering walks none (m4/2b).
    let polls_allowed = (refreshes + 1) * sessions;
    println!("main_repo walks over those {} frames: {walks} (at most {polls_allowed}: {sessions} sessions × {} polls)", stats.frames, refreshes + 1);
    if walks > polls_allowed {
        bail!("step 8: {walks} main_repo walks over {} idle frames, at most {polls_allowed} expected: the render path walks the filesystem", stats.frames);
    }
    Ok(())
}

const CLIPBOARD_TRIES: usize = 5;

/// The clipboard's text once `own` accepts it, or what it held after 1 s.
async fn read_own(
    cx: &mut AsyncWindowContext,
    read: &impl Fn(&mut AsyncWindowContext) -> anyhow::Result<Option<String>>,
    own: impl Fn(&str) -> bool,
) -> anyhow::Result<Option<String>> {
    let started = Instant::now();
    loop {
        let text = read(cx)?;
        if text.as_deref().is_some_and(&own) || started.elapsed() > Duration::from_secs(1) {
            return Ok(text);
        }
        sleep(cx, Duration::from_millis(50)).await;
    }
}

/// Step 6b (m5/9), on step 6's shell tab: a left drag dispatched over a known output line selects exactly
/// that line's text, Ctrl+Shift+C puts it on the clipboard, and Ctrl+V types the clipboard into the shell
/// (PSReadLine echoes it at the prompt). The clipboard is put back as it was.
async fn select_copy_paste(terminal: &Entity<Terminal>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let saved = cx.update(|_, cx| cx.read_from_clipboard())?;
    let result = async {
        let (bounds, found) = terminal.read_with(cx, |t, _| {
            let content = t.last_content();
            let mut rows: std::collections::BTreeMap<i32, String> = Default::default();
            for cell in &content.cells {
                rows.entry(cell.point.line).or_default().push(cell.cell.character());
            }
            let found = rows.into_iter().find_map(|(line, text)| {
                let text = text.trim_end().to_string();
                text.starts_with("wheel line ").then_some((line + content.display_offset as i32, text))
            });
            (content.terminal_bounds, found)
        });
        let (row, text) = found.ok_or_else(|| anyhow!("no `wheel line` row on screen"))?;
        let y = bounds.bounds.origin.y + bounds.line_height * (row as f32 + 0.5);
        let x = |column: f32| bounds.bounds.origin.x + bounds.cell_width * column;
        let (from, to) = (point(x(0.25), y), point(x(text.chars().count() as f32 - 0.25), y));
        let left = Some(MouseButton::Left);
        cx.update(|window, cx| {
            let none = Modifiers::default();
            window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position: from, pressed_button: None, modifiers: none }), cx);
            window.dispatch_event(PlatformInput::MouseDown(MouseDownEvent { button: MouseButton::Left, position: from, modifiers: none, click_count: 1, first_mouse: false }), cx);
            for step in 1..=4 {
                let position = point(from.x + (to.x - from.x) * (step as f32 / 4.), y);
                window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position, pressed_button: left, modifiers: none }), cx);
            }
            window.dispatch_event(PlatformInput::MouseUp(MouseUpEvent { button: MouseButton::Left, position: to, modifiers: none, click_count: 1 }), cx);
        })?;
        sleep(cx, Duration::from_millis(200)).await;
        let selected = terminal.read_with(cx, |t, _| t.last_content().selection_text.clone());
        if selected.as_deref() != Some(text.as_str()) {
            bail!("dragged over display row {row} ({text:?}) from {from:?} to {to:?}; selection is {selected:?}");
        }

        // Another selftest may write the system clipboard at any moment (m5/15): the copied line carries
        // this run's tag, and each read waits for this run's own value, pressing again if a foreign one
        // replaced it.
        let read = |cx: &mut AsyncWindowContext| cx.update(|_, cx| cx.read_from_clipboard().and_then(|c| c.text()));
        let mut copied = None;
        for _ in 0..CLIPBOARD_TRIES {
            cx.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string("not copied".into())))?;
            key(cx, "c", Modifiers { control: true, shift: true, ..Default::default() })?;
            copied = read_own(cx, &read, |c| c == text).await?;
            if copied.as_deref() == Some(text.as_str()) {
                break;
            }
        }
        if copied.as_deref() != Some(text.as_str()) {
            bail!("Ctrl+Shift+C with {text:?} selected left the clipboard at {copied:?} ({CLIPBOARD_TRIES} tries)");
        }

        let marker = format!("fm-paste-{}", hex6());
        let echoed = |cx: &mut AsyncWindowContext| {
            let content = terminal.read_with(cx, |t, _| t.get_content());
            content.lines().rev().find(|l| !l.trim().is_empty()).is_some_and(|l| l.contains(&marker))
        };
        let pasted = Instant::now();
        'paste: for _ in 0..CLIPBOARD_TRIES {
            cx.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string(marker.clone())))?;
            key(cx, "v", Modifiers::control())?;
            let tried = Instant::now();
            while tried.elapsed() < Duration::from_secs(2) {
                if echoed(cx) {
                    break 'paste;
                }
                sleep(cx, Duration::from_millis(50)).await;
            }
        }
        if !echoed(cx) {
            bail!("Ctrl+V: {marker} not echoed at the prompt in {CLIPBOARD_TRIES} tries of 2 s");
        }
        Ok(format!(
            "drag over row {row} selected {text:?}; Ctrl+Shift+C → clipboard {text:?}; Ctrl+V → {marker} at the prompt after {} ms",
            pasted.elapsed().as_millis()
        ))
    }
    .await;
    if let Some(item) = saved {
        cx.update(|_, cx| cx.write_to_clipboard(item))?;
    }
    result
}

fn key(cx: &mut AsyncWindowContext, key: &str, modifiers: Modifiers) -> anyhow::Result<()> {
    let keystroke = Keystroke { modifiers, key: key.into(), key_char: None };
    cx.update(|window, cx| {
        window.dispatch_event(PlatformInput::KeyDown(KeyDownEvent { keystroke, is_held: false, prefer_character_input: false }), cx);
    })?;
    Ok(())
}

/// Step 6 (tf/6): a `+` tab with more than two screens of output; wheel notches dispatched over its
/// grid move the scrollback (display_offset) up and back down to the bottom.
async fn wheel_scroll(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let before = view.read_with(cx, |v, _| v.tabs.iter().map(|t| t.id).max());
    // the user's profile may print another prompt, or take long; the perf mode does the same
    let args = view.update(cx, |v, _| std::mem::replace(&mut v.shell_args, vec!["-NoLogo".into(), "-NoProfile".into()]));
    let clicked = click(view, "new-shell", cx).await;
    view.update(cx, |v, _| v.shell_args = args);
    clicked?;
    let tab = view
        .read_with(cx, |v, _| v.tabs.iter().map(|t| t.id).max())
        .filter(|id| Some(*id) != before)
        .ok_or_else(|| anyhow!("clicking + opened no tab"))?;
    let result = async {
        let terminal = wait_for_tab_terminal(view, tab, Duration::from_secs(10), cx).await?;
        let opened = Instant::now();
        // a terminal is built with placeholder bounds at the origin (6 lines); the grid's first prepaint
        // gives it the real ones. Sized from the placeholder, the output fit on screen and left 1 line of
        // scrollback for two notches to scroll (m4/2c).
        let lines = loop {
            let bounds = terminal.read_with(cx, |t, _| t.last_content().terminal_bounds);
            if bounds.bounds.origin != point(px(0.), px(0.)) {
                break bounds.num_lines();
            }
            if opened.elapsed() > Duration::from_secs(10) {
                bail!("tab {tab}: the grid never laid the terminal out (bounds {bounds:?})");
            }
            sleep(cx, Duration::from_millis(20)).await;
        };
        let count = lines * 2 + 20;
        // the tag makes step 6b's copied line this run's own
        let tag = format!("fm-{}", hex6());
        let last = format!("wheel line {count} {tag}");
        let mut typed = false;
        loop {
            let content = terminal.read_with(cx, |t, _| t.get_content());
            if !typed && content.contains("PS ") {
                let command = format!("1..{count} | % {{ \"wheel line $_ {tag}\" }}\r");
                terminal.update(cx, |t, _| t.input(command.into_bytes()));
                typed = true;
            }
            if typed && content.lines().any(|l| l.trim_end() == last) && content.trim_end().ends_with('>') {
                break;
            }
            if opened.elapsed() > Duration::from_secs(15) {
                let tail = content.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or_default().to_string();
                bail!("tab {tab}: no prompt after {last:?} within 15 s (typed: {typed}; last line {tail:?})");
            }
            sleep(cx, Duration::from_millis(50)).await;
        }
        let (grid, scrollback) = terminal.read_with(cx, |t, _| {
            let bounds = t.last_content().terminal_bounds;
            (bounds.bounds, t.total_lines().saturating_sub(bounds.num_lines()))
        });
        if scrollback < 6 {
            bail!("{count} lines in a {lines}-line grid left {scrollback} lines of scrollback; two notches up need 6");
        }
        let offset = async |notches: f32, cx: &mut AsyncWindowContext| -> anyhow::Result<usize> {
            let position = grid.center();
            cx.update(|window, cx| {
                window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position, pressed_button: None, modifiers: Modifiers::default() }), cx);
                // a Windows wheel notch: 3 lines, positive is up (gpui_windows events.rs)
                let event = ScrollEvent { position, delta: ScrollDelta::Lines(point(0., 3. * notches)), ..Default::default() };
                window.dispatch_event(PlatformInput::Scroll(event), cx);
            })?;
            sleep(cx, Duration::from_millis(200)).await;
            Ok(terminal.read_with(cx, |t, _| t.last_content().display_offset))
        };
        let up = offset(2., cx).await?;
        if up == 0 {
            bail!("two wheel notches up over the grid {grid:?} left display_offset at 0");
        }
        let half = offset(-1., cx).await?;
        let bottom = offset(-1., cx).await?;
        if !(half > 0 && half < up) || bottom != 0 {
            bail!("wheel down: display_offset {up} → {half} → {bottom}, expected {up} → 1..{up} → 0");
        }
        let wheel = format!("{count} lines in a {lines}-line grid; display_offset 0 → {up} (2 notches up) → {half} → {bottom} (2 down)");
        let copied = select_copy_paste(&terminal, cx).await.context("step 6b")?;
        println!("step 6b: {copied}");
        Ok(wheel)
    }
    .await;
    view.update(cx, |v, cx| v.close_tab(tab, cx));
    result
}

/// `--selftest-detach <n>`: one session, n attach/close cycles through the UI, alive after each.
async fn detach_cycles(view: &Entity<FleetView>, cycles: u32, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let name = session_name()?;
    let (row, listed_after) = wait_attachable(view, &name, cx).await?;
    let id = row.session.id.clone();
    println!("{name} ({id}) listed as {} after {} ms", row.health, listed_after.as_millis());
    for cycle in 1..=cycles {
        let result: anyhow::Result<String> = async {
            wait_attachable(view, &name, cx).await?;
            let attached = open_attach(view, &format!("attach-{id}"), &id, cx).await?;
            let closed = close_attach(view, &attached, &name, cx).await?;
            Ok(format!("attach pid {} painted after {} ms; {closed}", attached.pid, attached.after.as_millis()))
        }
        .await;
        match result {
            Ok(line) => println!("cycle {cycle}/{cycles} PASS {line}"),
            Err(e) => {
                println!("cycle {cycle}/{cycles} FAIL {e:#}");
                bail!("{} of {cycles} cycles passed", cycle - 1);
            }
        }
    }
    println!("{cycles} of {cycles} cycles passed");
    Ok(())
}

/// What the shutdown variant hands to [`finish`], which checks it after the window is gone.
struct ShutdownProbe {
    name: String,
    attach_pid: u32,
    session_pid: u32,
    before: String,
    asked: Instant,
}

static SHUTDOWN: Mutex<Option<ShutdownProbe>> = Mutex::new(None);

/// `--selftest-detach-shutdown`: attach, then close the window the way the OS does (WM_CLOSE). The
/// window must detach the tab and close by itself; [`finish`] checks the session afterwards.
async fn detach_on_shutdown(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let name = session_name()?;
    let (row, _) = wait_attachable(view, &name, cx).await?;
    let attached = open_attach(view, &format!("attach-{}", row.session.id), &row.session.id, cx).await?;
    let before = session_now(&name, cx).await.context("before the window close")?;
    println!(
        "attached: tab \"{}\" (claude attach, pid {}); {name} [{}]",
        attached.title,
        attached.pid,
        raw(&before)
    );
    *SHUTDOWN.lock().unwrap_or_else(|e| e.into_inner()) = Some(ShutdownProbe {
        name: name.clone(),
        attach_pid: attached.pid,
        session_pid: before.pid.unwrap_or_default(),
        before: raw(&before),
        asked: Instant::now(),
    });
    let posted = post_close_to_own_windows();
    println!("posted WM_CLOSE to {posted} window(s)");
    let asked = Instant::now();
    while asked.elapsed() < DETACH_WITHIN + Duration::from_secs(3) {
        sleep(cx, Duration::from_millis(50)).await;
        if cx.update(|_, _| ()).is_err() {
            return Ok(());
        }
    }
    let state = view.read_with(cx, |v, _| v.tabs.iter().map(|t| format!("{} {:?}", t.title, t.detach)).collect::<Vec<_>>());
    bail!("the window is still open {} ms after WM_CLOSE; tabs: {state:?}", asked.elapsed().as_millis())
}

#[cfg(windows)]
fn post_close_to_own_windows() -> usize {
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::WindowsAndMessaging::{EnumThreadWindows, IsWindowVisible, PostMessageW, WM_CLOSE};
    unsafe extern "system" fn post(hwnd: HWND, count: LPARAM) -> windows::core::BOOL {
        unsafe {
            if IsWindowVisible(hwnd).as_bool() && PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)).is_ok() {
                *(count.0 as *mut usize) += 1;
            }
        }
        true.into()
    }
    let mut count = 0usize;
    unsafe {
        let _ = EnumThreadWindows(GetCurrentThreadId(), Some(post), LPARAM(&mut count as *mut usize as isize));
    }
    count
}

#[cfg(not(windows))]
fn post_close_to_own_windows() -> usize {
    0
}

/// After the event loop ended. The shutdown variant's verdict: the window closed only after the
/// attach process exited, and the session is the same live process.
pub fn finish(mode: Mode) {
    if mode != Mode::SelftestDetachShutdown || EXIT_CODE.load(Ordering::SeqCst) != 0 {
        return;
    }
    let Some(probe) = SHUTDOWN.lock().unwrap_or_else(|e| e.into_inner()).take() else {
        println!("FAIL the window closed before the selftest attached");
        EXIT_CODE.store(1, Ordering::SeqCst);
        return;
    };
    let closed_after = probe.asked.elapsed();
    let result = (|| {
        if process_alive(probe.attach_pid) {
            bail!("the window closed with the attach process {} still running", probe.attach_pid);
        }
        std::thread::sleep(Duration::from_secs(2));
        let after = session_alive_in_cli(&probe.name)?;
        if after.pid != Some(probe.session_pid) {
            bail!("{} was pid {} before the window close and is {:?} after it", probe.name, probe.session_pid, after.pid);
        }
        Ok(format!(
            "window closed {} ms after WM_CLOSE, attach pid {} exited; {} alive as the same pid {}: before [{}] after [{}] = {}",
            closed_after.as_millis(),
            probe.attach_pid,
            probe.name,
            probe.session_pid,
            probe.before,
            raw(&after),
            after.health(now_ms())
        ))
    })();
    match result {
        Ok(line) => println!("shutdown: {line}\nPASS"),
        Err(e) => {
            println!("FAIL shutdown: {e:#}");
            EXIT_CODE.store(1, Ordering::SeqCst);
        }
    }
}

async fn perf(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let started = Instant::now();
    while view.read_with(cx, |v, _| v.refreshes) == 0 {
        if started.elapsed() > Duration::from_secs(15) {
            bail!("the panel never refreshed");
        }
        sleep(cx, Duration::from_millis(50)).await;
    }
    view.update(cx, |v, _| v.shell_args = vec!["-NoLogo".into(), "-NoProfile".into()]);
    let (base_private, base_ws) = memory();
    println!("0 tabs: private bytes {} (working set {})", mib(base_private), mib(base_ws));
    for target in [1usize, 10] {
        while view.read_with(cx, |v, _| v.tabs.len()) < target {
            let before = view.read_with(cx, |v, _| v.tabs.iter().map(|t| t.id).max());
            click(view, "new-shell", cx).await?;
            let tab = view
                .read_with(cx, |v, _| v.tabs.iter().map(|t| t.id).max())
                .filter(|id| Some(*id) != before)
                .ok_or_else(|| anyhow!("clicking + opened no tab"))?;
            let terminal = wait_for_tab_terminal(view, tab, Duration::from_secs(10), cx).await?;
            let opened = Instant::now();
            while !terminal.read_with(cx, |t, _| t.get_content()).contains("PS ") {
                if opened.elapsed() > Duration::from_secs(10) {
                    bail!("tab {tab}: no pwsh prompt within 10 s");
                }
                sleep(cx, Duration::from_millis(20)).await;
            }
        }
        sleep(cx, Duration::from_secs(2)).await;
        let (private, ws) = memory();
        let stats = frames::record(view, Duration::from_secs(5), cx).await.stats;
        println!("{target} tabs: private bytes {} (working set {})", mib(private), mib(ws));
        println!("{target} tabs: frame time over 5 s: {stats}");
    }
    let ids: Vec<u64> = view.read_with(cx, |v, _| v.tabs.iter().map(|t| t.id).collect());
    view.update(cx, |v, cx| {
        for id in ids {
            v.close_tab(id, cx);
        }
    });
    sleep(cx, Duration::from_millis(500)).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// <temp>/fm-selftest-discovery-<hex>/
    ///   Root/Repo/.git/                       main checkout
    ///   Root/Repo.worktrees/abs/.git          file → <abs Repo>/.git/worktrees/abs
    ///   Root/Repo.worktrees/rel/crates/app    below a .git file → ../../Repo/.git/worktrees/rel
    ///   Bare/                                 no .git anywhere
    #[test]
    fn find_main_repo_from_checkout_worktrees_and_nowhere() {
        let tmp = std::env::temp_dir().join(format!("fm-selftest-discovery-{}", hex6()));
        let repo = tmp.join("Root").join("Repo");
        std::fs::create_dir_all(repo.join(".git").join("worktrees")).unwrap();
        std::fs::create_dir_all(repo.join("crates").join("foreman-app")).unwrap();
        let wts = tmp.join("Root").join("Repo.worktrees");
        std::fs::create_dir_all(wts.join("abs")).unwrap();
        let abs = format!("gitdir: {}\n", repo.join(".git").join("worktrees").join("abs").display());
        std::fs::write(wts.join("abs").join(".git"), abs).unwrap();
        std::fs::create_dir_all(wts.join("rel").join("crates").join("app")).unwrap();
        std::fs::write(wts.join("rel").join(".git"), "gitdir: ../../Repo/.git/worktrees/rel\n").unwrap();
        std::fs::create_dir_all(tmp.join("Bare")).unwrap();

        let check = |from: &Path, label: &str| {
            let main = find_main_repo(from).unwrap_or_else(|e| panic!("{label}: {e:#}"));
            assert_eq!(main.file_name().unwrap(), "Repo", "{label}: {}", main.display());
            assert_eq!(main.parent().unwrap().file_name().unwrap(), "Root", "{label}: workspace root");
        };
        check(&repo.join("crates").join("foreman-app"), ".git dir");
        check(&wts.join("abs"), "absolute gitdir");
        check(&wts.join("rel").join("crates").join("app"), "relative gitdir");
        if main_repo(&tmp).is_none() {
            let error = find_main_repo(&tmp.join("Bare")).unwrap_err().to_string();
            assert!(error.contains("no .git") && error.contains("Bare"), "{error}");
        }
        std::fs::remove_dir_all(&tmp).ok();
    }
}
