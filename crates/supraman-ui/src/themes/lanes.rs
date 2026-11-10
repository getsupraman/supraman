//! The Git tab's lane palette: Supraman's ten colours, a theme's `gitman.lane.0`–`9` over them, held to the lane floors.

use std::{collections::HashMap, sync::Mutex};

use gpui::Hsla;
use theme::Theme;

use super::{Adjustment, NAME_FLOOR, SECONDARY_FLOOR, contrast, hex, nearest_holding, over, parse_color};

pub const LANES: usize = 10;

/// The theme keys, in lane order.
pub const KEYS: [&str; LANES] = [
    "gitman.lane.0", "gitman.lane.1", "gitman.lane.2", "gitman.lane.3", "gitman.lane.4",
    "gitman.lane.5", "gitman.lane.6", "gitman.lane.7", "gitman.lane.8", "gitman.lane.9",
];

/// Cyan, blue, violet, magenta, red, orange, amber, lime, green, teal.
pub const DARK: [&str; LANES] = ["#4CC4EE", "#5B8DEF", "#9D7CF2", "#D86BC9", "#EB6A6A", "#EE9550", "#E5C04A", "#9BCF56", "#4FC48A", "#2FB5A3"];
pub const LIGHT: [&str; LANES] = ["#0A86AD", "#2D63D6", "#7446D4", "#B4359F", "#CC3B3B", "#BD5A0E", "#9A7300", "#5A8A12", "#198A55", "#0B7F76"];

/// A lane's alpha over the background: the row band, a ref pill, an avatar's fill.
pub const BAND: f32 = 0.14;
pub const PILL: f32 = 0.22;
pub const AVATAR: f32 = 0.25;
/// A lane's line and node against the background (WCAG 1.4.11, non-text).
pub const LINE_FLOOR: f32 = 3.;

/// Each fitted theme's lanes by `Theme::id`.
static FITTED: Mutex<Option<HashMap<String, [Hsla; LANES]>>> = Mutex::new(None);

/// The lanes `theme` paints with: its fitted ones, else Supraman's for its appearance.
pub fn lanes(theme: &Theme) -> [Hsla; LANES] {
    let fitted = FITTED.lock().unwrap_or_else(|e| e.into_inner());
    fitted.as_ref().and_then(|m| m.get(theme.id.as_str()).copied()).unwrap_or_else(|| defaults(theme))
}

pub fn defaults(theme: &Theme) -> [Hsla; LANES] {
    let values = if theme.appearance.is_light() { LIGHT } else { DARK };
    values.map(|v| parse_color(v).expect("the shipped lanes parse"))
}

/// The lane a theme key sets.
pub fn key_lane(key: &str) -> Option<usize> {
    KEYS.iter().position(|k| *k == key)
}

/// `colour` at `alpha` over `under`, opaque.
pub fn blend(colour: Hsla, alpha: f32, under: Hsla) -> Hsla {
    over(colour.opacity(alpha), under).into()
}

/// The initials on an avatar `fill`: the theme's text, or its background where that reads better.
pub fn initials(text: Hsla, background: Hsla, fill: Hsla) -> Hsla {
    if contrast(text, fill) >= contrast(background, fill) { text } else { background }
}

/// The lowest of a lane's floor margins (ratio over floor): under 1 fails one.
pub fn margin(theme: &Theme, lane: Hsla) -> f32 {
    let (text, bg) = (theme.colors().text, theme.colors().editor_background);
    let avatar = blend(lane, AVATAR, bg);
    [
        contrast(lane, bg) / LINE_FLOOR,
        contrast(text, blend(lane, BAND, bg)) / NAME_FLOOR,
        contrast(text, blend(lane, PILL, bg)) / SECONDARY_FLOOR,
        contrast(initials(text, bg, avatar), avatar) / SECONDARY_FLOOR,
    ]
    .into_iter()
    .fold(f32::INFINITY, f32::min)
}

