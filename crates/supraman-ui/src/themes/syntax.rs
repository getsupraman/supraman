//! Code colours: a theme's own `syntax` block over Supraman's defaults for its appearance, each colour held
//! to [`SYNTAX_FLOOR`] on the theme's editor background.

use std::{borrow::Cow, sync::Arc};

use gpui::{FontStyle, FontWeight, HighlightStyle, Hsla};
use serde_json::Value;
use theme::{Appearance, SyntaxTheme, Theme};

use super::{Adjustment, SECONDARY_FLOOR, contrast, hex, over, parse_color};

/// Code is text: every syntax colour against the editor background (WCAG AA).
pub const SYNTAX_FLOOR: f32 = SECONDARY_FLOOR;

/// The key prefix of a syntax colour's [`Adjustment`].
pub const ADJUSTED: &str = "syntax.";

/// Supraman's code colours on a dark editor, in the spirit of VS Code Dark Modern. `""` is no colour.
pub const DARK: &[(&str, &str)] = &[
    ("attribute", "#94D3FA"), ("boolean", "#5A9FDB"), ("comment", "#71A35C"), ("comment.doc", "#71A35C"),
    ("constant", "#53C3F9"), ("constructor", "#52C7AE"), ("embedded", "#D2D2D2"), ("emphasis", ""),
    ("emphasis.strong", ""), ("enum", "#52C7AE"), ("function", "#DBD8A6"), ("function.method", "#DBD8A6"),
    ("function.definition", "#DBD8A6"), ("hint", "#8C8C8C"), ("keyword", "#5A9FDB"), ("keyword.control", "#C98BC4"),
    ("label", "#C8C8C8"), ("link_text", "#5A9FDB"), ("link_uri", "#52C7AE"), ("number", "#B3CFA6"),
    ("operator", "#D2D2D2"), ("predictive", "#8A8A8A"), ("preproc", "#C98BC4"), ("primary", "#D2D2D2"),
    ("property", "#94D3FA"), ("punctuation", "#CACACA"), ("punctuation.bracket", "#CACACA"),
    ("punctuation.delimiter", "#CACACA"), ("punctuation.list_marker", "#CACACA"), ("punctuation.special", "#CACACA"),
    ("string", "#CF9479"), ("string.escape", "#D6BB80"), ("string.regex", "#D46D6D"), ("string.special", "#D46D6D"),
    ("string.special.symbol", "#D46D6D"), ("tag", "#5A9FDB"), ("text.literal", "#CF9479"), ("title", "#5A9FDB"),
    ("type", "#52C7AE"), ("variable", "#94D3FA"), ("variable.special", "#5A9FDB"), ("variant", "#52C7AE"),
    ("diff.plus", "#7FC77A"), ("diff.minus", "#E06C6C"),
];

/// Supraman's code colours on a light editor, in the spirit of VS Code Light Modern.
pub const LIGHT: &[(&str, &str)] = &[
    ("attribute", "#C42B1C"), ("boolean", "#1A40C8"), ("comment", "#2E7D32"), ("comment.doc", "#2E7D32"),
    ("constant", "#0B6AB0"), ("constructor", "#1F7A8C"), ("embedded", "#1F1F1F"), ("emphasis", ""),
    ("emphasis.strong", ""), ("enum", "#1F7A8C"), ("function", "#74581F"), ("function.method", "#74581F"),
    ("function.definition", "#74581F"), ("hint", "#6E6E6E"), ("keyword", "#1A40C8"), ("keyword.control", "#9A1FB8"),
    ("label", "#3B3B3B"), ("link_text", "#1A40C8"), ("link_uri", "#1F7A8C"), ("number", "#11795A"),
    ("operator", "#3B3B3B"), ("predictive", "#707070"), ("preproc", "#9A1FB8"), ("primary", "#1F1F1F"),
    ("property", "#163C8F"), ("punctuation", "#3B3B3B"), ("punctuation.bracket", "#3B3B3B"),
    ("punctuation.delimiter", "#3B3B3B"), ("punctuation.list_marker", "#3B3B3B"), ("punctuation.special", "#3B3B3B"),
    ("string", "#A3271F"), ("string.escape", "#B03A0A"), ("string.regex", "#86274A"), ("string.special", "#86274A"),
    ("string.special.symbol", "#86274A"), ("tag", "#8B1E1E"), ("text.literal", "#A3271F"), ("title", "#1A40C8"),
    ("type", "#1F7A8C"), ("variable", "#163C8F"), ("variable.special", "#1A40C8"), ("variant", "#1F7A8C"),
    ("diff.plus", "#2E7D32"), ("diff.minus", "#C42B1C"),
];

