//! The tab strip: one header per tab, the active one marked with an accent line, each with its close
//! button; the owner keeps the tabs themselves and appends its own buttons after the strip's ⌄ list. Tabs share
//! the width evenly down to [`TAB_MIN`], then the strip scrolls sideways (wheel or touchpad, a fade on a cut
//! edge); the active tab is scrolled whole into view whenever it changes or is clicked. A tab drags as a
//! [`DraggedTab`]; dropped on another tab of the strip, the owner moves it there, and a drag near an edge
//! scrolls the strip. A session row drags as a [`DraggedSession`]. A right click on a tab is the owner's `on_menu`.

mod fit;
mod list;

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AnyElement, AnyView, App, ClickEvent, Context, Anchor, Div, DragMoveEvent, Entity, FocusHandle, Hsla, MouseButton, MouseDownEvent, Pixels,
    Point, ScrollHandle, SharedString, Stateful, Subscription, Svg, Window, anchored, canvas, deferred, div, linear_color_stop, linear_gradient, point,
    prelude::*, px,
};
use theme::ActiveTheme as _;

pub use fit::{CLOSE_ON_HOVER_BELOW, EDGE, TAB_MAX, TAB_MIN, cut, edge_scroll, max_scroll, revealed, span, tab_width};
pub use list::{LIST, TabEntry};

use crate::button;
use crate::fonts;
use crate::hit::{HitTargets, hit_target};
use crate::icon::Icon;
use crate::palette::{Palette, PaletteEvent};
use crate::tooltip::Tooltip;

/// Longer program titles are cut to this many characters, with an ellipsis.
pub const MAX_TITLE: usize = 200;
/// The strip's viewport: its element id, its hit key, and the key its state is kept under.
pub const STRIP: &str = "tab-strip";
/// The ⌄ that lists every tab.
pub const LIST_BUTTON: &str = "tab-strip-list";
const FADE: Pixels = px(28.);

pub struct TabHeader {
    pub id: u64,
    pub label: AnyElement,
    /// What the dragged tab shows under the cursor, and the ⌄ list's line.
    pub title: SharedString,
    pub tooltip: Option<Box<dyn Fn(&mut Window, &mut App) -> AnyView>>,
}

/// The payload of a tab being dragged, from any strip; other drop targets take it by its `id`.
#[derive(Clone)]
pub struct DraggedTab {
    pub id: u64,
    pub title: SharedString,
}

/// The payload of a session row being dragged, by the session's `session_id`.
#[derive(Clone)]
pub struct DraggedSession {
    pub session_id: String,
    pub title: SharedString,
}

impl Render for DraggedTab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        chip(self.title.clone(), cx)
    }
}

impl Render for DraggedSession {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        chip(self.title.clone(), cx)
    }
}

/// A dragged thing's chip under the pointer: a tab, a session row, a graph pill.
pub fn chip<V>(title: SharedString, cx: &Context<V>) -> Div {
    let colors = cx.theme().colors();
    div()
        .font_family(fonts::UI)
        .text_sm()
        .px_3()
        .py_1()
        .max_w(px(280.))
        .overflow_hidden()
        .whitespace_nowrap()
        .rounded_sm()
        .border_1()
        .border_color(colors.border_focused)
        .bg(colors.tab_active_background)
        .text_color(colors.text)
        .opacity(0.9)
        .child(title)
}

/// What the strip keeps across frames, under [`STRIP`] in its owner's element state.
#[derive(Default)]
struct Strip {
    scroll: ScrollHandle,
    /// The viewport's width as last painted; zero until then.
    room: Rc<Cell<Pixels>>,
    shown_active: Option<u64>,
    reveal: Option<u64>,
    list: Option<Listed>,
    /// A press on ⌄ since its last click.
    pressed: bool,
    /// The press that closed the list was on ⌄: its click must not open it again.
    swallow: bool,
}

struct Listed {
    palette: Entity<Palette<TabEntry>>,
    entries: Vec<TabEntry>,
    restore: Option<FocusHandle>,
    _events: Subscription,
}

