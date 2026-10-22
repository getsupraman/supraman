//! `--selftest` (the M1 gate) and `--selftest-perf`.
//!
//! Both run against a scratch `FOREMAN_HOME` under `<workspace>/target/` holding one platform: a
//! read-only scan of the folder that contains the main repo, found through the nearest `.git` above
//! the workspace (P:/Projects/LarchSys → `larchsys`, from the main checkout or any linked worktree).
//! The gate also launches one `fm-selftest-<hex>` background session with its cwd in
//! `<workspace>/target/selftest-cwd`: inside the already-trusted tree the exe was built in, so from a
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
use foreman_claude::{Session, check_supported, list_sessions, main_repo, now_ms};
use foreman_core::{DEFAULT_DEPTH, Platform, Registry, scan};
use foreman_term::Terminal;
use foreman_ui::{memory, mib};
use gpui::{
    App, AsyncWindowContext, Entity, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, PlatformInput, Window,
};

use crate::{
    EXIT_CODE, Mode,
    fleet::{self, SessionRow},
    view::{FleetView, TabKind},
};

pub const PLATFORM: &str = "larchsys";
const PROMPT: &str = "Reply with the word ready and then wait for input";
const LISTED_WITHIN: Duration = Duration::from_secs(15);
const RENDERED_WITHIN: Duration = Duration::from_secs(10);

struct Scratch {
    dirs: Vec<PathBuf>,
    session_name: Option<String>,
}

static SCRATCH: Mutex<Option<Scratch>> = Mutex::new(None);
static SESSION_NAME: Mutex<Option<String>> = Mutex::new(None);
/// The main repo's folder name, the row the gate expects its session under.
static REPO: OnceLock<String> = OnceLock::new();

fn repo() -> &'static str {
    REPO.get().map_or("", String::as_str)
}

fn workspace_root() -> &'static Path {
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

fn hex6() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!("{:06x}", (nanos as u64 ^ (std::process::id() as u64) << 20) & 0xff_ffff)
}

fn remove_dir_retrying(dir: &Path) -> std::io::Result<()> {
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

fn claude(args: &[&str], cwd: Option<&Path>) -> anyhow::Result<String> {
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

/// Before the window: scratch dirs, the platform registered through foreman-core, and (gate only)
/// the throwaway session launched from the scratch cwd.
pub fn prepare(mode: Mode) -> anyhow::Result<()> {
    let target = workspace_root().join("target");
    let home = target.join("selftest-home");
    let cwd = target.join("selftest-cwd");
    for dir in [&home, &cwd] {
        remove_dir_retrying(dir).with_context(|| format!("cannot clear {}", dir.display()))?;
    }
    *lock() = Some(Scratch {
        dirs: vec![home.clone(), cwd.clone()],
        session_name: None,
    });
    install_guards(mode);

    // Still single-threaded apart from the watchdog, which never reads the environment.
    unsafe { std::env::set_var("FOREMAN_HOME", &home) };
    std::fs::create_dir_all(&cwd)?;
    // <root>/AutoWork or <root>/AutoWork.worktrees/<unit> → main repo <root>/AutoWork → <root>.
    let main = find_main_repo(workspace_root())?;
    let platform_root = main
        .parent()
        .ok_or_else(|| anyhow!("main repo {} has no parent", main.display()))?;
    let repo_name = main
        .file_name()
        .ok_or_else(|| anyhow!("main repo {} has no folder name", main.display()))?
        .to_string_lossy()
        .into_owned();
    let registry = Registry::from_env()?;
    let repos = scan(platform_root, DEFAULT_DEPTH)?;
    if !repos.iter().any(|r| r.name == repo_name) {
        bail!("scan of {} found no {repo_name} repo (main repo {})", platform_root.display(), main.display());
    }
    REPO.set(repo_name).ok();
    let count = repos.len();
    let saved = registry.save(&Platform::new(PLATFORM, platform_root, repos), true)?;
    println!(
        "step 1: FOREMAN_HOME={} ; platform {PLATFORM} = {} ({count} repos, read-only scan) registered at {}",
        home.display(),
        platform_root.display(),
        saved.display()
    );

    if mode == Mode::Selftest {
        let version = check_supported()?;
        println!("claude CLI: {}", version.raw);
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

fn lock() -> MutexGuard<'static, Option<Scratch>> {
    SCRATCH.lock().unwrap_or_else(|e| e.into_inner())
}

fn install_guards(mode: Mode) {
    let budget = match mode {
        Mode::Selftest => Duration::from_secs(150),
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
                _ => perf(&view, cx).await,
            };
            match result {
                Ok(()) => println!("PASS"),
                Err(e) => {
                    println!("FAIL {e:#}");
                    EXIT_CODE.store(1, Ordering::SeqCst);
                }
            }
            // Off the UI thread: cleanup blocks on `claude` processes and file deletes.
            cx.background_executor().spawn(async { cleanup() }).await;
            cx.update(|_, cx| cx.quit()).ok();
        })
        .detach();
}

async fn sleep(cx: &AsyncWindowContext, duration: Duration) {
    cx.background_executor().timer(duration).await;
}

/// A left click at the centre of the element painted under `key`, through the window's own input
/// dispatch, so it lands only if that element is really there and hit-testable.
async fn click(view: &Entity<FleetView>, key: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let bounds = view
        .read_with(cx, |v, _| v.hits.borrow().get(key).copied())
        .ok_or_else(|| anyhow!("nothing painted under {key}"))?;
    let position = bounds.center();
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
                button: MouseButton::Left,
                position,
                modifiers,
                click_count: 1,
                first_mouse: false,
            }),
            cx,
        );
        window.dispatch_event(
            PlatformInput::MouseUp(MouseUpEvent {
                button: MouseButton::Left,
                position,
                modifiers,
                click_count: 1,
            }),
            cx,
        );
    })?;
    Ok(())
}

