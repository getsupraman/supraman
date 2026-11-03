//! Themes: Zed theme JSON (`https://zed.dev/schema/themes/v0.2.0.json`) read tolerantly over a
//! base theme, the bundled VS Code Dark Modern (the default) and Supraman Dark (One Dark with Campbell's
//! ANSI colours), files in `<SUPRAMAN_HOME>\themes\*.json`, and the terminal colours
//! switch. Everything the app paints reads `cx.theme()`, so [`apply`] is the one place a theme changes.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use supraman_term::terminal_settings::{TerminalConfig, TerminalPalette, campbell};
use gpui::{App, BorrowAppContext as _, Hsla, Rgba};
use serde::Deserialize;
use serde_json::Value;
use theme::{AccentColors, Appearance, StatusColors, Theme, ThemeColors};

pub const DARK_MODERN: &str = "VS Code Dark Modern";
pub const SUPRAMAN_DARK: &str = "Supraman Dark";
/// The name an older `settings.toml` may hold for [`SUPRAMAN_DARK`].
pub const SUPRAMAN_DARK_LEGACY: &str = "Foreman Dark";
pub const DEFAULT_THEME: &str = DARK_MODERN;

/// kevcamel/vscode_dark_modern.zed (MIT, © 2024 Kevin Camellini), `themes/vscode-dark-modern.json` as of
/// its 2026-09-06 commit; its licence ships next to it and in the release (`THIRD_PARTY.md`).
pub const DARK_MODERN_JSON: &str = include_str!("../assets/themes/vscode-dark-modern.json");
pub const DARK_MODERN_LICENSE: &str = include_str!("../assets/themes/vscode-dark-modern.LICENSE");

/// The tree's text against the panel (m5/26, LEARNINGS 2026-10-27): blends of the theme's text. A name
/// in [`QUIET`] keeps >= [`NAME_FLOOR`]:1 and secondary text in [`SECONDARY`] >= [`SECONDARY_FLOOR`]:1
/// (WCAG); a row faded to [`DIMMED`] (a foreign or gone session) keeps >= 4.5 and >= 3.
pub const QUIET: f32 = 0.78;
pub const SECONDARY: f32 = 0.62;
pub const DIMMED: f32 = 0.8;
pub const NAME_FLOOR: f32 = 7.;
pub const SECONDARY_FLOOR: f32 = 4.5;

/// A Messages row's tint over the panel (its status colour at this alpha), and under the pointer.
pub const MESSAGE_TINT: f32 = 0.10;
pub const MESSAGE_TINT_HOVER: f32 = 0.16;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TerminalColors {
    /// ANSI 0-15 from the theme.
    #[default]
    Theme,
    /// Windows Terminal's Campbell for ANSI 0-15; foreground, background and cursor stay the theme's.
    Campbell,
}

impl TerminalColors {
    /// `settings.toml`'s `terminal_colors`: "campbell", else the theme's.
    pub fn from_setting(value: Option<&str>) -> Self {
        match value {
            Some(v) if v.eq_ignore_ascii_case("campbell") => Self::Campbell,
            _ => Self::Theme,
        }
    }

    pub fn setting(self) -> Option<String> {
        match self {
            Self::Theme => None,
            Self::Campbell => Some("campbell".into()),
        }
    }
}

/// A Zed theme family file. Every field is optional and unknown keys are ignored.
#[derive(Debug, Default, Deserialize)]
pub struct ZedThemeFamily {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub themes: Vec<ZedTheme>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct ZedTheme {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub appearance: Option<String>,
    #[serde(default)]
    pub style: serde_json::Map<String, Value>,
}

pub fn parse_family(json: &str) -> Result<ZedThemeFamily, serde_json::Error> {
    serde_json::from_str(json)
}

/// `#RRGGBB` or `#RRGGBBAA` (any case); anything else is no colour.
pub fn parse_color(text: &str) -> Option<Hsla> {
    let hex = text.trim().strip_prefix('#')?;
    if !(hex.len() == 6 || hex.len() == 8) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok().map(|b| b as f32 / 255.);
    let a = if hex.len() == 8 { byte(6)? } else { 1. };
    Some(Rgba { r: byte(0)?, g: byte(2)?, b: byte(4)?, a }.into())
}

/// `#RRGGBB`, or `#RRGGBBAA` when not opaque.
pub fn hex(color: Hsla) -> String {
    let c = Rgba::from(color);
    let b = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
    if b(c.a) == 255 {
        format!("#{:02X}{:02X}{:02X}", b(c.r), b(c.g), b(c.b))
    } else {
        format!("#{:02X}{:02X}{:02X}{:02X}", b(c.r), b(c.g), b(c.b), b(c.a))
    }
}

/// Zed's key with `.` for `_` is the field's name (`text.muted` → `text_muted`).
macro_rules! fields {
    ($fn:ident, $ty:ty, [$($f:ident),* $(,)?]) => {
        fn $fn<'a>(c: &'a mut $ty, field: &str) -> Option<&'a mut Hsla> {
            Some(match field {
                $(stringify!($f) => &mut c.$f,)*
                _ => return None,
            })
        }
    };
}

