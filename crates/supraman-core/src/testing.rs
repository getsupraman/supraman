//! Waiting in a test for something another thread or process does, with a deadline and a named failure.

use std::time::{Duration, Instant};

/// Calls `check` until it gives `Some`, `wake` (given the time left) between tries; panics naming `what` once
/// `within` passes.
#[track_caller]
pub fn eventually<T>(within: Duration, what: &str, wake: impl FnMut(Duration), check: impl FnMut() -> Option<T>) -> T {
    match until(within, wake, check) {
        Some(found) => found,
        None => panic!("{what}: not within {within:?}"),
    }
}

/// Whether `done` held within `within`, `wake` between tries: for a wait whose failure the caller asserts after
/// its own cleanup.
pub fn wait_until(within: Duration, wake: impl FnMut(Duration), mut done: impl FnMut() -> bool) -> bool {
    until(within, wake, || done().then_some(())).is_some()
}

/// The wake of state no event announces (a file, a pid, a fake's counter): a sleep of `every`, or of the time left.
pub fn polled(every: Duration) -> impl FnMut(Duration) {
    move |left| std::thread::sleep(every.min(left))
}

fn until<T>(within: Duration, mut wake: impl FnMut(Duration), mut check: impl FnMut() -> Option<T>) -> Option<T> {
    let deadline = Instant::now() + within;
    loop {
        if let Some(found) = check() {
            return Some(found);
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return None;
        }
        wake(left);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    #[test]
    fn eventually_returns_the_first_some_and_wait_until_says_whether_it_came() {
        let tries = Cell::new(0);
        let found = eventually(Duration::from_secs(5), "the third try", polled(Duration::from_millis(1)), || {
            tries.set(tries.get() + 1);
            (tries.get() == 3).then_some(tries.get())
        });
        assert_eq!(found, 3);
        let woken = Cell::new(0);
        assert!(!wait_until(Duration::from_millis(30), |left| {
            woken.set(woken.get() + 1);
            polled(Duration::from_millis(5))(left)
        }, || false));
        assert!(woken.get() >= 2, "woken between the tries: {}", woken.get());
    }

    #[test]
    #[should_panic(expected = "the never: not within")]
    fn eventually_names_what_never_came() {
        eventually::<()>(Duration::from_millis(10), "the never", polled(Duration::from_millis(1)), || None);
    }
}
