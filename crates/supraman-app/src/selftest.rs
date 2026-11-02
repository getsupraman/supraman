//! The panel also shows the user's real sessions; the selftest only ever acts on its own row, by name.

use std::{
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard, OnceLock, atomic::{AtomicBool, Ordering}},
    time::{Duration, Instant},
};

use anyhow::{Context as _, anyhow, bail};
use supraman_claude::{Claude, Removal, Session, WRITE_TIMEOUT, check_supported, list_sessions, main_repo, main_repo_walks, now_ms};
use supraman_core::{DEFAULT_DEPTH, Workspace, Registry, process, scan};
use supraman_term::Terminal;
use supraman_ui::{memory, mib};
use gpui::{
    App, AsyncWindowContext, Bounds, Entity, KeyDownEvent, Keystroke, Modifiers, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, PlatformInput, ScrollDelta, ScrollEvent, Window,
    point, px,
};

use crate::{
    EXIT_CODE, Mode,
    fleet::{self, SessionRow},
    view::{DETACH_WITHIN, Detach, FleetView, TabKind},
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
    /// `--selftest-supervise`: every row whose name starts with this is the run's own.
    pub(crate) name_prefix: Option<String>,
    /// Processes this run started (pid, creation time).
    pub(crate) kill: Vec<(u32, u64)>,
    /// `(sessionId, cwd)` of this run's own `fm-selftest` sessions: their transcripts in the real claude dir are the only write a selftest makes there.
    pub(crate) transcripts: Vec<(String, PathBuf)>,
}

pub(crate) fn forget_own_transcript(session_id: &str, cwd: &Path) {
    if let Some(scratch) = lock().as_mut()
        && !scratch.transcripts.iter().any(|(s, _)| s == session_id)
    {
        scratch.transcripts.push((session_id.to_string(), cwd.to_path_buf()));
    }
}

/// From the claude dir `claude` itself writes to (not the fake one).
fn forget_transcripts(transcripts: &[(String, PathBuf)]) -> bool {
    let Some(dir) = supraman_claude::claude_dir() else {
        return transcripts.is_empty();
    };
    let mut ok = true;
    for (sid, cwd) in transcripts {
        // claude may still be writing the last lines of a session it just stopped
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match supraman_claude::forget_transcript(&dir, sid, Some(cwd)) {
                Ok(gone) => {
                    println!("cleanup: deleted {} transcript(s), {} dir(s) of {sid}", gone.transcripts.len(), gone.dirs.len());
                    break;
                }
                Err(e) if Instant::now() > deadline => {
                    println!("cleanup: cannot delete the transcript of {sid}: {e}");
                    ok = false;
                    break;
                }
                Err(_) => std::thread::sleep(Duration::from_millis(250)),
            }
        }
    }
    ok
}

static SCRATCH: Mutex<Option<Scratch>> = Mutex::new(None);
static SESSION_NAME: Mutex<Option<String>> = Mutex::new(None);
static REPO: OnceLock<String> = OnceLock::new();

fn repo() -> &'static str {
    REPO.get().map_or("", String::as_str)
}

pub(crate) fn cargo_root() -> &'static Path {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest.ancestors().nth(2).unwrap_or(manifest)
}

fn find_main_repo(from: &Path) -> anyhow::Result<PathBuf> {
    main_repo(from).ok_or_else(|| {
        anyhow!(
            "no .git directory or worktree .git file (gitdir: <repo>/.git/worktrees/<name>) at or above {}",
            from.display()
        )
    })
}

pub(crate) fn main_repo_of_gate() -> anyhow::Result<PathBuf> {
    find_main_repo(cargo_root())
}

/// This run's `<hex>`: its sessions are `fm-selftest-<hex>` and `fm-selftest-<hex>-…`.
pub(crate) fn run_hex() -> &'static str {
    static RUN: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    RUN.get_or_init(hex6)
}

pub(crate) use supraman_core::time::hex6;

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