fields!(color_field, ThemeColors, [
    border, border_variant, border_focused, border_selected, border_transparent, border_disabled,
    elevated_surface_background, surface_background, background, element_background, element_hover,
    element_active, element_selected, element_selection_background, element_disabled, drop_target_background,
    drop_target_border, ghost_element_background, ghost_element_hover, ghost_element_active,
    ghost_element_selected, ghost_element_disabled, text, text_muted, text_placeholder, text_disabled,
    text_accent, icon, icon_muted, icon_disabled, icon_placeholder, icon_accent, status_bar_background,
    title_bar_background, title_bar_inactive_background, toolbar_background, tab_bar_background,
    tab_inactive_background, tab_active_background, search_match_background, search_active_match_background,
    panel_background, panel_focused_border, panel_indent_guide, panel_indent_guide_hover,
    panel_indent_guide_active, pane_focused_border, pane_group_border, scrollbar_thumb_background,
    scrollbar_thumb_hover_background, scrollbar_thumb_active_background, scrollbar_thumb_border,
    scrollbar_track_background, scrollbar_track_border, editor_foreground, editor_background,
    editor_gutter_background, editor_subheader_background, editor_active_line_background,
    editor_highlighted_line_background, editor_line_number, editor_active_line_number, editor_invisible,
    editor_wrap_guide, editor_active_wrap_guide, editor_indent_guide, editor_indent_guide_active,
    terminal_background, terminal_foreground, terminal_bright_foreground, terminal_dim_foreground,
    terminal_ansi_background, terminal_ansi_black, terminal_ansi_bright_black, terminal_ansi_dim_black,
    terminal_ansi_red, terminal_ansi_bright_red, terminal_ansi_dim_red, terminal_ansi_green,
    terminal_ansi_bright_green, terminal_ansi_dim_green, terminal_ansi_yellow, terminal_ansi_bright_yellow,
    terminal_ansi_dim_yellow, terminal_ansi_blue, terminal_ansi_bright_blue, terminal_ansi_dim_blue,
    terminal_ansi_magenta, terminal_ansi_bright_magenta, terminal_ansi_dim_magenta, terminal_ansi_cyan,
    terminal_ansi_bright_cyan, terminal_ansi_dim_cyan, terminal_ansi_white, terminal_ansi_bright_white,
    terminal_ansi_dim_white, link_text_hover, version_control_added, version_control_deleted,
    version_control_modified, version_control_renamed, version_control_conflict, version_control_ignored,
]);

fields!(status_field, StatusColors, [
    conflict, conflict_background, conflict_border, created, created_background, created_border, deleted,
    deleted_background, deleted_border, error, error_background, error_border, hidden, hidden_background,
    hidden_border, hint, hint_background, hint_border, ignored, ignored_background, ignored_border, info,
    info_background, info_border, modified, modified_background, modified_border, predictive,
    predictive_background, predictive_border, renamed, renamed_background, renamed_border, success,
    success_background, success_border, unreachable, unreachable_background, unreachable_border, warning,
    warning_background, warning_border,
]);

/// A key the loader changed so the m5/26 floors hold.
#[derive(Clone, Debug, PartialEq)]
pub struct Adjustment {
    pub key: &'static str,
    pub old: String,
    pub new: String,
    /// The contrast before and after, against the darkest-contrast surface.
    pub was: f32,
    pub now: f32,
}

#[derive(Clone, Debug)]
pub struct Loaded {
    pub theme: Theme,
    /// Style keys that set a colour.
    pub applied: usize,
    /// Keys with a value that is neither a colour nor null, or that Foreman does not know.
    pub ignored: Vec<String>,
    pub adjusted: Vec<Adjustment>,
}

static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Each built theme gets its own id, so a cache keyed on it ([`crate::term_element`]) never serves a
/// palette of the theme it replaced.
fn fresh_id(name: &str) -> String {
    format!("{name}#{}", GENERATION.fetch_add(1, Ordering::Relaxed))
}

/// One Dark: the base every loaded key falls back to.
pub fn one_dark() -> Theme {
    let registry = theme::ThemeRegistry::default();
    let theme = registry.get(theme::DEFAULT_DARK_THEME).expect("the fallback theme is One Dark");
    (*theme).clone()
}

/// Supraman Dark: One Dark with Campbell's ANSI colours; only the backgrounds of the
/// marked row and the search matches are faded to [`BACKGROUND_FLOORS`].
pub fn supraman_dark() -> Loaded {
    let mut theme = one_dark();
    theme.name = SUPRAMAN_DARK.into();
    theme.id = fresh_id(SUPRAMAN_DARK);
    set_ansi(&mut theme.styles.colors, campbell());
    let mut adjusted = fit_backgrounds(&mut theme);
    adjusted.extend(fit_placeholder(&mut theme));
    Loaded { theme, applied: 0, ignored: Vec::new(), adjusted }
}

pub fn dark_modern() -> Loaded {
    let family = parse_family(DARK_MODERN_JSON).expect("the bundled theme parses");
    let mut loaded = from_zed(&one_dark(), &family.themes[0]);
    loaded.theme.name = DARK_MODERN.into();
    loaded.theme.id = fresh_id(DARK_MODERN);
    loaded
}