/// Supraman's code colours for `appearance`.
pub fn defaults(appearance: Appearance) -> Arc<SyntaxTheme> {
    let table = if appearance.is_light() { LIGHT } else { DARK };
    Arc::new(SyntaxTheme::new(table.iter().map(|(key, color)| {
        let mut style = HighlightStyle { color: parse_color(color), ..Default::default() };
        match *key {
            "emphasis" => style.font_style = Some(FontStyle::Italic),
            "emphasis.strong" | "title" => style.font_weight = Some(FontWeight::BOLD),
            "link_uri" => style.font_style = Some(FontStyle::Italic),
            _ => {}
        }
        (key.to_string(), style)
    })))
}

/// A Zed theme's `syntax` block over `base`, per key and per part: a part it leaves out or nulls keeps the
/// base's. Each entry that sets a part counts in `applied`; any other is `ignored` as `syntax.<key>`.
pub fn merged(base: Arc<SyntaxTheme>, block: &Value, applied: &mut usize, ignored: &mut Vec<String>) -> Arc<SyntaxTheme> {
    let Some(block) = block.as_object() else {
        ignored.push("syntax".into());
        return base;
    };
    let mut styles = Vec::new();
    for (key, entry) in block {
        match entry.as_object().map(style_of) {
            Some(style) if style != HighlightStyle::default() => {
                styles.push((key.clone(), style));
                *applied += 1;
            }
            _ if entry.is_null() => {}
            _ => ignored.push(format!("syntax.{key}")),
        }
    }
    SyntaxTheme::merge(base, styles)
}

fn style_of(entry: &serde_json::Map<String, Value>) -> HighlightStyle {
    let text = |part: &str| entry.get(part).and_then(Value::as_str);
    HighlightStyle {
        color: text("color").and_then(parse_color),
        font_weight: entry.get("font_weight").and_then(Value::as_f64).map(|w| FontWeight(w as f32)),
        font_style: match text("font_style") {
            Some("italic") => Some(FontStyle::Italic),
            Some("oblique") => Some(FontStyle::Oblique),
            Some("normal") => Some(FontStyle::Normal),
            _ => None,
        },
        ..Default::default()
    }
}

/// Every `(capture name, style)` of `syntax`, in its order.
pub fn entries(syntax: &SyntaxTheme) -> Vec<(String, HighlightStyle)> {
    (0..).map_while(|i: usize| Some((syntax.get_capture_name(i)?.to_string(), *syntax.get(i)?))).collect()
}

