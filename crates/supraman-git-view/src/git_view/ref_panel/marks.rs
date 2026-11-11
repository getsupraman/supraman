//! A ref row's mark, right-click menu and double-click; hit keys `<list>-mark-<path>`, `menu-ref-solo`, `menu-ref-hide`.

use gpui::{AnyElement, ClickEvent, Context, MouseButton, MouseDownEvent, Pixels, Point, SharedString, Window, div, prelude::*, px};
use supraman_ui::button;
use supraman_ui::hit::hit_target;
use supraman_ui::icon::Icon;
use supraman_ui::menu::{ContextMenu, MenuItem, item};
use supraman_ui::table::row_group;
use supraman_ui::tooltip::Tooltip;
use supraman_ui::tree::{Item, Row};
use theme::ActiveTheme as _;

use super::overlay::OVERLAY;
use super::{Entry, Keys, RefEvent, RefPanel, Section};
use crate::git_view::cells;
use crate::git_view::stash::{self, StashAct, StashRef};
use supraman_ui::themes::secondary;

/// The mark slot's width.
pub(super) const MARK: f32 = 18.;
const ICON: f32 = 12.;

/// What the ref menu does: the hide and solo keys of the ref or folder it was opened on, or a stash's write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RefAction {
    ToggleSolo(Vec<String>),
    ToggleHidden(Vec<String>),
    Stash(StashAct, StashRef),
    /// Every other open section closed.
    Maximize(Section),
}

enum Mark {
    Current,
    Soloed(Vec<String>),
    Hidden(Vec<String>),
    Eye(Vec<String>),
}

impl RefPanel {
    fn mark_of(&self, hide: Vec<String>, solo: Vec<String>, current: bool) -> Option<Mark> {
        if current {
            Some(Mark::Current)
        } else if self.all_soloed(&solo) {
            Some(Mark::Soloed(solo))
        } else if self.all_hidden(&hide) {
            Some(Mark::Hidden(hide))
        } else {
            (!hide.is_empty()).then_some(Mark::Eye(hide))
        }
    }

    fn keys_of(row: &Row<'_, Entry, Keys>) -> (Vec<String>, Vec<String>, bool) {
        match row.item {
            Item::File(entry) => (entry.hide_key().into_iter().collect(), entry.solo_key().into_iter().collect(), matches!(entry, Entry::Branch { current: true, .. })),
            Item::Folder { aggregate, .. } => (aggregate.hide.clone(), aggregate.solo.clone(), false),
        }
    }

