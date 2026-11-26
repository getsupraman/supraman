//! Every scroller's one primitive (UI-KIT §4.4): [`area`], a vertical scroll box with its bar (Page Up, Page Down, Home and
//! End while the focus is inside it and nothing there takes them), and [`Scrollbar`], the bar over a list's or a custom
//! viewport's [`ScrollbarHandle`].

use std::time::Duration;

use gpui::{
    AnyElement, App, Div, ElementId, Global, InteractiveElement, Interactivity, IntoElement, KeyDownEvent, Keystroke, ListState, ParentElement, Pixels, Point, RenderOnce, ScrollHandle, StatefulInteractiveElement,
    StyleRefinement, Styled, UniformListScrollHandle, Window, div, point,
};
pub use supraman_core::Scrollbars;

use crate::hit::HitTargets;

mod bar;

pub use bar::Scrollbar;
#[cfg(test)]
pub(crate) use bar::Thumb;

/// A scroller the bar reads and moves; offsets are gpui's, `y` zero at the top and negative below it.
pub trait ScrollbarHandle: 'static {
    fn viewport_bounds(&self) -> gpui::Bounds<Pixels>;
    fn offset(&self) -> Point<Pixels>;
    fn set_offset(&self, offset: Point<Pixels>);
    /// The whole content's size: the viewport plus how far it scrolls.
    fn content_size(&self) -> gpui::Size<Pixels>;
    fn start_drag(&self) {}
    fn end_drag(&self) {}
    /// Whether a wheel over the area is its own until it reaches its edge, then its parent's (gpui-kit's `ScrollableMask`).
    fn takes_wheel(&self) -> bool {
        false
    }
}

impl ScrollbarHandle for ScrollHandle {
    fn viewport_bounds(&self) -> gpui::Bounds<Pixels> {
        self.bounds()
    }

    fn offset(&self) -> Point<Pixels> {
        ScrollHandle::offset(self)
    }

    fn set_offset(&self, offset: Point<Pixels>) {
        ScrollHandle::set_offset(self, offset);
    }

    fn content_size(&self) -> gpui::Size<Pixels> {
        (self.max_offset() + self.bounds().size.into()).into()
    }

    fn takes_wheel(&self) -> bool {
        true
    }
}

impl ScrollbarHandle for UniformListScrollHandle {
    fn viewport_bounds(&self) -> gpui::Bounds<Pixels> {
        self.0.borrow().base_handle.bounds()
    }

    fn offset(&self) -> Point<Pixels> {
        self.0.borrow().base_handle.offset()
    }

    fn set_offset(&self, offset: Point<Pixels>) {
        self.0.borrow_mut().base_handle.set_offset(offset);
    }

    fn content_size(&self) -> gpui::Size<Pixels> {
        let base = &self.0.borrow().base_handle;
        (base.max_offset() + base.bounds().size.into()).into()
    }

    fn takes_wheel(&self) -> bool {
        true
    }
}

impl ScrollbarHandle for ListState {
    fn viewport_bounds(&self) -> gpui::Bounds<Pixels> {
        ListState::viewport_bounds(self)
    }

    fn offset(&self) -> Point<Pixels> {
        self.scroll_px_offset_for_scrollbar()
    }

    fn set_offset(&self, offset: Point<Pixels>) {
        self.set_offset_from_scrollbar(offset);
    }

    fn content_size(&self) -> gpui::Size<Pixels> {
        self.viewport_bounds().size + self.max_offset_for_scrollbar().into()
    }

    fn start_drag(&self) {
        self.scrollbar_drag_started();
    }

    fn end_drag(&self) {
        self.scrollbar_drag_ended();
    }
}

struct Shown(Scrollbars);

impl Global for Shown {}

/// `[appearance] scrollbars`, as `main` read it; every window paints with it from its next frame.
pub fn set_mode(mode: Scrollbars, cx: &mut App) {
    cx.set_global(Shown(mode));
    cx.refresh_windows();
}

pub fn mode(cx: &App) -> Scrollbars {
    cx.try_global::<Shown>().map_or_else(Scrollbars::default, |s| s.0)
}

/// How long a bar stays after the last scroll, then how long it fades.
pub(crate) const IDLE: Duration = crate::tokens::motion::SCROLLBAR_IDLE;
pub(crate) const FADE: Duration = crate::tokens::motion::NORMAL;

/// A vertical scroll box: style and children are the scrolled element's, its bar is laid over its right edge.
pub fn area(id: impl Into<ElementId>) -> ScrollArea {
    ScrollArea { id: id.into(), handle: None, content: div(), hit: None, mode: None }
}

#[derive(IntoElement)]
pub struct ScrollArea {
    id: ElementId,
    handle: Option<ScrollHandle>,
    content: Div,
    hit: Option<(HitTargets, String)>,
    mode: Option<Scrollbars>,
}

impl ScrollArea {
    /// The owner's handle, where it reads or moves the scroll itself; else the area keeps one.
    pub fn track(mut self, handle: &ScrollHandle) -> Self {
        self.handle = Some(handle.clone());
        self
    }

    /// Records the thumb as [`Scrollbar::hit`] does.
    pub fn hit(mut self, hits: &HitTargets, key: impl Into<String>) -> Self {
        self.hit = Some((hits.clone(), key.into()));
        self
    }

    /// In place of the setting's.
    pub fn mode(mut self, mode: Scrollbars) -> Self {
        self.mode = Some(mode);
        self
    }
}

impl Styled for ScrollArea {
    fn style(&mut self) -> &mut StyleRefinement {
        self.content.style()
    }
}

impl ParentElement for ScrollArea {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.content.extend(elements);
    }
}

impl InteractiveElement for ScrollArea {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.content.interactivity()
    }
}

impl StatefulInteractiveElement for ScrollArea {}

impl RenderOnce for ScrollArea {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let handle = match self.handle {
            Some(handle) => handle,
            None => window.use_keyed_state(self.id.clone(), cx, |_, _| ScrollHandle::new()).read(cx).clone(),
        };
        let mut bar = Scrollbar::new(ElementId::from((self.id.clone(), "bar")), &handle);
        if let Some((hits, key)) = self.hit {
            bar = bar.hit(&hits, key);
        }
        if let Some(mode) = self.mode {
            bar = bar.mode(mode);
        }
        let keys = handle.clone();
        self.content
            .id(self.id)
            .overflow_y_scroll()
            .track_scroll(&handle)
            .on_key_down(move |e: &KeyDownEvent, window, cx| {
                let Some(y) = keyed(&keys, &e.keystroke) else { return };
                bar::user_scrolled(cx);
                keys.set_offset(point(keys.offset().x, y));
                window.refresh();
                cx.stop_propagation();
            })
            .child(bar)
    }
}

/// The offset Page Up, Page Down, Home or End asks for; none for another key or one with a modifier.
fn keyed(handle: &ScrollHandle, key: &Keystroke) -> Option<Pixels> {
    if key.modifiers.modified() {
        return None;
    }
    let (offset, page, max) = (handle.offset().y, handle.bounds().size.height, handle.max_offset().y);
    let y = match key.key.as_str() {
        "pageup" => offset + page,
        "pagedown" => offset - page,
        "home" => Pixels::ZERO,
        "end" => -max,
        _ => return None,
    };
    Some(y.clamp(-max, Pixels::ZERO))
}

#[cfg(test)]
mod tests;
