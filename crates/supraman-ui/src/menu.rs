//! A right-click menu of items carrying the owner's action type, anchored to the element it belongs to
//! ([`crate::popup`]). One menu at a time; a click outside it, Esc or choosing an item closes it.

use std::{
    cell::{Cell, OnceCell, RefCell},
    rc::Rc,
    time::Instant,
};

use gpui::{
    AnyElement, App, Bounds, ClickEvent, Context, Div, FocusHandle, KeyDownEvent, MouseDownEvent, MouseMoveEvent, Pixels, Point, Role, ScrollHandle, SharedString, Stateful, Toggled, Window,
    anchored, canvas, deferred, div, point, prelude::*, px,
};
use theme::{ActiveTheme as _, ThemeColors};

use crate::hit::{HitTargets, hit_target};
use crate::icon::Icon;
use crate::label::{Label, LabelMark};
use crate::popup::Popup;
use crate::state::{self, Fills, State};
use crate::tokens::{self, Layer};

mod nav;

/// A menu's least width.
const MIN_WIDTH: Pixels = px(180.);
/// How long the pointer rests on an item before its submenu opens, or another's closes.
const SUBMENU_DELAY: std::time::Duration = tokens::motion::NORMAL;
/// How long typed letters add up to one type-ahead.
const TYPED_FOR: std::time::Duration = std::time::Duration::from_millis(800);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem<A> {
    /// Unique within the menu; painted under `menu-<key>`.
    pub key: SharedString,
    pub label: String,
    /// What a pick runs; a submenu's item never runs it.
    pub action: A,
    /// Starts a section: a rule, then this caption (none when empty), above the item.
    pub section: Option<String>,
    /// Why it cannot act now: listed dimmed with this as its tooltip, its click a no-op.
    pub dimmed: Option<String>,
    /// A tick box before the label, ticked or not.
    pub ticked: Option<bool>,
    pub icon: Option<Icon>,
    /// Right-aligned keys that do the same, in words (`Ctrl+Z`).
    pub shortcut: Option<SharedString>,
    /// A submenu's items: it opens on hover, Right or a pick.
    pub submenu: Vec<MenuItem<A>>,
}

pub fn item<A>(key: impl Into<SharedString>, label: impl Into<String>, action: A) -> MenuItem<A> {
    MenuItem { key: key.into(), label: label.into(), action, section: None, dimmed: None, ticked: None, icon: None, shortcut: None, submenu: Vec::new() }
}

impl<A: Clone> MenuItem<A> {
    /// An item opening `items` beside it; `None` without any.
    pub fn submenu(key: impl Into<SharedString>, label: impl Into<String>, items: Vec<MenuItem<A>>) -> Option<Self> {
        let action = items.first()?.action.clone();
        Some(MenuItem { submenu: items, ..item(key, label, action) })
    }
}

impl<A> MenuItem<A> {
    pub fn under(self, caption: impl Into<String>) -> Self {
        Self { section: Some(caption.into()), ..self }
    }

    /// Starts a group: a rule above the item.
    pub fn rule(self) -> Self {
        self.under("")
    }

    pub fn dimmed(self, why: impl Into<String>) -> Self {
        Self { dimmed: Some(why.into()), ..self }
    }

    /// A tick box before the label ([`crate::button::tick`]).
    pub fn ticked(self, on: bool) -> Self {
        Self { ticked: Some(on), ..self }
    }

    pub fn icon(self, icon: Icon) -> Self {
        Self { icon: Some(icon), ..self }
    }

    pub fn shortcut(self, keys: impl Into<SharedString>) -> Self {
        Self { shortcut: Some(keys.into()), ..self }
    }

    fn acts(&self) -> bool {
        self.dimmed.is_none()
    }
}

/// The highlight, the submenu open and the type-ahead, shared by the menu's handlers.
#[derive(Default)]
struct Nav {
    highlighted: Option<usize>,
    sub: Option<Sub>,
    typed: String,
    typed_at: Option<Instant>,
    scroll: ScrollHandle,
    viewport: Option<Pixels>,
    /// Bumped by each pointer move onto an item, so only the last one's delay acts.
    epoch: u64,
}

