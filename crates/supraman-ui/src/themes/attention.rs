//! The attention fill: a prompt band's background, Supraman's indigo or a theme's `supraman.attention`, held so the theme's text on it holds `NAME_FLOOR` and it stands off the editor background.

use gpui::Hsla;
use theme::Theme;

use super::{Adjustment, Kept, NAME_FLOOR, contrast, hex, nearest_holding, parse_color};

pub const KEY: &str = "supraman.attention";

/// Indigo: away from the warning's amber, the info's blue and the selection's.
pub const DARK: &str = "#342E7F";
pub const LIGHT: &str = "#DCD4FF";

/// The least contrast of the fill against the editor background, so the band stands out.
pub const SHOWS: f32 = 1.2;

static FITTED: Kept<Hsla> = Kept::new();

/// The fill `theme` paints a band with: its fitted one, else Supraman's for its appearance.
pub fn attention(theme: &Theme) -> Hsla {
    FITTED.get(&theme.id).unwrap_or_else(|| default(theme))
}

pub fn default(theme: &Theme) -> Hsla {
    parse_color(if theme.appearance.is_light() { LIGHT } else { DARK }).expect("the shipped attention parses")
}

/// Whether the theme's text reads on `fill` and `fill` shows on the editor background.
pub fn holds(theme: &Theme, fill: Hsla) -> bool {
    let c = theme.colors();
    contrast(c.text, fill) >= NAME_FLOOR && contrast(fill, c.editor_background) >= SHOWS
}

/// Supraman's fill or `set`, moved to the nearest lightness that [`holds`], kept for `theme`. Returns what changed.
pub fn fit(theme: &Theme, set: Option<Hsla>) -> Option<Adjustment> {
    let old = set.map_or_else(|| default(theme), |c| Hsla { a: 1., ..c });
    let fill = nearest_holding(old, |c| holds(theme, c)).unwrap_or(old);
    FITTED.keep(&theme.id, fill);
    let text = |c: Hsla| contrast(theme.colors().text, c);
    (hex(old) != hex(fill)).then(|| Adjustment { key: KEY.into(), old: hex(old), new: hex(fill), was: text(old), now: text(fill) })
}

pub(super) fn carry(from: &str, to: &str) {
    FITTED.carry(from, to);
}

#[cfg(test)]
mod tests {
    use super::super::{TerminalColors, ZedTheme, dark_modern, from_zed, one_dark, parse_family, selftest_light, supraman_dark, with_terminal};
    use super::*;

    fn zed(style: serde_json::Value) -> ZedTheme {
        ZedTheme { name: "Attention".into(), appearance: Some("dark".into()), style: style.as_object().unwrap().clone() }
    }

    #[test]
    fn the_fill_holds_the_text_floor_and_shows_in_dark_and_light_and_is_not_the_warning() {
        let light = from_zed(&one_dark(), &parse_family(selftest_light::LIGHT).unwrap().themes[0]).theme;
        for theme in [dark_modern().theme, supraman_dark().theme, light] {
            let fill = attention(&theme);
            assert!(holds(&theme, fill), "{} {}", theme.name, hex(fill));
            assert_eq!(hex(fill), if theme.appearance.is_light() { LIGHT } else { DARK }, "{}: Supraman's fill holds as shipped", theme.name);
            assert_ne!(hex(fill), hex(theme.status().warning), "{}: not the warning colour", theme.name);
        }
    }

    #[test]
    fn a_theme_sets_its_attention_and_one_too_light_for_its_text_is_moved() {
        let dark = from_zed(&one_dark(), &zed(serde_json::json!({ KEY: "#9A90FF" })));
        let moved = dark.adjusted.iter().find(|a| a.key == KEY).expect("a fill too light for the text moves");
        assert_eq!(moved.old, "#9A90FF");
        assert!(holds(&dark.theme, attention(&dark.theme)) && moved.now >= NAME_FLOOR, "{moved:?}");
        let campbell = with_terminal(dark.theme.clone(), TerminalColors::Campbell);
        assert_eq!(hex(attention(&campbell)), hex(attention(&dark.theme)), "a new id keeps the fill");
        assert!(from_zed(&one_dark(), &zed(serde_json::json!({ KEY: "violet" }))).ignored.contains(&KEY.to_string()));
    }
}
