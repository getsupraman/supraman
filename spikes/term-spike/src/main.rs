// term-spike: one gpui window running one crates/foreman-term terminal (alacritty_terminal + ConPTY).
//   term-spike                       interactive pwsh
//   term-spike --attach <id>         runs `claude attach <id>`
//   term-spike --selftest            nonce round-trip, then frame times + private bytes under CPU load
//   term-spike --selftest-external   same, but the load is a cold release build of term-spike
//                                    into <workspace>/target-load

mod term_element;

use std::{
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicI32, Ordering},
    },
    time::{Duration, Instant},
};

use collections::HashMap;
use foreman_term as terminal;
use gpui::{
    App, AppContext as _, AsyncWindowContext, Bounds, Context, Entity, FocusHandle, KeyDownEvent,
    Render, WeakEntity, Window, WindowBounds, WindowOptions, div, prelude::*, px, size,
};
use terminal::{
    Event as TerminalEvent, Terminal, TerminalBuilder, TerminalMode,
    terminal_settings::{AlternateScroll, CursorShape},
};
use theme::ActiveTheme as _;
use util::paths::PathStyle;

static EXIT_CODE: AtomicI32 = AtomicI32::new(0);

#[derive(Clone, Copy, PartialEq)]
enum Load {
    InProcess,
    External,
}

#[derive(Clone)]
enum Mode {
    Interactive,
    Attach(String),
    Selftest(Load),
}

fn parse_args() -> Mode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--selftest") => Mode::Selftest(Load::InProcess),
        Some("--selftest-external") => Mode::Selftest(Load::External),
        Some("--attach") => match args.get(1) {
            Some(id) => Mode::Attach(id.clone()),
            None => {
                eprintln!("usage: term-spike --attach <id>");
                std::process::exit(2);
            }
        },
        None => Mode::Interactive,
        Some(other) => {
            eprintln!(
                "unknown argument {other}; usage: term-spike [--selftest | --selftest-external | --attach <id>]"
            );
            std::process::exit(2);
        }
    }
}

fn shell_for(mode: &Mode) -> util::shell::Shell {
    let (program, args) = match mode {
        Mode::Interactive => ("pwsh", vec!["-NoLogo".to_string()]),
        Mode::Selftest(_) => ("pwsh", vec!["-NoLogo".into(), "-NoProfile".into()]),
        Mode::Attach(id) => ("claude", vec!["attach".into(), id.clone()]),
    };
    util::shell::Shell::WithArguments {
        program: program.to_string(),
        args,
        title_override: None,
    }
}

/// Prints gpui platform info lines (GPU adapter, D3D feature level) plus warnings and errors.
struct StderrLogger;

impl log::Log for StderrLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Warn
            || (metadata.level() <= log::Level::Info && metadata.target().starts_with("gpui"))
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            eprintln!("[{} {}] {}", record.level(), record.target(), record.args());
        }
    }

    fn flush(&self) {}
}

#[cfg(windows)]
fn memory() -> (usize, usize) {
    use windows::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows::Win32::System::Threading::GetCurrentProcess;
    let mut counters = PROCESS_MEMORY_COUNTERS_EX {
        cb: size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        ..Default::default()
    };
    let ok = unsafe {
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut counters as *mut PROCESS_MEMORY_COUNTERS_EX as *mut PROCESS_MEMORY_COUNTERS,
            counters.cb,
        )
    };
    if ok.is_err() {
        return (0, 0);
    }
    (counters.PrivateUsage, counters.WorkingSetSize)
}

#[cfg(not(windows))]
fn memory() -> (usize, usize) {
    (0, 0)
}

fn mib(bytes: usize) -> String {
    format!("{:.1} MiB ({bytes} B)", bytes as f64 / (1024.0 * 1024.0))
}

/// Timestamps of frames the window drew and handed to the platform for presentation.
#[derive(Default)]
struct FrameProbe {
    recording: bool,
    frames: Vec<Instant>,
}

struct SpikeView {
    terminal: Option<Entity<Terminal>>,
    focus: FocusHandle,
    probe: FrameProbe,
}

impl SpikeView {
    fn key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(terminal) = &self.terminal else {
            return;
        };
        terminal.update(cx, |terminal, _| {
            if !terminal.try_keystroke(&event.keystroke, false)
                && let Some(text) = &event.keystroke.key_char
            {
                terminal.input(text.clone().into_bytes());
            }
        });
        cx.stop_propagation();
    }
}