#[derive(Clone, Copy)]
struct Fit {
    width: Pixels,
    content: Pixels,
    room: Pixels,
}

impl Strip {
    /// Reveals the active tab when it changed, or a tab asked for; a reveal waits while the room is unmeasured.
    fn settle(&mut self, ids: &[u64], active: Option<u64>, fit: Fit) {
        if active != self.shown_active {
            self.shown_active = active;
            if active.is_some() {
                self.reveal = active;
            }
        }
        if fit.room <= px(0.) {
            return;
        }
        let Some(id) = self.reveal.take() else { return };
        let Some(ix) = ids.iter().position(|&t| t == id) else { return };
        let (start, end) = span(ix, fit.width);
        let to = revealed(-self.scroll.offset().x, fit.room, fit.content, start, end);
        self.scroll.set_offset(point(-to, px(0.)));
    }

    fn close_list(&mut self, window: &mut Window, cx: &mut App) {
        if let Some(listed) = self.list.take()
            && let Some(focus) = listed.restore
        {
            window.focus(&focus, cx);
        }
    }
}

/// The strip: its viewport painted under [`STRIP`] (fades under `tab-strip-fade-left`/`-right`), each tab under
/// `tab-<id>`, its close button under `close-tab-<id>`, the ⌄ under [`LIST_BUTTON`] and its list's rows under
/// `tab-list-row-<id>`. `close_icon` draws the close glyph; a middle click closes too. A tab dropped on another
/// gets `on_move(from, to)`, indices into `tabs`; the target shows a bar on the side the tab lands. A right click
/// on a tab gets `on_menu(id, position)`, the pointer's window position. `active` changing, by whatever means,
/// scrolls that tab into view.
#[allow(clippy::too_many_arguments)]
pub fn tab_strip<V: 'static>(
    hits: &HitTargets,
    tabs: Vec<TabHeader>,
    active: Option<u64>,
    close_icon: impl Fn() -> Svg,
    on_activate: impl Fn(&mut V, u64, &mut Context<V>) + Clone + 'static,
    on_close: impl Fn(&mut V, u64, &mut Context<V>) + Clone + 'static,
    on_move: impl Fn(&mut V, usize, usize, &mut Context<V>) + Clone + 'static,
    on_menu: impl Fn(&mut V, u64, Point<Pixels>, &mut Context<V>) + Clone + 'static,
    window: &mut Window,
    cx: &mut Context<V>,
) -> Div {
    let colors = cx.theme().colors();
    let (bar, border) = (colors.tab_bar_background, colors.border);
    let strip = window.use_keyed_state(STRIP, cx, |_, _| Strip::default());
    let ids: Rc<[u64]> = tabs.iter().map(|t| t.id).collect();
    let entries: Vec<TabEntry> = tabs.iter().map(|t| TabEntry { id: t.id, title: t.title.clone(), active: active == Some(t.id) }).collect();
    let room = strip.read(cx).room.get();
    let width = tab_width(room, tabs.len());
    let fit = Fit { width, content: width * tabs.len() as f32, room };
    strip.update(cx, |s, _| s.settle(&ids, active, fit));
    sync_list(&strip, entries.clone(), cx);
    let mut row = div().id("tab-strip-scroll").size_full().flex().overflow_x_scroll().track_scroll(&strip.read(cx).scroll);
    for (ix, tab) in tabs.into_iter().enumerate() {
        let header = Header { ix, active, width, ids: &ids, strip: &strip };
        let handlers = Handlers { activate: &on_activate, close: &on_close, moved: &on_move, menu: &on_menu };
        row = row.child(header.render(hits, tab, &close_icon, handlers, cx));
    }
    div()
        .flex()
        .flex_none()
        .h(px(32.))
        .items_end()
        .bg(bar)
        .border_b_1()
        .border_color(border)
        .child(viewport(hits, &strip, row, fit, bar, cx))
        .child(list_button(hits, entries, &strip, on_activate, cx))
}

