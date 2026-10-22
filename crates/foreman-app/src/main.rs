// foreman: the read-only fleet viewer. Platforms → repos → sessions on the left, attached terminals
// in tabs on the right. It shows and attaches; it never starts or stops a session.
//   foreman                  the app
//   foreman --selftest       the M1 gate: one throwaway session, attach, detach, cleanup, perf
//   foreman --selftest-detach <n>       one session, n attach/close cycles, alive after each
//   foreman --selftest-detach-shutdown  attach, close the window, session still alive
//   foreman --selftest-perf  1 then 10 pwsh tabs: private bytes and frame times

mod fleet;
mod selftest;
mod view;

use std::sync::atomic::{AtomicI32, Ordering};

use gpui::{App, AppContext as _, Bounds, WindowBounds, WindowOptions, px, size};

use crate::view::FleetView;

pub static EXIT_CODE: AtomicI32 = AtomicI32::new(0);

#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    App,
    Selftest,
    SelftestDetach(u32),
    SelftestDetachShutdown,
    SelftestPerf,
}

impl Mode {
    pub fn launches_session(self) -> bool {
        matches!(self, Mode::Selftest | Mode::SelftestDetach(_) | Mode::SelftestDetachShutdown)
    }
}

const USAGE: &str = "usage: foreman [--selftest | --selftest-detach <n> | --selftest-detach-shutdown | --selftest-perf]";

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
        other => {
            eprintln!("unknown arguments {other:?}; {USAGE}");
            std::process::exit(2);
        }
    }
}

fn main() {
    foreman_ui::process_start();
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
