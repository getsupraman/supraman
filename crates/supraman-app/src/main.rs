// The selftest-only items of the app's own code stay built without the feature.
#![cfg_attr(not(feature = "selftest"), allow(dead_code, unused_imports, unused_variables))]

mod brand;
mod compose;
mod dismissed_messages;
mod elevate;
mod fleet;
mod hand;
mod instance;
mod keeping;
mod row_details;
mod saved_tabs;
mod schedules;
mod search;
#[cfg(feature = "selftest")]
mod selftest;
#[cfg(feature = "selftest")]
mod selftest_clipboard;
#[cfg(feature = "selftest")]
mod selftest_closed;
#[cfg(feature = "selftest")]
mod selftest_delete;
#[cfg(feature = "selftest")]
mod selftest_desktop;
#[cfg(feature = "selftest")]
mod selftest_drop;
#[cfg(feature = "selftest")]
mod selftest_first_run;
#[cfg(feature = "selftest")]
mod selftest_font;
#[cfg(feature = "selftest")]
mod selftest_git;
#[cfg(feature = "selftest")]
mod selftest_git_media;
#[cfg(feature = "selftest")]
mod selftest_hand;
#[cfg(feature = "selftest")]
mod selftest_launch;
#[cfg(feature = "selftest")]
mod selftest_left;
#[cfg(feature = "selftest")]
mod selftest_link;
#[cfg(feature = "selftest")]
mod selftest_manage;
#[cfg(feature = "selftest")]
mod selftest_mark;
#[cfg(feature = "selftest")]
mod selftest_messages;
#[cfg(feature = "selftest")]
mod selftest_notices;
#[cfg(feature = "selftest")]
mod selftest_overflow;
#[cfg(feature = "selftest")]
mod selftest_refresh;
#[cfg(feature = "selftest")]
mod selftest_reopen;
#[cfg(feature = "selftest")]
mod selftest_responsive;
#[cfg(feature = "selftest")]
mod selftest_schedules;
#[cfg(feature = "selftest")]
mod selftest_shells;
#[cfg(feature = "selftest")]
mod selftest_search;
mod selftest_seams;
#[cfg(feature = "selftest")]
mod selftest_shots;
#[cfg(feature = "selftest")]
mod selftest_start;
#[cfg(feature = "selftest")]
mod selftest_supervise;
#[cfg(feature = "selftest")]
mod selftest_tab_keys;
#[cfg(feature = "selftest")]
mod selftest_tab_menu;
#[cfg(feature = "selftest")]
mod selftest_tabs;
#[cfg(feature = "selftest")]
mod selftest_title;
#[cfg(feature = "selftest")]
mod selftest_updates;
#[cfg(feature = "selftest")]
mod selftest_term;
#[cfg(feature = "selftest")]
mod selftest_work;
#[cfg(feature = "selftest")]
mod selftest_zoom;
mod settings_window;
mod stars;
mod supervise;
mod tab_card;
mod titlebar;
mod signin;
#[cfg(test)]
mod test_auth;
mod transcripts_load;
mod tray;
mod updates;
mod view;
mod workman_setup;

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI32, Ordering};

use supraman_ui::text_input;
use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender};
use gpui::{AnyWindowHandle, App, AppContext as _, Bounds, Entity, SharedString, TitlebarOptions, WindowBounds, WindowHandle, WindowOptions, px, size};

use crate::instance::Acquired;
use crate::tray::{Command, Tray};
use crate::view::FleetView;
use supraman_git_view::git_view;
use supraman_work_view::work_view;

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
    SelftestShots,
}

impl Mode {
    pub fn launches_session(self) -> bool {
        matches!(self, Mode::Selftest | Mode::SelftestDetach(_) | Mode::SelftestDetachShutdown)
    }
}

const USAGE: &str =
    "usage: supraman [--selftest | --selftest-detach <n> | --selftest-detach-shutdown | --selftest-perf | --selftest-supervise [--release] | --selftest-shots] [--visible]";

fn parse_args() -> (Mode, Option<elevate::Relaunched>) {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    // the selftest's stand-in claude (selftest_start.rs): its argv as a JSON array
    if let ["--selftest-argv", out, rest @ ..] = args.as_slice() {
        let code = match supraman_core::fs::write_atomic(std::path::Path::new(out), serde_json::to_string(rest).unwrap_or_default().as_bytes()) {
            Ok(()) => 0,
            Err(_) => 1,
        };
        std::process::exit(code);
    }
    if let Some(relaunched) = elevate::parse_relaunch(&args) {
        // SAFETY: no other thread exists yet.
        unsafe { std::env::set_var(supraman_core::HOME_VAR, &relaunched.home) };
        return (Mode::App, Some(relaunched));
    }
    if args.is_empty() {
        return (Mode::App, None);
    }
    #[cfg(feature = "selftest")]
    return (selftest_mode(&args), None);
    #[cfg(not(feature = "selftest"))]
    {
        eprintln!("unknown arguments {args:?}: this supraman is built without its selftests");
        std::process::exit(2);
    }
}

