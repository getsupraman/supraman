// foreman: the fleet app. Workspaces → supervised sessions and repos → sessions on the left, with
// Start/Stop per workspace, the Inbox and the shared busy-session limit; attached terminals in tabs on
// the right. Only workspaces the human started are supervised (`foreman-supervisor`).
//   foreman                  the app
//   foreman --selftest       the M1 gate: one throwaway session, attach, detach, the Work tab on a fake
//                            WorkState, cleanup, perf
//   foreman --selftest-detach <n>       one session, n attach/close cycles, alive after each
//   foreman --selftest-detach-shutdown  attach, close the window, session still alive
//   foreman --selftest-perf  1 then 10 pwsh tabs: private bytes and frame times
//   foreman --selftest-supervise        the M2 gate: Start, kill → resume, trust, park, budget, Stop
// One Foreman per FOREMAN_HOME (`instance`); closing its window hides it, the tray icon brings it back
// or quits (`tray`), so supervision outlives the window (m5/1). Log lines go to <home>\logs\foreman.log
// (a selftest's home has its own), never to a console (m5/22).

mod compose;
mod elevate;
mod fleet;
mod hand;
mod instance;
mod selftest;
mod selftest_first_run;
mod selftest_left;
mod selftest_manage;
mod selftest_shell;
mod selftest_supervise;
mod selftest_tabs;
mod selftest_term;
mod selftest_work;
mod settings_window;
mod supervise;
mod titlebar;
mod tokens;
mod tray;
mod view;
mod work_view;
mod workstate_setup;

use std::cell::RefCell;
use std::sync::atomic::{AtomicI32, Ordering};

use foreman_core::Registry;
use futures::StreamExt as _;
use gpui::{AnyWindowHandle, App, AppContext as _, Bounds, SharedString, TitlebarOptions, WindowBounds, WindowOptions, px, size};

use crate::instance::Acquired;
use crate::tray::{Command, Tray};
use crate::view::FleetView;

thread_local! {
    /// Dropped after the event loop, so the icon leaves the notification area with the process.
    static TRAY: RefCell<Option<Tray>> = const { RefCell::new(None) };
}

pub static EXIT_CODE: AtomicI32 = AtomicI32::new(0);

#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    App,
    Selftest,
    SelftestDetach(u32),
    SelftestDetachShutdown,
    SelftestPerf,
    SelftestSupervise,
}

impl Mode {
    pub fn launches_session(self) -> bool {
        matches!(self, Mode::Selftest | Mode::SelftestDetach(_) | Mode::SelftestDetachShutdown)
    }
}

const USAGE: &str =
    "usage: foreman [--selftest | --selftest-detach <n> | --selftest-detach-shutdown | --selftest-perf | --selftest-supervise]";

/// The mode, and for the elevated copy [`elevate::relaunch`] started what it was given.
fn parse_args() -> (Mode, Option<elevate::Relaunched>) {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    if let Some(relaunched) = elevate::parse_relaunch(&args) {
        // SAFETY: no other thread exists yet.
        unsafe { std::env::set_var("FOREMAN_HOME", &relaunched.home) };
        return (Mode::App, Some(relaunched));
    }
    let mode = match args.as_slice() {
        [] => Mode::App,
        ["--selftest"] => Mode::Selftest,
        ["--selftest-detach", n] if n.parse::<u32>().is_ok_and(|n| n > 0) => {
            Mode::SelftestDetach(n.parse().unwrap_or(1))
        }
        ["--selftest-detach-shutdown"] => Mode::SelftestDetachShutdown,
        ["--selftest-perf"] => Mode::SelftestPerf,
        ["--selftest-supervise"] => Mode::SelftestSupervise,
        other => {
            eprintln!("unknown arguments {other:?}; {USAGE}");
            std::process::exit(2);
        }
    };
    (mode, None)
}