/// Supraman's lanes with `set`'s overrides, each failing lane moved to the nearest lightness that holds
/// every floor, kept for `theme`. Returns what changed.
pub fn fit(theme: &Theme, set: &[Option<Hsla>; LANES]) -> Vec<Adjustment> {
    let mut lanes = defaults(theme);
    let mut adjusted = Vec::new();
    for (i, lane) in lanes.iter_mut().enumerate() {
        let old = set[i].map(|c| Hsla { a: 1., ..c }).unwrap_or(*lane);
        *lane = nearest_holding(old, |c| margin(theme, c) >= 1.).unwrap_or(old);
        if hex(old) != hex(*lane) {
            let line = |c: Hsla| contrast(c, theme.colors().editor_background);
            adjusted.push(Adjustment { key: KEYS[i].into(), old: hex(old), new: hex(*lane), was: line(old), now: line(*lane) });
        }
    }
    FITTED.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_default().insert(theme.id.clone(), lanes);
    adjusted
}

/// Keeps `from`'s lanes for the same theme under its new id.
pub(super) fn carry(from: &str, to: &str) {
    let mut fitted = FITTED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(m) = fitted.as_mut()
        && let Some(lanes) = m.get(from).copied()
    {
        m.insert(to.to_owned(), lanes);
    }
}

#[cfg(test)]
mod tests {
    use super::super::{TerminalColors, ZedTheme, dark_modern, from_zed, one_dark, supraman_dark, with_terminal};
    use super::*;

    fn zed(name: &str, appearance: &str, style: serde_json::Value) -> ZedTheme {
        ZedTheme { name: name.into(), appearance: Some(appearance.into()), style: style.as_object().unwrap().clone() }
    }

    fn light() -> Theme {
        let style = serde_json::json!({ "background": "#F3F3F3", "panel.background": "#F3F3F3", "editor.background": "#FFFFFF", "text": "#1F1F1F" });
        from_zed(&one_dark(), &zed("Light lanes", "light", style)).theme
    }

    #[test]
    fn dark_and_light_themes_get_ten_distinct_lanes_that_hold_the_floors() {
        for (theme, shipped) in [(dark_modern().theme, DARK), (supraman_dark().theme, DARK), (light(), LIGHT)] {
            let lanes = lanes(&theme);
            assert_eq!(lanes.map(hex), shipped, "{}: Supraman's own lanes, none moved", theme.name);
            let distinct: std::collections::HashSet<String> = lanes.iter().map(|l| hex(*l)).collect();
            assert_eq!(distinct.len(), LANES, "{}", theme.name);
            for (i, lane) in lanes.iter().enumerate() {
                assert!(margin(&theme, *lane) >= 1., "{} lane {i} {}: {}", theme.name, hex(*lane), margin(&theme, *lane));
            }
        }
    }

    #[test]
    fn a_theme_sets_its_lanes_with_gitman_lane_keys() {
        let style = serde_json::json!({
            "gitman.lane.3": "#FF8800",
            "gitman.lane.5": "#2A2A2A",
            "gitman.lane.6": null,
            "gitman.lane.7": "orange",
            "gitman.lane.10": "#FF8800",
        });
        let loaded = from_zed(&one_dark(), &zed("Lanes", "dark", style));
        let lanes = lanes(&loaded.theme);
        assert_eq!(hex(lanes[3]), "#FF8800", "the theme's lane");
        assert_eq!([lanes[6], lanes[7]].map(hex), [DARK[6], DARK[7]], "null and a bad value keep Supraman's");
        let mut ignored = loaded.ignored.clone();
        ignored.sort();
        assert_eq!(ignored, ["gitman.lane.10", "gitman.lane.7"]);
        let lifted = loaded.adjusted.iter().find(|a| a.key == "gitman.lane.5").expect("a lane too dark to see is lifted");
        assert_eq!(lifted.old, "#2A2A2A");
        assert!(margin(&loaded.theme, lanes[5]) >= 1. && lifted.now >= LINE_FLOOR, "{lifted:?}");
        let campbell = with_terminal(loaded.theme.clone(), TerminalColors::Campbell);
        assert_ne!(campbell.id, loaded.theme.id);
        assert_eq!(super::lanes(&campbell).map(hex), lanes.map(hex), "a new id keeps the theme's lanes");
    }
}
