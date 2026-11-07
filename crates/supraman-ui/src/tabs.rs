//! The tab strip: one header per tab, the active one marked with an accent line, each with its close
//! button; the owner keeps the tabs themselves and appends its own buttons after them. A tab drags as a
//! [`DraggedTab`]; dropped on another tab of the strip, the owner moves it there. A session row drags as a
//! [`DraggedSession`].

use gpui::{AnyElement, AnyView, App, ClickEvent, Context, Div, MouseButton, MouseDownEvent, SharedString, Svg, Window, div, prelude::*, px};
use theme::ActiveTheme as _;

use crate::button;
use crate::fonts;
use crate::hit::{HitTargets, hit_target};

/// Longer program titles are cut to this many characters, with an ellipsis.
pub const MAX_TITLE: usize = 200;

pub struct TabHeader {
    pub id: u64,
    pub label: AnyElement,
    /// What the dragged tab shows under the cursor.
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

/// The strip, painted under `tab-<id>`, its close button under `close-tab-<id>`. `close_icon` draws the
/// close glyph; a middle click closes too. A tab dropped on another gets `on_move(from, to)`, indices
/// into `tabs`; the target shows a bar on the side the tab lands.
#[allow(clippy::too_many_arguments)]
pub fn tab_strip<V: 'static>(
    hits: &HitTargets,
    tabs: Vec<TabHeader>,
    active: Option<u64>,
    close_icon: impl Fn() -> Svg,
    on_activate: impl Fn(&mut V, u64, &mut Context<V>) + Clone + 'static,
    on_close: impl Fn(&mut V, u64, &mut Context<V>) + Clone + 'static,
    on_move: impl Fn(&mut V, usize, usize, &mut Context<V>) + Clone + 'static,
    cx: &mut Context<V>,
) -> Div {
    let colors = cx.theme().colors().clone();
    let ids: std::rc::Rc<[u64]> = tabs.iter().map(|t| t.id).collect();
    let mut bar = div()
        .flex()
        .flex_none()
        .h(px(32.))
        .items_end()
        .bg(colors.tab_bar_background)
        .border_b_1()
        .border_color(colors.border);
    for (ix, tab) in tabs.into_iter().enumerate() {
        let id = tab.id;
        let is_active = active == Some(id);
        let (close, middle, activate) = (on_close.clone(), on_close.clone(), on_activate.clone());
        let (moved, over_ids, drop_ids) = (on_move.clone(), ids.clone(), ids.clone());
        let drop_border = colors.drop_target_border;
        bar = bar.child(
            div()
                .id(SharedString::from(format!("tab-{id}")))
                .relative()
                .flex()
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
                .child(tab.label)
                .when_some(tab.tooltip, |d, tooltip| d.tooltip(move |window, cx| tooltip(window, cx)))
                .child(button::icon(hits, format!("close-tab-{id}"), close_icon(), px(18.), cx).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        close(this, id, cx);
                    },
                )))
                .child(hit_target(hits, format!("tab-{id}")))
                .on_mouse_down(
                    MouseButton::Middle,
                    cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        middle(this, id, cx);
                    }),
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| activate(this, id, cx)))
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
                })),
        );
    }
    bar
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
    use super::*;

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