/// `zed` over `base`: each key that holds a colour replaces the base's, every other key keeps the base's
/// (terminal keys fall back to the theme's own terminal colours first), then [`fit_floors`].
pub fn from_zed(base: &Theme, zed: &ZedTheme) -> Loaded {
    let mut theme = base.clone();
    theme.name = zed.name.clone().into();
    theme.id = fresh_id(&zed.name);
    theme.appearance = match zed.appearance.as_deref() {
        Some("light") => Appearance::Light,
        _ => Appearance::Dark,
    };
    let (mut applied, mut ignored, mut set) = (0, Vec::new(), HashSet::new());
    let styles = &mut theme.styles;
    for (key, value) in &zed.style {
        match key.as_str() {
            "players" => {
                if let Some(local) = value.as_array().and_then(|a| a.first()) {
                    let mut players = styles.player.0.clone();
                    let mut first = players[0];
                    for (part, slot) in [("cursor", &mut first.cursor), ("background", &mut first.background), ("selection", &mut first.selection)] {
                        if let Some(color) = local.get(part).and_then(Value::as_str).and_then(parse_color) {
                            *slot = color;
                            applied += 1;
                            set.insert(format!("players.{part}"));
                        }
                    }
                    players[0] = first;
                    styles.player = theme::PlayerColors(players);
                }
            }
            "accents" => {
                let colors: Vec<Hsla> = value.as_array().into_iter().flatten().filter_map(|v| v.as_str().and_then(parse_color)).collect();
                if !colors.is_empty() {
                    styles.accents = AccentColors(colors.into());
                    applied += 1;
                }
            }
            // known, and nothing Foreman paints
            "syntax" | "background.appearance" => {}
            _ => {
                let field = key.replace('.', "_");
                let slot = color_field(&mut styles.colors, &field).or_else(|| status_field(&mut styles.status, &field));
                match (slot, value) {
                    (Some(_), Value::Null) => {}
                    (Some(slot), Value::String(text)) if parse_color(text).is_some() => {
                        *slot = parse_color(text).unwrap();
                        applied += 1;
                        set.insert(field);
                    }
                    _ => ignored.push(key.clone()),
                }
            }
        }
    }
    let colors = &mut styles.colors;
    let has = |k: &str| set.contains(k);
    if has("terminal_background") && !has("terminal_ansi_background") {
        colors.terminal_ansi_background = colors.terminal_background;
    }
    if has("terminal_foreground") {
        if !has("terminal_bright_foreground") {
            colors.terminal_bright_foreground = colors.terminal_foreground;
        }
        if !has("terminal_dim_foreground") {
            colors.terminal_dim_foreground = dim(colors.terminal_foreground);
        }
    }
    let names = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"];
    for name in names {
        if has(&format!("terminal_ansi_{name}")) && !has(&format!("terminal_ansi_dim_{name}")) {
            let normal = *color_field(colors, &format!("terminal_ansi_{name}")).unwrap();
            *color_field(colors, &format!("terminal_ansi_dim_{name}")).unwrap() = dim(normal);
        }
    }
    if !has("element_selection_background") && has("players.selection") {
        let mut selection = styles.player.local().selection;
        if selection.a == 1. {
            selection.a = 0.25;
        }
        styles.colors.element_selection_background = selection;
    }
    let adjusted = fit_floors(&mut theme);
    Loaded { theme, applied, ignored, adjusted }
}

fn dim(mut color: Hsla) -> Hsla {
    color.l *= 0.7;
    color
}

/// ANSI 0-15 (black..white, then the bright ones) and the dims derived from them.
pub fn set_ansi(c: &mut ThemeColors, ansi: [Hsla; 16]) {
    [
        c.terminal_ansi_black, c.terminal_ansi_red, c.terminal_ansi_green, c.terminal_ansi_yellow,
        c.terminal_ansi_blue, c.terminal_ansi_magenta, c.terminal_ansi_cyan, c.terminal_ansi_white,
        c.terminal_ansi_bright_black, c.terminal_ansi_bright_red, c.terminal_ansi_bright_green,
        c.terminal_ansi_bright_yellow, c.terminal_ansi_bright_blue, c.terminal_ansi_bright_magenta,
        c.terminal_ansi_bright_cyan, c.terminal_ansi_bright_white,
    ] = ansi;
    [
        c.terminal_ansi_dim_black, c.terminal_ansi_dim_red, c.terminal_ansi_dim_green, c.terminal_ansi_dim_yellow,
        c.terminal_ansi_dim_blue, c.terminal_ansi_dim_magenta, c.terminal_ansi_dim_cyan, c.terminal_ansi_dim_white,
    ] = std::array::from_fn(|i| dim(ansi[i]));
}

/// The terminal colours of `theme`: what the emulator answers OSC colour queries with and what the grid
/// painter maps ANSI colours through.
pub fn palette(theme: &Theme) -> TerminalPalette {
    let c = theme.colors();
    TerminalPalette {
        ansi: [
            c.terminal_ansi_black, c.terminal_ansi_red, c.terminal_ansi_green, c.terminal_ansi_yellow,
            c.terminal_ansi_blue, c.terminal_ansi_magenta, c.terminal_ansi_cyan, c.terminal_ansi_white,
            c.terminal_ansi_bright_black, c.terminal_ansi_bright_red, c.terminal_ansi_bright_green,
            c.terminal_ansi_bright_yellow, c.terminal_ansi_bright_blue, c.terminal_ansi_bright_magenta,
            c.terminal_ansi_bright_cyan, c.terminal_ansi_bright_white,
        ],
        dim: [
            c.terminal_ansi_dim_black, c.terminal_ansi_dim_red, c.terminal_ansi_dim_green, c.terminal_ansi_dim_yellow,
            c.terminal_ansi_dim_blue, c.terminal_ansi_dim_magenta, c.terminal_ansi_dim_cyan, c.terminal_ansi_dim_white,
        ],
        foreground: c.terminal_foreground,
        background: c.terminal_background,
        bright_foreground: c.terminal_bright_foreground,
        cursor: theme.players().local().cursor,
    }
}

/// WCAG 2 contrast of `fg` (its alpha blended over `bg`) against the opaque `bg`.
pub fn contrast(fg: Hsla, bg: Hsla) -> f32 {
    let (lf, lb) = (luminance(over(fg, bg)), luminance(Rgba::from(bg)));
    (lf.max(lb) + 0.05) / (lf.min(lb) + 0.05)
}

fn over(fg: Hsla, bg: Hsla) -> Rgba {
    let (f, b) = (Rgba::from(fg), Rgba::from(bg));
    let blend = |f: f32, b: f32| f * fg.a + b * (1. - fg.a);
    Rgba { r: blend(f.r, b.r), g: blend(f.g, b.g), b: blend(f.b, b.b), a: 1. }
}

fn luminance(c: Rgba) -> f32 {
    let lin = |c: f32| if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
    0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
}

/// The surfaces text sits on: the tree's panel, the window, the tabs' content.
pub fn surfaces(theme: &Theme) -> [Hsla; 3] {
    let c = theme.colors();
    [c.panel_background, c.background, c.editor_background]
}

/// The lowest contrast of `fg` at `alpha` over `on`.
pub fn worst_contrast(theme: &Theme, fg: Hsla, alpha: f32, on: On) -> f32 {
    let panel = [theme.colors().panel_background];
    let all = surfaces(theme);
    let bgs: &[Hsla] = if on == On::Panel { &panel } else { &all };
    bgs.iter().map(|bg| contrast(fg.opacity(alpha), *bg)).fold(f32::INFINITY, f32::min)
}

