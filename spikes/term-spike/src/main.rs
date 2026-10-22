// term-spike: one gpui window running one Zed `terminal` (alacritty_terminal + ConPTY).
//   term-spike                 interactive pwsh
//   term-spike --attach <id>   runs `claude attach <id>`
//   term-spike --selftest      nonce round-trip, then frame times + private bytes under CPU load

mod term_element;

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicI32, Ordering},
    },
    time::{Duration, Instant},
};

use collections::HashMap;
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

#[derive(Clone)]
enum Mode {
    Interactive,
    Attach(String),
    Selftest,
}

fn parse_args() -> Mode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--selftest") => Mode::Selftest,
        Some("--attach") => match args.get(1) {
            Some(id) => Mode::Attach(id.clone()),
            None => {
                eprintln!("usage: term-spike --attach <id>");
                std::process::exit(2);
            }
        },
        None => Mode::Interactive,
        Some(other) => {
            eprintln!("unknown argument {other}; usage: term-spike [--selftest | --attach <id>]");
            std::process::exit(2);
        }
    }
}

fn shell_for(mode: &Mode) -> task::Shell {
    let (program, args) = match mode {
        Mode::Interactive => ("pwsh", vec!["-NoLogo".to_string()]),
        Mode::Selftest => ("pwsh", vec!["-NoLogo".into(), "-NoProfile".into()]),
        Mode::Attach(id) => ("claude", vec!["attach".into(), id.clone()]),
    };
    task::Shell::WithArguments {
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

fn main() {
    let mode = parse_args();
    log::set_logger(&StderrLogger).ok();
    log::set_max_level(log::LevelFilter::Info);

    if matches!(mode, Mode::Selftest) {
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_secs(90));
            eprintln!("selftest watchdog: no result after 90 s");
            std::process::exit(1);
        });
    }

    let started = Instant::now();
    gpui_platform::application().run(move |cx: &mut App| {
        release_channel::init(semver::Version::new(0, 1, 0), cx);
        settings::init(cx);
        theme_settings::init(theme::LoadThemes::JustBase, cx);

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
                        if matches!(mode, Mode::Selftest) {
                            selftest(this, started, cx).await;
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

async fn selftest(this: WeakEntity<SpikeView>, started: Instant, cx: &mut AsyncWindowContext) {
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
    let burners: Vec<_> = (0..cores)
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
    stop.store(true, Ordering::Relaxed);
    for burner in burners {
        burner.join().ok();
    }

    let mut intervals: Vec<f64> = frames
        .windows(2)
        .map(|w| (w[1] - w[0]).as_secs_f64() * 1000.0)
        .collect();
    intervals.sort_by(|a, b| a.total_cmp(b));

    println!("load: {cores} busy threads in-process (one per logical core), 10 s, window active: {window_active}");
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
    match seen_after {
        Some(elapsed) => println!("nonce seen after {} ms ({nonce})", elapsed.as_millis()),
        None => println!("nonce NOT seen within 10 s ({nonce})"),
    }
    EXIT_CODE.store(if seen_after.is_some() { 0 } else { 1 }, Ordering::SeqCst);

    terminal.update(cx, |terminal, _| terminal.input(b"exit\r".to_vec()));
    sleep(cx, Duration::from_millis(300)).await;
    cx.update(|_, cx| cx.quit()).ok();
}