struct Sub {
    parent: usize,
    highlighted: Option<usize>,
    scroll: ScrollHandle,
    bounds: Option<Bounds<Pixels>>,
}

/// Which of the menu's two levels a handler acts on.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Level {
    Main,
    Sub,
}

/// While shown it holds the focus, given back to what held it when it closes ([`crate::focus::popup`]); Up, Down,
/// Home and End move its highlight over the items that act, Enter or Space picks it, Right and Left open and close
/// a submenu, letters jump to an item, Esc closes it.
pub struct ContextMenu<A> {
    pub title: String,
    pub position: Point<Pixels>,
    pub items: Vec<MenuItem<A>>,
    popup: Popup,
    focus: OnceCell<FocusHandle>,
    asked: Cell<bool>,
    nav: Rc<RefCell<Nav>>,
}

impl<A> ContextMenu<A> {
    pub fn new(title: impl Into<String>, position: Point<Pixels>, items: Vec<MenuItem<A>>) -> Self {
        Self { title: title.into(), position, items, popup: Popup::open(None, Layer::Menu), focus: OnceCell::new(), asked: Cell::new(false), nav: Rc::default() }
    }

    /// Belonging to the element painted under `anchor` (a row's or a pill's hit key), which paints open while it is shown.
    pub fn anchored(self, anchor: impl Into<SharedString>) -> Self {
        Self { popup: Popup::open(Some(anchor.into()), Layer::Menu), ..self }
    }

    pub fn anchor(&self) -> Option<&SharedString> {
        self.popup.anchor()
    }

    /// The item the keys or the pointer highlight, and the submenu's.
    pub fn highlighted(&self) -> (Option<usize>, Option<usize>) {
        let nav = self.nav.borrow();
        (nav.highlighted, nav.sub.as_ref().and_then(|s| s.highlighted))
    }

    /// The item whose submenu is open.
    pub fn submenu_open(&self) -> Option<usize> {
        self.nav.borrow().sub.as_ref().map(|s| s.parent)
    }

    /// The gallery's highlighted item, its submenu open when it has one.
    #[cfg(any(test, feature = "gallery"))]
    pub(crate) fn show_highlighted(&self, ix: usize) {
        let mut nav = self.nav.borrow_mut();
        nav.highlighted = Some(ix);
        nav.sub = self.items.get(ix).filter(|it| !it.submenu.is_empty()).map(|it| Sub::new(ix, nav::end(&it.submenu.iter().map(MenuItem::acts).collect::<Vec<_>>(), false)));
    }