pub fn prepare(mode: Mode) -> anyhow::Result<()> {
    // The sign-in never opens a browser; its refresh tokens stay in the scratch home.
    crate::signin::use_http_browser();
    // Closed sessions are read from a fake claude dir in the scratch home, never the user's.
    unsafe { std::env::set_var(crate::view::CLAUDE_DIR_VAR, crate::selftest_closed::claude_dir(&home())) };
    // A closed loopback port as the releases API, never GitHub (C109); step 7+u serves its own fake.
    let closed = std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
    unsafe { std::env::set_var(supraman_workman::updates::API_VAR, format!("http://127.0.0.1:{closed}")) };
    if mode == Mode::SelftestSupervise {
        return crate::selftest_supervise::prepare();
    }
    if mode == Mode::SelftestShots {
        return crate::selftest_shots::prepare();
    }
    let target = cargo_root().join("target");
    let home = home();
    let cwd = target.join("selftest-cwd");
    let work = target.join("selftest-work");
    let add = target.join(crate::selftest_manage::FOLDER);
    let single = target.join(crate::selftest_manage::SINGLE);
    let sched = crate::selftest_schedules::folder();
    for dir in [&home, &cwd, &work, &add, &single, &sched] {
        remove_dir_retrying(dir).with_context(|| format!("cannot clear {}", dir.display()))?;
    }
    *lock() = Some(Scratch {
        dirs: vec![home.clone(), cwd.clone(), work.clone(), add, single, sched],
        session_name: None,
        name_prefix: None,
        kill: Vec::new(),
        transcripts: Vec::new(),
    });
    install_guards(mode);

    // Still single-threaded apart from the watchdog, which never reads the environment.
    unsafe { std::env::set_var(supraman_core::HOME_VAR, &home) };
    std::fs::create_dir_all(&cwd)?;
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
        "step 1: SUPRAMAN_HOME={} ; workspace {WORKSPACE} = {} ({count} repos, read-only scan) registered at {}",
        home.display(),
        workspace_root.display(),
        saved.display()
    );

    // A closed loopback port as the Workman server, never the real one; the gate's step 1w names its fake instead.
    let closed = std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
    let mut settings = supraman_core::Settings::load(&home)?;
    settings.workman.url = Some(format!("http://127.0.0.1:{closed}"));
    settings.save(&home)?;

    if mode == Mode::Selftest {
        crate::selftest_work::prepare(&home, &work)?;
        crate::selftest_closed::prepare(&home, &main)?;
        crate::selftest_delete::prepare(&home, &main)?;
    }

    if mode.launches_session() {
        let version = check_supported()?;
        println!("claude CLI: {}", version.raw);
        if let Some(warning) = version.untested() {
            println!("WARNING {warning}");
        }
        let name = format!("fm-selftest-{}", run_hex());
        // Recorded before launching, so cleanup looks for it even if the launch half-succeeds.
        if let Some(scratch) = lock().as_mut() {
            scratch.session_name = Some(name.clone());
        }
        *SESSION_NAME.lock().unwrap_or_else(|e| e.into_inner()) = Some(name.clone());
        let launched = Instant::now();
        let out = Claude::default().run_in(&["--bg", "--name", &name, "--permission-mode", "auto", PROMPT], Some(&cwd), WRITE_TIMEOUT)?;
        let out = out.trim();
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
        Mode::Selftest if std::env::var_os("FOREMAN_SHOTS").is_some() => Duration::from_secs(360),
        Mode::Selftest => Duration::from_secs(270),
        Mode::SelftestDetach(cycles) => Duration::from_secs(90 + 15 * u64::from(cycles)),
        Mode::SelftestSupervise => Duration::from_secs(900),
        _ => Duration::from_secs(120),
    };
    std::thread::spawn(move || {
        std::thread::sleep(budget);
        println!("FAIL watchdog in step {}: no result after {} s", current_step(), budget.as_secs());
        EXIT_CODE.store(1, Ordering::SeqCst);
        cleanup();
        std::process::exit(1);
    });
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        previous(info);
        println!("FAIL panic in step {}: {info}", current_step());
        EXIT_CODE.store(1, Ordering::SeqCst);
        // try_lock: a panic inside cleanup itself must not deadlock here.
        if let Ok(mut guard) = SCRATCH.try_lock() {
            run_cleanup(&mut guard);
        }
    }));
}

/// `main` takes the single-instance lock on it before `prepare` clears it.
pub fn home() -> PathBuf {
    cargo_root().join("target").join("selftest-home")
}

pub fn log_dir() -> PathBuf {
    home().join("logs")
}

/// What gpui logs for a window dropped under Windows' feet ([`crate::tray::close`]): none may appear.
pub(crate) fn teardown_errors() -> Vec<String> {
    teardown_errors_since(0)
}

pub(crate) fn log_offset() -> u64 {
    std::fs::metadata(log_dir().join("foreman.log")).map_or(0, |m| m.len())
}

pub(crate) fn teardown_errors_since(offset: u64) -> Vec<String> {
    teardown_lines(&std::fs::read(log_dir().join("foreman.log")).unwrap_or_default(), offset)
}