/// Makes each syntax colour opaque over the editor background, then lightens it (darkens it on a light theme)
/// until it holds [`SYNTAX_FLOOR`] there. Returns what changed, keyed `syntax.<capture>`.
pub fn fit(theme: &mut Theme) -> Vec<Adjustment> {
    let bg = theme.colors().editor_background;
    let step = if theme.appearance.is_light() { -0.005 } else { 0.005 };
    let mut adjusted = Vec::new();
    let fitted = entries(theme.syntax()).into_iter().map(|(key, mut style)| {
        if let Some(old) = style.color {
            let mut color: Hsla = over(old, bg).into();
            while contrast(color, bg) < SYNTAX_FLOOR && (0. ..=1.).contains(&(color.l + step)) {
                color.l += step;
            }
            if hex(color) != hex(old) {
                let name = Cow::Owned(format!("{ADJUSTED}{key}"));
                adjusted.push(Adjustment { key: name, old: hex(old), new: hex(color), was: contrast(old, bg), now: contrast(color, bg) });
            }
            style.color = Some(color);
        }
        (key, style)
    });
    let fitted: Vec<_> = fitted.collect();
    theme.styles.syntax = Arc::new(SyntaxTheme::new(fitted));
    adjusted
}

#[cfg(test)]
mod tests {
    use super::super::{TerminalColors, ZedTheme, dark_modern, from_zed, one_dark, parse_family, supraman_dark, with_terminal};
    use super::*;

    fn light(syntax: Value) -> ZedTheme {
        let mut style = serde_json::json!({ "background": "#F3F3F3", "panel.background": "#F3F3F3", "editor.background": "#FFFFFF", "text": "#1F1F1F" });
        if !syntax.is_null() {
            style["syntax"] = syntax;
        }
        ZedTheme { name: "Light code".into(), appearance: Some("light".into()), style: style.as_object().unwrap().clone() }
    }

    fn color(theme: &Theme, key: &str) -> Hsla {
        theme.syntax().style_for_name(key).and_then(|s| s.color).unwrap_or_else(|| panic!("{}: {key} has a colour", theme.name))
    }

    #[test]
    fn a_light_theme_without_syntax_colours_code_for_a_light_editor() {
        let theme = from_zed(&one_dark(), &light(Value::Null)).theme;
        let one_dark_keyword = color(&one_dark(), "keyword");
        assert_ne!(hex(color(&theme, "keyword")), hex(one_dark_keyword), "not One Dark's keyword");
        assert_eq!(hex(color(&theme, "keyword")), "#1A40C8", "Supraman's light keyword");
        let bg = theme.colors().editor_background;
        for (key, style) in entries(theme.syntax()) {
            if let Some(c) = style.color {
                assert!(contrast(c, bg) >= SYNTAX_FLOOR, "{key} {} on {}: {:.2}", hex(c), hex(bg), contrast(c, bg));
            }
        }
    }

    #[test]
    fn a_themes_own_syntax_key_wins_per_part() {
        let syntax = serde_json::json!({
            "keyword": { "color": "#7A1FA8", "font_style": "italic", "font_weight": null },
            "string": { "color": null, "font_weight": 700 },
            "made.up": { "color": "#204060" },
            "comment": null,
            "bad": "#FF0000",
        });
        let loaded = from_zed(&one_dark(), &light(syntax));
        let theme = &loaded.theme;
        let keyword = theme.syntax().style_for_name("keyword").unwrap();
        assert_eq!((hex(keyword.color.unwrap()), keyword.font_style), ("#7A1FA8".into(), Some(FontStyle::Italic)));
        let string = theme.syntax().style_for_name("string").unwrap();
        assert_eq!((hex(string.color.unwrap()), string.font_weight), ("#A3271F".into(), Some(FontWeight::BOLD)), "the default colour, the theme's weight");
        assert_eq!(hex(color(theme, "comment")), "#2E7D32", "null keeps the default");
        assert_eq!(hex(color(theme, "made.up")), "#204060", "a key Supraman has no default for");
        assert!(loaded.ignored.contains(&"syntax.bad".to_string()), "{:?}", loaded.ignored);
    }

    #[test]
    fn a_syntax_colour_below_the_floor_is_moved_and_reported() {
        let loaded = from_zed(&one_dark(), &light(serde_json::json!({ "keyword": { "color": "#C8C8FF" } })));
        let moved = loaded.adjusted.iter().find(|a| a.key == "syntax.keyword").expect("reported");
        assert_eq!(moved.old, "#C8C8FF");
        assert!(moved.was < SYNTAX_FLOOR && moved.now >= SYNTAX_FLOOR, "{moved:?}");
        assert_eq!(hex(color(&loaded.theme, "keyword")), moved.new);
    }

