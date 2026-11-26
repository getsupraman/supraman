use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Instant,
};

use supraman_core::timeline::ms_since_start;

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
    /// When the slowest frame was presented, in ms since [`supraman_core::timeline::process_start`].
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

pub fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let rank = ((p / 100.0) * (sorted.len() - 1) as f64).round() as usize;
    sorted[rank.min(sorted.len() - 1)]
}