#[cfg(feature = "selftest")]
fn selftest_mode(args: &[&str]) -> Mode {
    let visible = args.last() == Some(&selftest_desktop::VISIBLE_FLAG);
    let args = if visible { &args[..args.len() - 1] } else { args };
    let mode = match args {
        [] => Mode::App,
        ["--selftest"] => Mode::Selftest,
        ["--selftest-detach", n] if n.parse::<u32>().is_ok_and(|n| n > 0) => {
            Mode::SelftestDetach(n.parse().unwrap_or(1))
        }
        ["--selftest-detach-shutdown"] => Mode::SelftestDetachShutdown,
        ["--selftest-perf"] => Mode::SelftestPerf,
        ["--selftest-supervise"] => Mode::SelftestSupervise,
        ["--selftest-supervise", "--release"] => {
            selftest_supervise::RELEASE.store(true, Ordering::Relaxed);
            Mode::SelftestSupervise
        }
        ["--selftest-shots"] => Mode::SelftestShots,
        other => {
            eprintln!("unknown arguments {other:?}; {USAGE}");
            std::process::exit(2);
        }
    };
    let hideable = matches!(mode, Mode::Selftest | Mode::SelftestSupervise | Mode::SelftestDetachShutdown | Mode::SelftestShots);
    if visible && !hideable {
        eprintln!("{} is for --selftest, --selftest-supervise and --selftest-detach-shutdown; {USAGE}", selftest_desktop::VISIBLE_FLAG);
        std::process::exit(2);
    }
    if hideable {
        if selftest_desktop::relaunch_wanted(visible, |name| std::env::var(name).ok()) {
            let args: Vec<std::ffi::OsString> = args.iter().map(Into::into).collect();
            std::process::exit(selftest_desktop::run_hidden(&args));
        }
        println!("{}", selftest_desktop::placement());
    }
    if !matches!(mode, Mode::App) {
        match supraman_core::fs::heavy_job_lock("this selftest") {
            Ok(file) => {
                let _ = HEAVY_JOB.set(file);
            }
            Err(e) => eprintln!("cannot take the heavy-job lock ({e}); running without it"),
        }
    }
    mode
}

/// Held while a selftest runs, so no build or other selftest shares the machine.
#[cfg(feature = "selftest")]
static HEAVY_JOB: std::sync::OnceLock<std::fs::File> = std::sync::OnceLock::new();

/// Tool-shell variables a tab must not inherit; see docs/LEARNINGS.md, fleet UI.
fn env_scrub(get: impl Fn(&str) -> Option<String>) -> Vec<&'static str> {
    let mut scrub = vec!["CLAUDE_CODE_CHILD_SESSION"];
    if get("CLAUDECODE").as_deref() == Some("1") {
        scrub.extend(["CLAUDECODE", "NO_COLOR"]);
        scrub.extend(TOOL_SHELL);
    }
    scrub
}

/// A Claude Code 2.1 tool shell's other exports, as measured on 2026-10-26.
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

fn main() {
    supraman_ui::process_start();
    for name in env_scrub(|name| std::env::var(name).ok()) {
        // SAFETY: no other thread exists yet.
        unsafe { std::env::remove_var(name) };
    }
    let (mode, relaunched) = parse_args();
    if mode == Mode::App {
        supraman_core::platform::desktop::detach_console();
    }
    #[cfg(feature = "selftest")]
    let _machine = hold_machine(mode);
    let home = match mode {
        Mode::App => supraman_core::home_from_env(),
        #[cfg(feature = "selftest")]
        _ => Some(selftest::home()),
        #[cfg(not(feature = "selftest"))]
        _ => None,
    };
    // The old instance holds the home until it quits.
    if let Some(pid) = relaunched.as_ref().and_then(|r| r.after) {
        elevate::wait_for_exit(pid, elevate::AFTER_WITHIN);
    }
    let instance = single_instance(mode, home, relaunched.is_some());
    #[cfg(feature = "selftest")]
    prepare_selftest(mode);
    let (commands, received) = futures::channel::mpsc::unbounded::<Command>();
    if let Some(instance) = &instance {
        let commands = commands.clone();
        if let Err(error) = instance.on_open(move || {
            log::info!("a second launch asked for the window");
            commands.unbounded_send(Command::Open).ok();
        }) {
            eprintln!("a second launch cannot bring this window forward: {error:#}");
        }
    }
    gpui_platform::application().run(move |cx: &mut App| run(mode, commands, received, cx));
    TRAY.with(|t| t.borrow_mut().take());
    #[cfg(feature = "selftest")]
    if mode != Mode::App {
        selftest::finish(mode);
        selftest::cleanup();
    }
    std::process::exit(EXIT_CODE.load(Ordering::SeqCst));
}

#[cfg(feature = "selftest")]
fn prepare_selftest(mode: Mode) {
    if mode == Mode::App {
        return;
    }
    if let Err(error) = selftest::prepare(mode) {
        println!("FAIL setup: {error:#}");
        selftest::cleanup();
        std::process::exit(1);
    }
    // After `prepare` cleared the selftest's home.
    supraman_ui::init_logger(Some(&selftest::log_dir()));
    supraman_ui::log_panics();
}