/// The tabs' row clipped to the room left beside ⌄: it measures that room, fades a cut edge, and scrolls while a tab is
/// dragged near an edge.
fn viewport<V>(hits: &HitTargets, strip: &Entity<Strip>, row: Stateful<Div>, fit: Fit, bg: Hsla, cx: &Context<V>) -> Stateful<Div> {
    let (scroll, measured) = {
        let s = strip.read(cx);
        (s.scroll.clone(), s.room.clone())
    };
    let (cut_left, cut_right) = cut(-scroll.offset().x, fit.room, fit.content);
    let fade = |left: bool| {
        let side = if left { "left" } else { "right" };
        div()
            .absolute()
            .top_0()
            .bottom_0()
            .w(FADE)
            .when(left, |d| d.left_0())
            .when(!left, |d| d.right_0())
            .bg(linear_gradient(if left { 90. } else { 270. }, linear_color_stop(bg, 0.), linear_color_stop(bg.opacity(0.), 1.)))
            .child(hit_target(hits, format!("{STRIP}-fade-{side}")))
    };
    let dragged = strip.clone();
    div()
        .id(STRIP)
        .relative()
        .flex_1()
        .min_w_0()
        .h_full()
        .child(row)
        .child(hit_target(hits, STRIP.into()))
        .child(
            canvas(
                move |bounds, window, _| {
                    if measured.get() != bounds.size.width {
                        measured.set(bounds.size.width);
                        window.refresh();
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .size_full(),
        )
        .when(cut_left, |d| d.child(fade(true)))
        .when(cut_right, |d| d.child(fade(false)))
        .on_drag_move(move |e: &DragMoveEvent<DraggedTab>, _, cx| {
            let (b, at) = (e.bounds, e.event.position);
            if at.y < b.top() || at.y > b.bottom() {
                return;
            }
            let by = edge_scroll(at.x - b.left(), b.size.width);
            if by != px(0.) {
                dragged.update(cx, |s, cx| {
                    let to = (-s.scroll.offset().x + by).clamp(px(0.), max_scroll(fit.content, b.size.width));
                    s.scroll.set_offset(point(-to, px(0.)));
                    cx.notify();
                });
            }
        })
}

struct Handlers<'a, A, C, M, U> {
    activate: &'a A,
    close: &'a C,
    moved: &'a M,
    menu: &'a U,
}

struct Header<'a> {
    ix: usize,
    active: Option<u64>,
    width: Pixels,
    ids: &'a Rc<[u64]>,
    strip: &'a Entity<Strip>,
}

impl Header<'_> {
    fn render<V: 'static, A, C, M, U>(self, hits: &HitTargets, tab: TabHeader, close_icon: &impl Fn() -> Svg, on: Handlers<'_, A, C, M, U>, cx: &mut Context<V>) -> AnyElement
    where
        A: Fn(&mut V, u64, &mut Context<V>) + Clone + 'static,
        C: Fn(&mut V, u64, &mut Context<V>) + Clone + 'static,
        M: Fn(&mut V, usize, usize, &mut Context<V>) + Clone + 'static,
        U: Fn(&mut V, u64, Point<Pixels>, &mut Context<V>) + Clone + 'static,
    {
        let colors = cx.theme().colors().clone();
        let (id, ix) = (tab.id, self.ix);
        let is_active = self.active == Some(id);
        let group = SharedString::from(format!("tab-{id}"));
        let (close, middle, activate, menu) = (on.close.clone(), on.close.clone(), on.activate.clone(), on.menu.clone());
        let (moved, over_ids, drop_ids) = (on.moved.clone(), self.ids.clone(), self.ids.clone());
        let drop_border = colors.drop_target_border;
        let clicked = self.strip.clone();
        let close_button = button::icon(hits, format!("close-tab-{id}"), close_icon(), px(18.), cx).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            cx.stop_propagation();
            close(this, id, cx);
        }));
        let close_button = button::reveal(close_button, group.clone(), is_active || self.width >= CLOSE_ON_HOVER_BELOW);
        div()
            .id(group.clone())
            .group(group)
            .relative()
            .flex()
            .flex_none()
            .w(self.width)
            .items_center()
            .gap_2()
            .h_full()
            .px_3()
            .text_sm()
            .border_r_1()
            .border_color(colors.border)
            .cursor_pointer()
            .bg(if is_active { colors.tab_active_background } else { colors.tab_inactive_background })
            .when(!is_active, |d| d.text_color(colors.text_muted).hover(|s| s.bg(colors.ghost_element_hover).text_color(colors.text)))
            .when(is_active, |d| d.child(div().absolute().top_0().left_0().right_0().h(px(2.)).bg(colors.border_focused)))
            .child(div().flex().flex_1().min_w_0().overflow_hidden().child(tab.label))
            .when_some(tab.tooltip, |d, tooltip| d.tooltip(move |window, cx| tooltip(window, cx)))
            .child(close_button)
            .child(hit_target(hits, format!("tab-{id}")))
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    middle(this, id, cx);
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    menu(this, id, e.position, cx);
                }),
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                clicked.update(cx, |s, _| s.reveal = Some(id));
                activate(this, id, cx);
                cx.notify();
            }))
            .on_drag(DraggedTab { id, title: tab.title }, |tab, _, _, cx| cx.new(|_| tab.clone()))
            .drag_over::<DraggedTab>(move |style, dragged: &DraggedTab, _, _| {
                match over_ids.iter().position(|&d| d == dragged.id).and_then(|from| landing(from, ix)) {
                    Some(Side::Before) => style.border_0().border_l_2().border_color(drop_border),
                    Some(Side::After) => style.border_0().border_r_2().border_color(drop_border),
                    None => style,
                }
            })
            .on_drop(cx.listener(move |this, dragged: &DraggedTab, _, cx| {
                if let Some(from) = drop_ids.iter().position(|&d| d == dragged.id)
                    && from != ix
                {
                    moved(this, from, ix, cx);
                }
            }))
            .into_any_element()
    }
}

