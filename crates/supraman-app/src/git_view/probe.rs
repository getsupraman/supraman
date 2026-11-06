//! What a Git tab shows, as plain values for the selftest to compare.

use gpui::{App, Pixels};

use super::details::State as PanelState;
use super::{GitView, Selected};

#[derive(Debug, Clone)]
pub struct ProbeRow {
    pub sha: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProbeDiff {
    pub path: String,
    /// [`super::diff::DiffView::shown`].
    pub shown: String,
}

#[derive(Debug, Clone)]
pub struct Probe {
    pub rows: Vec<ProbeRow>,
    pub selected: Option<Selected>,
    /// The commit panel's sha and parents, once it shows one.
    pub commit: Option<(String, Vec<String>)>,
    pub diff: Option<ProbeDiff>,
    pub panel_width: Pixels,
    /// The ref panel's hidden refs.
    pub hidden: Vec<String>,
}

impl Probe {
    pub fn row_of(&self, sha: &str) -> Option<usize> {
        self.rows.iter().position(|r| r.sha == sha)
    }
}

impl GitView {
    pub fn probe(&self, cx: &App) -> Probe {
        let rows = self.rows.iter().map(|r| ProbeRow { sha: r.sha.to_string() }).collect();
        let commit = match self.panel.read(cx).state() {
            PanelState::Shown(s) => Some((s.sha.to_string(), s.parents.iter().map(|p| p.to_string()).collect())),
            _ => None,
        };
        let diff = self.diff().map(|d| {
            let d = d.read(cx);
            ProbeDiff { path: d.target.path.clone(), shown: d.shown() }
        });
        let hidden = self.ref_panel().read(cx).walk().hidden.iter().cloned().collect();
        Probe { rows, selected: self.selected(), commit, diff, panel_width: self.panel_width, hidden }
    }
}
