// Copyright 2024 - 2026 Longbridge <https://longbridge.com>
//
// Licensed under the Apache License, Version 2.0 (the "License"); you may not use this file except in compliance with the
// License. You may obtain a copy of the License at http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software distributed under the License is distributed on an
// "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied. See the License for the specific
// language governing permissions and limitations under the License.
//
// From gpui-kit (https://github.com/longbridge/gpui-kit) `crates/base/src/scrollbar.rs` and
// `crates/base/src/scrollable_mask.rs` @ f807c1b. Changed for Supraman: vertical only, one mode setting, the kit's tokens
// and colour roles, a fade in place of its motion presets, a track click pages, and the vertical mask folded into the bar.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    App, Bounds, CursorStyle, Element, ElementId, GlobalElementId, Hitbox, HitboxBehavior, InspectorElementId, IntoElement, LayoutId, MouseButton, MouseDownEvent, MouseExitEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Position, ScrollEvent, Style, Window, fill, point, px, size,
};

use super::{FADE, IDLE, ScrollbarHandle, Scrollbars};
use crate::hit::{Hit, HitTargets};

const THUMB_WIDTH: Pixels = px(6.);
const THUMB_ACTIVE_WIDTH: Pixels = px(8.);
const MIN_THUMB: Pixels = px(32.);
/// The gap between the thumb and the viewport's edges.
const INSET: Pixels = crate::tokens::space::XS;
/// The strip at the right edge that takes a press: the active thumb and its inset.
const TRACK: Pixels = px(10.);
/// An offset change this soon after a wheel, a key or a drag is the user's scroll; any other (a reveal) shows no bar.
const INPUT_WINDOW: Duration = Duration::from_millis(500);

thread_local! {
    static INPUT_AT: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// The user moved a scroller now: the bar whose offset changes next shows.
pub(crate) fn user_scrolled(cx: &App) {
    INPUT_AT.set(Some(cx.background_executor().now()));
}

fn by_user(now: Instant) -> bool {
    INPUT_AT.get().is_some_and(|at| now.saturating_duration_since(at) < INPUT_WINDOW)
}

/// The thumb on its track: where it lies for an offset, and the offset a pointer on the track asks for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Thumb {
    pub top: Pixels,
    pub length: Pixels,
    origin: Pixels,
    travel: Pixels,
    extent: Pixels,
    viewport: Pixels,
}

impl Thumb {
    /// `None` while the content fits its viewport.
    pub fn new(track_top: Pixels, track: Pixels, viewport: Pixels, content: Pixels, offset: Pixels) -> Option<Self> {
        if content - viewport < px(0.5) || track <= Pixels::ZERO {
            return None;
        }
        let length = (viewport / content * track).max(MIN_THUMB).min(track);
        let (travel, extent) = (track - length, content - viewport);
        let top = track_top + (-offset / extent).clamp(0., 1.) * travel;
        Some(Self { top, length, origin: track_top, travel, extent, viewport })
    }

    /// The offset that puts the thumb's point `grab` below its top at `y`.
    pub fn offset_at(&self, y: Pixels, grab: Pixels) -> Pixels {
        if self.travel <= Pixels::ZERO {
            return Pixels::ZERO;
        }
        -self.extent * ((y - self.origin - grab) / self.travel).clamp(0., 1.)
    }

    /// A page from `offset` toward `y`.
    pub fn paged(&self, offset: Pixels, y: Pixels) -> Pixels {
        let page = if y < self.top { self.viewport } else { -self.viewport };
        (offset + page).clamp(-self.extent, Pixels::ZERO)
    }

