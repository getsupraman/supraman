//! What every Supraman gpui binary shares: the widgets, themes, terminal painter, init and the selftests' probes.

pub mod banner;
mod builtin_glyphs;
pub mod button;
pub mod clipped;
pub mod confirm;
pub mod details;
pub mod folder_picker;
pub mod fonts;
pub mod hit;
pub mod hover_card;
pub mod icon;
pub mod ink;
pub mod keyed_list;
pub mod marks;
pub mod matcher;
pub mod menu;
pub mod notices;
pub mod nudge;
pub mod open;
pub mod palette;
pub mod placeholder;
pub mod segmented;
pub mod settings_page;
pub mod split;
pub mod stall;
pub mod store;
pub mod table;
pub mod tabs;
pub mod term_element;
pub mod text_input;
pub mod themes;
pub mod toast;
pub mod tooltip;
pub mod tree;

use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use supraman_term::terminal_settings::TerminalConfig;
use gpui::App;

pub use themes::palette;

/// The theme globals with [`themes::DEFAULT_THEME`] and the terminal config; `main` then applies the user's ([`themes::apply`]).
pub fn init_globals(cx: &mut App, program_version: &str) {
    theme::init(theme::LoadThemes::JustBase, cx);
    cx.set_global(TerminalConfig {
        program_version: program_version.to_string(),
        ..Default::default()
    });
    themes::apply(cx, None, themes::TerminalColors::Theme, None);
    table::bind_keys(cx);
}

/// gpui's platform info, warnings and errors: to `<dir>/`[`LOG_FILE`] with a log folder, else to stderr.
pub struct Logger {
    file: Option<PathBuf>,
    /// One writer at a time, so a rotation never races an append.
    write: Mutex<()>,
}

pub const LOG_FILE: &str = "supraman.log";

/// Where a log file rotates: the previous one is kept as `supraman.log.1`, so at most twice this.
pub const LOG_CAP: u64 = 1024 * 1024;

impl log::Log for Logger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Warn
            || (metadata.level() <= log::Level::Info && ["gpui", "supraman"].iter().any(|t| metadata.target().starts_with(t)))
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let message = record.args().to_string();
        if is_vsync_race(record.target(), &message) {
            return;
        }
        let line = format!("[{} {}] {message}", record.level(), record.target());
        match &self.file {
            Some(file) => {
                let _guard = self.write.lock().unwrap_or_else(|e| e.into_inner());
                let stamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
                let _ = supraman_core::fs::append_capped(file, format!("{stamp} {line}\n").as_bytes(), LOG_CAP);
            }
            None => eprintln!("{line}"),
        }
    }

    fn flush(&self) {}
}

/// gpui_windows' VSync redraw of a window between WM_DESTROY and its handling (0x80070578): harmless, not ours to fix.
pub fn is_vsync_race(target: &str, message: &str) -> bool {
    target == "gpui_windows::platform" && message.starts_with("Invalid window handle") && message.contains("0x80070578")
}

/// Log lines to `<dir>/`[`LOG_FILE`] (created here) when `dir` is given, else to stderr. Call once.
pub fn init_logger(dir: Option<&Path>) {
    let file = dir.and_then(|dir| std::fs::create_dir_all(dir).ok().map(|_| dir.join(LOG_FILE)));
    let logger = Box::leak(Box::new(Logger { file, write: Mutex::new(()) }));
    log::set_logger(logger).ok();
    log::set_max_level(log::LevelFilter::Info);
}

/// Every panic to the log before the previous hook: the only trace of one that aborts (in a window procedure).
pub fn log_panics() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let backtrace = std::backtrace::Backtrace::force_capture();
        log::error!(target: "supraman::panic", "thread '{}' {info}\nbacktrace:\n{backtrace}", thread.name().unwrap_or("<unnamed>"));
        previous(info);
    }));
}

pub fn mib(bytes: usize) -> String {
    format!("{:.1} MiB ({bytes} B)", bytes as f64 / (1024.0 * 1024.0))
}

static PROCESS_START: OnceLock<Instant> = OnceLock::new();

/// The selftests' "ms since process start" clock: first call wins, so `main` calls it first.
pub fn process_start() -> Instant {
    *PROCESS_START.get_or_init(Instant::now)
}

/// Frames presented and each one's own work: [`FrameProbe::on_render`] in the root's `render`, [`FrameProbe::paint_marker`] its last child.
#[derive(Default)]
pub struct FrameProbe {
    pub recording: bool,
    pub frames: Vec<Instant>,
    drawing: Rc<Cell<Option<Instant>>>,
    work: Rc<RefCell<Vec<f64>>>,
    painted: Rc<RefCell<Vec<Instant>>>,
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
        let (drawing, work, painted) = (self.drawing.clone(), self.work.clone(), self.painted.clone());
        gpui::canvas(
            |_, _, _| (),
            move |_, _, _, _| {
                if let Some(started) = drawing.take() {
                    work.borrow_mut().push(started.elapsed().as_secs_f64() * 1000.0);
                    painted.borrow_mut().push(Instant::now());
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
        self.painted.borrow_mut().clear();
    }

    /// When each recorded frame finished painting.
    pub fn painted(&self) -> Vec<Instant> {
        self.painted.borrow().clone()
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
    /// The window's own work per frame ([`FrameProbe::paint_marker`]); NaN without a marker.
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

pub fn timeline(phase: &str, took: Duration) {
    log::info!("startup {phase}: {} ms (at +{:.0} ms)", took.as_millis(), ms_since_start(Instant::now()));
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
    use super::is_vsync_race;

    #[test]
    fn the_app_binarys_info_lines_are_kept() {
        use log::Log;
        let logger = super::Logger { file: None, write: std::sync::Mutex::new(()) };
        let info = |target: &str| logger.enabled(&log::Metadata::builder().level(log::Level::Info).target(target).build());
        assert!(info("supraman") && info("supraman::selftest") && info("supraman_core::home") && info("gpui_windows"));
        assert!(!info("ureq::unit"));
    }

    #[test]
    fn only_the_vsync_race_line_is_silenced() {
        assert!(is_vsync_race("gpui_windows::platform", "Invalid window handle. (0x80070578)"));
        assert!(!is_vsync_race("gpui_windows::window", "Invalid window handle. (0x80070578)"), "a window's own teardown still shows");
        assert!(!is_vsync_race("gpui::window", "window not found"));
        assert!(!is_vsync_race("gpui_windows::platform", "Device lost"));
    }
}
