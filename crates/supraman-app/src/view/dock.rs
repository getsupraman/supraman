//! A tab's dock: the panels it shows beside its own, laid out by the kit's [`Layout`], the focused one and each
//! panel's focus handle. Every panel is a [`Tab`] in the window's list; one shown in another's dock is no tab of the strip.

use gpui::{AnyElement, App, Context, FocusHandle, Focusable as _, MouseButton, MouseDownEvent, SharedString, Window, div, prelude::*};
use supraman_ui::dock::layout::Layout;
use supraman_ui::dock::view::{Dock, Panel};
use supraman_ui::hit::HitTargets;
use supraman_ui::icon::Icon;

use super::{FleetView, PanelKey, Tab, TabKind};

/// A tab of two or more panels; a tab of one has none.
pub struct TabDock {
    pub tab: u64,
    pub layout: Layout<u64>,
    pub focused: u64,
    focus: Vec<(u64, FocusHandle)>,
}

#[derive(Default)]
pub struct Docks {
    docks: Vec<TabDock>,
}

/// A panel's title bar as it drags.
#[derive(Clone)]
pub struct DraggedPanel {
    pub panel: u64,
    pub title: SharedString,
}

impl supraman_ui::dnd::Payload for DraggedPanel {
    fn title(&self) -> SharedString {
        self.title.clone()
    }
}

impl Docks {
    pub fn of(&self, tab: u64) -> Option<&TabDock> {
        self.docks.iter().find(|d| d.tab == tab)
    }

    fn holding(&self, panel: u64) -> Option<&TabDock> {
        self.docks.iter().find(|d| d.layout.contains(panel))
    }

    /// The tab whose dock shows `panel`; a panel in no dock is its own tab.
    pub fn tab_of(&self, panel: u64) -> u64 {
        self.holding(panel).map_or(panel, |d| d.tab)
    }

    pub fn is_inner(&self, panel: u64) -> bool {
        self.tab_of(panel) != panel
    }

    pub fn panels(&self, tab: u64) -> Vec<u64> {
        self.of(tab).map_or_else(|| vec![tab], |d| d.layout.panels())
    }

    pub fn focused(&self, tab: u64) -> u64 {
        self.of(tab).map_or(tab, |d| d.focused)
    }

    pub fn handle(&self, panel: u64) -> Option<&FocusHandle> {
        self.holding(panel)?.focus.iter().find(|(p, _)| *p == panel).map(|(_, h)| h)
    }

    /// Whether the dock's focus moved to `panel`.
    pub fn focus(&mut self, panel: u64) -> bool {
        let Some(dock) = self.docks.iter_mut().find(|d| d.layout.contains(panel)) else { return false };
        std::mem::replace(&mut dock.focused, panel) != panel
    }

    /// `panel`, in no dock, added to `tab`'s by the layout's rule and focused; false when refused.
    #[cfg_attr(not(test), allow(dead_code, reason = "the dock's menu and drag units add panels"))]
    pub fn add(&mut self, tab: u64, panel: u64, mut handle: impl FnMut() -> FocusHandle) -> bool {
        if tab == panel || self.holding(panel).is_some() || self.is_inner(tab) {
            return false;
        }
        match self.docks.iter_mut().find(|d| d.tab == tab) {
            Some(dock) => {
                let Some(layout) = dock.layout.add(panel) else { return false };
                dock.layout = layout;
                dock.focus.push((panel, handle()));
                dock.focused = panel;
            }
            None => {
                let Some(layout) = Layout::Leaf(tab).add(panel) else { return false };
                self.docks.push(TabDock { tab, layout, focused: panel, focus: vec![(tab, handle()), (panel, handle())] });
            }
        }
        true
    }

    /// A divider's drag or a move inside the dock; a layout of other panels is refused.
    pub fn set_layout(&mut self, tab: u64, layout: Layout<u64>) -> bool {
        let Some(dock) = self.docks.iter_mut().find(|d| d.tab == tab) else { return false };
        let (mut was, mut now) = (dock.layout.panels(), layout.panels());
        was.sort_unstable();
        now.sort_unstable();
        if was != now || !layout.is_valid() {
            return false;
        }
        dock.layout = layout;
        true
    }

    /// `panel` out of its dock: the tab it was in and the tab that dock is now, its next first panel when `panel`
    /// was the tab's own; a dock left with one panel goes.
    pub fn remove(&mut self, panel: u64) -> Option<(u64, u64)> {
        let at = self.docks.iter().position(|d| d.layout.contains(panel))?;
        let dock = &mut self.docks[at];
        let rest = dock.layout.remove(panel)?;
        let was = dock.tab;
        let now = if was == panel { rest.panels()[0] } else { was };
        if rest.len() == 1 {
            self.docks.remove(at);
            return Some((was, now));
        }
        dock.layout = rest;
        dock.tab = now;
        dock.focus.retain(|(p, _)| *p != panel);
        if dock.focused == panel {
            dock.focused = now;
        }
        Some((was, now))
    }
}

