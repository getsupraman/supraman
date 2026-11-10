//! The tab strip's browser chords; the full list is docs/KEYS.md.

use std::path::PathBuf;

use gpui::{Context, KeyBinding, Window};
use supraman_core::{TabKeys, holder_of, paths::within};
use supraman_ui::tabs;

use super::{Detach, FleetView, KEY_CONTEXT, TabKind};
use crate::saved_tabs::SavedTab;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabKey {
    Next,
    Previous,
    /// Ctrl+T and Ctrl+W: a terminal's program keeps them under [`TabKeys::OutsideTerminals`].
    New,
    Close,
    NewAnywhere,
    CloseAnywhere,
    Reopen,
    /// 0..=7; `None` is the last tab.
    Nth(Option<usize>),
    Move(isize),
}

#[derive(Clone, PartialEq, Debug, gpui::Action)]
#[action(namespace = supraman, no_json)]
pub struct TabChord(pub TabKey);

pub const CHORDS: [(&str, TabKey); 20] = [
    ("ctrl-tab", TabKey::Next),
    ("ctrl-pagedown", TabKey::Next),
    ("ctrl-shift-tab", TabKey::Previous),
    ("ctrl-pageup", TabKey::Previous),
    ("ctrl-t", TabKey::New),
    ("ctrl-w", TabKey::Close),
    ("ctrl-shift-t", TabKey::NewAnywhere),
    ("ctrl-shift-w", TabKey::CloseAnywhere),
    ("ctrl-alt-t", TabKey::Reopen),
    ("ctrl-1", TabKey::Nth(Some(0))),
    ("ctrl-2", TabKey::Nth(Some(1))),
    ("ctrl-3", TabKey::Nth(Some(2))),
    ("ctrl-4", TabKey::Nth(Some(3))),
    ("ctrl-5", TabKey::Nth(Some(4))),
    ("ctrl-6", TabKey::Nth(Some(5))),
    ("ctrl-7", TabKey::Nth(Some(6))),
    ("ctrl-8", TabKey::Nth(Some(7))),
    ("ctrl-9", TabKey::Nth(None)),
    ("ctrl-shift-pageup", TabKey::Move(-1)),
    ("ctrl-shift-pagedown", TabKey::Move(1)),
];

const CLOSED_KEPT: usize = 20;

/// Matched before [`FleetView::key_down`], so a terminal sees none of these unless [`passes`] says so.
pub fn bind(cx: &mut gpui::App) {
    cx.bind_keys(CHORDS.map(|(keys, key)| KeyBinding::new(keys, TabChord(key), Some(KEY_CONTEXT))));
}

/// Whether the chord goes on to the active tab's terminal instead.
pub fn passes(key: TabKey, mode: TabKeys, terminal_active: bool) -> bool {
    matches!(key, TabKey::New | TabKey::Close) && mode == TabKeys::OutsideTerminals && terminal_active
}

pub struct TabKeyState {
    pub mode: TabKeys,
    closed: Vec<SavedTab>,
}

impl TabKeyState {
    pub fn new(mode: TabKeys) -> Self {
        Self { mode, closed: Vec::new() }
    }

    pub fn has_closed(&self) -> bool {
        !self.closed.is_empty()
    }

    fn push(&mut self, saved: SavedTab) {
        if self.closed.len() == CLOSED_KEPT {
            self.closed.remove(0);
        }
        self.closed.push(saved);
    }

    pub(super) fn pop(&mut self) -> Option<SavedTab> {
        self.closed.pop()
    }
}

impl FleetView {
    pub(super) fn on_tab_chord(&mut self, chord: &TabChord, window: &mut Window, cx: &mut Context<Self>) {
        if passes(chord.0, self.tab_keys.read(cx).mode, self.active_terminal().is_some()) {
            cx.propagate();
            return;
        }
        let at = self.active.and_then(|id| self.tabs.iter().position(|t| t.id == id));
        match chord.0 {
            TabKey::Next => self.cycle_tab(1, cx),
            TabKey::Previous => self.cycle_tab(-1, cx),
            TabKey::New | TabKey::NewAnywhere => self.new_task_tab(window, cx),
            TabKey::Close | TabKey::CloseAnywhere => {
                if let Some(id) = self.active {
                    self.close_tab(id, cx);
                }
            }
            TabKey::Reopen => self.reopen_closed(cx),
            TabKey::Nth(n) => {
                if let Some(i) = tabs::nth(self.tabs.len(), n) {
                    self.activate_tab(self.tabs[i].id, cx);
                }
            }
            TabKey::Move(step) => {
                if let Some((from, to)) = at.and_then(|at| Some((at, tabs::moved(self.tabs.len(), at, step)?))) {
                    tabs::reorder(&mut self.tabs, from, to);
                    cx.notify();
                }
            }
        }
    }

