//! UI-thread stalls: a [`section`] that runs over [`STALL`] is logged with its label, and [`watch`] logs every time the
//! foreground resumes [`STALL`] late, with the sections that ran meanwhile; [`take`] hands both to a selftest.

use std::{
    cell::RefCell,
    collections::VecDeque,
    time::{Duration, Instant},
};

pub const STALL: Duration = Duration::from_millis(100);
/// A section this long is kept for [`watch`]'s report of what ran.
const NOTED: Duration = Duration::from_millis(5);
const TICK: Duration = Duration::from_millis(50);
const KEEP: usize = 512;

#[derive(Debug, Clone, PartialEq)]
pub struct Stall {
    pub ended: Instant,
    pub took: Duration,
    /// A section's label, or `blocked: ` and the sections that ran while the foreground was late.
    pub what: String,
}

impl std::fmt::Display for Stall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ms {}", self.took.as_millis(), self.what)
    }
}

thread_local! {
    static RAN: RefCell<VecDeque<(Instant, Duration, &'static str)>> = const { RefCell::new(VecDeque::new()) };
    static STALLS: RefCell<Vec<Stall>> = const { RefCell::new(Vec::new()) };
}

#[must_use]
pub struct Section {
    label: &'static str,
    started: Instant,
}

pub fn section(label: &'static str) -> Section {
    Section { label, started: Instant::now() }
}

impl Drop for Section {
    fn drop(&mut self) {
        let took = self.started.elapsed();
        if took < NOTED {
            return;
        }
        let ended = Instant::now();
        RAN.with_borrow_mut(|ran| {
            if ran.len() == KEEP {
                ran.pop_front();
            }
            ran.push_back((ended, took, self.label));
        });
        if took >= STALL {
            log::warn!("ui stall: {} took {} ms", self.label, took.as_millis());
            record(Stall { ended, took, what: self.label.to_string() });
        }
    }
}

fn record(stall: Stall) {
    STALLS.with_borrow_mut(|s| {
        if s.len() == KEEP {
            s.remove(0);
        }
        s.push(stall);
    });
}

/// The sections that ended in `[from, to]`, longest first.
fn ran_between(from: Instant, to: Instant) -> String {
    let mut ran: Vec<(Duration, &str)> = RAN.with_borrow(|r| r.iter().filter(|(end, ..)| *end >= from && *end <= to).map(|(_, took, label)| (*took, *label)).collect());
    if ran.is_empty() {
        return "no labelled section".into();
    }
    ran.sort_by(|a, b| b.0.cmp(&a.0));
    ran.iter().take(6).map(|(took, label)| format!("{label} {} ms", took.as_millis())).collect::<Vec<_>>().join(", ")
}

/// A heartbeat on the foreground: a tick that resumes late measures how long the thread was held.
pub fn watch(cx: &mut gpui::App) {
    let timer = cx.background_executor().clone();
    cx.spawn(async move |_| {
        loop {
            let asked = Instant::now();
            timer.timer(TICK).await;
            let now = Instant::now();
            let late = now.saturating_duration_since(asked).saturating_sub(TICK);
            if late >= STALL {
                let what = format!("blocked: {}", ran_between(asked, now));
                log::warn!("ui thread held {} ms; {what}", late.as_millis());
                record(Stall { ended: now, took: late, what });
            }
        }
    })
    .detach();
}

/// The stalls recorded on this thread since the last call.
pub fn take() -> Vec<Stall> {
    STALLS.with_borrow_mut(std::mem::take)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_section_is_a_stall_and_a_short_one_is_not() {
        take();
        drop(Section { label: "quick", started: Instant::now() });
        drop(Section { label: "slow", started: Instant::now() - Duration::from_millis(150) });
        let stalls = take();
        assert_eq!(stalls.len(), 1, "{stalls:?}");
        assert_eq!(stalls[0].what, "slow");
        assert!(stalls[0].took >= Duration::from_millis(150));
        assert!(take().is_empty());
    }

    #[test]
    fn a_late_foreground_names_the_sections_that_ran() {
        let from = Instant::now();
        drop(Section { label: "render", started: Instant::now() - Duration::from_millis(40) });
        drop(Section { label: "tree", started: Instant::now() - Duration::from_millis(30) });
        let said = ran_between(from, Instant::now());
        assert!(said.starts_with("render ") && said.contains("tree "), "{said}");
        assert_eq!(ran_between(Instant::now(), Instant::now()), "no labelled section");
        take();
    }
}
