//! Every popup's anchor and the window's registry of the popups open (UI-KIT §4.2): one per [`Layer`], the anchor's key
//! [`is_open`] until its popup closes or another of its layer opens. Focus return stays [`crate::focus::popup`]'s.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::{Rc, Weak},
};

use gpui::{App, Global, Hsla, IntoElement, Pixels, SharedString, Styled as _, Window, WindowId, canvas, fill};

use crate::tokens::Layer;

/// A popup's hold on its anchor, kept by the popup: dropped, the anchor is no longer open.
#[derive(Clone)]
pub struct Popup(Rc<Open>);

struct Open {
    anchor: Option<SharedString>,
    layer: Layer,
    displaced: Cell<bool>,
}

impl Popup {
    /// A popup of `layer` belonging to the element painted under `anchor` (`None`: to no element).
    pub fn open(anchor: Option<SharedString>, layer: Layer) -> Self {
        Self(Rc::new(Open { anchor, layer, displaced: Cell::new(false) }))
    }

    pub fn anchor(&self) -> Option<&SharedString> {
        self.0.anchor.as_ref()
    }

    /// Records it as its layer's one popup as it paints; another of its layer is displaced and should close.
    pub fn shown(&self, window: &mut Window, cx: &mut App) {
        let registry = registry(window, cx);
        let mut layers = registry.borrow_mut();
        let held = layers.get(&self.0.layer).and_then(Weak::upgrade);
        if held.as_ref().is_some_and(|h| Rc::ptr_eq(h, &self.0)) {
            return;
        }
        if let Some(held) = held {
            held.displaced.set(true);
        }
        layers.insert(self.0.layer, Rc::downgrade(&self.0));
        window.refresh();
    }

    /// Another popup of its layer opened since it was shown.
    pub fn displaced(&self) -> bool {
        self.0.displaced.get()
    }
}

/// Whether a popup belonging to the element under `anchor` is open in this window.
pub fn is_open(anchor: &str, window: &Window, cx: &mut App) -> bool {
    registry(window, cx).borrow().values().filter_map(Weak::upgrade).any(|o| !o.displaced.get() && o.anchor.as_deref() == Some(anchor))
}

/// The open fill of a row or pill that builds its own element: the first child of its `relative` element, painted
/// over its rest, hover and selection fills while the popup `anchor` names is open ([`crate::state::resolve`]).
pub fn open_fill(anchor: impl Into<SharedString>) -> OpenFill {
    OpenFill { anchor: anchor.into(), radius: Pixels::ZERO, fill: None }
}

pub struct OpenFill {
    anchor: SharedString,
    radius: Pixels,
    fill: Option<Hsla>,
}

impl OpenFill {
    /// The corners of the element it fills.
    pub fn radius(self, radius: Pixels) -> Self {
        Self { radius, ..self }
    }

    /// In place of [`crate::tokens::Roles::open`] (a pill's own hover fill, in its lane's colour).
    pub fn fill(self, fill: Hsla) -> Self {
        Self { fill: Some(fill), ..self }
    }
}

impl IntoElement for OpenFill {
    type Element = gpui::Canvas<()>;

    fn into_element(self) -> Self::Element {
        let OpenFill { anchor, radius, fill: chosen } = self;
        canvas(
            |_, _, _| {},
            move |bounds, _, window, cx| {
                let state = crate::state::State { open: is_open(&anchor, window, cx), ..Default::default() };
                let open = chosen.unwrap_or_else(|| crate::tokens::roles(cx).open);
                if let Some(bg) = crate::state::resolve(state, crate::state::Fills { open: Some(open), ..Default::default() }).fill {
                    note_painted(&anchor);
                    window.paint_quad(fill(bounds, bg).corner_radii(radius));
                }
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
    }
}

thread_local! {
    static PAINTED: RefCell<HashMap<SharedString, u64>> = RefCell::default();
}

/// How many frames painted `anchor`'s open fill: a selftest's read of what only paint shows.
pub fn painted_open(anchor: &str) -> u64 {
    PAINTED.with(|p| p.borrow().get(anchor).copied().unwrap_or(0))
}

pub(crate) fn note_painted(anchor: &SharedString) {
    PAINTED.with(|p| *p.borrow_mut().entry(anchor.clone()).or_default() += 1);
}

type Layers = HashMap<Layer, Weak<Open>>;

#[derive(Default)]
struct Windows(HashMap<WindowId, Rc<RefCell<Layers>>>);

impl Global for Windows {}

fn registry(window: &Window, cx: &mut App) -> Rc<RefCell<Layers>> {
    let id = window.window_handle().window_id();
    if let Some(r) = cx.default_global::<Windows>().0.get(&id) {
        return r.clone();
    }
    let open: Vec<WindowId> = cx.windows().iter().map(|w| w.window_id()).collect();
    let windows = &mut cx.global_mut::<Windows>().0;
    windows.retain(|kept, _| open.contains(kept));
    windows.entry(id).or_default().clone()
}
