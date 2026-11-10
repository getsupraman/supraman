//! The selftests' hooks in the app's own code, built with or without the `selftest` feature.

use std::sync::atomic::{AtomicBool, Ordering};

pub(crate) static FRAME_ASKED: AtomicBool = AtomicBool::new(false);
pub(crate) static FIXTURES_ONLY: AtomicBool = AtomicBool::new(false);

/// `FleetView::shown` scrolled the sidebar; the next wait redraws the window so the row is painted.
pub(crate) fn ask_frame() {
    FRAME_ASKED.store(true, Ordering::SeqCst);
}

/// The session list holds the injected rows alone.
pub(crate) fn fixtures_only() -> bool {
    FIXTURES_ONLY.load(Ordering::SeqCst)
}

/// `SUPRAMAN_SELFTEST_FRAME_SLEEP_MS=<n>`: every root render sleeps n ms, the red check for these checks.
#[cfg(feature = "selftest")]
pub(crate) fn injected_frame_sleep() {
    use std::{sync::OnceLock, time::Duration};
    static SLEEP: OnceLock<Option<Duration>> = OnceLock::new();
    let sleep = SLEEP.get_or_init(|| {
        std::env::var("SUPRAMAN_SELFTEST_FRAME_SLEEP_MS").ok().and_then(|v| v.parse().ok()).map(Duration::from_millis)
    });
    if let Some(sleep) = sleep {
        std::thread::sleep(*sleep);
    }
}

#[cfg(not(feature = "selftest"))]
pub(crate) fn injected_frame_sleep() {}