#[cfg(feature = "selftest")]
fn hold_machine(mode: Mode) -> Option<selftest_clipboard::Held> {
    match mode {
        Mode::Selftest | Mode::SelftestSupervise | Mode::SelftestDetachShutdown | Mode::SelftestShots => match selftest_clipboard::hold_machine() {
            Ok(held) => Some(held),
            Err(error) => {
                println!("FAIL setup: {error:#}");
                std::process::exit(1);
            }
        },
        _ => None,
    }
}

fn single_instance(mode: Mode, home: Option<PathBuf>, relaunched: bool) -> Option<instance::Instance> {
    // Before `prepare`, which clears a home a running selftest may use, and before any UAC prompt.
    let mut acquired = home.as_deref().map(instance::acquire);
    if mode == Mode::App {
        supraman_ui::init_logger(home.as_deref().map(|h| h.join("logs")).as_deref());
        supraman_ui::log_panics();
    }
    if let Some(home) = home.as_deref()
        && matches!(acquired, Some(Ok(Acquired::First(_))))
    {
        let toggle = mode == Mode::App && elevate::toggle(home);
        let elevated = supraman_claude::is_elevated();
        let start = elevate::start(mode == Mode::App, toggle, elevated, relaunched, || {
            // the copy takes the lock
            acquired = None;
            elevate::relaunch(home, None)
        });
        match start {
            elevate::Start::Exit => std::process::exit(0),
            elevate::Start::Run { declined } => {
                if let Some(why) = declined {
                    log::warn!("Run as administrator is on, but Supraman runs unelevated: {why}");
                }
                if acquired.is_none() {
                    acquired = Some(instance::acquire(home));
                }
            }
        }
    }
    match acquired {
        Some(Ok(Acquired::First(instance))) => Some(instance),
        Some(Ok(Acquired::Running)) => {
            let home = home.unwrap_or_default();
            if mode == Mode::App {
                println!("Supraman is already running for {}: asked it to show its window.", home.display());
                std::process::exit(0);
            }
            println!("FAIL setup: another Supraman runs on {}", home.display());
            std::process::exit(1);
        }
        Some(Err(error)) => {
            eprintln!("cannot take the single-instance lock: {error:#}");
            None
        }
        None => None,
    }
}

fn run(mode: Mode, commands: UnboundedSender<Command>, mut received: UnboundedReceiver<Command>, cx: &mut App) {
    supraman_editor::init(supraman_ui::fonts::MONO, 13., cx);
    supraman_ui::init_globals(cx, env!("CARGO_PKG_VERSION"));
    supraman_ui::stall::watch(cx);
    let home = supraman_core::home_from_env();
    if let Some(home) = home.clone() {
        keeping::start(home, cx.background_executor());
    }
    if let Some(problem) = settings_window::apply_theme(home.as_deref(), cx) {
        log::warn!("{problem}");
    }
    view::bind_keys(cx);
    text_input::bind_keys(cx);
    git_view::bind_keys(cx);
    cx.on_window_closed(|cx, _| {
        if cx.windows().is_empty() {
            cx.quit();
        }
    })
    .detach();
    let handle = open_main_window(mode, cx);
    cx.activate(true);
    match tray::install(commands) {
        Ok(tray) => TRAY.with(|t| *t.borrow_mut() = Some(tray)),
        Err(error) => eprintln!("no tray icon: {error:#}"),
    }
    let fleet = handle.update(cx, |_, _, cx| cx.entity()).expect("the window just opened");
    follow_tray(&fleet, cx);
    let window = AnyWindowHandle::from(handle);
    cx.spawn(async move |cx| {
        while let Some(command) = received.next().await {
            if window.update(cx, |_, window, cx| tray::apply(command, &fleet, window, cx)).is_err() {
                return;
            }
        }
    })
    .detach();
}

fn open_main_window(mode: Mode, cx: &mut App) -> WindowHandle<FleetView> {
    let bounds = Bounds::centered(None, size(px(1400.), px(820.)), cx);
    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some(SharedString::from("Supraman")),
                appears_transparent: true,
                traffic_light_position: None,
            }),
            ..Default::default()
        },
        move |window, cx| {
            let view = cx.new(|cx| FleetView::new(window, cx));
            #[cfg(feature = "selftest")]
            if mode != Mode::App {
                selftest::start(mode, view.clone(), window, cx);
            }
            view
        },
    )
    .expect("failed to open window")
}

fn follow_tray(fleet: &Entity<FleetView>, cx: &mut App) {
    cx.observe(fleet, |fleet, cx| {
        let tip = fleet.read(cx).tray_tooltip();
        TRAY.with(|t| {
            if let Some(t) = t.borrow().as_ref() {
                t.set_tooltip(&tip);
            }
        });
    })
    .detach();
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(tray::FOLLOW_TASKBAR_EVERY).await;
            cx.update(|_| {
                TRAY.with(|t| {
                    if let Some(t) = t.borrow().as_ref() {
                        t.follow_taskbar();
                    }
                })
            });
        }
    })
    .detach();
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