    /// Focuses the menu as it first prepaints, before its list, so the frame that first shows it holds the focus; records it in the window's registry.
    fn take_focus(&self, cx: &mut App) -> impl IntoElement + use<A> {
        let focus = self.focus.get_or_init(|| cx.focus_handle()).clone();
        let ask = !self.asked.replace(true);
        let (popup, nav) = (self.popup.clone(), self.nav.clone());
        canvas(
            move |_, window, cx| {
                popup.shown(window, cx);
                let height = window.viewport_size().height;
                if nav.borrow_mut().viewport.replace(height) != Some(height) {
                    window.refresh();
                }
                if ask {
                    window.focus(&focus, cx)
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_0()
    }
}

/// What the menu's handlers share: the owner's pick and dismiss.
struct Acts<V, A> {
    pick: Rc<dyn Fn(&mut V, A, &mut Window, &mut Context<V>)>,
    dismiss: Rc<dyn Fn(&mut V, &mut Context<V>)>,
}

impl<V, A> Clone for Acts<V, A> {
    fn clone(&self) -> Self {
        Self { pick: self.pick.clone(), dismiss: self.dismiss.clone() }
    }
}

impl<A: Clone + 'static> ContextMenu<A> {
    /// Over every layer but the dialog ([`tokens::Layer::Menu`]), kept in the window, under `menu`; `on_pick` and `on_dismiss` (a click outside, Esc) must close it.
    pub fn render<V: 'static>(
        &self,
        hits: &HitTargets,
        on_pick: impl Fn(&mut V, A, &mut Window, &mut Context<V>) + Clone + 'static,
        on_dismiss: impl Fn(&mut V, &mut Context<V>) + 'static,
        cx: &mut Context<V>,
    ) -> AnyElement {
        let acts = Acts { pick: Rc::new(on_pick), dismiss: Rc::new(on_dismiss) };
        if self.popup.displaced() {
            let dismiss = acts.dismiss.clone();
            cx.spawn(async move |this, cx| this.update(cx, |this, cx| dismiss(this, cx)).ok()).detach();
            return div().into_any_element();
        }
        let colors = cx.theme().colors().clone();
        let taken = self.take_focus(cx);
        let focus = self.focus.get().cloned().expect("take_focus made it");
        let items = Rc::new(self.items.clone());
        let (nav, keys_items, keys_acts) = (self.nav.clone(), items.clone(), acts.clone());
        let main = panel(hits, "context-menu", Some(&self.title), &self.items, Level::Main, &self.nav, &items, &acts, &colors, cx)
            .track_focus(&focus)
            .capture_key_down(cx.listener(move |this, e: &KeyDownEvent, window, cx| {
                if key(&nav, &keys_items, &keys_acts, e, this, window, cx) {
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .on_mouse_down_out({
                let (nav, dismiss) = (self.nav.clone(), acts.dismiss.clone());
                cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                    let in_sub = nav.borrow().sub.as_ref().and_then(|s| s.bounds).is_some_and(|b| b.contains(&e.position));
                    if !in_sub {
                        dismiss(this, cx);
                    }
                })
            })
            .child(hit_target(hits, "menu".into()));
        let main = anchored().position(self.position).snap_to_window_with_margin(tokens::space::L).child(main);
        let sub = self.render_sub(hits, &items, &acts, &colors, cx);
        let menu = div().child(crate::focus::popup([focus.clone()])).child(taken).child(main).children(sub);
        deferred(menu).with_priority(Layer::Menu.priority()).into_any_element()
    }

    /// The open submenu beside its item, once the item's bounds are known.
    fn render_sub<V: 'static>(&self, hits: &HitTargets, items: &Rc<Vec<MenuItem<A>>>, acts: &Acts<V, A>, colors: &ThemeColors, cx: &mut Context<V>) -> Option<AnyElement> {
        let parent = self.nav.borrow().sub.as_ref()?.parent;
        let Some(at) = self.nav.borrow().scroll.bounds_for_item(parent) else {
            cx.notify();
            return None;
        };
        let children = &items.get(parent)?.submenu;
        let nav = self.nav.clone();
        let bounds = canvas(move |b, _, _| if let Some(s) = nav.borrow_mut().sub.as_mut() { s.bounds = Some(b) }, |_, _, _, _| {}).absolute().size_full();
        let sub = panel(hits, "context-submenu", None, children, Level::Sub, &self.nav, items, acts, colors, cx).child(bounds).child(hit_target(hits, "menu-sub".into()));
        let position = point(at.right(), at.top() - tokens::space::S);
        Some(anchored().position(position).snap_to_window_with_margin(tokens::space::L).child(sub).into_any_element())
    }
}

/// A level's panel: its caption, then its items in a scroll area that keeps the highlighted one in view.
#[allow(clippy::too_many_arguments)]
fn panel<V: 'static, A: Clone + 'static>(
    hits: &HitTargets,
    id: &'static str,
    title: Option<&str>,
    shown: &[MenuItem<A>],
    level: Level,
    nav: &Rc<RefCell<Nav>>,
    items: &Rc<Vec<MenuItem<A>>>,
    acts: &Acts<V, A>,
    colors: &ThemeColors,
    cx: &mut Context<V>,
) -> Stateful<Div> {
    let (highlighted, scroll, viewport) = {
        let n = nav.borrow();
        match level {
            Level::Main => (n.highlighted, n.scroll.clone(), n.viewport),
            Level::Sub => (n.sub.as_ref().and_then(|s| s.highlighted), n.sub.as_ref().map_or_else(ScrollHandle::new, |s| s.scroll.clone()), n.viewport),
        }
    };
    let lead = shown.iter().any(|i| i.icon.is_some() || i.ticked.is_some());
    let rows: Vec<AnyElement> = shown.iter().enumerate().map(|(ix, it)| entry(hits, it, ix, highlighted == Some(ix), lead, level, nav, items, acts, colors, cx)).collect();
    let list = crate::scroll::area(SharedString::from(format!("{id}-items"))).track(&scroll).flex().flex_col().flex_shrink(1.).min_h_0().children(rows);
    div()
        .id(id)
        .role(Role::Menu)
        .relative()
        .flex()
        .flex_col()
        .min_w(MIN_WIDTH)
        .when_some(viewport, |d, h| d.max_h(h - tokens::space::L * 2.))
        .py(tokens::space::S)
        .rounded(tokens::radius::M)
        .border(tokens::stroke::HAIRLINE)
        .border_color(colors.border)
        .bg(colors.elevated_surface_background)
        .shadow_md()
        .text_size(tokens::text::BODY)
        .occlude()
        .children(title.map(|t| caption(t, colors)))
        .child(list)
}

/// One item with the section that starts above it, as one child of the list.
#[allow(clippy::too_many_arguments)]
fn entry<V: 'static, A: Clone + 'static>(
    hits: &HitTargets,
    it: &MenuItem<A>,
    ix: usize,
    highlighted: bool,
    lead: bool,
    level: Level,
    nav: &Rc<RefCell<Nav>>,
    items: &Rc<Vec<MenuItem<A>>>,
    acts: &Acts<V, A>,
    colors: &ThemeColors,
    cx: &mut Context<V>,
) -> AnyElement {
    let key = format!("menu-{}", it.key);
    let look = state::resolve(State { hovered: highlighted, disabled: !it.acts(), ..State::default() }, Fills { hover: Some(colors.element_hover), ..Fills::default() });
    let mark = match (it.ticked, it.icon) {
        (Some(on), _) => Some(LabelMark::Tick(on)),
        (None, Some(icon)) => Some(LabelMark::Svg(icon.svg(tokens::icon::M, colors.text_muted))),
        (None, None) => None,
    };
    let label = Label::new(SharedString::from(it.label.clone())).mark_if(mark).gap(tokens::space::L).flex_1().hit(hits, key.clone());
    let label = if lead { label.slot(crate::button::TICK) } else { label };
    let shortcut = it.shortcut.clone().map(|s| div().flex_none().text_size(tokens::text::SMALL).text_color(colors.text_muted).child(s));
    let chevron = (!it.submenu.is_empty()).then(|| Icon::ChevronRight.svg(tokens::icon::S, colors.text_muted));
    let mut row = div()
        .id(SharedString::from(key.clone()))
        .role(if it.ticked.is_some() { Role::MenuItemCheckBox } else { Role::MenuItem })
        .aria_label(SharedString::from(it.label.clone()))
        .when_some(it.ticked, |d, on| d.aria_toggled(if on { Toggled::True } else { Toggled::False }))
        .when_some(it.shortcut.clone(), |d, s| d.aria_keyshortcuts(s))
        .when(!it.submenu.is_empty(), |d| d.aria_expanded(nav.borrow().sub.as_ref().is_some_and(|s| level == Level::Main && s.parent == ix)))
        .relative()
        .flex()
        .items_center()
        .gap(tokens::space::L)
        .h(tokens::control::MEDIUM.height)
        .px(tokens::space::XL)
        .child(label)
        .children(shortcut)
        .children(chevron)
        .child(hit_target(hits, key));
    row = look.paint(row, None);
    let nav_move = nav.clone();
    let (acts_ix, has_sub) = (it.acts(), !it.submenu.is_empty());
    row = row.on_mouse_move(move |_: &MouseMoveEvent, window, cx| {
        if acts_ix && pointed(&nav_move, level, ix, has_sub, window, cx) {
            window.refresh();
        }
    });
    row = match &it.dimmed {
        Some(why) => row.text_color(colors.text_disabled).tooltip(crate::tooltip::Tooltip::text(why.clone())).on_click(|_, _, cx| cx.stop_propagation()),
        None => {
            let (nav, items, acts) = (nav.clone(), items.clone(), acts.clone());
            row.cursor_pointer().on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                cx.stop_propagation();
                activate(&nav, &items, &acts, level, ix, false, this, window, cx);
                cx.notify();
            }))
        }
    };
    let section = it.section.as_ref().map(|section| {
        div()
            .child(div().my(tokens::space::S).h(tokens::stroke::HAIRLINE).bg(colors.border_variant))
            .when(!section.is_empty(), |d| d.child(caption(section, colors)))
    });
    div().flex().flex_col().children(section).child(row).into_any_element()
}

