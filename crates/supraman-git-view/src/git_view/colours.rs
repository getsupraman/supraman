//! The Git tab's colours, all from the selected theme (C152): its lanes ([`supraman_ui::themes::lanes`]), and the bands, pill
//! fills and connectors as those lanes at fixed alpha steps over the tab's background.

use gpui::Hsla;
use supraman_ui::themes::lanes::{self, AVATAR, BAND, PILL};
use theme::Theme;

const _: () = assert!(lanes::LANES == supraman_gitman::LANE_COLOURS);

const CONNECTOR: f32 = 0.7;
const GHOST: f32 = 0.5;

#[derive(Clone, Debug)]
pub(crate) struct Palette {
    lanes: [Hsla; lanes::LANES],
    pub(crate) background: Hsla,
    pub(crate) text: Hsla,
    pub(crate) muted: Hsla,
}

impl Palette {
    pub(crate) fn of(theme: &Theme) -> Palette {
        let c = theme.colors();
        Palette { lanes: lanes::lanes(theme), background: c.editor_background, text: c.text, muted: supraman_ui::themes::secondary(c) }
    }

    pub(crate) fn lane(&self, colour: usize) -> Hsla {
        self.lanes[colour % lanes::LANES]
    }

    pub(crate) fn band(&self, colour: usize) -> Hsla {
        self.lane(colour).opacity(BAND)
    }

    pub(crate) fn pill(&self, colour: usize) -> Hsla {
        self.lane(colour).opacity(PILL)
    }

    /// The pill fill over the background, solid: a list that covers the rows below.
    pub(crate) fn pill_solid(&self, colour: usize) -> Hsla {
        blend(self.pill(colour), self.background)
    }

    pub(crate) fn connector(&self, colour: usize) -> Hsla {
        self.lane(colour).opacity(CONNECTOR)
    }

    pub(crate) fn ghost(&self) -> f32 {
        GHOST
    }

    /// An avatar's fill: the lane mixed into the background, solid.
    pub(crate) fn avatar(&self, colour: usize) -> Hsla {
        lanes::blend(self.lane(colour), AVATAR, self.background)
    }

    /// The initials on an avatar: the theme's text, or its background where that reads better.
    pub(crate) fn initials(&self, colour: usize) -> Hsla {
        lanes::initials(self.text, self.background, self.avatar(colour))
    }
}

pub(super) fn blend(top: Hsla, under: Hsla) -> Hsla {
    lanes::blend(top, 1., under)
}
