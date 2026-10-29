//! A busy machine stretches the frame interval, so it is judged only after a quiet [`baseline`]; own work per frame always.

use std::{sync::OnceLock, time::Duration};

use anyhow::bail;
use foreman_ui::FrameStats;
use gpui::{AsyncWindowContext, Entity};

use super::sleep;
use crate::view::FleetView;

/// p10: a per-frame cost lifts every frame, preemption only some; measured 5–10 ms, the red check gives 20–24 ms.
pub(crate) const OWN_WORK_P10_MS: f64 = 16.0;
/// Idle at 60 Hz is 16.67 ms; a baseline whose median is above this is not a quiet machine.
const QUIET_P50_MS: f64 = 18.5;
/// At 97 % other-process CPU one frame of 58 ms was seen with the UI thread raised.
const BUSY_CPU: f64 = 0.75;
const BASELINE: Duration = Duration::from_secs(1);

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

pub(crate) async fn baseline(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> Stretch {
    record(view, BASELINE, cx).await
}

pub(crate) async fn judge(
    phase: &str,
    before: &Stretch,
    during: &Stretch,
    view: &Entity<FleetView>,
    cx: &mut AsyncWindowContext,
) -> anyhow::Result<String> {
    let stats = &during.stats;
    if !(stats.work_p10 <= OWN_WORK_P10_MS) {
        bail!("{phase}: own work per frame p10 {:.2} ms, over the {OWN_WORK_P10_MS} ms budget: {stats}", stats.work_p10);
    }
    let busy = before.busy().map(|why| format!("baseline {why}")).or_else(|| {
        (during.others_cpu >= BUSY_CPU).then(|| format!("other processes {:.0} % CPU during the phase", during.others_cpu * 100.))
    });
    if let Some(why) = busy {
        return Ok(format!("{stats}; interval checks SKIP (machine busy: {why})"));
    }
    if stats.over_50 > 0 {
        let after = baseline(view, cx).await;
        if let Some(why) = after.busy() {
            return Ok(format!("{stats}; interval checks SKIP (machine busy: baseline after the phase {why})"));
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
    fn others_share_leaves_out_this_process() {
        let a = Cpu { idle: 0, total: 0, own: 0 };
        assert_eq!(a.others_share(&Cpu { idle: 400, total: 1000, own: 100 }), 0.5);
        assert_eq!(a.others_share(&a), 0.);
    }
}