/// The pointer rests on item `ix` of `level`: it is highlighted, and after [`SUBMENU_DELAY`] its submenu (`has_sub`) opens or another closes.
fn pointed(nav: &Rc<RefCell<Nav>>, level: Level, ix: usize, has_sub: bool, window: &mut Window, cx: &mut App) -> bool {
    let mut n = nav.borrow_mut();
    n.epoch += 1;
    if level == Level::Sub {
        n.highlighted = n.sub.as_ref().map(|s| s.parent);
        return n.sub.as_mut().is_some_and(|sub| sub.highlighted.replace(ix) != Some(ix));
    }
    if n.highlighted == Some(ix) {
        return false;
    }
    n.highlighted = Some(ix);
    let epoch = n.epoch;
    drop(n);
    let nav = nav.clone();
    window
        .spawn(cx, async move |cx| {
            cx.background_executor().timer(SUBMENU_DELAY).await;
            cx.update(|window, _| {
                let mut n = nav.borrow_mut();
                if n.epoch != epoch || n.highlighted != Some(ix) || n.sub.as_ref().is_some_and(|s| s.parent == ix) {
                    return;
                }
                n.sub = has_sub.then(|| Sub::new(ix, None));
                window.refresh();
            })
            .ok();
        })
        .detach();
    true
}