/// Where a [`FLOORS`] colour is painted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum On {
    /// The tree's blends: the panel only, as m5/26 measured them.
    Panel,
    /// Every one of the [`surfaces`].
    Surfaces,
}

/// What [`fit_floors`] holds: (key, alpha the colour is painted at, floor, where).
pub const FLOORS: [(&str, f32, f32, On); 9] = [
    ("text (names)", QUIET, NAME_FLOOR, On::Panel),
    ("text (secondary)", SECONDARY, SECONDARY_FLOOR, On::Panel),
    ("text.muted", 1., SECONDARY_FLOOR, On::Surfaces),
    ("text.accent", 1., SECONDARY_FLOOR, On::Surfaces),
    ("error", 1., SECONDARY_FLOOR, On::Surfaces),
    ("warning", 1., SECONDARY_FLOOR, On::Surfaces),
    ("success", 1., SECONDARY_FLOOR, On::Surfaces),
    ("info", 1., SECONDARY_FLOOR, On::Surfaces),
    ("hint", 1., SECONDARY_FLOOR, On::Surfaces),
];

pub fn floor_color<'a>(theme: &'a mut Theme, key: &str) -> &'a mut Hsla {
    let s = &mut theme.styles;
    match key {
        "text (names)" | "text (secondary)" => &mut s.colors.text,
        "text.muted" => &mut s.colors.text_muted,
        "text.accent" => &mut s.colors.text_accent,
        "error" => &mut s.status.error,
        "warning" => &mut s.status.warning,
        "success" => &mut s.status.success,
        "info" => &mut s.status.info,
        _ => &mut s.status.hint,
    }
}

/// Makes every [`FLOORS`] colour readable where it is painted: a translucent one is first made opaque over
/// the panel, then lightened (darkened on a light theme) until it holds. Returns what changed.
pub fn fit_floors(theme: &mut Theme) -> Vec<Adjustment> {
    let mut adjusted = Vec::new();
    for (key, alpha, floor, on) in FLOORS {
        let old = *floor_color(theme, key);
        let was = worst_contrast(theme, old, alpha, on);
        if was >= floor {
            continue;
        }
        let panel = theme.colors().panel_background;
        let mut color: Hsla = over(old, panel).into();
        let step = if theme.appearance.is_light() { -0.005 } else { 0.005 };
        while worst_contrast(theme, color, alpha, on) < floor && (0. ..=1.).contains(&(color.l + step)) {
            color.l += step;
        }
        *floor_color(theme, key) = color;
        adjusted.push(Adjustment { key, old: hex(old), new: hex(color), was, now: worst_contrast(theme, color, alpha, on) });
    }
    adjusted.extend(fit_backgrounds(theme));
    adjusted.extend(fit_placeholder(theme));
    adjusted
}

/// Placeholder and hint text: legible, yet clearly below the [`SECONDARY_FLOOR`] that content
/// holds, so it reads as a hint. `text.placeholder` keeps [`PLACEHOLDER_FLOOR`] <= ratio <
/// [`PLACEHOLDER_CEILING`] on the panel and the editor background (the inputs').
pub const PLACEHOLDER_FLOOR: f32 = 2.5;
pub const PLACEHOLDER_CEILING: f32 = 4.0;
/// Where [`fit_placeholder`] moves a placeholder that is out of range.
const PLACEHOLDER_TARGET: f32 = 3.0;

/// The lowest and highest contrast of `color` on the panel and the editor background.
pub fn placeholder_contrast(theme: &Theme, color: Hsla) -> (f32, f32) {
    let c = theme.colors();
    let ratios = [contrast(color, c.panel_background), contrast(color, c.editor_background)];
    (ratios[0].min(ratios[1]), ratios[0].max(ratios[1]))
}

/// Makes `text.placeholder` opaque (blended over the editor background; text drawn with an alpha
/// colour is not relied on), then, when it is out of range, moves its lightness toward
/// [`PLACEHOLDER_TARGET`]. Returns what changed.
pub fn fit_placeholder(theme: &mut Theme) -> Option<Adjustment> {
    let old = theme.colors().text_placeholder;
    let was = placeholder_contrast(theme, old).0;
    let mut color: Hsla = over(old, theme.colors().editor_background).into();
    let lighter = if theme.appearance.is_light() { -0.005 } else { 0.005 };
    let in_range = |theme: &Theme, color: Hsla| {
        let (low, high) = placeholder_contrast(theme, color);
        low >= PLACEHOLDER_FLOOR && high < PLACEHOLDER_CEILING
    };
    if !in_range(theme, color) {
        let low = |color: Hsla| placeholder_contrast(theme, color).0;
        let (step, short) = if low(color) < PLACEHOLDER_TARGET { (lighter, true) } else { (-lighter, false) };
        while (low(color) < PLACEHOLDER_TARGET) == short && (0. ..=1.).contains(&(color.l + step)) {
            color.l += step;
        }
    }
    theme.styles.colors.text_placeholder = color;
    (hex(old) != hex(color)).then(|| Adjustment { key: "text.placeholder", old: hex(old), new: hex(color), was, now: placeholder_contrast(theme, color).0 })
}

/// Backgrounds text is painted on, over the panel: (key, the alpha of the theme's text on it, floor).
/// The marked row holds its ids and states in secondary text; a search result marks
/// the matched part of a name.
pub const BACKGROUND_FLOORS: [(&str, f32, f32); 2] = [
    ("ghost_element.selected", SECONDARY, SECONDARY_FLOOR),
    ("search.match_background", 1., NAME_FLOOR),
];

pub fn background_color<'a>(theme: &'a mut Theme, key: &str) -> &'a mut Hsla {
    let c = &mut theme.styles.colors;
    match key {
        "ghost_element.selected" => &mut c.ghost_element_selected,
        _ => &mut c.search_match_background,
    }
}

