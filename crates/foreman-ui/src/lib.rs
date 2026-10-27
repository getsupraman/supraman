//! What every Foreman gpui binary shares: the terminal grid painter, theme → terminal palette,
//! global init, and the frame / memory probes the selftests print.

pub mod term_element;

use std::{
    cell::{Cell, RefCell},
    io::Write as _,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Mutex, OnceLock},
    time::Instant,
};

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

/// gpui platform info lines (GPU adapter, D3D feature level) plus warnings and errors: to
/// `<dir>/foreman.log` when it has a log folder, else to stderr.
pub struct Logger {
    file: Option<PathBuf>,
    /// One writer at a time, so a rotation never races an append.
    write: Mutex<()>,
}

/// Where a log file rotates: the previous one is kept as `foreman.log.1`, so at most twice this.
pub const LOG_CAP: u64 = 1024 * 1024;

impl log::Log for Logger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Warn
            || (metadata.level() <= log::Level::Info && metadata.target().starts_with("gpui"))
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format!("[{} {}] {}", record.level(), record.target(), record.args());
        match &self.file {
            Some(file) => {
                let _guard = self.write.lock().unwrap_or_else(|e| e.into_inner());
                let stamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
                append_capped(file, &format!("{stamp} {line}\n"), LOG_CAP);
            }
            None => eprintln!("{line}"),
        }
    }

    fn flush(&self) {}
}

