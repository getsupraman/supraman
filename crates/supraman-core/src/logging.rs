use std::path::{Path, PathBuf};
use std::sync::Mutex;

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
                let _ = crate::fs::append_capped(file, format!("{stamp} {line}\n").as_bytes(), LOG_CAP);
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
