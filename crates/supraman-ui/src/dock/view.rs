//! A dock's view: the owner's [`Layout`] over nested [`SplitPane`]s, each slot the owner's panel under a title bar while
//! it shows two or more, and its slots as zoned drop targets painting the dropped layout with the [`GHOST`] slot.

use std::rc::Rc;

use gpui::{AnyElement, App, Context, IntoElement, ParentElement, Pixels, SharedString, Styled, Window, div, prelude::*, relative};

use super::layout::{Dropped, Layout, RATIO, Zone};
use crate::controls::button::Button;
use crate::controls::tooltip::Tooltip;
use crate::dnd::{self, DropZone, Payload};
use crate::hit::{HitTargets, hit_target};
use crate::icon::Icon;
use crate::label::Label;
use crate::split::SplitPane;
use crate::tokens;

/// The dragged panel's slot in the dropped layout, painted in the `drop_over` look: `<id>-ghost`.
pub const GHOST: &str = "ghost";

/// One panel as its owner paints it: its element, its title bar's icon and words, and what its title bar drags.
pub struct Panel<D> {
    pub element: AnyElement,
    pub icon: Icon,
    pub words: SharedString,
    pub payload: D,
}

type OnLayout<V, P> = Rc<dyn Fn(&mut V, Layout<P>, &mut Context<V>)>;
type OnClose<V, P> = Rc<dyn Fn(&mut V, P, &mut Context<V>)>;
type OnCentre<V, P, D> = Rc<dyn Fn(&mut V, P, &D, &mut Window, &mut Context<V>)>;
type PanelOf<P, D> = Rc<dyn Fn(&D, &App) -> Option<P>>;

/// in the painted layout's [`Layout::panels`], `<id>-aim-<n>` the current layout's slot as a drop target (a dock of one's
/// only while a payload is dragged), `<id>-split<path>` each split, and `<id>-ghost`.
/// payload is dragged, `<id>-split<path>` each split, and `<id>-ghost`.
pub struct Dock<V, P, D> {
    id: SharedString,
    hits: HitTargets,
    focused: Option<P>,
    panel_of: PanelOf<P, D>,
    on_layout: OnLayout<V, P>,
    on_close: OnClose<V, P>,
    on_centre: Option<OnCentre<V, P, D>>,
}

impl<V, P: Copy, D> Clone for Dock<V, P, D> {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            hits: self.hits.clone(),
            focused: self.focused,
            panel_of: self.panel_of.clone(),
            on_layout: self.on_layout.clone(),
            on_close: self.on_close.clone(),
            on_centre: self.on_centre.clone(),
        }
    }
}

impl<V: 'static, P: Copy + Eq + 'static, D: Payload> Dock<V, P, D> {
    /// `panel_of` names the panel a payload is (one of this dock's moves, another's joins), none for one only a centre takes.
    pub fn new(hits: &HitTargets, id: impl Into<SharedString>, panel_of: impl Fn(&D, &App) -> Option<P> + 'static) -> Self {
        Self {
            id: id.into(),
            hits: hits.clone(),
            focused: None,
            panel_of: Rc::new(panel_of),
            on_layout: Rc::new(|_, _, _| {}),
            on_close: Rc::new(|_, _, _| {}),
            on_centre: None,
        }
    }

    /// The panel whose title shows in the text colour; the others' are muted.
    pub fn focused(mut self, panel: Option<P>) -> Self {
        self.focused = panel;
        self
    }

    /// A divider's drag and a drop: the new layout.
    pub fn on_layout(mut self, f: impl Fn(&mut V, Layout<P>, &mut Context<V>) + 'static) -> Self {
        self.on_layout = Rc::new(f);
        self
    }

    /// A title bar's ×.
    pub fn on_close(mut self, f: impl Fn(&mut V, P, &mut Context<V>) + 'static) -> Self {
        self.on_close = Rc::new(f);
        self
    }

    /// A payload let go on a panel's centre; without it the centre takes nothing.
    pub fn on_centre(mut self, f: impl Fn(&mut V, P, &D, &mut Window, &mut Context<V>) + 'static) -> Self {
        self.on_centre = Some(Rc::new(f));
        self
    }