/// Variables Foreman must not pass on when it was started from a Claude Code tool shell
/// (LEARNINGS, fleet UI; tf/5). Without `CLAUDE_CODE_CHILD_SESSION` gone, every `claude` a tab runs
/// keeps no transcript. `CLAUDECODE=1` marks the tool shell: with it, `NO_COLOR` is that shell's,
/// not the user's, and would render every tab monochrome; `CLAUDECODE` itself makes a tab's
/// `claude` count as nested. A `NO_COLOR` without `CLAUDECODE` is the user's choice and stays.
/// The rest of `TOOL_SHELL` is the parent session's identity (tf/8): its messaging socket and
/// token would let a tab's process speak as that session. Without `CLAUDECODE` they stay too, so a
/// `CLAUDE_EFFORT` a user set on purpose is kept.
fn env_scrub(get: impl Fn(&str) -> Option<String>) -> Vec<&'static str> {
    let mut scrub = vec!["CLAUDE_CODE_CHILD_SESSION"];
    if get("CLAUDECODE").as_deref() == Some("1") {
        scrub.extend(["CLAUDECODE", "NO_COLOR"]);
        scrub.extend(TOOL_SHELL);
    }
    scrub
}

/// What a Claude Code 2.1 tool shell exports besides `CLAUDECODE`, `NO_COLOR` and
/// `CLAUDE_CODE_CHILD_SESSION`, as measured on 2026-10-26.
const TOOL_SHELL: [&str; 10] = [
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_PID",
    "CLAUDE_CODE_SESSION_ATTENDED",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_MESSAGING_TOKEN",
    "CLAUDE_CODE_EXECPATH",
    "CLAUDE_CODE_ALT_SCREEN_FULL_REPAINT",
    "CLAUDE_EFFORT",
    "CLAUDE_JOB_DIR",
];

/// foreman.exe is a console program whose manifest asks for no console of its own
/// (`consoleAllocationPolicy` detached, assets/foreman.manifest): started from Explorer it has none,
/// from a shell it shares the shell's, so `--selftest` prints there and the shell waits for its exit
/// code (a GUI-subsystem exe gets neither from pwsh). A Windows without that policy gives the app a
/// console of its own, which goes here, after a flash.
#[cfg(windows)]
fn release_own_console() {
    use windows::Win32::System::Console::{FreeConsole, GetConsoleProcessList};
    let mut attached = [0u32; 2];
    if unsafe { GetConsoleProcessList(&mut attached) } == 1 {
        unsafe {
            let _ = FreeConsole();
        }
    }
}

#[cfg(not(windows))]
fn release_own_console() {}