impl Sub {
    fn new(parent: usize, highlighted: Option<usize>) -> Self {
        Self { parent, highlighted, scroll: ScrollHandle::new(), bounds: None }
    }
}

/// The items `level` shows.
fn level_items<A>(nav: &Nav, items: &[MenuItem<A>], level: Level) -> Option<std::ops::Range<usize>> {
    match level {
        Level::Main => Some(0..items.len()),
        Level::Sub => nav.sub.as_ref().and_then(|s| items.get(s.parent)).map(|p| 0..p.submenu.len()),
    }
}

fn item_at<A>(nav: &Nav, items: &[MenuItem<A>], level: Level, ix: usize) -> Option<MenuItem<A>>
where
    A: Clone,
{
    match level {
        Level::Main => items.get(ix).cloned(),
        Level::Sub => nav.sub.as_ref().and_then(|s| items.get(s.parent)).and_then(|p| p.submenu.get(ix)).cloned(),
    }
}

/// Item `ix` of `level` picked: a submenu's item opens it (its first item highlighted when `by_key`), any other runs the owner's pick.
#[allow(clippy::too_many_arguments)]
fn activate<V: 'static, A: Clone + 'static>(nav: &Rc<RefCell<Nav>>, items: &[MenuItem<A>], acts: &Acts<V, A>, level: Level, ix: usize, by_key: bool, this: &mut V, window: &mut Window, cx: &mut Context<V>) {
    let Some(it) = item_at(&nav.borrow(), items, level, ix) else { return };
    if !it.acts() {
        return;
    }
    if !it.submenu.is_empty() {
        let first = by_key.then(|| nav::end(&it.submenu.iter().map(MenuItem::acts).collect::<Vec<_>>(), false)).flatten();
        let mut n = nav.borrow_mut();
        n.highlighted = Some(ix);
        n.sub = Some(Sub::new(ix, first));
        return;
    }
    (acts.pick)(this, it.action, window, cx);
}