/// The open list follows the tabs as they change.
fn sync_list<V>(strip: &Entity<Strip>, entries: Vec<TabEntry>, cx: &mut Context<V>) {
    let Some(palette) = strip.read(cx).list.as_ref().filter(|l| l.entries != entries).map(|l| l.palette.clone()) else { return };
    strip.update(cx, |s, _| {
        if let Some(listed) = s.list.as_mut() {
            listed.entries = entries.clone();
        }
    });
    palette.update(cx, |p, cx| p.set_entries(entries, cx));
}

/// The ⌄ at the strip's right end and, while open, the list under it.
fn list_button<V: 'static>(
    hits: &HitTargets,
    entries: Vec<TabEntry>,
    strip: &Entity<Strip>,
    on_activate: impl Fn(&mut V, u64, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> AnyElement {
    let colors = cx.theme().colors().clone();
    let open = strip.read(cx).list.as_ref().map(|l| l.palette.clone());
    let (toggled, list_hits, pressed) = (strip.clone(), hits.clone(), strip.clone());
    button::ghost(hits, LIST_BUTTON, cx)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .h_full()
        .w(px(28.))
        .border_l_1()
        .border_color(colors.border)
        .when(open.is_some(), |d| d.bg(colors.ghost_element_selected))
        .child(Icon::ChevronDown.svg(14., colors.text_muted).group_hover(LIST_BUTTON, |s| s.text_color(colors.text)))
        .tooltip(Tooltip::text("All tabs"))
        .capture_any_mouse_down(move |e: &MouseDownEvent, _, cx| {
            if e.button == MouseButton::Left {
                pressed.update(cx, |s, _| s.pressed = true);
            }
        })
        .on_click(cx.listener(move |_, _: &ClickEvent, window, cx| {
            toggle_list(&toggled, &list_hits, entries.clone(), on_activate.clone(), window, cx);
        }))
        .when_some(open, |d, palette| {
            d.child(
                div()
                    .absolute()
                    .top_full()
                    .right_0()
                    .child(deferred(anchored().anchor(Anchor::TopRight).snap_to_window_with_margin(px(8.)).child(palette)).with_priority(crate::menu::FLOAT)),
            )
        })
        .into_any_element()
}

fn toggle_list<V: 'static>(
    strip: &Entity<Strip>,
    hits: &HitTargets,
    entries: Vec<TabEntry>,
    on_activate: impl Fn(&mut V, u64, &mut Context<V>) + 'static,
    window: &mut Window,
    cx: &mut Context<V>,
) {
    let (swallow, open) = {
        let s = strip.read(cx);
        (s.swallow, s.list.is_some())
    };
    strip.update(cx, |s, _| s.pressed = false);
    if swallow || open {
        strip.update(cx, |s, cx| {
            s.swallow = false;
            s.close_list(window, cx);
            cx.notify();
        });
        return;
    }
    let (paint, palette_hits, shown) = (list::paint(hits.clone()), hits.clone(), entries.clone());
    let palette = cx.new(|cx| {
        let mut palette = Palette::new(LIST, palette_hits, shown, paint, window, cx);
        palette.set_top("Filter tabs…", "TABS", cx);
        palette
    });
    let listed = strip.clone();
    let events = cx.subscribe_in(&palette, window, move |this: &mut V, _, event: &PaletteEvent<TabEntry>, window, cx| match event {
        PaletteEvent::Open(entry) => {
            let id = entry.id;
            listed.update(cx, |s, cx| {
                s.close_list(window, cx);
                s.reveal = Some(id);
                cx.notify();
            });
            on_activate(this, id, cx);
            cx.notify();
        }
        PaletteEvent::Close => {
            listed.update(cx, |s, cx| {
                s.swallow = std::mem::take(&mut s.pressed);
                s.close_list(window, cx);
                cx.notify();
            });
        }
        PaletteEvent::Query(_) => {}
    });
    let restore = window.focused(cx);
    palette.update(cx, |p, cx| p.focus(window, cx));
    strip.update(cx, |s, cx| {
        s.list = Some(Listed { palette, entries, restore, _events: events });
        cx.notify();
    });
}