impl FleetView {
    /// The strip's tabs: every panel but those shown in another's dock.
    pub fn headers(&self) -> impl Iterator<Item = &Tab> {
        self.tabs.iter().filter(|t| !self.docks.is_inner(t.id))
    }

    pub(super) fn header_ids(&self) -> Vec<u64> {
        self.headers().map(|t| t.id).collect()
    }

    /// The strip's tab at `from` moved to `to`, both as the strip counts them.
    pub(super) fn reorder_tabs(&mut self, from: usize, to: usize) {
        let ids = self.header_ids();
        let at = |id: Option<&u64>, tabs: &[Tab]| id.and_then(|id| tabs.iter().position(|t| t.id == *id));
        if let (Some(from), Some(to)) = (at(ids.get(from), &self.tabs), at(ids.get(to), &self.tabs)) {
            supraman_ui::tabs::reorder(&mut self.tabs, from, to);
        }
    }

    /// `id`'s entry moved to just before `before`'s.
    fn place_before(&mut self, id: u64, before: u64) {
        let (Some(from), Some(to)) = (self.tabs.iter().position(|t| t.id == id), self.tabs.iter().position(|t| t.id == before)) else { return };
        let tab = self.tabs.remove(from);
        self.tabs.insert(if from < to { to - 1 } else { to }, tab);
    }

    /// `panel`, a tab of one, joins `tab`'s dock focused; false when the layout refuses it.
    #[cfg_attr(not(test), allow(dead_code, reason = "the dock's menu and drag units add panels"))]
    pub fn add_panel(&mut self, tab: u64, panel: u64, cx: &mut Context<Self>) -> bool {
        if self.tab(tab).is_none() || self.tab(panel).is_none() || self.docks.of(panel).is_some() || !self.docks.add(tab, panel, || cx.focus_handle()) {
            return false;
        }
        if self.active == Some(panel) {
            self.active = Some(tab);
        }
        cx.notify();
        true
    }

    /// `panel` out of its dock, a tab of its own after the tab it leaves.
    #[cfg_attr(not(test), allow(dead_code, reason = "the panel drag's unit takes panels out"))]
    pub fn remove_panel(&mut self, panel: u64, cx: &mut Context<Self>) -> bool {
        let Some((was, now)) = self.docks.remove(panel) else { return false };
        if was == panel {
            self.place_before(now, panel);
            if self.active == Some(panel) {
                self.active = Some(now);
            }
        } else if let Some(next) = self.tabs.iter().skip_while(|t| t.id != was).skip(1).find(|t| !self.docks.is_inner(t.id) && t.id != panel).map(|t| t.id) {
            self.place_before(panel, next);
        } else if let Some(at) = self.tabs.iter().position(|t| t.id == panel) {
            let tab = self.tabs.remove(at);
            self.tabs.push(tab);
        }
        cx.notify();
        true
    }

    /// A panel left the window: the dock it was in closes up, its tab passing to the next panel when it was the tab's own.
    pub(super) fn undock_removed(&mut self, panel: u64) {
        if let Some((was, now)) = self.docks.remove(panel)
            && was == panel
        {
            self.place_before(now, panel);
            if self.active == Some(panel) {
                self.active = Some(now);
            }
        }
    }

    /// The panel showing `key`, in any tab's dock.
    pub fn find_panel(&self, key: &PanelKey, cx: &App) -> Option<u64> {
        self.tabs.iter().find(|t| t.key(cx) == *key).map(|t| t.id)
    }

    /// `panel`'s tab active and `panel` focused in its dock.
    pub fn show_panel(&mut self, panel: u64, cx: &mut Context<Self>) {
        let tab = self.docks.tab_of(panel);
        let shown = self.active == Some(tab);
        self.docks.focus(panel);
        self.activate_tab(tab, cx);
        if shown && self.docks.of(tab).is_some() {
            let (window, entry) = (self.window, self.panel_entry(panel, cx));
            cx.defer(move |cx| {
                window.update(cx, |_, window, cx| supraman_ui::focus::request(&entry, window, cx)).ok();
            });
        }
    }