    /// The mark of a row of `list`.
    pub(super) fn paint_mark(&self, list: &str, row: Row<'_, Entry, Keys>, cx: &mut Context<Self>) -> Option<AnyElement> {
        let key = format!("{list}-mark-{}", row.path);
        if let Item::File(Entry::Worktree { .. }) = row.item {
            let whole = self.whole(list, row.path);
            return self.worktree_mark(key, &whole, cx);
        }
        let (hide, solo, current) = Self::keys_of(&row);
        let colors = cx.theme().colors();
        let (accent, muted) = (colors.text_accent, secondary(cx.theme().colors()));
        let (icon, colour, shown, tip, keys, solos) = match self.mark_of(hide, solo, current)? {
            Mark::Current => {
                let mark = cells::current_mark(accent);
                return Some(div().relative().flex().items_center().child(mark).child(hit_target(&self.hits, key)).into_any_element());
            }
            Mark::Soloed(keys) => (Icon::Solo, accent, true, "Stop soloing", keys, true),
            Mark::Hidden(keys) => (Icon::EyeOff, muted, true, "Show in the graph", keys, false),
            Mark::Eye(keys) => (Icon::Eye, muted, false, "Hide from the graph", keys, false),
        };
        let mark = button::revealed(&self.hits, key, icon.svg(ICON, colour), px(MARK), row_group(list), shown, cx)
            .tooltip(Tooltip::text(tip))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                if solos { this.toggle_solo(&keys, cx) } else { this.toggle_hidden(&keys, cx) }
            }));
        Some(mark.into_any_element())
    }

    /// A row of `list`'s path in the panel's whole tree.
    pub(super) fn whole(&self, list: &str, path: &str) -> String {
        match self.overlay().filter(|_| list == OVERLAY) {
            Some(section) => format!("{}/{path}", section.key()),
            None => path.to_string(),
        }
    }

    /// A ref row's content, taking its right-click and its double-click.
    pub(super) fn interactive(&self, list: &str, path: &SharedString, content: gpui::Div, cx: &mut Context<Self>) -> AnyElement {
        let whole = self.whole(list, path);
        let (menu, double) = (whole.clone(), whole);
        content
            .id(SharedString::from(format!("{list}-ref-{path}")))
            .on_mouse_down(MouseButton::Right, cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                this.open_menu(&menu, e.position, cx);
            }))
            .on_click(cx.listener(move |this, e: &ClickEvent, _, cx| {
                if e.click_count() == 2 {
                    this.double_clicked(&double, cx);
                }
            }))
            .into_any_element()
    }

    /// A hidden ref shown, its commit selected once the graph walks it again; the tab checks it out.
    fn double_clicked(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some(entry) = self.entry(path).cloned() else { return };
        if let Some(key) = entry.hide_key().filter(|k| self.walk.hidden.contains(k)) {
            self.toggle_hidden(&[key], cx);
            if let Some(sha) = entry.commit() {
                cx.emit(RefEvent::Shown(SharedString::from(sha.to_string())));
            }
        }
        cx.emit(RefEvent::Checkout(path.to_string()));
    }

    /// The menu of the ref or folder at `path`: a stash's writes, Solo or Stop soloing, then Hide or Show.
    pub(crate) fn menu_items(&self, path: &str) -> Vec<MenuItem<RefAction>> {
        let (hide, solo) = match self.entry(path) {
            Some(entry) => (entry.hide_key().into_iter().collect(), entry.solo_key().into_iter().collect()),
            None => self.tree.aggregate(path).map(|k| (k.hide, k.solo)).unwrap_or_default(),
        };
        let mut items: Vec<MenuItem<RefAction>> = match self.entry(path) {
            Some(Entry::Stash { sha, label, .. }) => {
                let of = StashRef { sha: sha.clone(), label: label.clone() };
                stash::menu_items(&of, RefAction::Stash)
            }
            _ => Vec::new(),
        };
        let stash_items = items.len();
        if !solo.is_empty() {
            let words = if self.all_soloed(&solo) { "Stop soloing" } else { "Solo" };
            items.push(item("ref-solo", words, RefAction::ToggleSolo(solo)));
        }
        if !hide.is_empty() {
            let words = if self.all_hidden(&hide) { "Show" } else { "Hide" };
            let hide = item("ref-hide", words, RefAction::ToggleHidden(hide));
            items.push(if stash_items > 0 { hide.under("Graph") } else { hide });
        }
        items
    }

    fn open_menu(&mut self, path: &str, position: Point<Pixels>, cx: &mut Context<Self>) {
        if matches!(self.entry(path), Some(Entry::Branch { .. } | Entry::Remote { .. } | Entry::Tag { .. } | Entry::Detached { .. } | Entry::Worktree { .. })) || super::remote_folder(path).is_some() {
            return cx.emit(RefEvent::Menu(path.to_string(), position));
        }
        let items = self.menu_items(path);
        if items.is_empty() {
            return;
        }
        let title = path.split_once('/').map_or(path, |(_, rest)| rest).to_string();
        self.menu = Some(ContextMenu { title, position, items });
        cx.notify();
    }

    fn pick(&mut self, action: RefAction, _: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        self.act(action, cx);
    }

    /// What a menu item of the panel's own does, from its menu or a ref menu of the tab's.
    pub(crate) fn act(&mut self, action: RefAction, cx: &mut Context<Self>) {
        match action {
            RefAction::ToggleSolo(keys) => self.toggle_solo(&keys, cx),
            RefAction::ToggleHidden(keys) => self.toggle_hidden(&keys, cx),
            RefAction::Stash(act, stash) => cx.emit(RefEvent::Stash(act, stash)),
            RefAction::Maximize(section) => self.maximize(section, cx),
        }
        cx.notify();
    }

    /// A section header's menu: Hide all or Show all, and Maximize this section, dimmed while it is the only one open.
    pub(crate) fn section_items(&self, section: Section) -> Vec<MenuItem<RefAction>> {
        let mut items = Vec::new();
        let hide = self.tree.aggregate(section.key()).map(|k| k.hide).unwrap_or_default();
        let hidden: Vec<String> = hide.iter().filter(|k| self.walk.hidden.contains(*k)).cloned().collect();
        let shown: Vec<String> = hide.iter().filter(|k| !self.walk.hidden.contains(*k)).cloned().collect();
        if section.hides() && !shown.is_empty() {
            items.push(item("section-hide-all", "Hide all", RefAction::ToggleHidden(shown)));
        }
        if section.hides() && !hidden.is_empty() {
            items.push(item("section-show-all", "Show all", RefAction::ToggleHidden(hidden)));
        }
        if self.tree.is_open(section.key()) {
            let others = Section::ALL.into_iter().filter(|s| *s != section && self.tree.shows(*s) && self.tree.is_open(s.key())).count();
            let maximize = item("section-maximize", "Maximize this section", RefAction::Maximize(section));
            let maximize = if others == 0 { maximize.dimmed("It is the only section open") } else { maximize };
            items.push(if items.is_empty() { maximize } else { maximize.rule() });
        }
        items
    }

    pub(super) fn open_section_menu(&mut self, section: Section, position: Point<Pixels>, cx: &mut Context<Self>) {
        let items = self.section_items(section);
        if !items.is_empty() {
            self.menu = Some(ContextMenu { title: section.title().to_string(), position, items });
            cx.notify();
        }
    }

    fn maximize(&mut self, section: Section, cx: &mut Context<Self>) {
        for other in Section::ALL.into_iter().filter(|s| *s != section && self.tree.is_open(s.key())) {
            self.tree.toggle(other.key());
        }
        cx.emit(RefEvent::Layout);
    }

    pub(super) fn render_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.menu.as_ref()?;
        Some(menu.render(&self.hits, |this: &mut RefPanel, action, window, cx| this.pick(action, window, cx), |this: &mut RefPanel, cx| {
            this.menu = None;
            cx.notify();
        }, cx))
    }
}