#[derive(Debug, PartialEq)]
enum Side {
    Before,
    After,
}

/// Where the tab at `from`, dropped on the one at `to`, lands: before it when it comes from its right.
fn landing(from: usize, to: usize) -> Option<Side> {
    match from.cmp(&to) {
        std::cmp::Ordering::Greater => Some(Side::Before),
        std::cmp::Ordering::Less => Some(Side::After),
        std::cmp::Ordering::Equal => None,
    }
}

/// `items` with the one at `from` taken out and put back at `to`; out of range changes nothing.
pub fn reorder<T>(items: &mut Vec<T>, from: usize, to: usize) {
    if from < items.len() && to < items.len() {
        let item = items.remove(from);
        items.insert(to, item);
    }
}

/// The index `step` tabs away from `at`, wrapping; with none active, the first (forward) or last.
pub fn cycled(len: usize, at: Option<usize>, step: isize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let at = at.map_or(if step > 0 { -1 } else { 0 }, |at| at as isize);
    Some((at + step).rem_euclid(len as isize) as usize)
}

/// The browser's Ctrl+1..8 (`n` 0..=7) and Ctrl+9 (`n` `None`, the last tab).
pub fn nth(len: usize, n: Option<usize>) -> Option<usize> {
    match n {
        Some(n) => (n < len).then_some(n),
        None => len.checked_sub(1),
    }
}

/// Where the tab at `at` moves `step` places away; none past either end.
pub fn moved(len: usize, at: usize, step: isize) -> Option<usize> {
    let to = at.checked_add_signed(step)?;
    (at < len && to < len && to != at).then_some(to)
}

/// `raw` (an OSC 0/2 title) as a tab label: control and bidi-format characters dropped, runs of
/// whitespace as one space, trimmed, at most [`MAX_TITLE`] characters. Nothing left reads as no title.
pub fn sanitize(raw: &str) -> Option<String> {
    let mut out = String::new();
    let mut space = false;
    let mut count = 0;
    for c in raw.chars() {
        if c.is_whitespace() {
            space = !out.is_empty();
            continue;
        }
        if c.is_control() || is_format(c) {
            continue;
        }
        if count == MAX_TITLE {
            out.push('…');
            break;
        }
        if space {
            out.push(' ');
            count += 1;
            space = false;
            if count == MAX_TITLE {
                out.push('…');
                break;
            }
        }
        out.push(c);
        count += 1;
    }
    (!out.is_empty()).then_some(out)
}

