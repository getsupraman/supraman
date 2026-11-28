//! What a dragged panel, tab or session row does over a tab's dock, and a panel let go on the strip.

use gpui::Context;
use supraman_ui::dnd::Zone;
use supraman_ui::dock::layout::Layout;
use supraman_ui::tabs::DraggedTab;

use super::drop::Source;
use super::{FleetView, Tab};

/// What is dragged, as the rules weigh it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dragged<'a> {
    /// A panel's title bar, out of the dock of the tab `dock`.
    Panel { id: u64, dock: u64 },
    /// A strip's tab of `panels` panels.
    Tab { id: u64, panels: usize },
    Session(&'a str),
}

/// What the panel dropped on shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Terminal,
    Git,
    Work,
    Details,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    Nothing,
    /// The dragged session's SendMessage name pasted in the target's terminal.
    Paste,
    /// This panel placed on the target's edge, as the dock's layout allows.
    Place(u64),
}

/// The panel a dock's edge places for `dragged`: a panel, or a tab of one; a tab of more and a session row none.
pub fn placed(dragged: Dragged) -> Option<u64> {
    match dragged {
        Dragged::Panel { id, .. } | Dragged::Tab { id, panels: 1 } => Some(id),
        Dragged::Tab { .. } | Dragged::Session(_) => None,
    }
}

/// What `dragged` let go on `zone` of the panel `target`, which shows `kind`, does.
pub fn panel_drop(target: u64, kind: Kind, zone: Zone, dragged: Dragged) -> Rule {
    let own = matches!(dragged, Dragged::Panel { id, .. } | Dragged::Tab { id, .. } if id == target);
    if own {
        return Rule::Nothing;
    }
    match (zone, dragged) {
        (_, Dragged::Session(_)) | (Zone::Centre, _) => if kind == Kind::Terminal { Rule::Paste } else { Rule::Nothing },
        _ => placed(dragged).map_or(Rule::Nothing, Rule::Place),
    }
}

/// A dragged tab of `panels` panels, or a panel out of a dock.
pub fn dragged(d: &DraggedTab, panels: usize) -> Dragged<'static> {
    match d.dock {
        Some(dock) => Dragged::Panel { id: d.id, dock },
        None => Dragged::Tab { id: d.id, panels },
    }
}

pub fn kind_of(tab: &Tab) -> Kind {
    use super::TabKind;
    match &tab.kind {
        TabKind::Git(_) => Kind::Git,
        TabKind::Work(_) => Kind::Work,
        TabKind::Details(_) => Kind::Details,
        _ if tab.terminal.is_some() => Kind::Terminal,
        _ => Kind::Other,
    }
}

impl FleetView {
    fn dragged_of(&self, d: &DraggedTab) -> Dragged<'static> {
        dragged(d, self.docks.panels(d.id).len())
    }

    /// A tab or panel let go on `target`'s centre.
    pub(super) fn centre_drop(&mut self, target: u64, d: &DraggedTab, cx: &mut Context<Self>) {
        let Some(kind) = self.tab(target).map(kind_of) else { return };
        if panel_drop(target, kind, Zone::Centre, self.dragged_of(d)) == Rule::Paste {
            let session = self.tab(d.id).and_then(|t| self.tab_session_id(t));
            self.drop_on_terminal(target, Source::Tab(d.id, session.as_deref()), d.title.clone(), cx);
        }
    }

    /// `tab`'s dock laid out as `layout`: a move inside it, or one panel more brought from its own tab or another dock.
    pub(super) fn lay_out_dock(&mut self, tab: u64, layout: Layout<u64>, cx: &mut Context<Self>) {
        let held = self.docks.panels(tab);
        let mut come: Vec<u64> = layout.panels().into_iter().filter(|p| !held.contains(p)).collect();
        let placed = match (come.pop(), come.is_empty()) {
            (None, _) => self.docks.set_layout(tab, layout),
            (Some(panel), true) => self.join_dock(tab, layout, panel, cx),
            (Some(_), false) => false,
        };
        if placed {
            cx.notify();
        }
    }

    fn join_dock(&mut self, tab: u64, layout: Layout<u64>, panel: u64, cx: &mut Context<Self>) -> bool {
        let mut want = self.docks.panels(tab);
        want.push(panel);
        let fits = self.tab(panel).is_some() && self.docks.of(panel).is_none() && self.docks.fits(tab, &layout, &want);
        if !fits {
            return false;
        }
        if self.docks.is_inner(panel) {
            self.undock_removed(panel);
        }
        if !self.docks.place(tab, layout, panel, || cx.focus_handle()) {
            return false;
        }
        self.show_panel(panel, cx);
        true
    }

    /// A dock's panel let go on the strip: its own tab at the strip's place `at`, its dock closing up and still shown.
    pub(super) fn panel_to_strip(&mut self, panel: u64, at: usize, cx: &mut Context<Self>) {
        let dock = self.docks.tab_of(panel);
        let Some(mate) = self.docks.panels(dock).into_iter().find(|p| *p != panel) else { return };
        let before = self.header_ids().get(at).copied();
        if !self.remove_panel(panel, cx) {
            return;
        }
        let before = before.map(|id| if id == panel { self.docks.tab_of(mate) } else { id });
        self.place_header(panel, before);
    }
}

#[cfg(test)]
mod tests;
