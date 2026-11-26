//! The kit's decisions by role (UI-KIT §3): spacing, radii, text, controls, icons, strokes, motion, layers and the colour roles.

use std::time::Duration;

use gpui::{Pixels, Rems, px, rems};

pub(crate) mod roles;

pub use roles::{Roles, roles};

/// The spacing scale: gaps, paddings and margins.
pub mod space {
    use super::*;

    pub const XS: Pixels = px(2.);
    pub const S: Pixels = px(4.);
    pub const M: Pixels = px(6.);
    pub const L: Pixels = px(8.);
    pub const XL: Pixels = px(12.);
    pub const XXL: Pixels = px(16.);
    pub const XXXL: Pixels = px(20.);
    pub const PAGE: Pixels = px(24.);
}

/// Corner radii.
pub mod radius {
    use super::*;

    /// Buttons, fields, pills, tick boxes.
    pub const S: Pixels = px(4.);
    /// Toolbar buttons, menus, cards.
    pub const M: Pixels = px(6.);
    /// Dialogs.
    pub const L: Pixels = px(8.);
    /// A dot or a round badge.
    pub const FULL: Pixels = px(9999.);
}

/// A text size and its line height (gpui's φ of the size, what every text here lays out at).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Text {
    pub size: Rems,
}

impl Text {
    pub const LINE: f32 = 1.618034;

    pub fn line_height(self) -> Pixels {
        px(self.size.0 * 16. * Self::LINE)
    }
}

/// The text sizes.
pub mod text {
    use super::*;

    /// A menu section's caption.
    pub const CAPTION: Text = Text { size: rems(0.625) };
    /// A row's, a panel's, a toolbar's.
    pub const SMALL: Text = Text { size: rems(0.75) };
    /// Body text: a form's, a question's, a settings page's.
    pub const BODY: Text = Text { size: rems(0.875) };
    pub const TITLE: Text = Text { size: rems(1.) };
    pub const HEADING: Text = Text { size: rems(1.125) };
}

/// A control's height, side padding and text.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Control {
    pub height: Pixels,
    pub padding: Pixels,
    pub text: Text,
}

/// The control sizes.
pub mod control {
    use super::*;

    pub const SMALL: Control = Control { height: px(20.), padding: space::L, text: text::SMALL };
    /// As high as a one-line field.
    pub const MEDIUM: Control = Control { height: px(26.), padding: space::XL, text: text::BODY };
    pub const LARGE: Control = Control { height: px(32.), padding: space::XXL, text: text::BODY };
}

/// Icon sizes, as [`crate::icon::Icon::svg`] takes them.
pub mod icon {
    /// A tick's check.
    pub const XS: f32 = 10.;
    pub const S: f32 = 12.;
    pub const M: f32 = 14.;
    pub const L: f32 = 16.;
}

/// Border and ring widths.
pub mod stroke {
    use super::*;

    pub const HAIRLINE: Pixels = px(1.);
    /// A focus ring, a default answer's, a focused tick box's border.
    pub const FOCUS: Pixels = px(2.);
}

/// Durations.
pub mod motion {
    use super::*;

    pub const FAST: Duration = Duration::from_millis(80);
    pub const NORMAL: Duration = Duration::from_millis(160);
    /// One turn of a turning icon.
    pub const TURN: Duration = Duration::from_secs(1);
    pub const SCROLLBAR_IDLE: Duration = Duration::from_millis(1500);
    pub const CARET_BLINK: Duration = Duration::from_millis(530);
}

/// The deferred layers, lowest first: each draws over every layer before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Layer {
    /// A pill group's list.
    Float = 1,
    /// A side form laid over its view, and its picker.
    Form,
    /// The ref panel's overlay card.
    Card,
    /// The Git tab's graph toast, palette and prompt band.
    OverForm,
    /// A context menu.
    Menu,
    /// The modal dialog.
    Dialog,
}

impl Layer {
    pub const fn priority(self) -> usize {
        self as usize
    }
}

/// The opacity of a control that cannot act now.
pub const DIMMED: f32 = 0.4;

impl From<Text> for gpui::AbsoluteLength {
    fn from(text: Text) -> Self {
        text.size.into()
    }
}

#[cfg(test)]
mod tests;
