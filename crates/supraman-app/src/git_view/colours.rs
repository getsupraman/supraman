//! The Git tab's colours, all from the selected theme (C152): lanes from its accents then its players, and the bands, pill fills
//! and connectors as those lanes at fixed alpha steps over the tab's background.

use gpui::Hsla;
use supraman_ui::themes::{contrast, hex};
use theme::Theme;

/// A lane's alpha over the background for each use.
const BAND: f32 = 0.14;
const PILL: f32 = 0.22;
const CONNECTOR: f32 = 0.7;
const GHOST: f32 = 0.5;
const AVATAR: f32 = 0.25;

#[derive(Clone, Debug)]
pub struct Palette {
    lanes: Vec<Hsla>,
    pub background: Hsla,
    pub text: Hsla,
    pub muted: Hsla,
}

impl Palette {
    pub fn of(theme: &Theme) -> Palette {
        let mut lanes: Vec<Hsla> = Vec::new();
        for colour in theme.accents().0.iter().copied().chain(theme.players().0.iter().map(|p| p.cursor)) {
            if !lanes.iter().any(|l| hex(*l) == hex(colour)) {
                lanes.push(colour);
            }
        }
        if lanes.is_empty() {
            lanes.push(theme.colors().text_accent);
        }
        let c = theme.colors();
        Palette { lanes, background: c.editor_background, text: c.text, muted: c.text.opacity(supraman_ui::themes::SECONDARY) }
    }

    pub fn lane(&self, colour: usize) -> Hsla {
        self.lanes[colour % self.lanes.len()]
    }

    pub fn band(&self, colour: usize) -> Hsla {
        self.lane(colour).opacity(BAND)
    }

    pub fn pill(&self, colour: usize) -> Hsla {
        self.lane(colour).opacity(PILL)
    }

    /// The pill fill over the background, solid: a list that covers the rows below.
    pub fn pill_solid(&self, colour: usize) -> Hsla {
        blend(self.pill(colour), self.background)
    }

    pub fn connector(&self, colour: usize) -> Hsla {
        self.lane(colour).opacity(CONNECTOR)
    }

    pub fn ghost(&self) -> f32 {
        GHOST
    }

    /// An avatar's fill: the lane mixed into the background, solid.
    pub fn avatar(&self, colour: usize) -> Hsla {
        blend(self.lane(colour).opacity(AVATAR), self.background)
    }

    /// The initials on an avatar: the theme's text, or its background where that reads better.
    pub fn initials(&self, colour: usize) -> Hsla {
        let fill = self.avatar(colour);
        if contrast(self.text, fill) >= contrast(self.background, fill) { self.text } else { self.background }
    }
}

pub(super) fn blend(top: Hsla, under: Hsla) -> Hsla {
    let (t, u) = (gpui::Rgba::from(top), gpui::Rgba::from(under));
    let mix = |a: f32, b: f32| a * top.a + b * (1. - top.a);
    gpui::Rgba { r: mix(t.r, u.r), g: mix(t.g, u.g), b: mix(t.b, u.b), a: 1. }.into()
}