impl Render for SpikeView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.probe.recording {
            // render() runs only inside Window::draw, which is immediately followed by
            // Window::present, so each timestamp marks one frame handed to the swap chain.
            self.probe.frames.push(Instant::now());
            window.request_animation_frame();
        }
        let root = div()
            .size_full()
            .bg(cx.theme().colors().terminal_background)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down));
        match &self.terminal {
            Some(terminal) => root.child(term_element::terminal_grid(terminal.clone())),
            None => root,
        }
    }
}

fn build_terminal(
    mode: &Mode,
    window: &mut Window,
    cx: &mut Context<SpikeView>,
) -> gpui::Task<anyhow::Result<terminal::TerminalBuilder>> {
    let window_id = window.window_handle().window_id().as_u64();
    TerminalBuilder::new(
        std::env::current_dir().ok().map(PathBuf::from),
        TerminalMode::interactive(),
        shell_for(mode),
        HashMap::default(),
        CursorShape::Block,
        AlternateScroll::On,
        None,
        Vec::new(),
        Duration::ZERO,
        false,
        window_id,
        cx,
        Vec::new(),
        PathStyle::local(),
    )
}

/// The active theme's terminal colours, for foreman-term's OSC colour replies and the painter's
/// indexed colours.
pub fn palette(theme: &theme::Theme) -> terminal::terminal_settings::TerminalPalette {
    let c = theme.colors();
    terminal::terminal_settings::TerminalPalette {
        ansi: [
            c.terminal_ansi_black,
            c.terminal_ansi_red,
            c.terminal_ansi_green,
            c.terminal_ansi_yellow,
            c.terminal_ansi_blue,
            c.terminal_ansi_magenta,
            c.terminal_ansi_cyan,
            c.terminal_ansi_white,
            c.terminal_ansi_bright_black,
            c.terminal_ansi_bright_red,
            c.terminal_ansi_bright_green,
            c.terminal_ansi_bright_yellow,
            c.terminal_ansi_bright_blue,
            c.terminal_ansi_bright_magenta,
            c.terminal_ansi_bright_cyan,
            c.terminal_ansi_bright_white,
        ],
        dim: [
            c.terminal_ansi_dim_black,
            c.terminal_ansi_dim_red,
            c.terminal_ansi_dim_green,
            c.terminal_ansi_dim_yellow,
            c.terminal_ansi_dim_blue,
            c.terminal_ansi_dim_magenta,
            c.terminal_ansi_dim_cyan,
            c.terminal_ansi_dim_white,
        ],
        foreground: c.terminal_foreground,
        background: c.terminal_background,
        bright_foreground: c.terminal_bright_foreground,
        cursor: theme.players().local().cursor,
    }
}

