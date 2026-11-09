//! Marks and text on one visual centre. gpui centres a font's ascent and descent in the line box, so Segoe UI's
//! letters sit below the line's middle, where `items_center` puts an icon or a tick box beside them.

use gpui::{AnyElement, App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement, LayoutId, Pixels, TextStyle, Window, point, px};

/// How far the letters' middle (halfway between the x-height's and the cap height's) sits below the middle of a
/// line of `style`, in whole device pixels.
pub fn offset(style: &TextStyle, window: &Window) -> Pixels {
    let size = style.font_size.to_pixels(window.rem_size());
    let text = window.text_system();
    let font = text.resolve_font(&style.font());
    let ascent = text.ascent(font, size);
    let descent = text.descent(font, size).abs();
    let letters = (text.cap_height(font, size) + text.x_height(font, size)) / 2.;
    let scale = window.scale_factor();
    ((ascent - descent - letters) / 2. * scale).round() / scale
}

/// Whether two vertical centres are one, within the quarter pixel a line box's fractional height rounds by.
pub fn level(a: Pixels, b: Pixels) -> bool {
    (a - b).abs() <= px(0.25)
}

/// The offset of the text around the element being laid out.
pub fn ambient(window: &Window) -> Pixels {
    offset(&window.text_style(), window)
}

/// `child`, a mark beside text (an icon, a tick box), moved down onto the letters' middle.
pub fn mark(child: impl IntoElement) -> Ink {
    Ink { child: child.into_any_element(), down: true }
}

/// `child`, text centred in its box (a pill's name), moved up so its letters are.
pub fn text(child: impl IntoElement) -> Ink {
    Ink { child: child.into_any_element(), down: false }
}

/// A child laid out in place and painted moved by the text's [`offset`]; its hit targets move with it.
pub struct Ink {
    child: AnyElement,
    down: bool,
}

impl IntoElement for Ink {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Ink {
    type RequestLayoutState = Pixels;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, window: &mut Window, cx: &mut App) -> (LayoutId, Pixels) {
        let shift = ambient(window);
        (self.child.request_layout(window, cx), if self.down { shift } else { -shift })
    }

    fn prepaint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, _: Bounds<Pixels>, shift: &mut Pixels, window: &mut Window, cx: &mut App) {
        window.with_element_offset(point(px(0.), *shift), |window| self.child.prepaint(window, cx));
    }

    fn paint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, _: Bounds<Pixels>, _: &mut Pixels, _: &mut (), window: &mut Window, cx: &mut App) {
        self.child.paint(window, cx);
    }
}