    /// Called before a tab closes, by its × or a chord; a tab already detaching was remembered by the close that began it.
    pub(super) fn remember_closed(&mut self, id: u64, cx: &mut gpui::App) {
        let closing = self.tab(id).filter(|tab| !matches!(tab.detach, Detach::Detaching | Detach::Pending));
        if let Some(saved) = closing.and_then(|tab| self.saved_tab(tab, cx)) {
            self.tab_keys.update(cx, |keys, _| keys.push(saved));
        }
    }

    /// A Start page in the active tab's workspace and folder, else in the first workspace.
    fn new_task_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((workspace, folder)) = self.active_workspace(cx).or_else(|| self.workspaces.first().map(|w| (w.name.clone(), None))) else {
            self.pane_toast.show_why("No workspace to start a task in: add one first.".into(), |v: &mut Self| &mut v.pane_toast, cx);
            return;
        };
        self.open_start(&workspace, folder, false, window, cx);
    }

    fn active_workspace(&self, cx: &gpui::App) -> Option<(String, Option<PathBuf>)> {
        let tab = self.active.and_then(|id| self.tab(id))?;
        let cwd = match &tab.kind {
            TabKind::Work(view) => return Some((view.read(cx).workspace.clone(), None)),
            TabKind::Start(page) => return Some((page.workspace.clone(), Some(page.folder.clone()))),
            TabKind::Git(view) => view.read(cx).folder.clone(),
            _ => tab.cwd.clone()?,
        };
        let ws = holder_of(&cwd, &self.workspaces)?;
        let folder = ws.folders.iter().filter(|f| within(&cwd, &f.path)).max_by_key(|f| f.path.as_os_str().len()).map(|f| f.path.clone());
        Some((ws.name.clone(), folder))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bound(keys: &str) -> Option<TabKey> {
        CHORDS.iter().find(|(k, _)| *k == keys).map(|(_, key)| *key)
    }

    #[test]
    fn the_chord_table_is_the_browsers() {
        assert_eq!(bound("ctrl-t"), Some(TabKey::New));
        assert_eq!(bound("ctrl-w"), Some(TabKey::Close));
        assert_eq!(bound("ctrl-shift-t"), Some(TabKey::NewAnywhere));
        assert_eq!(bound("ctrl-shift-w"), Some(TabKey::CloseAnywhere));
        assert_eq!(bound("ctrl-alt-t"), Some(TabKey::Reopen));
        for n in 1..=8 {
            assert_eq!(bound(&format!("ctrl-{n}")), Some(TabKey::Nth(Some(n - 1))), "ctrl-{n}");
        }
        assert_eq!(bound("ctrl-9"), Some(TabKey::Nth(None)));
        assert_eq!(bound("ctrl-0"), None, "the terminal's font reset");
        assert_eq!(bound("ctrl-shift-pageup"), Some(TabKey::Move(-1)));
        assert_eq!(bound("ctrl-shift-pagedown"), Some(TabKey::Move(1)));
        assert_eq!((bound("ctrl-tab"), bound("ctrl-pagedown")), (Some(TabKey::Next), Some(TabKey::Next)));
        assert_eq!((bound("ctrl-shift-tab"), bound("ctrl-pageup")), (Some(TabKey::Previous), Some(TabKey::Previous)));
        let mut keys: Vec<&str> = CHORDS.iter().map(|(k, _)| *k).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), CHORDS.len(), "each chord bound once");
    }

    #[test]
    fn only_ctrl_t_and_ctrl_w_reach_a_terminal_and_only_outside_terminals_mode() {
        for (_, key) in CHORDS {
            let through = matches!(key, TabKey::New | TabKey::Close);
            assert_eq!(passes(key, TabKeys::OutsideTerminals, true), through, "{key:?} in a terminal, mode A");
            assert!(!passes(key, TabKeys::OutsideTerminals, false), "{key:?} outside a terminal, mode A");
            assert!(!passes(key, TabKeys::Everywhere, true), "{key:?} in a terminal, mode B");
        }
    }

    #[test]
    fn the_closed_tabs_keep_the_last_twenty() {
        let mut state = TabKeyState::new(TabKeys::default());
        for i in 0..25 {
            state.push(SavedTab::Work { title: format!("w{i}"), workspace: "ws".into() });
        }
        assert_eq!(state.closed.len(), CLOSED_KEPT);
        assert!(matches!(state.closed.pop(), Some(SavedTab::Work { title, .. }) if title == "w24"));
        assert!(matches!(state.closed.first(), Some(SavedTab::Work { title, .. }) if title == "w5"));
    }
}