fn init_globals(cx: &mut App) {
    theme::init(theme::LoadThemes::JustBase, cx);
    let palette = palette(cx.theme());
    cx.set_global(terminal::terminal_settings::TerminalConfig {
        program_version: "0.1.0".to_string(),
        palette,
        ..Default::default()
    });
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn load_target_dir() -> PathBuf {
    workspace_root().join("target-load")
}

/// Starts a second release build of term-spike into target-load/.
fn start_external_load() -> std::io::Result<Child> {
    let log = std::fs::File::create(load_target_dir().join("load-build.log"))?;
    Command::new("cargo")
        .args(["build", "--release", "-p", "term-spike"])
        .current_dir(workspace_root())
        .env("CARGO_TARGET_DIR", load_target_dir())
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .spawn()
}

fn kill_tree(child: &mut Child) {
    Command::new("taskkill")
        .args(["/T", "/F", "/PID", &child.id().to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok();
    child.wait().ok();
}

fn main() {
    let mode = parse_args();
    log::set_logger(&StderrLogger).ok();
    log::set_max_level(log::LevelFilter::Info);
    println!("terminal: foreman-term");

    if matches!(mode, Mode::Selftest(Load::External)) {
        // Before the clock starts: wipe the previous load build so this one compiles from scratch.
        let cleaned = Instant::now();
        let release = load_target_dir().join("release");
        if release.exists()
            && let Err(error) = std::fs::remove_dir_all(&release)
        {
            eprintln!("cannot clean {}: {error}", release.display());
            std::process::exit(1);
        }
        std::fs::create_dir_all(load_target_dir()).ok();
        println!("cleaned target-load/release in {} ms", cleaned.elapsed().as_millis());
    }

    if matches!(mode, Mode::Selftest(_)) {
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_secs(90));
            eprintln!("selftest watchdog: no result after 90 s");
            std::process::exit(1);
        });
    }

    let started = Instant::now();
    gpui_platform::application().run(move |cx: &mut App| {
        init_globals(cx);

        let bounds = Bounds::centered(None, size(px(900.), px(560.)), cx);
        let mode_for_window = mode.clone();
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            move |window, cx| {
                let view = cx.new(|cx| SpikeView {
                    terminal: None,
                    focus: cx.focus_handle(),
                    probe: FrameProbe::default(),
                });
                let focus = view.read(cx).focus.clone();
                focus.focus(window, cx);

                let mode = mode_for_window;
                view.update(cx, |_, cx| {
                    let build = build_terminal(&mode, window, cx);
                    cx.spawn_in(window, async move |this, cx| {
                        let builder = match build.await {
                            Ok(builder) => builder,
                            Err(error) => {
                                eprintln!("failed to spawn terminal: {error:#}");
                                EXIT_CODE.store(1, Ordering::SeqCst);
                                cx.update(|_, cx| cx.quit()).ok();
                                return;
                            }
                        };
                        this.update(cx, |this, cx| {
                            let terminal = cx.new(|cx| builder.subscribe(cx));
                            cx.subscribe(&terminal, |_, _, event, cx| match event {
                                TerminalEvent::Wakeup | TerminalEvent::TitleChanged => cx.notify(),
                                TerminalEvent::CloseTerminal => cx.quit(),
                                _ => {}
                            })
                            .detach();
                            this.terminal = Some(terminal);
                            cx.notify();
                        })
                        .ok();
                        if let Mode::Selftest(load) = mode {
                            selftest(this, started, load, cx).await;
                        }
                    })
                    .detach();
                });
                view
            },
        )
        .expect("failed to open window");
        cx.activate(true);
    });
    std::process::exit(EXIT_CODE.load(Ordering::SeqCst));
}

fn nonce() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!("{:08x}", (nanos as u64 ^ (std::process::id() as u64) << 32) as u32)
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let rank = ((p / 100.0) * (sorted.len() - 1) as f64).round() as usize;
    sorted[rank.min(sorted.len() - 1)]
}

async fn sleep(cx: &AsyncWindowContext, duration: Duration) {
    cx.background_executor().timer(duration).await;
}