    /// The dock of `layout`, each panel's own from `panels`; a dock of one is its element alone while nothing is dragged.
    pub fn render(self, layout: &Layout<P>, panels: Vec<(P, Panel<D>)>, cx: &mut Context<V>) -> AnyElement {
        let carrying = dnd::carrying::<D>(cx);
        let preview = if carrying { self.preview(layout, cx) } else { None };
        let (shown, ghost) = match preview {
            Some((landed, ghost)) => (landed, Some(ghost)),
            None => (layout.clone(), None),
        };
        let mut panels: Vec<(P, Option<Panel<D>>)> = panels.into_iter().map(|(p, panel)| (p, Some(panel))).collect();
        let mut take = |p: P| panels.iter_mut().find(|(q, _)| *q == p).and_then(|(_, panel)| panel.take());
        if shown.len() == 1 && !carrying {
            return take(shown.panels()[0]).map_or_else(|| div().into_any_element(), |panel| panel.element);
        }
        let titled = shown.len() > 1;
        let mut slots: Vec<(P, AnyElement)> = Vec::new();
        for (n, p) in shown.panels().into_iter().enumerate() {
            let slot = match (Some(p) == ghost, take(p)) {
                (true, _) => self.ghost(cx),
                (false, Some(panel)) => self.slot(n, p, panel, titled, cx),
                (false, None) => div().size_full().into_any_element(),
            };
            slots.push((p, slot));
        }
        let current = Rc::new(layout.clone());
        let tree = self.tree(&shown, &mut Vec::new(), &mut slots, &current, cx);
        let aims = (carrying || layout.len() > 1).then(|| self.aims(&current, cx));
        div().relative().flex().size_full().min_w_0().min_h_0().bg(tokens::roles(cx).surface).child(tree).children(aims).into_any_element()
    }

    /// What a payload over `target`'s `zone` does to `layout`: a moved or joining panel's layout, or the centre's own.
    fn landing(&self, layout: &Layout<P>, target: P, zone: Zone, payload: &D, cx: &App) -> Option<Dropped<P>> {
        if zone == Zone::Centre {
            return (self.on_centre.is_some() && layout.contains(target)).then_some(Dropped::Owner);
        }
        let p = (self.panel_of)(payload, cx)?;
        if layout.contains(p) { layout.moved(p, target, zone) } else { layout.drop(target, zone, p) }
    }

    /// The layout a hovered zone would make and the panel landing there.
    fn preview(&self, layout: &Layout<P>, cx: &App) -> Option<(Layout<P>, P)> {
        layout.panels().into_iter().enumerate().find_map(|(n, target)| {
            let (zone, payload) = dnd::zone_over::<D>(&self.hits.key(&self.key("aim", n)), cx)?;
            match self.landing(layout, target, zone, &payload, cx)? {
                Dropped::Layout(landed) => Some((landed, (self.panel_of)(&payload, cx)?)),
                Dropped::Owner => None,
            }
        })
    }

    fn key(&self, part: &str, n: usize) -> String {
        format!("{}-{part}-{n}", self.id)
    }

    fn tree(&self, shown: &Layout<P>, path: &mut Vec<usize>, slots: &mut Vec<(P, AnyElement)>, current: &Rc<Layout<P>>, cx: &mut Context<V>) -> AnyElement {
        let (axis, ratio, first, second) = match shown {
            Layout::Split { axis, ratio, first, second } => (axis, ratio, first, second),
            Layout::Leaf(p) => {
                let at = slots.iter().position(|(q, _)| q == p);
                return at.map_or_else(|| div().into_any_element(), |at| slots.remove(at).1);
            }
        };
        let mut side = |which: usize, layout: &Layout<P>, slots: &mut Vec<(P, AnyElement)>, cx: &mut Context<V>| {
            path.push(which);
            let el = self.tree(layout, path, slots, current, cx);
            path.pop();
            el
        };
        let a = side(0, first, slots, cx);
        let b = side(1, second, slots, cx);
        let suffix: String = path.iter().map(|s| format!("-{s}")).collect();
        let split = SplitPane { id: format!("{}-split{suffix}", self.id).into(), min_first: tokens::bar::HEADER, min_second: tokens::bar::HEADER, default: Pixels::ZERO, axis: *axis, hits: self.hits.clone() };
        let (on_layout, current, at) = (self.on_layout.clone(), current.clone(), path.clone());
        let resized = move |v: &mut V, share: f32, cx: &mut Context<V>| {
            if let Some(laid) = current.with_ratio(&at, share) {
                on_layout(v, laid, cx);
            }
        };
        split.render_fraction(a, b, *ratio, RATIO, resized, |_, _| {}, cx).size_full().into_any_element()
    }