    pub fn contains(&self, y: Pixels) -> bool {
        y >= self.top && y < self.top + self.length
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Bar {
    seen: Option<Pixels>,
    scrolled_at: Option<Instant>,
    hovered: bool,
    on_thumb: bool,
    grab: Option<Pixels>,
    opacity: f32,
    fading_since: Option<Instant>,
    waking: bool,
}

type State = Rc<Cell<Bar>>;

/// The overlay bar of one scroller, laid out at no size and painted over its handle's viewport; [`super::area`]'s,
/// or beside a list or a custom viewport.
pub struct Scrollbar {
    id: ElementId,
    handle: Rc<dyn ScrollbarHandle>,
    mode: Option<Scrollbars>,
    hit: Option<(HitTargets, String)>,
}

impl Scrollbar {
    pub fn new(id: impl Into<ElementId>, handle: &(impl ScrollbarHandle + Clone)) -> Self {
        Self { id: id.into(), handle: Rc::new(handle.clone()), mode: None, hit: None }
    }

    pub fn mode(mut self, mode: Scrollbars) -> Self {
        self.mode = Some(mode);
        self
    }

    /// Records the track under `<key>-track`, the thumb's part of it under `<key>-thumb` and, while it is painted, the thumb
    /// under `<key>-thumb-shown`.
    pub fn hit(mut self, hits: &HitTargets, key: impl Into<String>) -> Self {
        self.hit = Some((hits.clone(), key.into()));
        self
    }
}

impl IntoElement for Scrollbar {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

pub struct Prepainted {
    state: State,
    shown: Option<Shown>,
}

struct Shown {
    viewport: Bounds<Pixels>,
    thumb: Thumb,
    area: Hitbox,
    track: Hitbox,
}

impl Element for Scrollbar {
    type RequestLayoutState = ();
    type PrepaintState = Prepainted;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, window: &mut Window, cx: &mut App) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.position = Position::Absolute;
        style.size = size(Pixels::ZERO.into(), Pixels::ZERO.into());
        (window.request_layout(style, None, cx), ())
    }

    fn prepaint(&mut self, id: Option<&GlobalElementId>, _: Option<&InspectorElementId>, _: Bounds<Pixels>, _: &mut (), window: &mut Window, cx: &mut App) -> Prepainted {
        let state: State = match id {
            Some(id) => window.with_element_state(id, |s: Option<State>, _| {
                let s = s.unwrap_or_default();
                (s.clone(), s)
            }),
            None => State::default(),
        };
        let viewport = self.handle.viewport_bounds();
        let offset = self.handle.offset().y;
        let always = self.mode.unwrap_or_else(|| super::mode(cx)) == Scrollbars::Always;
        self.settle(&state, offset, always, window, cx);
        if let Some((hits, key)) = &self.hit {
            let mut hits = hits.borrow_mut();
            for part in ["thumb", "thumb-shown", "track"] {
                hits.remove(&format!("{key}-{part}"));
            }
        }
        let Some(thumb) = Thumb::new(viewport.top() + INSET, viewport.size.height - INSET * 2., viewport.size.height, self.handle.content_size().height, offset) else {
            return Prepainted { state, shown: None };
        };
        let track_bounds = Bounds::new(point(viewport.right() - TRACK, viewport.top()), size(TRACK, viewport.size.height));
        let area = window.insert_hitbox(viewport, HitboxBehavior::Normal);
        let track = window.insert_hitbox(track_bounds, HitboxBehavior::Normal);
        if let Some((hits, key)) = &self.hit {
            let visible = window.content_mask().bounds;
            let bounds = Bounds::new(point(track_bounds.left(), thumb.top), size(TRACK, thumb.length));
            let mut hits = hits.borrow_mut();
            hits.insert(format!("{key}-thumb"), Hit { bounds, visible });
            hits.insert(format!("{key}-track"), Hit { bounds: track_bounds, visible });
        }
        Prepainted { state, shown: Some(Shown { viewport, thumb, area, track }) }
    }

    fn paint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, _: Bounds<Pixels>, _: &mut (), prepainted: &mut Prepainted, window: &mut Window, cx: &mut App) {
        let Some(shown) = &prepainted.shown else { return };
        let state = prepainted.state.clone();
        let bar = state.get();
        if bar.opacity > 0. {
            let roles = crate::tokens::roles(cx);
            let width = if bar.on_thumb || bar.grab.is_some() { THUMB_ACTIVE_WIDTH } else { THUMB_WIDTH };
            let color = match (bar.grab.is_some(), bar.on_thumb) {
                (true, _) => roles.scrollbar_thumb_active,
                (false, true) => roles.scrollbar_thumb_hover,
                _ => roles.scrollbar_thumb,
            };
            let bounds = Bounds::new(point(shown.viewport.right() - INSET - width, shown.thumb.top), size(width, shown.thumb.length));
            window.paint_quad(fill(bounds, color.opacity(bar.opacity)).corner_radii(width / 2.));
            window.set_cursor_style(CursorStyle::Arrow, &shown.track);
            if let Some((hits, key)) = &self.hit {
                hits.borrow_mut().insert(format!("{key}-thumb-shown"), Hit { bounds, visible: window.content_mask().bounds });
            }
        }
        self.listen(shown, &state, window);
    }
}

