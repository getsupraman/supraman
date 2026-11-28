//! A tab's dock: the panels it shows beside its own, laid out by the kit's [`Layout`], the focused one and each
//! panel's focus handle. Every panel is a [`Tab`] in the window's list; one shown in another's dock is no tab of the strip.

use gpui::{AnyElement, App, Context, FocusHandle, Focusable as _, MouseButton, MouseDownEvent, SharedString, Window, div, prelude::*};
use supraman_ui::dock::layout::Layout;
use supraman_ui::dock::view::{Dock, Panel};
use supraman_ui::hit::HitTargets;
use supraman_ui::icon::Icon;
use supraman_ui::tabs::DraggedTab;

use super::panel_drop::{dragged, placed};
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

    /// Whether `tab`'s dock takes one more panel by the layout's rule.
    pub fn has_room(&self, tab: u64) -> bool {
        self.of(tab).is_none_or(|d| d.layout.add(u64::MAX).is_some())
    }

    /// `panel`, in no dock, added to `tab`'s by the layout's rule and focused; false when refused.
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

    /// Whether `layout` lays out exactly `panels` as `tab`'s dock.
    pub fn fits(&self, tab: u64, layout: &Layout<u64>, panels: &[u64]) -> bool {
        let (mut want, mut now) = (panels.to_vec(), layout.panels());
        want.sort_unstable();
        now.sort_unstable();
        !self.is_inner(tab) && want == now && layout.is_valid()
    }

    /// `panel`, in no dock, placed in `tab`'s as `layout` lays it out, and focused; false when it does not fit.
    pub fn place(&mut self, tab: u64, layout: Layout<u64>, panel: u64, mut handle: impl FnMut() -> FocusHandle) -> bool {
        let mut want = self.panels(tab);
        want.push(panel);
        if tab == panel || self.holding(panel).is_some() || !self.fits(tab, &layout, &want) {
            return false;
        }
        match self.docks.iter_mut().find(|d| d.tab == tab) {
            Some(dock) => {
                dock.layout = layout;
                dock.focus.push((panel, handle()));
                dock.focused = panel;
            }
            None => self.docks.push(TabDock { tab, layout, focused: panel, focus: vec![(tab, handle()), (panel, handle())] }),
        }
        true
    }

    /// A reopened dock: `layout` over panels in no dock, `tab` and `focused` among them; false when it does not fit.
    pub fn restore(&mut self, tab: u64, layout: Layout<u64>, focused: u64, mut handle: impl FnMut() -> FocusHandle) -> bool {
        let panels = layout.panels();
        if panels.len() < 2 || !layout.is_valid() || !panels.contains(&tab) || !panels.contains(&focused) || panels.iter().any(|p| self.holding(*p).is_some()) {
            return false;
        }
        self.docks.push(TabDock { tab, layout, focused, focus: panels.into_iter().map(|p| (p, handle())).collect() });
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

    /// `id`'s entry moved to just before `before`'s, else last.
    pub(super) fn place_header(&mut self, id: u64, before: Option<u64>) {
        match before {
            Some(before) => self.place_before(id, before),
            None => {
                if let Some(at) = self.tabs.iter().position(|t| t.id == id) {
                    let tab = self.tabs.remove(at);
                    self.tabs.push(tab);
                }
            }
        }
    }

    /// `id`'s entry moved to just before `before`'s.
    fn place_before(&mut self, id: u64, before: u64) {
        let (Some(from), Some(to)) = (self.tabs.iter().position(|t| t.id == id), self.tabs.iter().position(|t| t.id == before)) else { return };
        let tab = self.tabs.remove(from);
        self.tabs.insert(if from < to { to - 1 } else { to }, tab);
    }

    /// `panel`, a tab of one, joins `tab`'s dock focused; false when the layout refuses it.
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

    /// The active tab's dock through the kit, a tab of one its panel alone until a payload is dragged: each docked panel
    /// under its scope `p<id>`, in its own focus region.
    pub(super) fn render_dock(&mut self, tab: u64, hits: &HitTargets, cx: &mut Context<Self>) -> AnyElement {
        let (layout, focused) = self.docks.of(tab).map_or((Layout::Leaf(tab), tab), |d| (d.layout.clone(), d.focused));
        let panels = match layout {
            Layout::Leaf(_) => {
                let focus = self.focus.clone();
                let element = self.render_panel(tab, hits, &focus, cx);
                vec![(tab, self.dock_panel(tab, tab, element))]
            }
            Layout::Split { .. } => layout.panels().into_iter().filter_map(|p| {
                let scope = hits.scoped(&format!("p{p}"));
                let focus = self.panel_focus(p);
                let element = self.render_panel(p, &scope, &focus, cx);
                let element = self.panel_frame(p, element, &scope, &focus, cx);
                self.tab(p).is_some().then(|| (p, self.dock_panel(tab, p, element)))
            }).collect(),
        };
        let sizes: Vec<(u64, usize)> = self.headers().map(|t| (t.id, self.docks.panels(t.id).len())).collect();
        let panel_of = move |d: &DraggedTab, _: &App| {
            let panels = sizes.iter().find(|(id, _)| *id == d.id).map_or(1, |(_, n)| *n);
            placed(dragged(d, panels))
        };
        let dock = Dock::new(hits, format!("dock-{tab}"), panel_of)
            .focused(Some(focused))
            .on_layout(move |this: &mut Self, layout, cx| this.lay_out_dock(tab, layout, cx))
            .on_close(|this: &mut Self, panel, cx| this.close_panel(panel, cx))
            .on_centre(|this: &mut Self, target, d: &DraggedTab, _, cx| this.centre_drop(target, d, cx));
        dock.render(&layout, panels, cx)
    }

    fn dock_panel(&self, tab: u64, p: u64, element: AnyElement) -> Panel<DraggedTab> {
        let (icon, words) = self.tab(p).map_or((Icon::Terminal, SharedString::default()), |t| (panel_icon(t), SharedString::from(t.label())));
        Panel { element, icon, words: words.clone(), payload: DraggedTab { id: p, title: words, dock: Some(tab) } }
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
pub(in crate::view) mod tests;