    /// The bundled themes' code colours, each held on their editor background; what moved is printed and pinned.
    #[test]
    fn the_bundled_themes_code_colours_hold_the_floor() {
        let (modern, dark) = (dark_modern(), supraman_dark());
        assert_eq!(hex(color(&modern.theme, "keyword")), "#569CD6", "Dark Modern's own syntax block");
        assert_eq!(hex(color(&modern.theme, "function.method")), "#DBD8A6", "a key it leaves out takes Supraman's dark default");
        for loaded in [&modern, &dark] {
            let theme = with_terminal(loaded.theme.clone(), TerminalColors::Campbell);
            let bg = theme.colors().editor_background;
            for (key, style) in entries(theme.syntax()) {
                if let Some(c) = style.color {
                    assert!(contrast(c, bg) >= SYNTAX_FLOOR, "{}: {key} {}: {:.2}", theme.name, hex(c), contrast(c, bg));
                }
            }
            for a in loaded.adjusted.iter().filter(|a| a.key.starts_with(ADJUSTED)) {
                println!("{} adjusted {}: {} -> {} ({:.2} -> {:.2})", loaded.theme.name, a.key, a.old, a.new, a.was, a.now);
            }
        }
        let moved = |l: &super::super::Loaded| l.adjusted.iter().filter(|a| a.key.starts_with(ADJUSTED)).map(|a| format!("{} {} {}", a.key, a.old, a.new)).collect::<Vec<_>>();
        assert_eq!(moved(&modern), EXPECTED_DARK_MODERN);
        assert_eq!(moved(&dark), EXPECTED_SUPRAMAN_DARK);
    }

    /// Dark Modern's own colours all hold on #1F1F1F.
    const EXPECTED_DARK_MODERN: &[&str] = &[];
    /// One Dark's grey comment is 2.31:1 on #282C34; its purple and red sit just under 4.5.
    const EXPECTED_SUPRAMAN_DARK: &[&str] = &[
        "syntax.attribute #BC74D2 #BE78D3",
        "syntax.comment #5C6370 #8C93A1",
        "syntax.comment.doc #5C6370 #8C93A1",
        "syntax.keyword #BC74D2 #BE78D3",
        "syntax.preproc #BC74D2 #BE78D3",
        "syntax.property #E06C75 #E17079",
        "syntax.string.regex #E06C75 #E17079",
        "syntax.variable.special #E06C75 #E17079",
        "syntax.diff.minus #E06C75 #E17079",
    ];

    #[test]
    fn the_defaults_parse_and_hold_their_floor() {
        for (appearance, table, bg) in [(Appearance::Dark, DARK, "#1F1F1F"), (Appearance::Light, LIGHT, "#FFFFFF")] {
            assert_eq!(table.iter().map(|(k, _)| *k).collect::<Vec<_>>(), DARK.iter().map(|(k, _)| *k).collect::<Vec<_>>());
            let bg = parse_color(bg).unwrap();
            for (key, style) in entries(&defaults(appearance)) {
                if let Some(c) = style.color {
                    assert!(contrast(c, bg) >= SYNTAX_FLOOR, "{appearance:?} {key} {}: {:.2}", hex(c), contrast(c, bg));
                }
            }
        }
        let family = parse_family(r#"{"themes":[{"name":"N","style":{"syntax":7}}]}"#).unwrap();
        let loaded = from_zed(&one_dark(), &family.themes[0]);
        assert_eq!(loaded.ignored, ["syntax"]);
        assert_eq!(hex(color(&loaded.theme, "keyword")), "#5A9FDB", "a bad block keeps the dark defaults");
    }
}