fn find_row(view: &FleetView, name: &str) -> Option<SessionRow> {
    view.tree()
        .platforms
        .into_iter()
        .find(|p| p.name == PLATFORM)?
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

async fn wait_for_tab_terminal(
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

async fn record_frames(
    view: &Entity<FleetView>,
    window: Duration,
    cx: &mut AsyncWindowContext,
) -> foreman_ui::FrameStats {
    view.update(cx, |v, cx| {
        v.probe.start();
        cx.notify();
    });
    sleep(cx, window).await;
    view.update(cx, |v, _| v.probe.stop())
}

fn session_alive_in_cli(name: &str) -> anyhow::Result<Session> {
    let rows = list_sessions()?;
    let row = rows
        .into_iter()
        .find(|s| s.name == name)
        .ok_or_else(|| anyhow!("{name} is no longer listed in claude agents --json --all"))?;
    let health = row.health(now_ms());
    if !fleet::is_alive(&health) {
        bail!("{name} is listed but {health} — the close stopped it instead of detaching");
    }
    Ok(row)
}

async fn gate(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let name = SESSION_NAME
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .ok_or_else(|| anyhow!("no session was launched"))?;
    let executor = cx.background_executor().clone();

    // 3. Listed under the scratch platform/repo with a live health and a painted [attach].
    let opened = Instant::now();
    let row = loop {
        let row = view.read_with(cx, |v, _| find_row(v, &name));
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
                format!("listed as {} (pid {:?}, attachable {})", r.health, r.session.pid, r.attachable)
            });
            bail!("step 3: {name} not live under {PLATFORM}/./{} within {} s: {seen}", repo(), LISTED_WITHIN.as_secs());
        }
        sleep(cx, Duration::from_millis(100)).await;
    };
    let id = row.session.id.clone();
    let name_for_resolve = name.clone();
    let resolved = executor
        .spawn(async move {
            list_sessions().map(|rows| {
                rows.into_iter()
                    .filter(|s| s.name == name_for_resolve)
                    .map(|s| s.id)
                    .collect::<Vec<_>>()
            })
        })
        .await?;
    if resolved != [id.clone()] {
        bail!("step 3: the panel row's id {id} is not the one id resolved by name from claude agents --json --all: {resolved:?}");
    }
    println!(
        "step 3: {name} ({id}) listed under {PLATFORM} / ./{} as {} after {} ms",
        repo(),
        row.health,
        opened.elapsed().as_millis()
    );

    // 4. Click [attach]; a tab opens; its painted grid gains output within 10 s.
    let clicked = Instant::now();
    click(view, &format!("attach-{id}"), cx).await?;
    let (tab, title) = view
        .read_with(cx, |v, _| {
            v.tabs.iter().find_map(|t| match &t.kind {
                TabKind::Attach { session_id } if *session_id == id => Some((t.id, t.title.clone())),
                _ => None,
            })
        })
        .ok_or_else(|| anyhow!("step 4: clicking attach-{id} opened no tab for {id}"))?;
    let tabs = view.read_with(cx, |v, _| v.tabs.len());
    if tabs != 1 {
        bail!("step 4: expected exactly one tab after the click, found {tabs}");
    }
    let terminal = wait_for_tab_terminal(view, tab, RENDERED_WITHIN, cx).await?;
    let pid = terminal
        .read_with(cx, |t, _| t.pid())
        .map(|p| p.as_u32())
        .ok_or_else(|| anyhow!("step 4: the attach terminal has no process id"))?;
    let at_start = painted_cells(&terminal, cx);
    let rendered = loop {
        let cells = painted_cells(&terminal, cx);
        if cells > at_start {
            break cells;
        }
        if clicked.elapsed() > RENDERED_WITHIN {
            bail!(
                "step 4: tab \"{title}\" painted no output within {} s (non-blank cells stayed at {at_start})",
                RENDERED_WITHIN.as_secs()
            );
        }
        sleep(cx, Duration::from_millis(20)).await;
    };
    let text = terminal.read_with(cx, |t, _| t.get_content());
    println!(
        "step 4: tab \"{title}\" (claude attach, pid {pid}) painted {rendered} non-blank cells (from {at_start}) {} ms after the click; first line: {}",
        clicked.elapsed().as_millis(),
        first_line(&text)
    );

    // 5. Close the tab: the attach process exits, the session stays alive.
    drop(terminal);
    click(view, &format!("close-tab-{tab}"), cx).await?;
    if view.read_with(cx, |v, _| v.tab(tab).is_some()) {
        bail!("step 5: clicking close-tab-{tab} left the tab open");
    }
    let closed = Instant::now();
    while process_alive(pid) {
        if closed.elapsed() > Duration::from_secs(5) {
            bail!("step 5: the attach process {pid} is still running 5 s after the tab closed");
        }
        sleep(cx, Duration::from_millis(20)).await;
    }
    let exited_after = closed.elapsed();
    sleep(cx, Duration::from_secs(2)).await;
    let name_for_check = name.clone();
    let session = executor.spawn(async move { session_alive_in_cli(&name_for_check) }).await?;
    println!(
        "step 5: tab closed, attach process {pid} exited after {} ms; {name} still alive in claude agents --json --all as {} (pid {:?})",
        exited_after.as_millis(),
        session.health(now_ms()),
        session.pid
    );

    // 7. Idle memory and frame times while the panel keeps refreshing.
    let (idle_private, idle_ws) = memory();
    let refreshes_before = view.read_with(cx, |v, _| v.refreshes);
    let stats = record_frames(view, Duration::from_secs(6), cx).await;
    let refreshes = view.read_with(cx, |v, _| v.refreshes) - refreshes_before;
    println!("idle private bytes: {} (working set {})", mib(idle_private), mib(idle_ws));
    println!("frame time over 6 s with {refreshes} panel refreshes: {stats}");
    if refreshes == 0 {
        bail!("step 7: the panel did not refresh during the 6 s frame window");
    }
    Ok(())
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
        let stats = record_frames(view, Duration::from_secs(5), cx).await;
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

#[cfg(windows)]
fn process_alive(pid: u32) -> bool {
    use windows::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return false;
        };
        let mut code = 0u32;
        let ok = GetExitCodeProcess(handle, &mut code).is_ok();
        let _ = CloseHandle(handle);
        ok && code == STILL_ACTIVE.0 as u32
    }
}

#[cfg(not(windows))]
fn process_alive(_pid: u32) -> bool {
    false
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
            assert_eq!(main.parent().unwrap().file_name().unwrap(), "Root", "{label}: platform root");
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