/// The contrast of the theme's text at `alpha` on `key`'s background over the panel.
pub fn on_background(theme: &mut Theme, key: &str, alpha: f32) -> f32 {
    let bg: Hsla = over(*background_color(theme, key), theme.colors().panel_background).into();
    contrast(theme.colors().text.opacity(alpha), bg)
}

/// Fades each [`BACKGROUND_FLOORS`] background toward the panel (lowers its alpha) until the text on it
/// holds the floor. Returns what changed.
pub fn fit_backgrounds(theme: &mut Theme) -> Vec<Adjustment> {
    let mut adjusted = Vec::new();
    for (key, alpha, floor) in BACKGROUND_FLOORS {
        let old = *background_color(theme, key);
        let was = on_background(theme, key, alpha);
        while on_background(theme, key, alpha) < floor && background_color(theme, key).a > 0.02 {
            background_color(theme, key).a -= 0.02;
        }
        let now = on_background(theme, key, alpha);
        if was < floor {
            adjusted.push(Adjustment { key, old: hex(old), new: hex(*background_color(theme, key)), was, now });
        }
    }
    adjusted
}

/// `theme` with ANSI 0-15 as `terminal` says.
pub fn with_terminal(mut theme: Theme, terminal: TerminalColors) -> Theme {
    if terminal == TerminalColors::Campbell {
        set_ansi(&mut theme.styles.colors, campbell());
        theme.id = fresh_id(&theme.name);
    }
    theme
}

/// A theme the Settings page can offer.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    /// None for a bundled one.
    pub file: Option<PathBuf>,
}

#[derive(Clone, Debug, Default)]
pub struct Catalog {
    pub entries: Vec<Entry>,
    /// `<file>: <why>` for each theme file that could not be read.
    pub errors: Vec<String>,
}

pub fn themes_dir(home: &Path) -> PathBuf {
    home.join("themes")
}

/// The bundled themes, then every theme in `<home>\themes\*.json` (by file name, then as listed).
pub fn catalog(home: Option<&Path>) -> Catalog {
    let mut catalog = Catalog {
        entries: [DARK_MODERN, SUPRAMAN_DARK].map(|name| Entry { name: name.into(), file: None }).to_vec(),
        errors: Vec::new(),
    };
    for (file, family) in theme_files(home, &mut catalog.errors) {
        for theme in &family.themes {
            catalog.entries.push(Entry { name: theme.name.clone(), file: Some(file.clone()) });
        }
    }
    catalog
}

/// A theme file's modification time and length, and what it parsed to.
type Parsed = (i64, u64, Result<Arc<ZedThemeFamily>, String>);

/// Each theme file parsed once, again only when its time or length changes.
static PARSED: Mutex<Option<HashMap<PathBuf, Parsed>>> = Mutex::new(None);

fn parse_file(file: &Path) -> Result<Arc<ZedThemeFamily>, String> {
    let meta = std::fs::metadata(file).map_err(|e| e.to_string())?;
    let stamp = (supraman_core::time::modified_ms(&meta), meta.len());
    let mut parsed = PARSED.lock().unwrap_or_else(|e| e.into_inner());
    let parsed = parsed.get_or_insert_default();
    if let Some((at, len, family)) = parsed.get(file)
        && (*at, *len) == stamp
    {
        return family.clone();
    }
    let family = std::fs::read_to_string(file)
        .map_err(|e| e.to_string())
        .and_then(|t| parse_family(&t).map(Arc::new).map_err(|e| e.to_string()));
    parsed.insert(file.to_owned(), (stamp.0, stamp.1, family.clone()));
    family
}

fn theme_files(home: Option<&Path>, errors: &mut Vec<String>) -> Vec<(PathBuf, Arc<ZedThemeFamily>)> {
    let Some(Ok(dir)) = home.map(|h| std::fs::read_dir(themes_dir(h))) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = dir
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("json")))
        .collect();
    files.sort();
    files
        .into_iter()
        .filter_map(|file| {
            let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            match parse_file(&file) {
                Ok(family) => Some((file, family)),
                Err(e) => {
                    errors.push(format!("{name}: {e}"));
                    None
                }
            }
        })
        .collect()
}

/// The theme to paint with.
#[derive(Clone, Debug)]
pub struct Resolved {
    pub theme: Theme,
    pub adjusted: Vec<Adjustment>,
    /// Why the theme asked for is not the one shown.
    pub problem: Option<String>,
}

/// A saved theme name as the catalog lists it.
pub fn canonical(name: &str) -> &str {
    if name == SUPRAMAN_DARK_LEGACY { SUPRAMAN_DARK } else { name }
}

/// `name` (None: [`DEFAULT_THEME`]) from the bundled themes or `<home>\themes`, with `terminal`'s ANSI.
pub fn resolve(name: Option<&str>, terminal: TerminalColors, home: Option<&Path>) -> Resolved {
    let name = canonical(name.unwrap_or(DEFAULT_THEME));
    let (theme, adjusted, problem) = if name == SUPRAMAN_DARK {
        let loaded = supraman_dark();
        (loaded.theme, loaded.adjusted, None)
    } else if name == DARK_MODERN {
        let loaded = dark_modern();
        (loaded.theme, loaded.adjusted, None)
    } else {
        let mut errors = Vec::new();
        let found = theme_files(home, &mut errors)
            .into_iter()
            .find_map(|(_, family)| family.themes.iter().find(|t| t.name == name).cloned());
        match found {
            Some(zed) => {
                let loaded = from_zed(&one_dark(), &zed);
                (loaded.theme, loaded.adjusted, None)
            }
            None => {
                let loaded = dark_modern();
                (loaded.theme, loaded.adjusted, Some(format!("No theme \"{name}\" in the themes folder; showing {DEFAULT_THEME}.")))
            }
        }
    };
    Resolved { theme: with_terminal(theme, terminal), adjusted, problem }
}