    /// The panel of the active tab the keys go to: the one holding the focus, else the dock's focused one.
    pub(super) fn focused_panel(&self, window: &Window, cx: &App) -> Option<u64> {
        let tab = self.active?;
        let Some(dock) = self.docks.of(tab) else { return Some(tab) };
        let holding = dock.layout.panels().into_iter().find(|p| self.docks.handle(*p).is_some_and(|h| h.contains_focused(window, cx)));
        Some(holding.unwrap_or(dock.focused))
    }

    /// The active tab's focused panel, as last painted.
    pub(super) fn shown_panel(&self) -> Option<u64> {
        self.active.map(|tab| self.docks.focused(tab))
    }

    /// The region a panel's keys and caret follow: its own in a dock, else the content area's.
    pub(super) fn panel_focus(&self, panel: u64) -> FocusHandle {
        self.docks.handle(panel).cloned().unwrap_or_else(|| self.focus.clone())
    }

    /// What a panel takes the keys with: a view's own, else its region's.
    pub(super) fn panel_entry(&self, panel: u64, cx: &App) -> FocusHandle {
        match self.tab(panel).map(|t| &t.kind) {
            Some(TabKind::Git(view)) => view.focus_handle(cx),
            Some(TabKind::Work(view)) => view.focus_handle(cx),
            _ => self.panel_focus(panel),
        }
    }

    /// The dock's focus follows the keys: F6 or a click into a panel.
    pub(super) fn follow_dock_focus(&mut self, window: &Window, cx: &App) {
        if let Some(panel) = self.focused_panel(window, cx) {
            self.docks.focus(panel);
        }
    }

    /// The active tab's dock through the kit: each panel under its scope `p<id>`, in its own focus region.
    pub(super) fn render_dock(&mut self, tab: u64, hits: &HitTargets, cx: &mut Context<Self>) -> Option<AnyElement> {
        let dock = self.docks.of(tab)?;
        let (layout, focused) = (dock.layout.clone(), dock.focused);
        let mut panels = Vec::new();
        for p in layout.panels() {
            let scope = hits.scoped(&format!("p{p}"));
            let focus = self.panel_focus(p);
            let element = self.render_panel(p, &scope, &focus, cx);
            let Some(t) = self.tab(p) else { continue };
            let (icon, words) = (panel_icon(t), SharedString::from(t.label()));
            let element = self.panel_frame(p, element, &scope, &focus, cx);
            panels.push((p, Panel { element, icon, words: words.clone(), payload: DraggedPanel { panel: p, title: words } }));
        }
        let dock = Dock::new(hits, format!("dock-{tab}"), |d: &DraggedPanel, _| Some(d.panel))
            .focused(Some(focused))
            .on_layout(move |this: &mut Self, layout, cx| {
                if this.docks.set_layout(tab, layout) {
                    cx.notify();
                }
            })
            .on_close(|this: &mut Self, panel, cx| this.close_panel(panel, cx));
        Some(dock.render(&layout, panels, cx))
    }

    /// A docked panel's region: a click focuses it, its pane registers under its scope, and it keeps its view's minimum size.
    fn panel_frame(&self, panel: u64, element: AnyElement, scope: &HitTargets, focus: &FocusHandle, cx: &mut Context<Self>) -> AnyElement {
        let tab = self.tab(panel);
        let min = match tab.map(|t| &t.kind) {
            Some(TabKind::Git(view)) => Some(view.read(cx).min_size()),
            Some(TabKind::Work(view)) => Some(view.read(cx).min_size()),
            _ => None,
        };
        let pane = tab.and_then(super::right::pane_of).map(|key| supraman_ui::focus::Pane::new(scope.key(key), focus).paint());
        div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .when_some(min, |d, min| d.min_w(min.width).min_h(min.height))
            .track_focus(focus)
            .capture_any_mouse_down(cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                if this.docks.focus(panel) {
                    cx.notify();
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                    if window.default_prevented() {
                        return;
                    }
                    let entry = this.panel_entry(panel, cx);
                    if !this.panel_focus(panel).contains_focused(window, cx) {
                        entry.focus(window, cx);
                    }
                    window.prevent_default();
                }),
            )
            .child(element)
            .children(pane)
            .into_any_element()
    }
}

fn panel_icon(tab: &Tab) -> Icon {
    match &tab.kind {
        TabKind::Git(_) => Icon::GitGraph,
        TabKind::Work(_) => Icon::Work,
        TabKind::Attach { .. } => Icon::Attach,
        TabKind::Schedule(_) => Icon::Clock,
        TabKind::Details(_) => Icon::File,
        TabKind::Start(_) => Icon::Task,
        TabKind::Shell if tab.claude.is_some() || tab.hand => Icon::Claude,
        TabKind::Shell | TabKind::Saved(_) => Icon::Terminal,
    }
}

#[cfg(test)]
mod tests;