async fn selftest(
    this: WeakEntity<SpikeView>,
    started: Instant,
    load: Load,
    cx: &mut AsyncWindowContext,
) {
    let Some(terminal) = this
        .read_with(cx, |view, _| view.terminal.clone())
        .ok()
        .flatten()
    else {
        EXIT_CODE.store(1, Ordering::SeqCst);
        cx.update(|_, cx| cx.quit()).ok();
        return;
    };
    let content = |cx: &mut AsyncWindowContext| {
        terminal.read_with(cx, |terminal, _| terminal.get_content())
    };

    // Wait for the pwsh prompt so the typed line is not raced against shell startup.
    let prompt_deadline = Instant::now() + Duration::from_secs(10);
    while !content(cx).contains("PS ") && Instant::now() < prompt_deadline {
        sleep(cx, Duration::from_millis(20)).await;
    }
    println!(
        "pwsh prompt after {} ms (from process start)",
        started.elapsed().as_millis()
    );

    // The echo is typed into the PTY; the nonce must show up twice in the grid:
    // once on the typed command line, once as the command's output.
    let nonce = format!("FOREMAN-TERM-{}", nonce());
    let typed = Instant::now();
    terminal.update(cx, |terminal, _| {
        terminal.input(format!("echo {nonce}\r").into_bytes())
    });
    let mut seen_after = None;
    while typed.elapsed() < Duration::from_secs(10) {
        if content(cx).matches(nonce.as_str()).count() >= 2 {
            seen_after = Some(typed.elapsed());
            break;
        }
        sleep(cx, Duration::from_millis(5)).await;
    }

    sleep(cx, Duration::from_secs(1)).await;
    let (idle_private, idle_ws) = memory();

    let window_active = cx
        .update(|window, _| window.is_window_active())
        .unwrap_or(false);
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let stop = Arc::new(AtomicBool::new(false));
    let burner_count = if load == Load::InProcess { cores } else { 0 };
    let mut load_build = None;
    if load == Load::External {
        match start_external_load() {
            Ok(child) => load_build = Some(child),
            Err(error) => {
                eprintln!("cannot start the external load build: {error}");
                EXIT_CODE.store(1, Ordering::SeqCst);
                cx.update(|_, cx| cx.quit()).ok();
                return;
            }
        }
    }
    let burners: Vec<_> = (0..burner_count)
        .map(|_| {
            let stop = stop.clone();
            std::thread::spawn(move || {
                let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
                while !stop.load(Ordering::Relaxed) {
                    for _ in 0..10_000 {
                        x ^= x << 13;
                        x ^= x >> 7;
                        x ^= x << 17;
                    }
                    std::hint::black_box(x);
                }
            })
        })
        .collect();

    this.update(cx, |view, cx| {
        view.probe = FrameProbe {
            recording: true,
            frames: Vec::new(),
        };
        cx.notify();
    })
    .ok();
    let load_started = Instant::now();
    let (mut peak_private, mut peak_ws) = (0, 0);
    while load_started.elapsed() < Duration::from_secs(10) {
        let (private, ws) = memory();
        peak_private = peak_private.max(private);
        peak_ws = peak_ws.max(ws);
        sleep(cx, Duration::from_millis(250)).await;
    }
    let frames = this
        .update(cx, |view, _| {
            view.probe.recording = false;
            std::mem::take(&mut view.probe.frames)
        })
        .unwrap_or_default();
    let (end_private, _) = memory();
    let window_active_at_end = cx
        .update(|window, _| window.is_window_active())
        .unwrap_or(false);
    stop.store(true, Ordering::Relaxed);
    for burner in burners {
        burner.join().ok();
    }
    let load_build_running = match load_build.as_mut() {
        Some(child) => {
            let running = matches!(child.try_wait(), Ok(None));
            kill_tree(child);
            Some(running)
        }
        None => None,
    };

    let timed: Vec<(f64, f64)> = frames
        .windows(2)
        .map(|w| {
            (
                (w[1] - w[0]).as_secs_f64() * 1000.0,
                (w[0] - load_started).as_secs_f64() * 1000.0,
            )
        })
        .collect();
    let mut worst = timed.clone();
    worst.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut intervals: Vec<f64> = timed.iter().map(|(ms, _)| *ms).collect();
    intervals.sort_by(|a, b| a.total_cmp(b));

    match load_build_running {
        None => println!(
            "load: {cores} busy threads in-process (one per logical core), 10 s, window active: {window_active} / at end {window_active_at_end}"
        ),
        Some(running) => println!(
            "load: external `cargo build --release -p term-spike` into target-load/, 10 s, still building at end: {running}, window active: {window_active} / at end {window_active_at_end}"
        ),
    }
    println!("idle private bytes: {} (working set {})", mib(idle_private), mib(idle_ws));
    println!(
        "private bytes under load: peak {} / at end {} (peak working set {})",
        mib(peak_private),
        mib(end_private),
        mib(peak_ws)
    );
    println!(
        "frame time under load: p50 {:.2} ms, p95 {:.2} ms, max {:.2} ms over {} frames",
        percentile(&intervals, 50.0),
        percentile(&intervals, 95.0),
        intervals.last().copied().unwrap_or(f64::NAN),
        frames.len()
    );
    println!(
        "frames > 50 ms: {}, frames > 100 ms: {}; worst: {}",
        intervals.iter().filter(|ms| **ms > 50.0).count(),
        intervals.iter().filter(|ms| **ms > 100.0).count(),
        worst
            .iter()
            .take(3)
            .map(|(ms, at)| format!("{ms:.1} ms at +{at:.0} ms"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    match seen_after {
        Some(elapsed) => println!("nonce seen after {} ms ({nonce})", elapsed.as_millis()),
        None => println!("nonce NOT seen within 10 s ({nonce})"),
    }
    EXIT_CODE.store(if seen_after.is_some() { 0 } else { 1 }, Ordering::SeqCst);

    terminal.update(cx, |terminal, _| terminal.input(b"exit\r".to_vec()));
    sleep(cx, Duration::from_millis(300)).await;
    cx.update(|_, cx| cx.quit()).ok();
}