fn teardown_lines(bytes: &[u8], offset: u64) -> Vec<String> {
    let from = usize::try_from(offset).ok().filter(|&o| o <= bytes.len()).unwrap_or(0);
    String::from_utf8_lossy(&bytes[from..])
        .lines()
        .filter(|l| l.contains("window not found") || l.contains("Invalid window handle"))
        .map(str::to_string)
        .collect()
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
    for &(pid, start) in &scratch.kill {
        crate::selftest_mark::kill_own(pid, start);
    }
    let mut transcripts = scratch.transcripts.clone();
    if let Some(prefix) = &scratch.name_prefix {
        let (removed, own) = crate::selftest_supervise::remove_own_rows(prefix);
        ok &= removed;
        transcripts.extend(own);
    }
    if let Some(name) = &scratch.session_name {
        // The row can trail `--bg` by a moment, so a missing row is retried before giving up.
        let deadline = Instant::now() + Duration::from_secs(10);
        let ids: Vec<String> = loop {
            match list_sessions() {
                Ok(rows) => {
                    let ids: Vec<String> =
                        rows.iter().filter(|s| &s.name == name).map(|s| s.id.clone()).collect();
                    transcripts.extend(rows.iter().filter(|s| &s.name == name).map(|s| (s.session_id.clone(), s.cwd.clone())));
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
        let claude = Claude::default();
        for id in &ids {
            match claude.stop(id) {
                Ok(out) => println!("cleanup: claude stop {id}: {}", out.trim()),
                Err(e) => {
                    println!("cleanup: claude stop {id}: {e}");
                    ok = false;
                }
            }
            match claude.remove(id, None) {
                Ok(Removal::Removed(out)) => println!("cleanup: claude rm {id}: {out}"),
                Ok(Removal::Kept(refusal)) => {
                    println!("cleanup: claude rm {id} kept it: {}", refusal.text);
                    ok = false;
                }
                Err(e) => {
                    println!("cleanup: claude rm {id}: {e}");
                    ok = false;
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
    transcripts.sort();
    transcripts.dedup();
    ok &= forget_transcripts(&transcripts);
    for e in crate::signin::forget_own_credentials() {
        println!("cleanup: cannot delete the selftest's credential: {e}");
        ok = false;
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
            let own = lock().as_ref().and_then(|s| s.name_prefix.clone()).unwrap_or_else(|| format!("fm-selftest-{}", run_hex()));
            view.update(cx, |v, _| {
                v.own_prefix = Some(own);
                v.registrar = crate::selftest_schedules::registrar();
            });
            let result = match mode {
                Mode::Selftest => gate(&view, cx).await,
                Mode::SelftestDetach(cycles) => detach_cycles(&view, cycles, cx).await,
                Mode::SelftestDetachShutdown => detach_on_shutdown(&view, cx).await,
                Mode::SelftestSupervise => crate::selftest_supervise::run(&view, cx).await,
                Mode::SelftestShots => crate::selftest_shots::run(&view, cx).await,
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
            VERDICT.store(true, Ordering::SeqCst);
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

static STEP: Mutex<&str> = Mutex::new("setup");

pub(crate) fn current_step() -> &'static str {
    *STEP.lock().unwrap_or_else(|e| e.into_inner())
}

fn set_step(name: &'static str) -> &'static str {
    std::mem::replace(&mut *STEP.lock().unwrap_or_else(|e| e.into_inner()), name)
}

async fn step<T>(name: &'static str, run: impl Future<Output = anyhow::Result<T>>) -> anyhow::Result<T> {
    let outer = set_step(name);
    let result = run.await.with_context(|| format!("step {name}"));
    if result.is_ok() {
        set_step(outer);
    }
    result
}

static FRAME_ASKED: AtomicBool = AtomicBool::new(false);

/// [`FleetView::shown`] scrolled the sidebar; the next wait redraws the window so the row is painted.
pub(crate) fn ask_frame() {
    FRAME_ASKED.store(true, Ordering::SeqCst);
}

pub(crate) fn frame_if_asked(cx: &mut AsyncWindowContext) {
    if FRAME_ASKED.swap(false, Ordering::SeqCst) {
        cx.update(|window, _| window.refresh()).ok();
    }
}

pub(crate) async fn until_with<T>(
    cx: &mut AsyncWindowContext,
    within: Duration,
    what: &str,
    mut check: impl FnMut(&mut AsyncWindowContext) -> Option<T>,
) -> anyhow::Result<T> {
    let started = Instant::now();
    loop {
        if let Some(t) = check(cx) {
            return Ok(t);
        }
        if started.elapsed() > within {
            bail!("{what}: not within {within:?}");
        }
        frame_if_asked(cx);
        sleep(cx, Duration::from_millis(25)).await;
    }
}

pub(crate) async fn until<T>(
    view: &Entity<FleetView>,
    cx: &mut AsyncWindowContext,
    within: Duration,
    what: &str,
    f: impl Fn(&FleetView) -> Option<T>,
) -> anyhow::Result<T> {
    until_with(cx, within, what, |cx| view.read_with(cx, |v, _| f(v))).await
}

pub(crate) async fn until_app<T>(
    view: &Entity<FleetView>,
    cx: &mut AsyncWindowContext,
    within: Duration,
    what: &str,
    f: impl Fn(&FleetView, &App) -> Option<T>,
) -> anyhow::Result<T> {
    until_with(cx, within, what, |cx| view.read_with(cx, |v, cx| f(v, cx))).await
}

/// Through the window's own input dispatch, so it lands only if that element is really there and hit-testable.
pub(crate) async fn click(view: &Entity<FleetView>, key: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    press(view, key, MouseButton::Left, cx).await
}

pub(crate) async fn press(view: &Entity<FleetView>, key: &str, button: MouseButton, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    presses(view, key, button, 1, cx).await
}

/// Two clicks as the platform reports a double-click: the second carries `click_count: 2`.
pub(crate) async fn double_click(view: &Entity<FleetView>, key: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    presses(view, key, MouseButton::Left, 2, cx).await
}

async fn presses(view: &Entity<FleetView>, key: &str, button: MouseButton, count: usize, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
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
        for click_count in 1..=count {
            window.dispatch_event(
                PlatformInput::MouseDown(MouseDownEvent {
                    button,
                    position,
                    modifiers,
                    click_count,
                    first_mouse: false,
                }),
                cx,
            );
            window.dispatch_event(
                PlatformInput::MouseUp(MouseUpEvent {
                    button,
                    position,
                    modifiers,
                    click_count,
                }),
                cx,
            );
        }
    })?;
    Ok(())
}

/// An element scrolled out of its container is painted but takes no click, so the container is scrolled with the wheel until it is fully in view.
pub(crate) async fn scrolled_into_view(view: &Entity<FleetView>, key: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<Bounds<Pixels>> {
    let margin = px(4.);
    let mut seen = Vec::new();
    for _ in 0..20 {
        // a refresh can rebuild the tree and drop the row from a frame or two, more often on a busy machine
        let missing = Instant::now();
        let mut revealed = false;
        let mut claimed = false;
        let hit = loop {
            if let Some(hit) = view.read_with(cx, |v, _| v.hits.borrow().get(key).copied()) {
                break hit;
            }
            // the sidebar paints only the rows in view (supraman_ui::keyed_list)
            if !revealed && missing.elapsed() > Duration::from_millis(300) {
                revealed = true;
                claimed = view.update(cx, |v, cx| {
                    cx.notify();
                    v.reveal_hit(key)
                });
                continue;
            }
            if claimed && missing.elapsed() > Duration::from_secs(1) && page_through(view, key, cx).await {
                continue;
            }
            if missing.elapsed() > Duration::from_secs(2) {
                bail!("nothing painted under {key} for 2 s (seen before: {})", if seen.is_empty() { "nothing".into() } else { seen.join(" → ") });
            }
            sleep(cx, Duration::from_millis(50)).await;
        };
        let (b, v) = (hit.bounds, hit.visible);
        seen.push(format!("{}..{} x {}..{} in {}..{} x {}..{}", f32::from(b.top()), f32::from(b.bottom()), f32::from(b.left()), f32::from(b.right()), f32::from(v.top()), f32::from(v.bottom()), f32::from(v.left()), f32::from(v.right())));
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

/// The sidebar from its top, a viewport at a time, until `key` is painted; false when it never is.
async fn page_through(view: &Entity<FleetView>, key: &str, cx: &mut AsyncWindowContext) -> bool {
    let painted = |cx: &mut AsyncWindowContext| view.read_with(cx, |v, _| v.hits.borrow().contains_key(key));
    view.update(cx, |v, cx| {
        v.left_list().to_top();
        cx.notify();
    });
    for _ in 0..400 {
        sleep(cx, Duration::from_millis(40)).await;
        if painted(cx) {
            return true;
        }
        let moved = view.update(cx, |v, cx| {
            cx.notify();
            v.left_list().page_down()
        });
        if !moved {
            sleep(cx, Duration::from_millis(40)).await;
            return painted(cx);
        }
    }
    false
}

fn find_row(view: &FleetView, name: &str) -> Option<SessionRow> {
    view.tree()
        .workspaces
        .clone()
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

/// `state` describes the turn, not the process, so `done` with a live pid is alive (LEARNINGS m1/7).
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
    if !process::alive(pid) {
        bail!("{name}'s pid {pid} is not a running process ({})", raw(&row));
    }
    Ok(row)
}

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
            let painted = view.read_with(cx, |v, _| v.shown(&key));
            frame_if_asked(cx);
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

pub(crate) async fn open_attach(view: &Entity<FleetView>, key: &str, id: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<Attached> {
    open_attach_by(view, key, id, false, cx).await
}

/// `double`: a double-click on `key` (a session row) in place of a click.
async fn open_attach_by(view: &Entity<FleetView>, key: &str, id: &str, double: bool, cx: &mut AsyncWindowContext) -> anyhow::Result<Attached> {
    let before = view.read_with(cx, |v, _| v.tabs.len());
    let clicked = Instant::now();
    if double {
        double_click(view, key, cx).await?;
    } else {
        click(view, key, cx).await?;
    }
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

/// A double-click on the row of the session the active tab runs opens nothing.
async fn double_click_shown(view: &Entity<FleetView>, session_id: &str, tab: u64, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let before = view.read_with(cx, |v, _| v.tabs.len());
    double_click(view, &format!("session-row-{session_id}"), cx).await?;
    sleep(cx, Duration::from_millis(500)).await;
    let (tabs, active) = view.read_with(cx, |v, _| (v.tabs.len(), v.active));
    if tabs != before || active != Some(tab) {
        bail!("a double-click on the row of the tab shown now changed the tabs: {before} → {tabs}, active {active:?}, expected {tab}");
    }
    Ok(format!("the row double-clicked again with its tab shown: still {tabs} tab(s), tab {tab} active"))
}

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
                process::alive(pid)
            ),
            Some(_) if closed.elapsed() > DETACH_WITHIN + Duration::from_secs(1) => {
                bail!("tab {tab} still open {} ms after its close was clicked", closed.elapsed().as_millis())
            }
            Some(_) => sleep(cx, Duration::from_millis(10)).await,
        }
    }
    let gone_after = closed.elapsed();
    if process::alive(pid) {
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

fn painted(v: &FleetView, key: &str) -> bool {
    v.shown(key)
}

async fn loading_until_listed(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    if view.read_with(cx, |v, _| v.sessions_read) {
        bail!("the first session list was applied before the selftest could see the loading state");
    }
    until(view, cx, LISTED_WITHIN, "the loading row and the turning Refresh icon before the first list", |v| {
        (!v.sessions_read && painted(v, "loading-sessions") && painted(v, "refresh-turning")).then_some(())
    })
    .await?;
    let shown = view.read_with(cx, |v, _| (v.sessions.len(), v.tree().workspaces.len()));
    if shown != (0, 0) {
        bail!("rows shown while loading: {} sessions, {} workspaces", shown.0, shown.1);
    }
    let started = Instant::now();
    let listed = std::cell::Cell::new(None);
    let gone = until_with(cx, LISTED_WITHIN, "the loading line gone once every source has loaded", |cx| {
        view.read_with(cx, |v, cx| {
            if v.sessions_read && listed.get().is_none() {
                listed.set(Some(started.elapsed()));
            }
            let shown = painted(v, "loading-sessions");
            if !v.loaded && !shown {
                return Some(Err(format!("the loading line went while {:?} still loaded", v.loading(cx))));
            }
            (v.loaded && !shown).then(|| {
                let done = v.sessions_read && v.refreshes > 0 && v.transcripts_read && !matches!(v.workstate, crate::workstate_setup::WsStatus::Checking);
                if done { Ok(()) } else { Err("loaded before the poll, the tick, the transcripts and Workman were all done".to_string()) }
            })
        })
    })
    .await?;
    gone.map_err(|e| anyhow::anyhow!(e))?;
    let loaded = started.elapsed();
    until(view, cx, LISTED_WITHIN, "the Refresh icon still once every source has loaded", |v| {
        (!v.refreshing() && !painted(v, "refresh-turning")).then_some(())
    })
    .await?;
    Ok(format!(
        "loading line and turning Refresh icon painted before the first list; rows {} ms later, every source {} ms later, icon still {} ms later",
        listed.get().unwrap_or_default().as_millis(),
        loaded.as_millis(),
        started.elapsed().as_millis()
    ))
}

/// The first list is painted within this of `claude agents` returning: its apply and one frame, never the supervisor tick.
const PAINTED_AFTER_LIST: f64 = 150.0;

fn startup_timeline() -> anyhow::Result<String> {
    let log = String::from_utf8_lossy(&std::fs::read(log_dir().join("foreman.log")).unwrap_or_default()).into_owned();
    let phases: Vec<&str> = log.lines().filter_map(|l| l.split_once("] startup ").map(|(_, phase)| phase)).collect();
    let at = |phase: &str| {
        phases.iter().find(|p| p.starts_with(phase)).and_then(|p| p.rsplit_once("(at +")?.1.strip_suffix(" ms)")?.parse::<f64>().ok())
    };
    let (Some(listed), Some(painted)) = (at("list_sessions"), at("first paint with sessions")) else {
        bail!("no list_sessions and first paint lines in foreman.log: {phases:?}");
    };
    let sources = ["list_sessions done", "desired state done", "supervisor tick done", "transcripts done", "workstate status done", "every source done"];
    if let Some(missing) = sources.iter().find(|s| at(s).is_none()) {
        bail!("no \"startup {missing}\" line in foreman.log: {phases:?}");
    }
    if painted - listed > PAINTED_AFTER_LIST {
        bail!("the first list painted {:.0} ms after claude agents returned, over {PAINTED_AFTER_LIST} ms: {}", painted - listed, phases.join("; "));
    }
    Ok(format!("{}; painted {:.0} ms after the list", phases.join("; "), painted - listed))
}

async fn gate(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let name = session_name()?;
    println!("UI thread priority raised to highest: {}", frames::raise_ui_thread());
    crate::view::tree_check::set(true);
    let loading = step("3a", loading_until_listed(view, cx)).await?;
    println!("step 3a: {loading}");
    let (row, listed_after) = step("3", wait_attachable(view, &name, cx)).await?;
    let id = row.session.id.clone();
    println!(
        "step 3: {name} ({id}) listed under {WORKSPACE} / ./{} as {} after {} ms",
        repo(),
        row.health,
        listed_after.as_millis()
    );
    println!("step 3s: {}", step("3s", async { startup_timeline() }).await?);

    let attached = step("4", open_attach_by(view, &format!("session-row-{}", row.session.session_id), &id, true, cx)).await?;
    let again = step("4", double_click_shown(view, &row.session.session_id, attached.tab, cx)).await?;
    println!(
        "step 4: tab \"{}\" (claude attach, pid {}) painted {} non-blank cells (from {}) {} ms after the double-click on its row; first line: {}",
        attached.title,
        attached.pid,
        attached.rendered,
        attached.from,
        attached.after.as_millis(),
        attached.first_line
    );
    println!("step 4: {again}");
    crate::selftest_manage::shot("m5-14-terminal-tab", cx).await;

    let closed = step("5", close_attach(view, &attached, &name, cx)).await?;
    println!("step 5: tab closed, {closed}");

    let scrolled = step("6", wheel_scroll(view, cx)).await?;
    println!("step 6: {scrolled}");

    let work = step("7", crate::selftest_work::run(view, &row.session, cx)).await?;
    println!("step 7: {work}");

    let managed = step("7b", crate::selftest_manage::run(view, &row.session, cx)).await?;
    println!("step 7b: {managed}");

    let bar = step("7c", crate::selftest_left::action_bar(view, cx)).await?;
    println!("step 7c: {bar}");

    let first_run = step("7d", crate::selftest_first_run::run(view, cx)).await?;
    println!("step 7d: {first_run}");

    let hotkeys = step("7e", crate::selftest_tabs::run(view, cx)).await?;
    println!("step 7e: {hotkeys}");

    let clipped = step("7f", crate::selftest_left::tree_clip(view, cx)).await?;
    println!("step 7f: {clipped}");

    let grid = step("7v", crate::selftest_left::tree_grid(view, cx)).await?;
    println!("step 7v: {grid}");

    let shell = step("7g", crate::selftest_shell::run(view, cx)).await?;
    println!("step 7g: {shell}");

    let closed = step("7h", crate::selftest_closed::run(view, &home(), cx)).await?;
    println!("step 7h: {closed}");

    let titled = step("7i", crate::selftest_title::run(view, cx)).await?;
    println!("step 7i: {titled}");

    let refreshed = step("7j", crate::selftest_refresh::run(view, &home(), cx)).await?;
    println!("step 7j: {refreshed}");

    let started = step("7w", crate::selftest_startup::run(view, &home(), &main_repo_of_gate()?, cx)).await?;
    println!("step 7w: {started}");

    let searched = step("7k", crate::selftest_search::run(view, cx)).await?;
    println!("step 7k: {searched}");

    let mark = step("7l", crate::selftest_mark::run(view, cx)).await?;
    println!("step 7l: {mark}");

    let themed = step("7m", crate::selftest_theme::run(view, &home(), cx)).await?;
    println!("step 7m: {themed}");

    let dismissed = step("7n", crate::selftest_messages::run(view, &home(), cx)).await?;
    println!("step 7n: {dismissed}");
    let hint = step("7n", crate::selftest_messages::placeholder(view, cx)).await?;
    println!("step 7n: {hint}");
    let composed = step("7n", crate::selftest_messages::compose(view, cx)).await?;
    println!("step 7n: {composed}");

    let notices = step("7o", crate::selftest_notices::run(view, cx)).await?;
    println!("step 7o: {notices}");

    let updated = step("7+u", crate::selftest_updates::run(view, &home(), cx)).await?;
    println!("step 7+u: {updated}");
    let installed = step("7+u", crate::selftest_updates::install(view, &home(), cx)).await?;
    println!("step 7+u: {installed}");

    let reopened = step("7x", crate::selftest_reopen::run(view, &home(), &session_name()?, cx)).await?;
    println!("step 7x: {reopened}");

    let hand = step("7+", crate::selftest_hand::run(view, &home(), cx)).await?;
    println!("step 7+: {hand}");

    let deleted = step("7p", crate::selftest_delete::run(view, &home(), &session_name()?, cx)).await?;
    println!("step 7p: {deleted}");

    let zoomed = step("7q", crate::selftest_zoom::run(view, &home(), cx)).await?;
    println!("step 7q: {zoomed}");

    let pasted = step("7r", crate::selftest_paste::run(view, cx)).await?;
    println!("step 7r: {pasted}");

    let font = step("7s", crate::selftest_font::run(view, &home(), cx)).await?;
    println!("step 7s: {font}");

    let boxed = step("7t", crate::selftest_box::run(view, &home(), cx)).await?;
    println!("step 7t: {boxed}");

    let reordered = step("7u", crate::selftest_tabs::reorder(view, cx)).await?;
    println!("step 7u: {reordered}");

    let scheduled = step("7y", crate::selftest_schedules::run(view, &home(), cx)).await?;
    println!("step 7y: {scheduled}");

    let tasked = step("7+t", crate::selftest_start::run(view, &home(), cx)).await?;
    println!("step 7+t: {tasked}");

    let responsive = step("7+r", crate::selftest_responsive::run(view, cx)).await?;
    println!("step 7+r: {responsive}");

    let branded = step("7z", crate::selftest_brand::run(view, cx)).await?;
    println!("step 7z: {branded}");

    let cached = step("7+c", async {
        crate::view::tree_check::set(false);
        let (mismatches, compared) = crate::view::tree_check::take();
        if !mismatches.is_empty() {
            bail!("{} of {compared} cached trees differ from a fresh build: {}", mismatches.len(), mismatches.join(" | "));
        }
        let (builds, renders) = crate::view::tree_check::counts();
        Ok(format!("{compared} cached trees each equal a fresh build at their instant; {builds} trees built over {renders} renders"))
    })
    .await?;
    println!("step 7+c: {cached}");

    set_step("8");
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
    // A poll walks each session cwd once (one may be in flight at either edge); rendering walks none.
    let polls_allowed = (refreshes + 1) * sessions;
    println!("main_repo walks over those {} frames: {walks} (at most {polls_allowed}: {sessions} sessions × {} polls)", stats.frames, refreshes + 1);
    if walks > polls_allowed {
        bail!("step 8: {walks} main_repo walks over {} idle frames, at most {polls_allowed} expected: the render path walks the filesystem", stats.frames);
    }
    Ok(())
}

pub(crate) const CLIPBOARD_TRIES: usize = 5;

pub(crate) async fn read_own(cx: &mut AsyncWindowContext, own: impl Fn(&str) -> bool) -> anyhow::Result<Option<String>> {
    let started = Instant::now();
    loop {
        let text = crate::selftest_clipboard::text(cx)?;
        if text.as_deref().is_some_and(&own) || started.elapsed() > Duration::from_secs(1) {
            return Ok(text);
        }
        sleep(cx, Duration::from_millis(50)).await;
    }
}

/// The row is read from the last frame but the drag lands on the grid as it is: it waits for a frame that caught up, and drags again when the grid moved.
pub(crate) async fn select_wheel_line(terminal: &Entity<Terminal>, cx: &mut AsyncWindowContext) -> anyhow::Result<(i32, String)> {
    let mut moved = Vec::new();
    for _ in 0..3 {
        let started = Instant::now();
        loop {
            let (frame, grid) = terminal.read_with(cx, |t, _| ((t.last_content().display_offset, t.last_content().total_lines), t.grid_extent()));
            if frame == grid {
                break;
            }
            if started.elapsed() > Duration::from_secs(2) {
                bail!("the frame never caught up with the grid: (display_offset, total_lines) {frame:?} painted, {grid:?} now");
            }
            sleep(cx, Duration::from_millis(20)).await;
        }
        let (bounds, found, grid) = terminal.read_with(cx, |t, _| {
            let content = t.last_content();
            let mut rows: std::collections::BTreeMap<i32, String> = Default::default();
            for cell in &content.cells {
                rows.entry(cell.point.line).or_default().push(cell.cell.character());
            }
            let found = rows.into_iter().find_map(|(line, text)| {
                let text = text.trim_end().to_string();
                text.starts_with("wheel line ").then_some((line + content.display_offset as i32, text))
            });
            (content.terminal_bounds, found, t.grid_extent())
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
        let (selected, now) = terminal.read_with(cx, |t, _| (t.selection_text(), t.grid_extent()));
        if selected.as_deref() == Some(text.as_str()) {
            return Ok((row, text));
        }
        if now == grid {
            bail!("dragged over display row {row} ({text:?}) from {from:?} to {to:?}; selection is {selected:?} (the grid held at {grid:?})");
        }
        moved.push(format!("{text:?} → {selected:?} while the grid went {grid:?} → {now:?}"));
    }
    bail!("output moved the grid under each of 3 drags: {}", moved.join("; "))
}

async fn select_copy_paste(terminal: &Entity<Terminal>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let _clipboard = crate::selftest_clipboard::borrow("6b", cx).await?;
    let saved = crate::selftest_clipboard::Snapshot::take()?;
    let result = async {
        let (mut row, mut text) = select_wheel_line(terminal, cx).await?;

        // Another selftest may write the system clipboard at any moment: each read waits for this run's own value, pressing again if a foreign one replaced it.
        let mut copied = None;
        for _ in 0..CLIPBOARD_TRIES {
            // A copy whose OpenClipboard failed (gpui_windows clipboard.rs) still clears the selection.
            if terminal.read_with(cx, |t, _| t.selection_text()).as_deref() != Some(text.as_str()) {
                (row, text) = select_wheel_line(terminal, cx).await?;
            }
            crate::selftest_clipboard::put_text("not copied", cx)?;
            key(cx, "c", Modifiers { control: true, shift: true, ..Default::default() })?;
            copied = read_own(cx, |c| c == text).await?;
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
            crate::selftest_clipboard::put_text(&marker, cx)?;
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
    saved.restore()?;
    result
}

pub(crate) fn key(cx: &mut AsyncWindowContext, key: &str, modifiers: Modifiers) -> anyhow::Result<()> {
    let keystroke = Keystroke { modifiers, key: key.into(), key_char: None };
    cx.update(|window, cx| {
        window.dispatch_event(PlatformInput::KeyDown(KeyDownEvent { keystroke, is_held: false, prefer_character_input: false }), cx);
    })?;
    Ok(())
}

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
        // a terminal is built with placeholder bounds at the origin; output sized from those leaves too little scrollback
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
        let copied = step("6b", select_copy_paste(&terminal, cx)).await?;
        println!("step 6b: {copied}");
        let keys = step("6c", crate::selftest_term::run(view, tab, &terminal, cx)).await?;
        println!("step 6c: {keys}");
        let linked = step("6d", crate::selftest_link::run(view, tab, &terminal, cx)).await?;
        println!("step 6d: {linked}");
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

struct ShutdownProbe {
    name: String,
    attach_pid: u32,
    session_pid: u32,
    before: String,
    asked: Instant,
    /// Set when Quit was applied: a window that closed before it closed on WM_CLOSE, which must only hide it.
    quit: bool,
}

static SHUTDOWN: Mutex<Option<ShutdownProbe>> = Mutex::new(None);

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
        quit: false,
    });
    let posted = post_close_to_own_windows();
    let closed = Instant::now();
    while cx.update(|window, _| crate::tray::is_shown(window))? {
        if closed.elapsed() > Duration::from_secs(5) {
            bail!("the window is still shown 5 s after WM_CLOSE (posted to {posted} window(s))");
        }
        sleep(cx, Duration::from_millis(50)).await;
    }
    sleep(cx, Duration::from_millis(500)).await;
    let tabs = view.read_with(cx, |v, _| v.tabs.iter().map(|t| (t.title.clone(), t.detach)).collect::<Vec<_>>());
    if !tabs.iter().any(|(title, detach)| *title == attached.title && *detach == Detach::None) || !process::alive(attached.pid) {
        bail!("WM_CLOSE hid the window but did not keep the attach tab: {tabs:?}");
    }
    println!(
        "posted WM_CLOSE to {posted} window(s) → hidden after {} ms, the tab still attached (pid {} alive); Quit",
        closed.elapsed().as_millis(),
        attached.pid
    );
    if let Some(probe) = SHUTDOWN.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        probe.asked = Instant::now();
        probe.quit = true;
    }
    let fleet = view.clone();
    cx.update(|window, cx| crate::tray::apply(crate::tray::Command::Quit, &fleet, window, cx))?;
    let asked = Instant::now();
    while asked.elapsed() < DETACH_WITHIN + Duration::from_secs(3) {
        sleep(cx, Duration::from_millis(50)).await;
        if cx.update(|_, _| ()).is_err() {
            return Ok(());
        }
    }
    let state = view.read_with(cx, |v, _| v.tabs.iter().map(|t| format!("{} {:?}", t.title, t.detach)).collect::<Vec<_>>());
    bail!("the window is still open {} ms after Quit; tabs: {state:?}", asked.elapsed().as_millis())
}

#[cfg(windows)]
pub(crate) fn post_close_to_own_windows() -> usize {
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
pub(crate) fn post_close_to_own_windows() -> usize {
    0
}

static VERDICT: AtomicBool = AtomicBool::new(false);

pub fn finish(mode: Mode) {
    let errors = teardown_errors();
    if !errors.is_empty() {
        println!(
            "FAIL log, last step {}: {} window-teardown line(s) in {}: {}",
            current_step(),
            errors.len(),
            log_dir().display(),
            errors.join(" | ")
        );
        EXIT_CODE.store(1, Ordering::SeqCst);
    }
    if mode != Mode::SelftestDetachShutdown {
        // the window closed under the selftest, which ended with it (closing must only hide it)
        if !VERDICT.load(Ordering::SeqCst) && EXIT_CODE.load(Ordering::SeqCst) == 0 {
            println!("FAIL the app quit in step {} before the selftest gave a verdict", current_step());
            EXIT_CODE.store(1, Ordering::SeqCst);
        }
        return;
    }
    if EXIT_CODE.load(Ordering::SeqCst) != 0 {
        return;
    }
    let Some(probe) = SHUTDOWN.lock().unwrap_or_else(|e| e.into_inner()).take() else {
        println!("FAIL the window closed before the selftest attached");
        EXIT_CODE.store(1, Ordering::SeqCst);
        return;
    };
    let closed_after = probe.asked.elapsed();
    let result = (|| {
        if !probe.quit {
            bail!("the window closed {} ms after WM_CLOSE, before Quit: closing must only hide it", closed_after.as_millis());
        }
        if process::alive(probe.attach_pid) {
            bail!("the window closed with the attach process {} still running", probe.attach_pid);
        }
        std::thread::sleep(Duration::from_secs(2));
        let after = session_alive_in_cli(&probe.name)?;
        if after.pid != Some(probe.session_pid) {
            bail!("{} was pid {} before Quit and is {:?} after it", probe.name, probe.session_pid, after.pid);
        }
        Ok(format!(
            "window closed {} ms after Quit, attach pid {} exited; {} alive as the same pid {}: before [{}] after [{}] = {}",
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

    #[test]
    fn teardown_lines_are_those_past_the_offset_or_all_after_a_rotation() {
        let log = b"a window not found\nb ok\nc Invalid window handle\n";
        let second = log.iter().position(|&b| b == b'\n').unwrap() as u64 + 1;
        assert_eq!(teardown_lines(log, 0), ["a window not found", "c Invalid window handle"]);
        assert_eq!(teardown_lines(log, second), ["c Invalid window handle"]);
        assert!(teardown_lines(log, log.len() as u64).is_empty());
        assert_eq!(teardown_lines(log, log.len() as u64 + 1).len(), 2);
    }

    #[test]
    fn find_main_repo_from_checkout_worktrees_and_nowhere() {
        let tmp = std::env::temp_dir().join(format!("fm-selftest-discovery-{}", hex6()));
        let repo = tmp.join("Root").join("Repo");
        std::fs::create_dir_all(repo.join(".git").join("worktrees")).unwrap();
        std::fs::create_dir_all(repo.join("crates").join("supraman-app")).unwrap();
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
        check(&repo.join("crates").join("supraman-app"), ".git dir");
        check(&wts.join("abs"), "absolute gitdir");
        check(&wts.join("rel").join("crates").join("app"), "relative gitdir");
        if main_repo(&tmp).is_none() {
            let error = find_main_repo(&tmp.join("Bare")).unwrap_err().to_string();
            assert!(error.contains("no .git") && error.contains("Bare"), "{error}");
        }
        std::fs::remove_dir_all(&tmp).ok();
    }
}
