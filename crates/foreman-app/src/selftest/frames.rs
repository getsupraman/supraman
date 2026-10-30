//! Another process's load stretches both the frame interval and the window's own work per frame, so the
//! budgets are judged only on a quiet machine: [`baseline`] waits for one, [`judge`] skips a busy window.

use std::{
    sync::OnceLock,
    time::{Duration, Instant},
};

use anyhow::bail;
use foreman_ui::FrameStats;
use gpui::{AsyncWindowContext, Entity};

use super::sleep;
use crate::view::FleetView;

/// p10: a per-frame cost lifts every frame, preemption only some; measured 5–10 ms, the red check gives 20–24 ms.
pub(crate) const OWN_WORK_P10_MS: f64 = 16.0;
/// Idle at 60 Hz is 16.67 ms; a baseline whose median is above this is not a quiet machine.
const QUIET_P50_MS: f64 = 18.5;
/// Own work p10 went over its budget (16.6–19.2 ms) while other units built and ran their selftests (F-94).
const BUSY_CPU: f64 = 0.35;
const BASELINE: Duration = Duration::from_secs(1);
const CALM_WITHIN: Duration = Duration::from_secs(60);

/// A build's rustc threads at normal priority otherwise preempt it mid-frame; the process class stays normal, so the sessions and shells it starts are not raised.
#[cfg(windows)]
pub(crate) fn raise_ui_thread() -> bool {
    use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_HIGHEST};
    unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST).is_ok() }
}

#[cfg(not(windows))]
pub(crate) fn raise_ui_thread() -> bool {
    false
}

/// `FOREMAN_SELFTEST_FRAME_SLEEP_MS=<n>`: every root render sleeps n ms, the red check for these checks.
pub(crate) fn injected_frame_sleep() {
    static SLEEP: OnceLock<Option<Duration>> = OnceLock::new();
    let sleep = SLEEP.get_or_init(|| {
        std::env::var("FOREMAN_SELFTEST_FRAME_SLEEP_MS").ok().and_then(|v| v.parse().ok()).map(Duration::from_millis)
    });
    if let Some(sleep) = sleep {
        std::thread::sleep(*sleep);
    }
}

pub(crate) struct Stretch {
    pub(crate) stats: FrameStats,
    pub(crate) others_cpu: f64,
}

impl Stretch {
    fn busy(&self) -> Option<String> {
        let s = &self.stats;
        (s.p50 > QUIET_P50_MS || s.over_50 > 0 || self.others_cpu >= BUSY_CPU).then(|| {
            format!("p50 {:.2} ms, max {:.2} ms, other processes {:.0} % CPU", s.p50, s.max, self.others_cpu * 100.)
        })
    }
}

pub(crate) struct Recording {
    cpu: Option<Cpu>,
}