impl Scrollbar {
    /// This frame's visibility: shown while wanted, else fading on animation frames while the window is active.
    fn settle(&self, state: &State, offset: Pixels, always: bool, window: &mut Window, cx: &mut App) {
        let now = cx.background_executor().now();
        let mut s = state.get();
        if s.seen.is_some_and(|seen| seen != offset) && (s.grab.is_some() || by_user(now)) {
            s.scrolled_at = Some(now);
        }
        s.seen = Some(offset);
        let scrolling = s.scrolled_at.map(|at| now.saturating_duration_since(at)).filter(|since| *since < IDLE);
        if always || s.hovered || s.grab.is_some() || scrolling.is_some() {
            s.opacity = 1.;
            s.fading_since = None;
        } else if s.opacity > 0. {
            if window.is_window_active() {
                let since = *s.fading_since.get_or_insert(now);
                s.opacity = 1. - now.saturating_duration_since(since).as_secs_f32() / FADE.as_secs_f32();
            } else {
                s.opacity = 0.;
            }
            if s.opacity > 0. {
                window.request_animation_frame();
            } else {
                s.opacity = 0.;
                s.fading_since = None;
            }
        }
        if let Some(since) = scrolling.filter(|_| !always && !s.hovered && s.grab.is_none() && !s.waking) {
            s.waking = true;
            let (state, view) = (state.clone(), window.current_view());
            window
                .spawn(cx, async move |cx| {
                    cx.background_executor().timer(IDLE - since).await;
                    state.set(Bar { waking: false, ..state.get() });
                    cx.update(|_, cx| cx.notify(view)).ok();
                })
                .detach();
        }
        state.set(s);
    }

    fn listen(&self, shown: &Shown, state: &State, window: &mut Window) {
        let view = window.current_view();
        let line_height = window.line_height();
        let (thumb, area, track) = (shown.thumb, shown.area.clone(), shown.track.clone());
        let visible = state.get().opacity > 0.;
        let edit = |state: &State, f: &dyn Fn(&mut Bar)| {
            let mut s = state.get();
            f(&mut s);
            state.set(s);
        };
        window.on_mouse_event({
            let (handle, area) = (self.handle.clone(), area.clone());
            move |e: &ScrollEvent, phase, window, cx| {
                if !phase.capture() || !area.should_handle_scroll(window) {
                    return;
                }
                user_scrolled(cx);
                if !handle.takes_wheel() || e.modifiers.control {
                    return;
                }
                let delta = e.delta.pixel_delta(line_height);
                if delta.x.abs() > delta.y.abs() {
                    return;
                }
                // Clamped first: a scroll that bubbled leaves the shared offset past the edge until the next prepaint.
                let max = (handle.content_size().height - handle.viewport_bounds().size.height).max(Pixels::ZERO);
                let offset = handle.offset();
                let current = offset.y.clamp(-max, Pixels::ZERO);
                let next = (current + delta.y).clamp(-max, Pixels::ZERO);
                if next != current {
                    handle.set_offset(point(offset.x, next));
                    cx.notify(view);
                    cx.stop_propagation();
                }
            }
        });
        window.on_mouse_event({
            let (handle, state, track) = (self.handle.clone(), state.clone(), track.clone());
            move |e: &MouseDownEvent, phase, window, cx| {
                if !phase.bubble() || e.button != MouseButton::Left || !visible || !track.is_hovered(window) {
                    return;
                }
                cx.stop_propagation();
                window.prevent_default();
                user_scrolled(cx);
                let offset = handle.offset();
                if thumb.contains(e.position.y) {
                    handle.start_drag();
                    edit(&state, &|s| s.grab = Some(e.position.y - thumb.top));
                } else {
                    handle.set_offset(point(offset.x, thumb.paged(offset.y, e.position.y)));
                }
                cx.notify(view);
            }
        });
        window.on_mouse_event({
            let (handle, state) = (self.handle.clone(), state.clone());
            move |e: &MouseMoveEvent, _, window, cx| {
                let before = state.get();
                let hovered = area.is_hovered(window);
                let on_thumb = hovered && track.is_hovered(window) && thumb.contains(e.position.y);
                edit(&state, &|s| {
                    s.hovered = hovered;
                    s.on_thumb = on_thumb;
                });
                if let Some(grab) = before.grab.filter(|_| e.dragging()) {
                    cx.stop_propagation();
                    user_scrolled(cx);
                    let offset = handle.offset();
                    let y = thumb.offset_at(e.position.y, grab);
                    if y != offset.y {
                        handle.set_offset(point(offset.x, y));
                        cx.notify(view);
                    }
                }
                if before.hovered != hovered || before.on_thumb != on_thumb {
                    cx.notify(view);
                }
            }
        });
        window.on_mouse_event({
            let (handle, state) = (self.handle.clone(), state.clone());
            move |e: &MouseUpEvent, phase, _, cx| {
                if phase.bubble() && e.button == MouseButton::Left && state.get().grab.is_some() {
                    let now = cx.background_executor().now();
                    handle.end_drag();
                    edit(&state, &|s| {
                        s.grab = None;
                        s.scrolled_at = Some(now);
                    });
                    cx.notify(view);
                }
            }
        });
        window.on_mouse_event({
            let state = state.clone();
            move |_: &MouseExitEvent, _, _, cx| {
                if state.get().hovered {
                    edit(&state, &|s| {
                        s.hovered = false;
                        s.on_thumb = false;
                    });
                    cx.notify(view);
                }
            }
        });
    }
}
