use gpui::{App, Global, Hsla};
use theme::{ActiveTheme as _, Theme};

use crate::themes;

/// The colour roles the kit paints with, derived from the theme once per load ([`themes::apply_resolved`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Roles {
    pub surface: Hsla,
    pub surface_raised: Hsla,
    pub surface_sunken: Hsla,
    pub border_subtle: Hsla,
    pub border_strong: Hsla,
    pub ring: Hsla,
    pub accent: Hsla,
    pub on_accent: Hsla,
    pub danger: Hsla,
    pub warning: Hsla,
    pub success: Hsla,
    pub info: Hsla,
    pub selection: Hsla,
    pub selection_inactive: Hsla,
    pub scrollbar_thumb: Hsla,
    pub scrollbar_thumb_hover: Hsla,
    pub scrollbar_thumb_active: Hsla,
}

impl Roles {
    pub fn of(theme: &Theme) -> Self {
        let (colors, status) = (theme.colors(), theme.status());
        Self {
            surface: colors.surface_background,
            surface_raised: colors.elevated_surface_background,
            surface_sunken: colors.editor_background,
            border_subtle: colors.border_variant,
            border_strong: colors.border,
            ring: colors.border_focused,
            accent: colors.text_accent,
            on_accent: themes::on(colors.text_accent),
            danger: status.error,
            warning: status.warning,
            success: status.success,
            info: status.info,
            selection: themes::selection(colors, true),
            selection_inactive: themes::selection(colors, false),
            scrollbar_thumb: colors.scrollbar_thumb_background,
            scrollbar_thumb_hover: colors.scrollbar_thumb_hover_background,
            scrollbar_thumb_active: colors.scrollbar_thumb_active_background,
        }
    }
}

struct Derived {
    theme: String,
    roles: Roles,
}

impl Global for Derived {}

/// Derives the roles of the theme just applied.
pub(crate) fn derive(cx: &mut App) {
    let theme = cx.theme().clone();
    cx.set_global(Derived { theme: theme.id.clone(), roles: Roles::of(&theme) });
}

/// The roles of the theme painting now.
pub fn roles(cx: &App) -> Roles {
    let theme = cx.theme();
    match cx.try_global::<Derived>() {
        Some(d) if d.theme == theme.id => d.roles,
        _ => Roles::of(theme),
    }
}