/// A key on the menu; whether it took it.
fn key<V: 'static, A: Clone + 'static>(nav: &Rc<RefCell<Nav>>, items: &[MenuItem<A>], acts: &Acts<V, A>, e: &KeyDownEvent, this: &mut V, window: &mut Window, cx: &mut Context<V>) -> bool {
    let k = &e.keystroke;
    if k.modifiers.control || k.modifiers.alt || k.modifiers.platform || k.modifiers.function {
        return false;
    }
    let level = if nav.borrow().sub.as_ref().is_some_and(|s| s.highlighted.is_some()) { Level::Sub } else { Level::Main };
    let (shown, at) = {
        let n = nav.borrow();
        let range = level_items(&n, items, level).unwrap_or(0..0);
        let shown: Vec<MenuItem<A>> = range.filter_map(|i| item_at(&n, items, level, i)).collect();
        let at = match level {
            Level::Main => n.highlighted,
            Level::Sub => n.sub.as_ref().and_then(|s| s.highlighted),
        };
        (shown, at)
    };
    let acting: Vec<bool> = shown.iter().map(MenuItem::acts).collect();
    let to = match k.key.as_str() {
        "up" => nav::step(&acting, at, true),
        "down" => nav::step(&acting, at, false),
        "home" => nav::end(&acting, false),
        "end" => nav::end(&acting, true),
        "enter" | "space" => {
            if let Some(ix) = at {
                activate(nav, items, acts, level, ix, true, this, window, cx);
            }
            return true;
        }
        "right" => {
            if let (Level::Main, Some(ix)) = (level, at) {
                if shown.get(ix).is_some_and(|it| !it.submenu.is_empty()) {
                    activate(nav, items, acts, level, ix, true, this, window, cx);
                }
            }
            return true;
        }
        "left" => return nav.borrow_mut().sub.take().is_some(),
        "escape" => {
            if nav.borrow_mut().sub.take().is_none() {
                (acts.dismiss)(this, cx);
            }
            return true;
        }
        typed if typed.chars().count() == 1 && typed.chars().all(|c| c.is_alphanumeric()) => {
            let mut n = nav.borrow_mut();
            let now = Instant::now();
            if n.typed_at.is_none_or(|t| now.duration_since(t) > TYPED_FOR) {
                n.typed.clear();
            }
            n.typed.push_str(typed);
            n.typed_at = Some(now);
            let labels: Vec<(&str, bool)> = shown.iter().map(|it| (it.label.as_str(), it.acts())).collect();
            nav::typed(&labels, at, &n.typed).or_else(|| {
                n.typed = typed.to_string();
                nav::typed(&labels, at, typed)
            })
        }
        _ => return false,
    };
    let Some(to) = to else { return true };
    let mut n = nav.borrow_mut();
    n.epoch += 1;
    match level {
        Level::Main => {
            n.highlighted = Some(to);
            n.sub = None;
            n.scroll.scroll_to_item(to);
        }
        Level::Sub => {
            if let Some(sub) = n.sub.as_mut() {
                sub.highlighted = Some(to);
                sub.scroll.scroll_to_item(to);
            }
        }
    }
    true
}

fn caption(text: &str, colors: &ThemeColors) -> Div {
    div().px(tokens::space::XL).pb(tokens::space::S).text_size(tokens::text::CAPTION).text_color(colors.text_placeholder).overflow_hidden().whitespace_nowrap().child(SharedString::from(text.to_string()))
}

#[cfg(test)]
mod tests;
