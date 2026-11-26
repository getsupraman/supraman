use std::sync::OnceLock;
use std::time::{Duration, Instant};

static PROCESS_START: OnceLock<Instant> = OnceLock::new();

/// The selftests' "ms since process start" clock: first call wins, so `main` calls it first.
pub fn process_start() -> Instant {
    *PROCESS_START.get_or_init(Instant::now)
}

pub fn ms_since_start(at: Instant) -> f64 {
    at.saturating_duration_since(process_start()).as_secs_f64() * 1000.0
}

/// A startup phase's log line, `startup <phase>: <ms> (at +<ms since start>)`, which the selftests read back.
pub fn phase(phase: &str, took: Duration) {
    log::info!("startup {phase}: {} ms (at +{:.0} ms)", took.as_millis(), ms_since_start(Instant::now()));
}