pub(crate) fn start(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> Recording {
    view.update(cx, |v, cx| {
        v.probe.start();
        cx.notify();
    });
    Recording { cpu: Cpu::now() }
}

impl Recording {
    pub(crate) fn stop(self, view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> Stretch {
        let stats = view.update(cx, |v, _| v.probe.stop());
        let others_cpu = match (self.cpu, Cpu::now()) {
            (Some(a), Some(b)) => a.others_share(&b),
            _ => 0.,
        };
        Stretch { stats, others_cpu }
    }
}

pub(crate) async fn record(view: &Entity<FleetView>, length: Duration, cx: &mut AsyncWindowContext) -> Stretch {
    let recording = start(view, cx);
    sleep(cx, length).await;
    recording.stop(view, cx)
}

/// The first second within [`CALM_WITHIN`] with other processes under [`BUSY_CPU`], else the last one measured.
pub(crate) async fn baseline(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> Stretch {
    let deadline = Instant::now() + CALM_WITHIN;
    loop {
        let stretch = record(view, BASELINE, cx).await;
        if stretch.others_cpu < BUSY_CPU || Instant::now() >= deadline {
            return stretch;
        }
    }
}

/// `Ok(Some(note))`: nothing more to judge; `Err`: own work over its budget with other processes quiet.
fn busy_or_over(phase: &str, before: &Stretch, during: &Stretch) -> Result<Option<String>, String> {
    let stats = &during.stats;
    let others = before.others_cpu.max(during.others_cpu);
    if others >= BUSY_CPU {
        return Ok(Some(format!("{stats}; machine busy ({:.0} % CPU by others): frame budget not judged", others * 100.)));
    }
    if !(stats.work_p10 <= OWN_WORK_P10_MS) {
        return Err(format!(
            "{phase}: own work per frame p10 {:.2} ms, over the {OWN_WORK_P10_MS} ms budget on a quiet machine ({:.0} % CPU by others): {stats}",
            stats.work_p10,
            others * 100.
        ));
    }
    Ok(before.busy().map(|why| format!("{stats}; interval checks SKIP (baseline {why})")))
}

pub(crate) async fn judge(
    phase: &str,
    before: &Stretch,
    during: &Stretch,
    view: &Entity<FleetView>,
    cx: &mut AsyncWindowContext,
) -> anyhow::Result<String> {
    let stats = &during.stats;
    match busy_or_over(phase, before, during) {
        Ok(Some(note)) => return Ok(note),
        Err(over) => bail!(over),
        Ok(None) => {}
    }
    if stats.over_50 > 0 {
        let after = record(view, BASELINE, cx).await;
        if let Some(why) = after.busy() {
            return Ok(format!("{stats}; interval checks SKIP (baseline after the phase {why})"));
        }
        bail!(
            "{phase}: frames over 50 ms on a quiet machine: {stats} (baseline before: {}; after: {})",
            before.stats,
            after.stats
        );
    }
    Ok(format!("{stats}; quiet (baseline p50 {:.2} ms, other processes {:.0} % CPU)", before.stats.p50, before.others_cpu * 100.))
}

/// 100 ns units; kernel time includes idle.
#[derive(Clone, Copy)]
struct Cpu {
    idle: u64,
    total: u64,
    own: u64,
}

impl Cpu {
    #[cfg(windows)]
    fn now() -> Option<Self> {
        use windows::Win32::Foundation::FILETIME;
        use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes, GetSystemTimes};
        let ticks = |t: FILETIME| (t.dwHighDateTime as u64) << 32 | t.dwLowDateTime as u64;
        let (mut idle, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
        let (mut created, mut exited, mut own_kernel, mut own_user) =
            (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        unsafe {
            GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)).ok()?;
            GetProcessTimes(GetCurrentProcess(), &mut created, &mut exited, &mut own_kernel, &mut own_user).ok()?;
        }
        Some(Self { idle: ticks(idle), total: ticks(kernel) + ticks(user), own: ticks(own_kernel) + ticks(own_user) })
    }

    #[cfg(not(windows))]
    fn now() -> Option<Self> {
        None
    }

    fn others_share(&self, later: &Cpu) -> f64 {
        let total = later.total.saturating_sub(self.total);
        if total == 0 {
            return 0.;
        }
        let busy = total.saturating_sub(later.idle.saturating_sub(self.idle));
        let others = busy.saturating_sub(later.own.saturating_sub(self.own));
        (others as f64 / total as f64).clamp(0., 1.)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats(p50: f64, over_50: usize) -> FrameStats {
        FrameStats { p50, over_50, ..FrameStats::from_frames(&[]) }
    }

    #[test]
    fn a_window_is_busy_on_slow_frames_or_other_processes() {
        let window = |p50, over_50, others_cpu| Stretch { stats: stats(p50, over_50), others_cpu };
        assert_eq!(window(16.67, 0, 0.1).busy(), None);
        assert!(window(21.0, 0, 0.1).busy().is_some(), "p50 over the quiet budget");
        assert!(window(16.67, 1, 0.1).busy().is_some(), "a frame over 50 ms");
        assert!(window(16.67, 0, 0.8).busy().unwrap().contains("80 % CPU"));
    }

    #[test]
    fn a_busy_window_is_a_note_and_a_quiet_overrun_fails() {
        let window = |work_p10, others_cpu| Stretch { stats: FrameStats { work_p10, ..stats(16.67, 0) }, others_cpu };
        assert_eq!(busy_or_over("p", &window(5., 0.1), &window(12., 0.1)), Ok(None));
        let over = busy_or_over("p", &window(5., 0.1), &window(19.2, 0.1)).unwrap_err();
        assert!(over.contains("over the 16 ms budget on a quiet machine"), "{over}");
        let note = busy_or_over("p", &window(5., 0.1), &window(19.2, 0.5)).unwrap().unwrap();
        assert!(note.contains("machine busy (50 % CPU by others): frame budget not judged"), "{note}");
        assert!(busy_or_over("p", &window(5., 0.6), &window(19.2, 0.1)).unwrap().unwrap().contains("60 % CPU by others"), "a busy baseline");
        let slow = Stretch { stats: stats(42., 3), others_cpu: 0.15 };
        assert!(busy_or_over("p", &slow, &window(19.2, 0.1)).is_err(), "slow frames alone still judge own work");
        assert!(busy_or_over("p", &slow, &window(12., 0.1)).unwrap().unwrap().contains("interval checks SKIP"));
    }

    #[test]
    fn others_share_leaves_out_this_process() {
        let a = Cpu { idle: 0, total: 0, own: 0 };
        assert_eq!(a.others_share(&Cpu { idle: 400, total: 1000, own: 100 }), 0.5);
        assert_eq!(a.others_share(&a), 0.);
    }
}
