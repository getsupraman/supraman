//! What every Foreman gpui binary shares: the terminal grid painter, theme → terminal palette,
//! global init, and the frame / memory probes the selftests print.

pub mod term_element;

use std::time::Instant;

use foreman_term::terminal_settings::{TerminalConfig, TerminalPalette};
use gpui::App;
use theme::ActiveTheme as _;

/// The active theme's terminal colours, for foreman-term's OSC colour replies and the painter's
/// indexed colours.
pub fn palette(theme: &theme::Theme) -> TerminalPalette {
    let c = theme.colors();
    TerminalPalette {
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

pub fn init_globals(cx: &mut App, program_version: &str) {
    theme::init(theme::LoadThemes::JustBase, cx);
    let palette = palette(cx.theme());
    cx.set_global(TerminalConfig {
        program_version: program_version.to_string(),
        palette,
        ..Default::default()
    });
}

/// Prints gpui platform info lines (GPU adapter, D3D feature level) plus warnings and errors.
pub struct StderrLogger;

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

pub fn init_logger() {
    static LOGGER: StderrLogger = StderrLogger;
    log::set_logger(&LOGGER).ok();
    log::set_max_level(log::LevelFilter::Info);
}

/// (private bytes, working set) of this process.
#[cfg(windows)]
pub fn memory() -> (usize, usize) {
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
pub fn memory() -> (usize, usize) {
    (0, 0)
}

pub fn mib(bytes: usize) -> String {
    format!("{:.1} MiB ({bytes} B)", bytes as f64 / (1024.0 * 1024.0))
}

/// Timestamps of frames the window drew and handed to the platform for presentation.
///
/// A view's `render` runs only inside `Window::draw`, which gpui immediately follows with
/// `Window::present`, so calling [`FrameProbe::on_render`] from the root view's `render` marks one
/// frame handed to the swap chain.
#[derive(Default)]
pub struct FrameProbe {
    pub recording: bool,
    pub frames: Vec<Instant>,
}

impl FrameProbe {
    pub fn on_render(&mut self, window: &mut gpui::Window) {
        if self.recording {
            self.frames.push(Instant::now());
            window.request_animation_frame();
        }
    }

    pub fn start(&mut self) {
        self.recording = true;
        self.frames.clear();
    }

    pub fn stop(&mut self) -> FrameStats {
        self.recording = false;
        FrameStats::from_frames(&std::mem::take(&mut self.frames))
    }
}

pub struct FrameStats {
    pub frames: usize,
    pub p50: f64,
    pub p95: f64,
    pub max: f64,
    pub over_50: usize,
    pub over_100: usize,
}

impl FrameStats {
    pub fn from_frames(frames: &[Instant]) -> Self {
        let mut intervals: Vec<f64> = frames
            .windows(2)
            .map(|w| (w[1] - w[0]).as_secs_f64() * 1000.0)
            .collect();
        intervals.sort_by(|a, b| a.total_cmp(b));
        Self {
            frames: frames.len(),
            p50: percentile(&intervals, 50.0),
            p95: percentile(&intervals, 95.0),
            max: intervals.last().copied().unwrap_or(f64::NAN),
            over_50: intervals.iter().filter(|ms| **ms > 50.0).count(),
            over_100: intervals.iter().filter(|ms| **ms > 100.0).count(),
        }
    }
}

impl std::fmt::Display for FrameStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "p50 {:.2} ms, p95 {:.2} ms, max {:.2} ms over {} frames; frames > 50 ms: {}, > 100 ms: {}",
            self.p50, self.p95, self.max, self.frames, self.over_50, self.over_100
        )
    }
}

pub fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let rank = ((p / 100.0) * (sorted.len() - 1) as f64).round() as usize;
    sorted[rank.min(sorted.len() - 1)]
}
