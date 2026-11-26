//! The window's one deferred focus change ([`request`]) and its popups' focus return ([`popup`]): a popup remembers
//! what held the focus as it first painted and gives it back when it closes holding the focus (UI-KIT §4.3).

use std::{cell::Cell, rc::Rc};

use gpui::{App, FocusHandle, Global, IntoElement, Styled as _, WeakFocusHandle, Window, canvas};

use super::state;

gpui::actions!(focus_probe, [Probe]);

/// Focuses `handle` once the next frame has painted, if that frame painted it; a later request replaces it.
pub fn request(handle: &FocusHandle, window: &mut Window, cx: &mut App) {
    let state = state(window, cx);
    let frame = state.borrow().frame;
    state.borrow_mut().requested = Some((handle.clone(), frame));
    window.refresh();
}

/// A popup's marker, a child of its element before the one tracking its focus: `region` (its first handle naming it)
/// contains every focusable thing in it. As it first paints it records what held the focus; closed while holding
/// the focus, it gives it back.
pub fn popup(region: impl IntoIterator<Item = FocusHandle>) -> impl IntoElement {
    let region: Vec<WeakFocusHandle> = region.into_iter().map(|h| h.downgrade()).collect();
    canvas(move |_, window, cx| opened(&region, window, cx), |_, _, _, _| {}).absolute().size_0()
}

pub(super) struct Popup {
    region: Vec<WeakFocusHandle>,
    previous: Option<WeakFocusHandle>,
    painted: u64,
    holding: bool,
}

impl Popup {
    fn contains_focused(&self, window: &Window, cx: &App) -> bool {
        self.region.iter().filter_map(WeakFocusHandle::upgrade).any(|r| r.contains_focused(window, cx))
    }
}

/// `closed` gone: a popup that opened from it gives the focus back to what `closed` would have.
fn hand_on(closed: &Popup, popups: &mut [Popup]) {
    for p in popups {
        if p.previous.as_ref().is_some_and(|prev| closed.region.contains(prev)) {
            p.previous = closed.previous.clone();
        }
    }
}

fn opened(region: &[WeakFocusHandle], window: &mut Window, cx: &mut App) {
    let Some(first) = region.first() else { return };
    let state = state(window, cx);
    let frame = state.borrow().frame;
    if let Some(p) = state.borrow_mut().popups.iter_mut().find(|p| p.region.first() == Some(first)) {
        p.region = region.to_vec();
        p.painted = frame;
        return;
    }
    let previous = match window.focused(cx) {
        Some(f) if painted(&f, window, cx) => Some(f.downgrade()),
        Some(_) => state.borrow().settled.clone(),
        None => None,
    };
    state.borrow_mut().popups.push(Popup { region: region.to_vec(), previous, painted: frame, holding: false });
}

/// After each frame: a request applied, what holds the focus kept, each popup's hold read.
pub(super) fn settle(window: &mut Window, cx: &mut App) {
    let state = state(window, cx);
    let (requested, frame) = {
        let mut s = state.borrow_mut();
        (s.requested.take(), s.frame)
    };
    if let Some((handle, at)) = requested {
        if painted(&handle, window, cx) {
            window.focus(&handle, cx);
        } else if at == frame {
            state.borrow_mut().requested = Some((handle, at));
            window.refresh();
        }
    }
    let focused = window.focused(cx);
    let shown = focused.as_ref().is_some_and(|f| painted(f, window, cx));
    let mut s = state.borrow_mut();
    match focused {
        None => s.settled = None,
        Some(f) if shown => s.settled = Some(f.downgrade()),
        Some(_) => {}
    }
    let (kept, gone): (Vec<Popup>, Vec<Popup>) = std::mem::take(&mut s.popups).into_iter().partition(|p| p.painted + 1 >= frame);
    s.popups = kept;
    for closed in &gone {
        hand_on(closed, &mut s.popups);
    }
    for p in &mut s.popups {
        p.holding = p.painted == frame && p.contains_focused(window, cx);
    }
}

/// What a popup closed holding the focus gives back.
pub(super) enum Given {
    /// What held the focus as it opened, focused again.
    Back,
    /// Nothing held the focus as it opened: only what encloses it may take it.
    Nothing,
}

/// The focus lost with a popup that held it, given back to what held it as it opened; `None` when no closed popup
/// held it or what held it is gone.
pub(super) fn give_back(window: &mut Window, cx: &mut App) -> Option<Given> {
    let state = state(window, cx);
    let frame = state.borrow().frame;
    let closed = {
        let mut s = state.borrow_mut();
        let at = s.popups.iter().rposition(|p| p.holding && p.painted < frame)?;
        let closed = s.popups.remove(at);
        hand_on(&closed, &mut s.popups);
        closed
    };
    let Some(previous) = closed.previous else { return Some(Given::Nothing) };
    let previous = previous.upgrade().filter(|p| painted(p, window, cx))?;
    window.focus(&previous, cx);
    Some(Given::Back)
}

struct Probed(Rc<Cell<bool>>);

impl Global for Probed {}

/// Whether the last frame painted an element tracking `handle`.
pub(super) fn painted(handle: &FocusHandle, window: &mut Window, cx: &mut App) -> bool {
    if !cx.has_global::<Probed>() {
        let seen = Rc::new(Cell::new(false));
        let mark = seen.clone();
        cx.on_action(move |_: &Probe, cx| {
            mark.set(true);
            cx.propagate();
        });
        cx.set_global(Probed(seen));
    }
    let seen = cx.global::<Probed>().0.clone();
    seen.set(false);
    handle.dispatch_action(&Probe, window, cx);
    seen.get()
}

#[cfg(test)]
mod tests;
