//! Its own process: a theme switch re-colours the languages every test in a process shares.

use std::sync::Arc;

use gpui::{TestAppContext, VisualTestContext};
use supraman_editor::{file_editor, highlighted, language_for, read_only_buffer};
use supraman_ui::themes::{self, syntax};
use theme::{ActiveTheme as _, GlobalTheme, SyntaxTheme};

const CSHARP: &str = "// note\nclass Greeter {\n    string Greet() { return \"hi\"; }\n}\n";

fn assert_painted(runs: &[(String, String)], theme: &str) {
    for (text, key) in [("// note", "comment"), ("class", "keyword"), ("return", "keyword"), ("\"hi\"", "string"), ("Greeter", "type")] {
        assert!(runs.iter().any(|(t, k)| t == text && k.split('.').next() == Some(key)), "{theme}: {text:?} painted as {key}; runs {runs:?}");
    }
}

fn keyword(cx: &mut VisualTestContext) -> String {
    cx.update(|_, cx| themes::hex(cx.theme().syntax().style_for_name("keyword").and_then(|s| s.color).expect("a keyword colour")))
}

#[gpui::test]
fn switching_the_theme_recolours_an_open_buffer(cx: &mut TestAppContext) {
    cx.update(|cx| supraman_editor::init("Cascadia Mono", 13., cx));
    cx.update(|cx| GlobalTheme::update_theme(cx, Arc::new(themes::supraman_dark().theme)));
    let cx: &mut VisualTestContext = cx.add_empty_window();
    let editor = cx.update(|window, cx| {
        let buffer = read_only_buffer(CSHARP.into(), language_for("Greeter.cs"), cx);
        file_editor(buffer, window, cx)
    });
    cx.run_until_parked();
    assert_painted(&cx.update(|_, cx| highlighted(&editor, cx)), "dark");
    let dark = keyword(cx);

    let json = serde_json::json!({ "themes": [{ "name": "Light", "appearance": "light",
        "style": { "background": "#F3F3F3", "panel.background": "#F3F3F3", "editor.background": "#FFFFFF", "text": "#1F1F1F" } }] });
    let mut light = themes::from_zed(&themes::one_dark(), &themes::parse_family(&json.to_string()).unwrap().themes[0]).theme;
    // two themes' capture ids differ when their keys come in another order
    light.styles.syntax = Arc::new(SyntaxTheme::new(syntax::entries(light.syntax()).into_iter().rev()));
    cx.update(|_, cx| GlobalTheme::update_theme(cx, Arc::new(light)));
    cx.run_until_parked();
    assert_painted(&cx.update(|_, cx| highlighted(&editor, cx)), "light");
    assert_ne!(keyword(cx), dark);
}