fn main() {
    foreman_ui::process_start();
    for name in env_scrub(|name| std::env::var(name).ok()) {
        // SAFETY: no other thread exists yet.
        unsafe { std::env::remove_var(name) };
    }
    let (mode, relaunched) = parse_args();
    if mode == Mode::App {
        release_own_console();
    }
    let home = match mode {
        Mode::App => Registry::from_env().ok().map(|r| r.home().to_path_buf()),
        _ => Some(selftest::home()),
    };
    if mode == Mode::App {
        foreman_ui::init_logger(home.as_deref().map(|h| h.join("logs")).as_deref());
    }
    // Restarted as administrator from the Inbox: the old instance still holds the home until it quits.
    if let Some(pid) = relaunched.as_ref().and_then(|r| r.after) {
        elevate::wait_for_exit(pid, elevate::AFTER_WITHIN);
    }
    // Before `prepare`: a selftest clears its home, which a running one on the same home still uses.
    // First, so a launch that only hands off to a running instance never asks UAC.
    let mut acquired = home.as_deref().map(instance::acquire);
    if let Some(home) = home.as_deref()
        && matches!(acquired, Some(Ok(Acquired::First(_))))
    {
        let toggle = mode == Mode::App && elevate::toggle(home);
        let elevated = foreman_claude::is_elevated();
        let start = elevate::start(mode == Mode::App, toggle, elevated, relaunched.is_some(), || {
            // the copy takes the lock
            acquired = None;
            elevate::relaunch(home, None)
        });
        match start {
            elevate::Start::Exit => std::process::exit(0),
            elevate::Start::Run { declined } => {
                if let Some(why) = declined {
                    log::warn!("Run as administrator is on, but Foreman runs unelevated: {why}");
                }
                if acquired.is_none() {
                    acquired = Some(instance::acquire(home));
                }
            }
        }
    }
    let instance = match acquired {
        Some(Ok(Acquired::First(instance))) => Some(instance),
        Some(Ok(Acquired::Running)) => {
            let home = home.unwrap_or_default();
            if mode == Mode::App {
                println!("Foreman is already running for {}: asked it to show its window.", home.display());
                std::process::exit(0);
            }
            println!("FAIL setup: another Foreman runs on {}", home.display());
            std::process::exit(1);
        }
        Some(Err(error)) => {
            eprintln!("cannot take the single-instance lock: {error:#}");
            None
        }
        None => None,
    };
    if mode != Mode::App
        && let Err(error) = selftest::prepare(mode)
    {
        println!("FAIL setup: {error:#}");
        selftest::cleanup();
        std::process::exit(1);
    }
    // After `prepare` cleared the selftest's home.
    if mode != Mode::App {
        foreman_ui::init_logger(Some(&selftest::log_dir()));
    }

    let (commands, mut received) = futures::channel::mpsc::unbounded::<Command>();
    if let Some(instance) = &instance {
        let commands = commands.clone();
        if let Err(error) = instance.on_open(move || {
            log::info!("a second launch asked for the window");
            commands.unbounded_send(Command::Open).ok();
        }) {
            eprintln!("a second launch cannot bring this window forward: {error:#}");
        }
    }

    gpui_platform::application().run(move |cx: &mut App| {
        foreman_ui::init_globals(cx, env!("CARGO_PKG_VERSION"));
        view::bind_keys(cx);
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let bounds = Bounds::centered(None, size(px(1400.), px(820.)), cx);
        let handle = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some(SharedString::from("Foreman")),
                    appears_transparent: true,
                    traffic_light_position: None,
                }),
                ..Default::default()
            },
            move |window, cx| {
                let view = cx.new(|cx| FleetView::new(window, cx));
                if mode != Mode::App {
                    selftest::start(mode, view.clone(), window, cx);
                }
                view
            },
        )
        .expect("failed to open window");
        cx.activate(true);
        match tray::install(commands) {
            Ok(tray) => TRAY.with(|t| *t.borrow_mut() = Some(tray)),
            Err(error) => eprintln!("no tray icon: {error:#}"),
        }
        let fleet = handle.update(cx, |_, _, cx| cx.entity()).expect("the window just opened");
        let window = AnyWindowHandle::from(handle);
        cx.spawn(async move |cx| {
            while let Some(command) = received.next().await {
                if window.update(cx, |_, window, cx| tray::apply(command, &fleet, window, cx)).is_err() {
                    return;
                }
            }
        })
        .detach();
    });
    TRAY.with(|t| t.borrow_mut().take());
    if mode != Mode::App {
        selftest::finish(mode);
        selftest::cleanup();
    }
    std::process::exit(EXIT_CODE.load(Ordering::SeqCst));
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{TOOL_SHELL, env_scrub};

    fn scrub(vars: &[(&str, &str)]) -> Vec<&'static str> {
        let env: HashMap<String, String> =
            vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        env_scrub(|name| env.get(name).cloned())
    }

    #[test]
    fn env_scrub_drops_no_color_from_a_claude_code_shell() {
        let scrubbed = scrub(&[("CLAUDECODE", "1"), ("NO_COLOR", "1")]);
        assert!(scrubbed.contains(&"NO_COLOR"));
        assert!(scrubbed.contains(&"CLAUDECODE"));
    }

    #[test]
    fn env_scrub_keeps_a_users_no_color() {
        let scrubbed = scrub(&[("NO_COLOR", "1")]);
        assert!(!scrubbed.contains(&"NO_COLOR"));
        assert!(!scrubbed.contains(&"CLAUDECODE"));
    }

    #[test]
    fn env_scrub_drops_the_parent_sessions_identity_from_a_claude_code_shell() {
        let scrubbed = scrub(&[("CLAUDECODE", "1")]);
        assert!(scrubbed.contains(&"CLAUDE_CODE_MESSAGING_SOCKET"));
        assert!(scrubbed.contains(&"CLAUDE_CODE_MESSAGING_TOKEN"));
        for name in TOOL_SHELL {
            assert!(scrubbed.contains(&name), "{name}");
        }
    }

    #[test]
    fn env_scrub_keeps_claude_vars_without_claudecode() {
        let scrubbed = scrub(&[("CLAUDE_EFFORT", "high"), ("CLAUDE_CODE_MESSAGING_SOCKET", "x")]);
        assert_eq!(scrubbed, ["CLAUDE_CODE_CHILD_SESSION"]);
    }

    #[test]
    fn env_scrub_always_drops_child_session() {
        assert!(scrub(&[]).contains(&"CLAUDE_CODE_CHILD_SESSION"));
        assert!(scrub(&[("CLAUDECODE", "1")]).contains(&"CLAUDE_CODE_CHILD_SESSION"));
    }
}