/// Paints every window with `name` and `terminal` from now on, the terminals' OSC colour answers too.
pub fn apply(cx: &mut App, name: Option<&str>, terminal: TerminalColors, home: Option<&Path>) -> Resolved {
    apply_resolved(cx, resolve(name, terminal, home))
}

/// [`apply`] of a theme [`resolve`]d elsewhere, off the UI thread.
pub fn apply_resolved(cx: &mut App, resolved: Resolved) -> Resolved {
    for a in &resolved.adjusted {
        log::info!("theme {}: {} {} → {} ({:.2} → {:.2}:1)", resolved.theme.name, a.key, a.old, a.new, a.was, a.now);
    }
    let palette = palette(&resolved.theme);
    theme::GlobalTheme::update_theme(cx, Arc::new(resolved.theme.clone()));
    if cx.has_global::<TerminalConfig>() {
        cx.update_global(|config: &mut TerminalConfig, _| config.palette = palette);
    }
    cx.refresh_windows();
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_are_six_or_eight_hex_digits() {
        let red = parse_color("#FF0000").unwrap();
        assert_eq!(hex(red), "#FF0000");
        assert_eq!(hex(parse_color("#ffffff06").unwrap()), "#FFFFFF06");
        assert_eq!(parse_color("#ffffff06").unwrap().a, 6. / 255.);
        for bad in ["FF0000", "#F00", "#FF00", "#GG0000", "#FF00000", "", "#"] {
            assert_eq!(parse_color(bad), None, "{bad}");
        }
    }

    #[test]
    fn the_bundled_file_loads_with_its_colours() {
        let family = parse_family(DARK_MODERN_JSON).unwrap();
        assert_eq!((family.name.as_str(), family.themes.len()), ("VSCode Dark Modern", 1));
        assert_eq!(family.themes[0].style.len(), 160);
        let loaded = dark_modern();
        let (c, s) = (loaded.theme.colors(), loaded.theme.status());
        assert_eq!(hex(c.background), "#1F1F1F");
        assert_eq!(hex(c.panel_background), "#181818");
        assert_eq!(hex(c.text), "#CCCCCC");
        assert_eq!(hex(c.border), "#2B2B2B");
        assert_eq!(hex(c.element_selected), "#04395E");
        assert_eq!(hex(c.ghost_element_hover), "#FFFFFF06");
        assert_eq!(hex(c.tab_active_background), "#1F1F1F");
        assert_eq!(hex(s.error), "#F14C4C");
        assert_eq!(hex(s.warning), "#CCA700");
        assert_eq!(hex(c.terminal_background), "#181818");
        assert_eq!(hex(c.terminal_ansi_background), "#181818", "falls back to terminal.background");
        assert_eq!(hex(c.terminal_bright_foreground), "#CCCCCC", "falls back to terminal.foreground");
        assert_eq!(hex(loaded.theme.players().local().cursor), "#AEAFAD");
        assert_eq!(loaded.theme.accents().0.len(), 9);
        // null keys keep One Dark's: success
        assert_eq!(s.success, one_dark().status().success);
        // 160 keys: two are skipped on purpose (syntax, background.appearance), the rest set a colour,
        // are null, or are unknown to Foreman
        println!("ignored: {:?}", loaded.ignored);
        assert_eq!(loaded.applied, 97);
        assert!(loaded.ignored.contains(&"version_control_conflict_ours_background".to_string()), "{:?}", loaded.ignored);
    }

    #[test]
    fn the_terminal_palette_is_the_files() {
        let theme = dark_modern().theme;
        let p = palette(&theme);
        let want = [
            "#000000", "#CD3131", "#0DBC79", "#E5E510", "#2472C8", "#BC3FBC", "#11A8CD", "#E5E5E5", //
            "#666666", "#F14C4C", "#23D18B", "#F5F543", "#3B8EEA", "#D670D6", "#29B8DB", "#E5E5E5",
        ];
        assert_eq!(p.ansi.map(hex), want);
        assert_eq!(hex(p.background), "#181818");
        assert_eq!(hex(p.foreground), "#CCCCCC");
        assert_eq!(p.dim[1], dim(p.ansi[1]), "missing dims are derived");
        let campbell_theme = with_terminal(theme, TerminalColors::Campbell);
        let c = palette(&campbell_theme);
        assert_eq!(c.ansi, campbell());
        assert_eq!(hex(c.background), "#181818", "Campbell changes ANSI 0-15 only");
        assert_eq!(palette(&supraman_dark().theme).ansi, campbell(), "Supraman Dark is One Dark with Campbell, as before m5/31");
    }

    #[test]
    fn the_legacy_name_resolves_to_supraman_dark() {
        let resolved = resolve(Some(SUPRAMAN_DARK_LEGACY), TerminalColors::Theme, None);
        assert_eq!((resolved.theme.name.as_ref(), resolved.problem), (SUPRAMAN_DARK, None));
        assert_eq!(canonical(SUPRAMAN_DARK_LEGACY), SUPRAMAN_DARK);
    }

    #[test]
    fn unknown_keys_bad_values_and_nulls_fall_back_per_key() {
        let json = r##"{
            "$schema": "x", "extra": [1, 2],
            "themes": [{ "name": "T", "appearance": "dark", "future": {"a": 1},
                "style": { "background": "#102030", "text": "not a colour", "border": null,
                           "made.up.key": "#FFFFFF", "panel.background": 7, "text.muted": "#ABC",
                           "terminal.ansi.red": "#FF000080" } }]
        }"##;
        let family = parse_family(json).unwrap();
        let base = one_dark();
        let loaded = from_zed(&base, &family.themes[0]);
        let (c, b) = (loaded.theme.colors(), base.colors());
        assert_eq!(hex(c.background), "#102030");
        assert_eq!(c.border, b.border, "null keeps the base");
        assert_eq!(c.panel_background, b.panel_background);
        assert_eq!(hex(c.terminal_ansi_red), "#FF000080");
        assert_eq!(c.terminal_ansi_dim_red, dim(c.terminal_ansi_red));
        assert_eq!(c.terminal_ansi_green, b.terminal_ansi_green);
        let mut ignored = loaded.ignored.clone();
        ignored.sort();
        assert_eq!(ignored, ["made.up.key", "panel.background", "text", "text.muted"]);
        assert_eq!(loaded.applied, 2);
        assert!(parse_family("{}").unwrap().themes.is_empty());
        assert!(parse_family("[").is_err());
    }

    /// m5/26's floors under each bundled theme: names (text at [`QUIET`]) >= 7:1, secondary (text at
    /// [`SECONDARY`]) >= 4.5:1 against every surface, a dimmed row >= 4.5 and >= 3, the hierarchy kept.
    /// Every key the loader had to change is printed (`cargo test -- --nocapture`) and pinned.
    #[test]
    fn the_contrast_floors_hold_under_each_bundled_theme() {
        let (modern, dark) = (dark_modern(), supraman_dark());
        for theme in [&modern.theme, &dark.theme] {
            let text = theme.colors().text;
            let ratio = |alpha: f32| worst_contrast(theme, text, alpha, On::Panel);
            let (name, quiet, secondary) = (ratio(1.), ratio(QUIET), ratio(SECONDARY));
            println!("{}: text {name:.2}, names {quiet:.2}, secondary {secondary:.2}", theme.name);
            assert!(name > quiet && quiet > secondary, "{}: {name} > {quiet} > {secondary}", theme.name);
            assert!(quiet >= NAME_FLOOR, "{}: names {quiet}", theme.name);
            assert!(secondary >= SECONDARY_FLOOR, "{}: secondary {secondary}", theme.name);
            assert!(ratio(QUIET * DIMMED) >= 4.5 && ratio(SECONDARY * DIMMED) >= 3., "{}: dimmed", theme.name);
        }
        // the loaded theme holds every floor
        let mut theme = modern.theme.clone();
        for (key, alpha, floor, on) in FLOORS {
            let color = *floor_color(&mut theme, key);
            let got = worst_contrast(&theme, color, alpha, on);
            println!("{}: {key} {got:.2} (floor {floor})", theme.name);
            assert!(got >= floor, "{key}: {got} < {floor}");
        }
        for loaded in [&modern, &dark] {
            for a in &loaded.adjusted {
                println!("{} adjusted {}: {} -> {} ({:.2} -> {:.2})", loaded.theme.name, a.key, a.old, a.new, a.was, a.now);
            }
        }
        let keys = |l: &Loaded| l.adjusted.iter().map(|a| (a.key, a.old.clone(), a.new.clone())).collect::<Vec<_>>();
        let pinned = |e: &[(&'static str, &str, &str)]| e.iter().map(|(k, o, n)| (*k, o.to_string(), n.to_string())).collect::<Vec<_>>();
        assert_eq!(keys(&modern), pinned(&EXPECTED_ADJUSTMENTS));
        assert_eq!(keys(&dark), pinned(&EXPECTED_SUPRAMAN_DARK));
    }

    /// Under each bundled theme, on their own backgrounds: the search box's text, a matched name, the
    /// marked row's name, secondary and warning text, the toast's text; the Messages row: its sender (text)
    /// and its workspace and age (quiet) on the row's info or warning tint, resting and under the pointer.
    #[test]
    fn the_newer_elements_read_under_each_bundled_theme() {
        for mut theme in [dark_modern().theme, supraman_dark().theme] {
            let (c, s) = (theme.colors().clone(), theme.status().clone());
            let marked: Hsla = over(c.ghost_element_selected, c.panel_background).into();
            let mut checks = vec![
                ("search box text".to_string(), contrast(c.text, c.editor_background), NAME_FLOOR),
                ("matched name".into(), on_background(&mut theme, "search.match_background", 1.), NAME_FLOOR),
                ("marked row name".into(), contrast(c.text, marked), NAME_FLOOR),
                ("marked row secondary".into(), on_background(&mut theme, "ghost_element.selected", SECONDARY), SECONDARY_FLOOR),
                ("marked row warning".into(), contrast(s.warning, marked), SECONDARY_FLOOR),
                ("toast text".into(), contrast(c.text, c.elevated_surface_background), NAME_FLOOR),
            ];
            for (tint, color) in [("info", s.info), ("warning", s.warning)] {
                for alpha in [MESSAGE_TINT, MESSAGE_TINT_HOVER] {
                    let row: Hsla = over(color.opacity(alpha), c.panel_background).into();
                    checks.push((format!("message sender on {tint} {alpha}"), contrast(c.text, row), NAME_FLOOR));
                    // its workspace and age are quiet, not secondary: secondary falls to 4.42 on the hover tint
                    checks.push((format!("message workspace and age on {tint} {alpha}"), contrast(c.text.opacity(QUIET), row), SECONDARY_FLOOR));
                }
            }
            for (what, got, floor) in checks {
                println!("{}: {what} {got:.2} (floor {floor})", theme.name);
                assert!(got >= floor, "{}: {what} {got} < {floor}", theme.name);
            }
            // the mark still shows: its background and its left edge differ from the panel
            assert!(contrast(marked, c.panel_background) > 1.1, "{}: the marked row's background is the panel's", theme.name);
            assert!(contrast(c.border_focused, c.panel_background) >= 3., "{}: the marked row's edge", theme.name);
        }
    }

    /// Placeholder text reads as a hint under each bundled theme, below what content holds, and is
    /// opaque; a theme whose placeholder is as bright as its text, or too dim, is moved into the range.
    #[test]
    fn the_placeholder_reads_as_a_hint_under_each_bundled_theme() {
        for theme in [dark_modern().theme, supraman_dark().theme] {
            let c = theme.colors();
            let (low, high) = placeholder_contrast(&theme, c.text_placeholder);
            println!("{}: placeholder {} {low:.2}..{high:.2}", theme.name, hex(c.text_placeholder));
            assert!(low >= PLACEHOLDER_FLOOR && high < PLACEHOLDER_CEILING, "{}: placeholder {low}..{high}", theme.name);
            assert_eq!(c.text_placeholder.a, 1., "{}: an opaque placeholder", theme.name);
            assert!(high < worst_contrast(&theme, c.text, SECONDARY, On::Panel), "{}: below secondary text", theme.name);
        }
        for (placeholder, why) in [("#CCCCCC", "as bright as the text"), ("#2A2A2A", "nearly the background")] {
            let json = format!(
                r##"{{ "themes": [{{ "name": "P", "style": {{ "panel.background": "#181818", "editor.background": "#1F1F1F",
                "text": "#CCCCCC", "text.placeholder": "{placeholder}" }} }}] }}"##
            );
            let loaded = from_zed(&one_dark(), &parse_family(&json).unwrap().themes[0]);
            let (low, high) = placeholder_contrast(&loaded.theme, loaded.theme.colors().text_placeholder);
            assert!(low >= PLACEHOLDER_FLOOR && high < PLACEHOLDER_CEILING, "{why}: {low}..{high}");
            assert!(loaded.adjusted.iter().any(|a| a.key == "text.placeholder"), "{why}: reported");
        }
    }

    /// What VS Code Dark Modern needed (reported in LEARNINGS m5/31).
    /// `info` is a 40% grey there (`#BFBFBF66`, 2.58:1 on #1F1F1F); the tree paints Idle in it.
    /// The marked row's `#04395E` left the ids on it at 3.82:1; a match's `#C6510666` a name at 6.94:1.
    /// Its translucent placeholder `#FFFFFF56` is made opaque (3.07:1 on the editor background).
    const EXPECTED_ADJUSTMENTS: [(&str, &str, &str); 4] = [
        ("info", "#BFBFBF66", "#868686"),
        ("ghost_element.selected", "#04395E", "#04395E80"),
        ("search.match_background", "#C6510666", "#C6510661"),
        ("text.placeholder", "#FFFFFF56", "#6B6B6B"),
    ];
    /// One Dark's marked-row background left the ids on it at 3.97:1.
    const EXPECTED_SUPRAMAN_DARK: [(&str, &str, &str); 1] = [("ghost_element.selected", "#3B3F4A", "#3B3F4A8A")];

    #[test]
    fn a_dim_theme_is_lifted_to_the_floors_and_reported() {
        let json = r##"{ "themes": [{ "name": "Dim", "style": {
            "background": "#181818", "panel.background": "#181818", "editor.background": "#181818",
            "text": "#808080", "info": "#BFBFBF66" } }] }"##;
        let loaded = from_zed(&one_dark(), &parse_family(json).unwrap().themes[0]);
        let keys: Vec<&str> = loaded.adjusted.iter().map(|a| a.key).collect();
        assert!(keys.contains(&"text (names)") && keys.contains(&"info"), "{keys:?}");
        let info = loaded.adjusted.iter().find(|a| a.key == "info").unwrap();
        assert_eq!(info.old, "#BFBFBF66");
        assert!(info.was < 4.5 && info.now >= 4.5 && info.new.len() == 7, "{info:?}");
        let theme = loaded.theme;
        assert!(worst_contrast(&theme, theme.colors().text, QUIET, On::Panel) >= NAME_FLOOR);
    }

    #[test]
    fn the_catalog_lists_bundled_then_files_and_reports_bad_ones() {
        let home = std::env::temp_dir().join(format!("foreman-themes-{}", std::process::id()));
        let dir = themes_dir(&home);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("b.json"), r##"{"themes":[{"name":"Bee","style":{"background":"#000000"}},{"name":"Bee 2"}]}"##).unwrap();
        std::fs::write(dir.join("a.json"), "not json").unwrap();
        std::fs::write(dir.join("notes.txt"), "ignored").unwrap();
        let catalog = catalog(Some(&home));
        let names: Vec<&str> = catalog.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, [DARK_MODERN, SUPRAMAN_DARK, "Bee", "Bee 2"]);
        assert_eq!(catalog.errors.len(), 1);
        assert!(catalog.errors[0].starts_with("a.json: "), "{:?}", catalog.errors);

        let bee = resolve(Some("Bee"), TerminalColors::Theme, Some(&home));
        assert_eq!((bee.theme.name.as_ref(), hex(bee.theme.colors().background).as_str(), bee.problem), ("Bee", "#000000", None));
        let gone = resolve(Some("Gone"), TerminalColors::Theme, Some(&home));
        assert_eq!(gone.theme.name.as_ref(), DARK_MODERN);
        assert!(gone.problem.unwrap().contains("Gone"));
        assert_eq!(resolve(None, TerminalColors::Theme, None).theme.name.as_ref(), DEFAULT_THEME);
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn a_theme_file_is_parsed_again_only_when_it_changes() {
        let home = std::env::temp_dir().join(format!("foreman-themes-cache-{}", std::process::id()));
        let dir = themes_dir(&home);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("c.json");
        std::fs::write(&file, r#"{"themes":[{"name":"Cee"}]}"#).unwrap();
        let first = parse_file(&file).unwrap();
        assert!(Arc::ptr_eq(&first, &parse_file(&file).unwrap()));

        std::fs::write(&file, r#"{"themes":[{"name":"Cee 2"}]}"#).unwrap();
        let names: Vec<String> = catalog(Some(&home)).entries.into_iter().map(|e| e.name).collect();
        assert_eq!(names.last().map(String::as_str), Some("Cee 2"));
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn terminal_colours_setting_round_trips() {
        assert_eq!(TerminalColors::from_setting(None), TerminalColors::Theme);
        assert_eq!(TerminalColors::from_setting(Some("Campbell")), TerminalColors::Campbell);
        assert_eq!(TerminalColors::from_setting(Some("other")), TerminalColors::Theme);
        for t in [TerminalColors::Theme, TerminalColors::Campbell] {
            assert_eq!(TerminalColors::from_setting(t.setting().as_deref()), t);
        }
    }
}

