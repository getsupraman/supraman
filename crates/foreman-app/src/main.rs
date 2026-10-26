// foreman: the fleet app. Workspaces → supervised sessions and repos → sessions on the left, with
// Start/Stop per workspace, the Inbox and the shared busy-session limit; attached terminals in tabs on
// the right. Only workspaces the human started are supervised (`foreman-supervisor`).
//   foreman                  the app
//   foreman --selftest       the M1 gate: one throwaway session, attach, detach, cleanup, perf
//   foreman --selftest-detach <n>       one session, n attach/close cycles, alive after each
//   foreman --selftest-detach-shutdown  attach, close the window, session still alive
//   foreman --selftest-perf  1 then 10 pwsh tabs: private bytes and frame times
//   foreman --selftest-supervise        the M2 gate: Start, kill → resume, trust, park, budget, Stop

mod fleet;
mod selftest;
mod selftest_supervise;
mod settings_window;
mod supervise;
mod titlebar;
mod tokens;
mod view;

use std::sync::atomic::{AtomicI32, Ordering};

use gpui::{App, AppContext as _, Bounds, SharedString, TitlebarOptions, WindowBounds, WindowOptions, px, size};

use crate::view::FleetView;

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

fn parse_args() -> Mode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
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
    }
}

/// Variables Foreman must not pass on when it was started from a Claude Code tool shell
/// (LEARNINGS, fleet UI; tf/5). Without `CLAUDE_CODE_CHILD_SESSION` gone, every `claude` a tab runs
/// keeps no transcript. `CLAUDECODE=1` marks the tool shell: with it, `NO_COLOR` is that shell's,
/// not the user's, and would render every tab monochrome; `CLAUDECODE` itself makes a tab's
/// `claude` count as nested. A `NO_COLOR` without `CLAUDECODE` is the user's choice and stays.
fn env_scrub(get: impl Fn(&str) -> Option<String>) -> Vec<&'static str> {
    let mut scrub = vec!["CLAUDE_CODE_CHILD_SESSION"];
    if get("CLAUDECODE").as_deref() == Some("1") {
        scrub.extend(["CLAUDECODE", "NO_COLOR"]);
    }
    scrub
}

fn main() {
    foreman_ui::process_start();
    for name in env_scrub(|name| std::env::var(name).ok()) {
        // SAFETY: no other thread exists yet.
        unsafe { std::env::remove_var(name) };
    }
    let mode = parse_args();
    foreman_ui::init_logger();
    if mode != Mode::App
        && let Err(error) = selftest::prepare(mode)
    {
        println!("FAIL setup: {error:#}");
        selftest::cleanup();
        std::process::exit(1);
    }

    gpui_platform::application().run(move |cx: &mut App| {
        foreman_ui::init_globals(cx, env!("CARGO_PKG_VERSION"));
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let bounds = Bounds::centered(None, size(px(1400.), px(820.)), cx);
        cx.open_window(
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
    });
    if mode != Mode::App {
        selftest::finish(mode);
        selftest::cleanup();
    }
    std::process::exit(EXIT_CODE.load(Ordering::SeqCst));
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::env_scrub;

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
    fn env_scrub_always_drops_child_session() {
        assert!(scrub(&[]).contains(&"CLAUDE_CODE_CHILD_SESSION"));
        assert!(scrub(&[("CLAUDECODE", "1")]).contains(&"CLAUDE_CODE_CHILD_SESSION"));
    }
}
