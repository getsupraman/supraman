//! What a Git tab shows, as plain values for the selftest to compare.

use gpui::{App, Pixels};
use supraman_ui::themes::contrast;
use theme::ActiveTheme as _;

use super::colours::{Palette, blend};
use super::details::State as PanelState;
use super::diff::{Layout, Mode};
use super::wip::State as WipState;
use super::{GitView, Selected, cells::ref_cell};

#[derive(Debug, Clone)]
pub struct ProbeRow {
    pub sha: String,
    pub refs: usize,
    pub ghost: bool,
    pub summary: String,
    pub stash: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProbeDiff {
    pub path: String,
    pub mode: Mode,
    pub layout: Layout,
    pub changes: usize,
    pub current: Option<usize>,
    /// [`super::diff::DiffView::shown`].
    pub shown: String,
}

/// The ref panel: its rows' paths, each section's `(shown, total)`, collapsed, the hidden and soloed refs, and
/// the open overlay's section key with its rows' paths.
#[derive(Debug, Clone)]
pub struct RefProbe {
    pub rows: Vec<String>,
    pub counts: [(usize, usize); 5],
    pub shut: bool,
    pub hidden: Vec<String>,
    pub solo: Vec<String>,
    pub overlay: Option<(&'static str, Vec<String>)>,
}

#[derive(Debug, Clone)]
pub struct Probe {
    pub rows: Vec<ProbeRow>,
    pub selected: Option<Selected>,
    pub listed: Option<usize>,
    /// The commit panel's sha and parents, once it shows one.
    pub commit: Option<(String, Vec<String>)>,
    /// The WIP panel's unstaged and staged paths, once it shows them.
    pub wip: Option<(Vec<String>, Vec<String>)>,
    pub diff: Option<ProbeDiff>,
    pub panel_width: Pixels,
    pub refs: RefProbe,
}

impl Probe {
    pub fn row_of(&self, sha: &str) -> Option<usize> {
        self.rows.iter().position(|r| r.sha == sha)
    }
}

impl GitView {
    pub fn probe(&self, cx: &App) -> Probe {
        let rows = self.rows.iter().map(|r| ProbeRow { sha: r.sha.to_string(), refs: r.row.refs.len(), ghost: ref_cell(&r.row, false).ghost, summary: r.summary.to_string(), stash: r.stash().is_some() }).collect();
        let commit = match self.panel.read(cx).state() {
            PanelState::Shown(s) => Some((s.sha.to_string(), s.parents.iter().map(|p| p.to_string()).collect())),
            _ => None,
        };
        let paths = |files: &[supraman_gitman::FileChange]| files.iter().map(|f| f.path.clone()).collect::<Vec<_>>();
        let wip = match self.wip_panel.read(cx).state() {
            WipState::Shown(s) => Some((paths(&s.unstaged), paths(&s.staged))),
            _ => None,
        };
        let diff = self.diff().map(|d| {
            let d = d.read(cx);
            ProbeDiff { path: d.target.path.clone(), mode: d.mode(), layout: d.layout(), changes: d.hunk_count(), current: d.current(), shown: d.shown() }
        });
        let panel = self.ref_panel().read(cx);
        let refs = RefProbe {
            rows: panel.visible().iter().map(|p| p.to_string()).collect(),
            counts: panel.counts().map(|c| (c.shown, c.total)),
            shut: panel.shut(),
            hidden: panel.walk().hidden.iter().cloned().collect(),
            solo: panel.walk().solo.iter().cloned().collect(),
            overlay: panel.overlay().map(|s| (s.key(), panel.overlay_rows().iter().map(|p| p.to_string()).collect())),
        };
        Probe { rows, selected: self.selected(), listed: self.listed(), commit, wip, diff, panel_width: self.panel_width, refs }
    }
}

/// The lowest contrast of the theme's text on a lane's band, and of its text on a pill or the date and sha on the background.
pub fn worst_contrast(cx: &App) -> (f32, f32) {
    let p = Palette::of(cx.theme());
    let on = |fg, layer| contrast(fg, blend(layer, p.background));
    let muted = contrast(p.muted, p.background);
    (0..supraman_gitman::LANE_COLOURS).fold((f32::MAX, muted), |(name, secondary), i| {
        (name.min(on(p.text, p.band(i))), secondary.min(on(p.text, p.pill(i))))
    })
}