    /// One panel's slot: its title bar while `titled`, then its element; no border, the dividers are the dock's lines.
    fn slot(&self, n: usize, p: P, panel: Panel<D>, titled: bool, cx: &mut Context<V>) -> AnyElement {
        let bar = titled.then(|| self.title_bar(n, p, panel.icon, panel.words, panel.payload, cx));
        div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h_0()
            .children(bar)
            .child(div().relative().flex().flex_col().flex_1().min_w_0().min_h_0().overflow_hidden().child(panel.element))
            .child(hit_target(&self.hits, self.key("slot", n)))
            .into_any_element()
    }

    fn title_bar(&self, n: usize, p: P, icon: Icon, words: SharedString, payload: D, cx: &mut Context<V>) -> AnyElement {
        let key = self.key("title", n);
        let label = Label::new(words).mark(icon).size(tokens::text::SMALL).clipped(self.hits.key(&format!("{key}-words")), false).flex_1();
        let label = if self.focused == Some(p) { label } else { label.muted() };
        let (on_close, owner) = (self.on_close.clone(), cx.entity().downgrade());
        let close = Button::new(&self.hits, self.key("close", n)).icon(Icon::Close).icon_only(tokens::bar::ICON_BUTTON).tooltip(Tooltip::text("Close")).on_click(move |_, _, cx| {
            cx.stop_propagation();
            owner.update(cx, |v, cx| on_close(v, p, cx)).ok();
        });
        let bar = div()
            .id(SharedString::from(self.hits.key(&key)))
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(tokens::space::S)
            .h(tokens::bar::HEADER)
            .pl(tokens::space::L)
            .pr(tokens::space::XS)
            .child(label)
            .child(close)
            .child(hit_target(&self.hits, key.clone()));
        dnd::source(bar, self.hits.key(&key), payload, cx).into_any_element()
    }

    fn ghost(&self, cx: &App) -> AnyElement {
        dnd::look(div().relative().size_full(), cx).child(hit_target(&self.hits, format!("{}-{GHOST}", self.id))).into_any_element()
    }

    /// The current layout's slots over the painted one, each a zoned target, so the dropped layout never moves them.
    fn aims(&self, current: &Rc<Layout<P>>, cx: &mut Context<V>) -> AnyElement {
        let mut over = div().absolute().top_0().left_0().size_full();
        for (n, (target, rect)) in current.rects(1., 1.).into_iter().enumerate() {
            let key = self.key("aim", n);
            let el = div().absolute().left(relative(rect.x)).top(relative(rect.y)).w(relative(rect.width)).h(relative(rect.height)).child(hit_target(&self.hits, key.clone()));
            over = over.child(self.aim(el, &key, target, current, cx));
        }
        over.into_any_element()
    }

    fn aim(&self, el: gpui::Div, key: &str, target: P, current: &Rc<Layout<P>>, cx: &mut Context<V>) -> gpui::Div {
        let dock = Rc::new(self.clone());
        let (takes, laid, owner) = (dock.clone(), current.clone(), cx.entity().downgrade());
        let layout = current.clone();
        let accepts = move |d: &D, zone: Zone, _: &Window, cx: &App| takes.landing(&layout, target, zone, d, cx).is_some();
        let dropped = move |d: &D, zone: Zone, window: &mut Window, cx: &mut App| {
            let Some(landed) = dock.landing(&laid, target, zone, d, cx) else { return };
            let dock = dock.clone();
            owner
                .update(cx, |v, cx| match landed {
                    Dropped::Layout(landed) => (dock.on_layout)(v, landed, cx),
                    Dropped::Owner => dock.on_centre.iter().for_each(|f| f(v, target, d, window, cx)),
                })
                .ok();
        };
        DropZone::new(self.hits.key(key), el).zoned::<D>(accepts, dropped).element()
    }
}

#[cfg(test)]
mod tests;