/// Appends `text` to `file`, first moving a file of `cap` bytes or more to `<file>.1`. Opened per line
/// (lines are rare), so the folder can be removed while the app runs, as a selftest's home is.
pub fn append_capped(file: &Path, text: &str, cap: u64) {
    if std::fs::metadata(file).is_ok_and(|m| m.len() >= cap) {
        let mut old = file.as_os_str().to_owned();
        old.push(".1");
        let _ = std::fs::rename(file, old);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(file) {
        let _ = f.write_all(text.as_bytes());
    }
}

/// Log lines go to `<dir>/foreman.log` (created here) when `dir` is given, never to a console;
/// without one, to stderr. Call once.
pub fn init_logger(dir: Option<&Path>) {
    let file = dir.and_then(|dir| std::fs::create_dir_all(dir).ok().map(|_| dir.join("foreman.log")));
    let logger = Box::leak(Box::new(Logger { file, write: Mutex::new(()) }));
    log::set_logger(logger).ok();
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

static PROCESS_START: OnceLock<Instant> = OnceLock::new();

/// The clock the selftests measure "ms since process start" against: first call wins, so `main`
/// calls it before anything else.
pub fn process_start() -> Instant {
    *PROCESS_START.get_or_init(Instant::now)
}

/// Timestamps of frames the window drew and handed to the platform for presentation, and how long the
/// window's own work took in each.
///
/// A view's `render` runs only inside `Window::draw`, which gpui immediately follows with
/// `Window::present`, so calling [`FrameProbe::on_render`] from the root view's `render` marks one
/// frame handed to the swap chain. [`FrameProbe::paint_marker`], the root's last child, paints last:
/// render → marker is the frame's render, layout, prepaint and paint, without the wait for vsync or for
/// the CPU, which another process can stretch (m5/15).
#[derive(Default)]
pub struct FrameProbe {
    pub recording: bool,
    pub frames: Vec<Instant>,
    drawing: Rc<Cell<Option<Instant>>>,
    work: Rc<RefCell<Vec<f64>>>,
}

impl FrameProbe {
    pub fn on_render(&mut self, window: &mut gpui::Window) {
        if self.recording {
            let now = Instant::now();
            self.frames.push(now);
            self.drawing.set(Some(now));
            window.request_animation_frame();
        }
    }

    /// A zero-size element that records the frame's own work when it paints.
    pub fn paint_marker(&self) -> impl gpui::IntoElement {
        use gpui::Styled as _;
        let (drawing, work) = (self.drawing.clone(), self.work.clone());
        gpui::canvas(
            |_, _, _| (),
            move |_, _, _, _| {
                if let Some(started) = drawing.take() {
                    work.borrow_mut().push(started.elapsed().as_secs_f64() * 1000.0);
                }
            },
        )
        .absolute()
        .size_0()
    }

    pub fn start(&mut self) {
        self.recording = true;
        self.frames.clear();
        self.drawing.set(None);
        self.work.borrow_mut().clear();
    }

    pub fn stop(&mut self) -> FrameStats {
        self.recording = false;
        self.drawing.set(None);
        let mut stats = FrameStats::from_frames(&std::mem::take(&mut self.frames));
        let mut work = std::mem::take(&mut *self.work.borrow_mut());
        work.sort_by(|a, b| a.total_cmp(b));
        stats.work_p10 = percentile(&work, 10.0);
        stats.work_p50 = percentile(&work, 50.0);
        stats.work_p95 = percentile(&work, 95.0);
        stats
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameStats {
    pub frames: usize,
    pub p50: f64,
    pub p95: f64,
    pub max: f64,
    pub over_50: usize,
    pub over_100: usize,
    /// When the slowest frame was presented, in ms since [`process_start`].
    pub max_at_ms: f64,
    /// The window's own work per frame ([`FrameProbe::paint_marker`]); NaN without a marker. Another
    /// process preempting the window stretches some frames' work; the fastest tenth stays near the cost.
    pub work_p10: f64,
    pub work_p50: f64,
    pub work_p95: f64,
}

impl FrameStats {
    pub fn from_frames(frames: &[Instant]) -> Self {
        let mut intervals: Vec<f64> = frames
            .windows(2)
            .map(|w| (w[1] - w[0]).as_secs_f64() * 1000.0)
            .collect();
        let max_at_ms = frames
            .windows(2)
            .max_by_key(|w| w[1] - w[0])
            .map_or(f64::NAN, |w| ms_since_start(w[1]));
        intervals.sort_by(|a, b| a.total_cmp(b));
        Self {
            frames: frames.len(),
            p50: percentile(&intervals, 50.0),
            p95: percentile(&intervals, 95.0),
            max: intervals.last().copied().unwrap_or(f64::NAN),
            over_50: intervals.iter().filter(|ms| **ms > 50.0).count(),
            over_100: intervals.iter().filter(|ms| **ms > 100.0).count(),
            max_at_ms,
            work_p10: f64::NAN,
            work_p50: f64::NAN,
            work_p95: f64::NAN,
        }
    }
}

impl std::fmt::Display for FrameStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "p50 {:.2} ms, p95 {:.2} ms, max {:.2} ms (at +{:.0} ms since process start) over {} frames; frames > 50 ms: {}, > 100 ms: {}; own work p10 {:.2} ms, p50 {:.2} ms, p95 {:.2} ms",
            self.p50, self.p95, self.max, self.max_at_ms, self.frames, self.over_50, self.over_100, self.work_p10, self.work_p50, self.work_p95
        )
    }
}

pub fn ms_since_start(at: Instant) -> f64 {
    at.saturating_duration_since(process_start()).as_secs_f64() * 1000.0
}

pub fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let rank = ((p / 100.0) * (sorted.len() - 1) as f64).round() as usize;
    sorted[rank.min(sorted.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::append_capped;

    #[test]
    fn log_file_rotates_at_the_cap_and_keeps_one_previous() {
        let dir = std::env::temp_dir().join(format!("foreman-ui-log-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("foreman.log");
        for n in 0..25 {
            append_capped(&file, &format!("line {n:02}\n"), 64);
        }
        let current = std::fs::read_to_string(&file).unwrap();
        let previous = std::fs::read_to_string(dir.join("foreman.log.1")).unwrap();
        assert!(current.ends_with("line 24\n") && current.len() < 64 + 8, "{current}");
        assert!(previous.len() >= 64 && previous.len() < 64 + 8, "{previous}");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2, "never a third file");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