/// Bidi embeddings, overrides and isolates, zero-width and BOM characters: they would reorder or hide
/// what the tab shows.
fn is_format(c: char) -> bool {
    matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}')
}

#[cfg(test)]
mod tests {
    use gpui::{Modifiers, TestAppContext, VisualTestContext, svg};

    use super::*;

    struct Tabs {
        hits: HitTargets,
        active: Option<u64>,
        menu: Option<(u64, Point<Pixels>)>,
    }

    impl Render for Tabs {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            self.hits.borrow_mut().clear();
            let tabs = (1..=3)
                .map(|id| TabHeader { id, label: div().child(format!("tab {id}")).into_any_element(), title: format!("tab {id}").into(), tooltip: None })
                .collect();
            let strip = tab_strip(&self.hits, tabs, self.active, svg, |this: &mut Tabs, id, _| this.active = Some(id), |_, _, _| {}, |_, _, _, _| {}, |this: &mut Tabs, id, at, _| this.menu = Some((id, at)), window, cx);
            div().size_full().child(strip)
        }
    }

    fn strip(cx: &mut TestAppContext) -> (Entity<Tabs>, &mut VisualTestContext) {
        cx.update(|cx| {
            crate::init_globals(cx, "test");
            crate::text_input::bind_keys(cx);
        });
        let (view, cx) = cx.add_window_view(|_, _| Tabs { hits: HitTargets::default(), active: Some(1), menu: None });
        cx.run_until_parked();
        (view, cx)
    }

    fn list_open(view: &Entity<Tabs>, cx: &mut VisualTestContext) -> bool {
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        view.read_with(cx, |v, _| v.hits.borrow().contains_key(LIST))
    }

    fn list_button_at(view: &Entity<Tabs>, cx: &mut VisualTestContext) -> gpui::Point<Pixels> {
        view.read_with(cx, |v, _| v.hits.borrow().get(LIST_BUTTON).copied()).expect("the ⌄ paints").bounds.center()
    }

    fn click_list_button(view: &Entity<Tabs>, cx: &mut VisualTestContext) {
        let at = list_button_at(view, cx);
        cx.simulate_click(at, Modifiers::none());
        cx.run_until_parked();
    }

    #[gpui::test]
    fn esc_with_the_pointer_on_the_list_button_leaves_its_next_click_opening_the_list(cx: &mut TestAppContext) {
        let (view, cx) = strip(cx);
        click_list_button(&view, cx);
        assert!(list_open(&view, cx), "a click on ⌄ opens the list");
        let at = list_button_at(&view, cx);
        cx.simulate_mouse_move(at, None, Modifiers::none());
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        assert!(!list_open(&view, cx), "Esc closes the list");
        click_list_button(&view, cx);
        assert!(list_open(&view, cx), "the next click on ⌄ opens the list again");
    }

    #[gpui::test]
    fn a_click_on_the_list_button_while_open_closes_the_list(cx: &mut TestAppContext) {
        let (view, cx) = strip(cx);
        click_list_button(&view, cx);
        assert!(list_open(&view, cx));
        click_list_button(&view, cx);
        assert!(!list_open(&view, cx), "the press that closed the list does not open it again on its click");
        click_list_button(&view, cx);
        assert!(list_open(&view, cx));
    }

    #[gpui::test]
    fn a_right_click_on_a_tab_asks_its_owner_for_the_menu_at_the_pointer_and_activates_nothing(cx: &mut TestAppContext) {
        let (view, cx) = strip(cx);
        let tab = view.read_with(cx, |v, _| v.hits.borrow().get("tab-2").copied()).expect("tab 2 paints").bounds;
        let at = tab.center();
        cx.simulate_mouse_down(at, MouseButton::Right, Modifiers::none());
        cx.simulate_mouse_up(at, MouseButton::Right, Modifiers::none());
        cx.run_until_parked();
        assert_eq!(view.read_with(cx, |v, _| (v.menu, v.active)), (Some((2, at)), Some(1)));
    }

    #[test]
    fn tabs_cycle_wrapping_both_ways() {
        assert_eq!(cycled(3, Some(0), 1), Some(1));
        assert_eq!(cycled(3, Some(2), 1), Some(0), "wraps forward");
        assert_eq!(cycled(3, Some(0), -1), Some(2), "wraps back");
        assert_eq!(cycled(1, Some(0), 1), Some(0));
        assert_eq!(cycled(3, None, 1), Some(0), "none active: the first");
        assert_eq!(cycled(3, None, -1), Some(2), "none active: the last");
        assert_eq!(cycled(0, None, 1), None);
    }

    #[test]
    fn nth_is_the_browsers_and_moved_stops_at_the_ends() {
        assert_eq!(nth(3, Some(0)), Some(0));
        assert_eq!(nth(3, Some(2)), Some(2));
        assert_eq!(nth(3, Some(3)), None, "no fourth tab");
        assert_eq!(nth(3, None), Some(2), "Ctrl+9: the last");
        assert_eq!(nth(9, Some(7)), Some(7));
        assert_eq!(nth(12, None), Some(11));
        assert_eq!(nth(0, None), None);
        assert_eq!(moved(3, 1, -1), Some(0));
        assert_eq!(moved(3, 1, 1), Some(2));
        assert_eq!(moved(3, 0, -1), None, "the first stays first");
        assert_eq!(moved(3, 2, 1), None, "the last stays last");
        assert_eq!(moved(1, 0, 1), None);
        assert_eq!(moved(3, 5, -1), None);
    }

    #[test]
    fn a_dropped_tab_takes_the_target_place() {
        let mut v = vec!['a', 'b', 'c', 'd'];
        reorder(&mut v, 1, 0);
        assert_eq!(v, ['b', 'a', 'c', 'd'], "second before the first");
        reorder(&mut v, 0, 3);
        assert_eq!(v, ['a', 'c', 'd', 'b'], "first to the end");
        reorder(&mut v, 2, 2);
        assert_eq!(v, ['a', 'c', 'd', 'b'], "onto itself");
        reorder(&mut v, 4, 0);
        reorder(&mut v, 0, 4);
        assert_eq!(v, ['a', 'c', 'd', 'b'], "out of range");
    }

    #[test]
    fn the_marker_is_on_the_side_the_tab_lands() {
        assert_eq!(landing(1, 0), Some(Side::Before));
        assert_eq!(landing(0, 3), Some(Side::After));
        assert_eq!(landing(2, 2), None);
    }

    #[test]
    fn control_characters_are_dropped_and_whitespace_folded() {
        assert_eq!(sanitize("✳ fix the tray").as_deref(), Some("✳ fix the tray"));
        assert_eq!(sanitize("  a\x07b\x1b[31m  c\r\n d\t e ").as_deref(), Some("ab[31m c d e"));
        assert_eq!(sanitize("x\u{9b}y\u{90}z\u{85}w").as_deref(), Some("xyz w"), "C1 controls; NEL is a line break");
        assert_eq!(sanitize("a\u{202E}cba\u{202C}\u{200B}!").as_deref(), Some("acba!"), "bidi override, zero width");
    }

    #[test]
    fn an_empty_or_blank_title_is_none() {
        assert_eq!(sanitize(""), None);
        assert_eq!(sanitize(" \t\r\n\x1b\u{FEFF}"), None);
    }

    #[test]
    fn a_long_title_is_cut_with_an_ellipsis() {
        let long = "é".repeat(MAX_TITLE + 50);
        let cut = sanitize(&long).unwrap();
        assert_eq!(cut.chars().count(), MAX_TITLE + 1);
        assert!(cut.ends_with('…'));
        let exact = "x".repeat(MAX_TITLE);
        assert_eq!(sanitize(&exact).as_deref(), Some(exact.as_str()), "exactly the cap is not cut");
        let spaced = format!("{} y", "x".repeat(MAX_TITLE - 1));
        assert_eq!(sanitize(&spaced).unwrap().chars().count(), MAX_TITLE + 1, "a space counts");
    }
}
