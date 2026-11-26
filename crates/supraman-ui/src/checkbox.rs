//! [`Checkbox`]: a tick box and its label as one control.

use std::rc::Rc;

use gpui::{App, ClickEvent, ElementId, FocusHandle, IntoElement, RenderOnce, SharedString, StyleRefinement, Window, div, prelude::*};
use theme::ActiveTheme as _;

use crate::focus_order::Press;
use crate::hit::{HitTargets, hit_target};
use crate::label::{Label, LabelMark};
use crate::tokens;
use crate::tooltip::Tooltip;

type OnToggle = Rc<dyn Fn(&bool, &mut Window, &mut App)>;

/// The square and its label are one hit target under `key` (the square alone `<key>-square`, the words `<key>-label`): a click anywhere on it,
/// Space or Enter toggles it, `on_toggle` given the new value. It takes the keyboard under [`Checkbox::focus`]'s handle
/// (a [`crate::focus_order`] member), else one it keeps, and wears [`crate::focus::ring`] round both while focused by keyboard.
#[derive(IntoElement)]
pub struct Checkbox {
    hits: HitTargets,
    key: SharedString,
    checked: bool,
    label: Option<SharedString>,
    size: Option<tokens::Text>,
    focus: Option<FocusHandle>,
    why: Option<SharedString>,
    tooltip: Option<SharedString>,
    on_toggle: Option<OnToggle>,
    style: StyleRefinement,
}

impl Checkbox {
    pub fn new(hits: &HitTargets, key: impl Into<SharedString>, checked: bool) -> Self {
        Self { hits: hits.clone(), key: key.into(), checked, label: None, size: None, focus: None, why: None, tooltip: None, on_toggle: None, style: StyleRefinement::default() }
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn size(mut self, size: tokens::Text) -> Self {
        self.size = Some(size);
        self
    }

    pub fn focus(mut self, handle: &FocusHandle) -> Self {
        self.focus = Some(handle.clone());
        self
    }

    /// Dimmed with `why` as its tooltip; it does not toggle.
    pub fn disabled(mut self, why: Option<impl Into<SharedString>>) -> Self {
        self.why = why.map(Into::into);
        self
    }

    pub fn tooltip(mut self, words: impl Into<SharedString>) -> Self {
        self.tooltip = Some(words.into());
        self
    }

    pub fn on_toggle(mut self, toggle: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_toggle = Some(Rc::new(toggle));
        self
    }
}

impl RenderOnce for Checkbox {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let key = self.key.clone();
        let focus = self.focus.unwrap_or_else(|| window.use_keyed_state(ElementId::Name(format!("{key}-focus").into()), cx, |_, cx| cx.focus_handle()).read(cx).clone());
        let colors = cx.theme().colors();
        let (on, disabled) = (self.checked, self.why.is_some());
        let hover = if on { colors.text_accent } else { colors.border_selected };
        let square = crate::button::tick(on, colors.text_accent, cx).relative().when(!disabled, |d| d.group_hover(key.clone(), move |s| s.border_color(hover))).child(hit_target(&self.hits, format!("{key}-square")));
        let label = match self.label {
            Some(words) => {
                let words = crate::clipped::clipped(format!("{key}-label"), words, false).flex_shrink(1.).child(hit_target(&self.hits, format!("{key}-label")));
                Label::element(words)
            }
            None => Label::none(),
        };
        let label = label.mark(LabelMark::Element(square.into_any_element())).gap(tokens::space::L).when_some(self.size, |l, s| l.size(s));
        let toggle = self.on_toggle.filter(|_| !disabled);
        let pressed = toggle.clone();
        let row = div()
            .id(ElementId::Name(key.clone()))
            .group(key.clone())
            .flex()
            .flex_none()
            .max_w_full()
            .min_w_0()
            .items_center()
            .rounded(tokens::radius::S)
            .child(label)
            .child(hit_target(&self.hits, key.to_string()));
        let row = match (&self.why, self.tooltip) {
            (Some(why), _) => row.opacity(tokens::DIMMED).cursor_default().tooltip(Tooltip::text(why.clone())),
            (None, Some(words)) => row.cursor_pointer().tooltip(Tooltip::text(words)),
            (None, None) => row.cursor_pointer(),
        };
        let row = row.on_click(move |_: &ClickEvent, window, cx| {
            cx.stop_propagation();
            if let Some(toggle) = &toggle {
                toggle(&!on, window, cx);
            }
        });
        let mut row = crate::focus_order::pressable(row, &focus, move |_: &Press, window, cx| {
            if let Some(toggle) = &pressed {
                toggle(&!on, window, cx);
            }
        });
        row.style().refine(&self.style);
        row
    }
}

impl Styled for Checkbox {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

#[cfg(test)]
mod tests;
