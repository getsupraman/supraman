mod brand;
mod clipped;
mod compose;
mod dismissed_messages;
mod elevate;
mod fleet;
mod hand;
mod instance;
mod row_details;
mod saved_tabs;
mod schedules;
mod search;
mod selftest;
mod selftest_capture;
mod selftest_clipboard;
mod selftest_closed;
mod selftest_delete;
mod selftest_desktop;
mod selftest_drop;
mod selftest_first_run;
mod selftest_font;
mod selftest_hand;
mod selftest_left;
mod selftest_link;
mod selftest_manage;
mod selftest_mark;
mod selftest_messages;
mod selftest_notices;
mod selftest_refresh;
mod selftest_reopen;
mod selftest_responsive;
mod selftest_schedules;
mod selftest_search;
mod selftest_shots;
mod selftest_start;
mod selftest_startup;
mod selftest_supervise;
mod selftest_tabs;
mod selftest_title;
mod selftest_updates;
mod selftest_term;
mod selftest_theme;
mod selftest_work;
mod selftest_zoom;
mod settings_window;
mod stars;
mod supervise;
mod tab_card;
mod titlebar;
mod signin;
#[cfg(test)]
mod tokens;
mod tray;
mod updates;
mod view;
mod work_view;
mod workstate_setup;

use std::cell::RefCell;
use std::sync::atomic::{AtomicI32, Ordering};

use supraman_ui::text_input;
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
    let visible = args.last() == Some(&selftest_desktop::VISIBLE_FLAG);
    let args = if visible { &args[..args.len() - 1] } else { &args[..] };
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
    (mode, None)
}

/// Held while a selftest runs, so no build or other selftest shares the machine.
static HEAVY_JOB: std::sync::OnceLock<std::fs::File> = std::sync::OnceLock::new();

/// Tool-shell variables a tab must not inherit; see LEARNINGS fleet UI, tf/5 and tf/8.
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

/// A Windows that ignores the manifest's detached `consoleAllocationPolicy` gives the app its own console.
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

/// Before any window exists: the one way a refused start reaches someone who double-clicked the exe.
#[cfg(windows)]
fn alert(text: &str) {
    use windows::Win32::UI::WindowsAndMessaging::{MB_ICONWARNING, MB_OK, MessageBoxW};
    use windows::core::HSTRING;
    unsafe {
        let _ = MessageBoxW(None, &HSTRING::from(text), &HSTRING::from("Supraman"), MB_OK | MB_ICONWARNING);
    }
}

#[cfg(not(windows))]
fn alert(_text: &str) {}

fn main() {
    supraman_ui::process_start();
    for name in env_scrub(|name| std::env::var(name).ok()) {
        // SAFETY: no other thread exists yet.
        unsafe { std::env::remove_var(name) };
    }
    let (mode, relaunched) = parse_args();
    if mode == Mode::App {
        release_own_console();
    }
    let _machine = match mode {
        Mode::Selftest | Mode::SelftestSupervise | Mode::SelftestDetachShutdown | Mode::SelftestShots => match selftest_clipboard::hold_machine() {
            Ok(held) => Some(held),
            Err(error) => {
                println!("FAIL setup: {error:#}");
                std::process::exit(1);
            }
        },
        _ => None,
    };
    let home = match mode {
        Mode::App => supraman_core::home_from_env(),
        _ => Some(selftest::home()),
    };
    let copied_from = match mode {
        Mode::App if supraman_core::env_home().is_none() => supraman_core::home::legacy_default_home(std::env::var_os("APPDATA")),
        _ => None,
    };
    // The old instance holds the home until it quits.
    if let Some(pid) = relaunched.as_ref().and_then(|r| r.after) {
        elevate::wait_for_exit(pid, elevate::AFTER_WITHIN);
    }
    // Before `prepare`, which clears a home a running selftest may use, before any UAC prompt, and before the copy.
    let mut acquired = home.as_deref().map(|h| instance::acquire(h, copied_from.as_deref()));
    if let Some(Ok(Acquired::Legacy(at))) = &acquired {
        let why = format!("Foreman is running on {}. Quit it (tray, Quit) before starting Supraman.", at.display());
        println!("{why}");
        if mode == Mode::App {
            alert(&why);
            std::process::exit(0);
        }
        std::process::exit(1);
    }
    let migrated = match mode {
        Mode::App if !matches!(acquired, Some(Ok(Acquired::Running))) => match supraman_core::home::migrate_default_home() {
            Ok(migrated) => migrated,
            Err(error) => {
                let why = format!("Supraman could not copy your Foreman home to its own folder, and did not start: {error}");
                println!("{why}");
                alert(&why);
                std::process::exit(1);
            }
        },
        _ => None,
    };
    if mode == Mode::App {
        supraman_ui::init_logger(home.as_deref().map(|h| h.join("logs")).as_deref());
        supraman_ui::log_panics();
    }
    if let Some(m) = migrated {
        log::info!("copied {} files of {} to {}", m.files, m.from.display(), m.to.display());
    }
    if let Some(home) = home.as_deref()
        && matches!(acquired, Some(Ok(Acquired::First(_))))
    {
        let toggle = mode == Mode::App && elevate::toggle(home);
        let elevated = supraman_claude::is_elevated();
        let start = elevate::start(mode == Mode::App, toggle, elevated, relaunched.is_some(), || {
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
                    acquired = Some(instance::acquire(home, copied_from.as_deref()));
                }
            }
        }
    }
    let instance = match acquired {
        Some(Ok(Acquired::First(instance))) => Some(instance),
        Some(Ok(Acquired::Running | Acquired::Legacy(_))) => {
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
        supraman_ui::init_logger(Some(&selftest::log_dir()));
        supraman_ui::log_panics();
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
        supraman_ui::init_globals(cx, env!("CARGO_PKG_VERSION"));
        supraman_ui::stall::watch(cx);
        let home = supraman_core::home_from_env();
        if let Some(problem) = settings_window::apply_theme(home.as_deref(), cx) {
            log::warn!("{problem}");
        }
        view::bind_keys(cx);
        text_input::bind_keys(cx);
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
                    title: Some(SharedString::from("Supraman")),
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
        cx.observe(&fleet, |fleet, cx| {
            let tip = fleet.read(cx).tray_tooltip();
            TRAY.with(|t| {
                if let Some(t) = t.borrow().as_ref() {
                    t.set_tooltip(&tip);
                    t.follow_taskbar();
                }
            });
        })
        .detach();
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
