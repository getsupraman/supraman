use std::sync::Arc;

use theme::SyntaxTheme;
use tree_sitter::{Parser, QueryCursor, StreamingIterator as _};

use super::*;

fn name(path: &str) -> Option<String> {
    language_for(path).map(|l| l.name().to_string())
}

fn theme() -> Arc<SyntaxTheme> {
    let registry = theme::ThemeRegistry::new(Box::new(()));
    let first = registry.list_names().into_iter().next().expect("Zed's fallback theme");
    registry.get(&first).unwrap().syntax().clone()
}

/// Every node of `sample` a capture of `language`'s highlights query takes, as its text and the theme key
/// that colours it.
fn coloured(language: &Language, sample: &str) -> Vec<(String, String)> {
    let theme = theme();
    let grammar = language.grammar().expect("a grammar");
    let query = &grammar.highlights_config.as_ref().expect("a highlights query").query;
    let mut parser = Parser::new();
    parser.set_language(&grammar.ts_language).unwrap();
    let tree = parser.parse(sample, None).unwrap();
    let mut cursor = QueryCursor::new();
    let mut captures = cursor.captures(query, tree.root_node(), sample.as_bytes());
    let mut out = Vec::new();
    while let Some((found, ix)) = captures.next() {
        let capture = found.captures[*ix];
        let name = query.capture_names()[capture.index as usize];
        let key = theme.highlight_id(name).and_then(|id| theme.get_capture_name(id as usize)).unwrap_or("-");
        out.push((sample[capture.node.byte_range()].to_string(), key.to_string()));
    }
    out
}

/// Asserts each `(text, key)` of `wanted` is a node of `sample` coloured as `key` (or a key below it).
fn assert_coloured(path: &str, sample: &str, wanted: &[(&str, &str)]) {
    let language = language_for(path).unwrap_or_else(|| panic!("{path} has a language"));
    let coloured = coloured(&language, sample);
    for (text, key) in wanted {
        let hit = coloured.iter().any(|(t, k)| t == text && (k == key || k.starts_with(&format!("{key}."))));
        assert!(hit, "{path}: {text:?} is coloured as {key}; got {:?}", coloured.iter().filter(|(t, _)| t == text).collect::<Vec<_>>());
    }
}

/// Capture names Zed's own queries also leave to a theme that may not colour them.
const UNTHEMED: &[&str] = &["module", "namespace", "none", "spell", "nospell", "conceal", "lifetime", "label"];

#[test]
fn every_compiled_capture_has_a_themed_name() {
    let theme = theme();
    for compiled in COMPILED {
        let language = language_for(compiled.suffixes[0]).unwrap_or_else(|| panic!("{} builds", compiled.name));
        let query = &language.grammar().unwrap().highlights_config.as_ref().unwrap().query;
        for name in query.capture_names() {
            let first = name.split('.').next().unwrap_or(name);
            let plain = name.starts_with('_') || UNTHEMED.contains(&first);
            assert!(plain || theme.highlight_id(name).is_some(), "{}: @{name} has no colour in Zed's themes", compiled.name);
        }
    }
}

#[test]
fn csharp_highlights() {
    let sample = "// note\nusing System;\nnamespace Demo {\n    public class Greeter {\n        int count = 42;\n        public string Greet() { return \"hi\"; }\n    }\n}\n";
    assert_coloured(
        "src/Greeter.cs",
        sample,
        &[("// note", "comment"), ("using", "keyword"), ("public", "keyword"), ("class", "keyword"), ("\"hi\"", "string"), ("42", "number"), ("Greeter", "type"), ("Greet", "function")],
    );
}

#[test]
fn languages_by_extension_and_name() {
    assert_eq!(name("src/main.rs").as_deref(), Some("Rust"));
    assert_eq!(name("a/b/app.tsx").as_deref(), Some("TSX"));
    assert_eq!(name("x.json").as_deref(), Some("JSON"));
    assert!(language_for("notes.unknownext").is_none());
    assert!(language_for("bin.dat").is_none());
    assert_eq!(name("tsconfig.json").as_deref(), Some("JSONC"), "a whole file name before its extension");
    let a = language_for("one.py").unwrap();
    let b = language_for("two.py").unwrap();
    assert!(Arc::ptr_eq(&a, &b), "built once");
}

#[test]
fn compiled_languages_by_extension_and_name() {
    for (path, want) in [("src\\Program.cs", "C#"), ("build.csx", "C#")] {
        assert_eq!(name(path).as_deref(), Some(want), "{path}");
    }
    assert!(language_for("Program.cs.unknownext").is_none(), "only the last extension counts");
}

mod rendered {
    use gpui::{TestAppContext, VisualTestContext};

    use crate::{Diff, SplitDiff, file_editor, highlighted, language_for, read_only_buffer};

    const OLD: &str = "// note\nclass Greeter {\n    string Greet() { return \"hi\"; }\n}\n";
    const NEW: &str = "// note\nclass Greeter {\n    string Greet() { return \"hello\"; }\n}\n";

    fn assert_csharp(runs: &[(String, String)], string: &str, side: &str) {
        for (text, key) in [("// note", "comment"), ("class", "keyword"), ("return", "keyword"), (string, "string"), ("Greeter", "type")] {
            assert!(runs.iter().any(|(t, k)| t == text && k.split('.').next() == Some(key)), "{side}: {text:?} painted as {key}; runs {runs:?}");
        }
    }

    #[gpui::test]
    fn a_csharp_file_is_coloured_in_the_file_view(cx: &mut TestAppContext) {
        cx.update(|cx| crate::init("Cascadia Mono", 13., cx));
        let cx: &mut VisualTestContext = cx.add_empty_window();
        let editor = cx.update(|window, cx| {
            let buffer = read_only_buffer(NEW.into(), language_for("Greeter.cs"), cx);
            file_editor(buffer, window, cx)
        });
        cx.run_until_parked();
        assert_csharp(&cx.update(|_, cx| highlighted(&editor, cx)), "\"hello\"", "file");
    }

    #[gpui::test]
    fn a_csharp_diff_is_coloured_on_both_sides_of_the_split(cx: &mut TestAppContext) {
        cx.update(|cx| crate::init("Cascadia Mono", 13., cx));
        let task = cx.update(|cx| Diff::compute(OLD.into(), NEW.into(), language_for("Greeter.cs"), cx));
        cx.run_until_parked();
        let diff = cx.foreground_executor().block_test(task);
        let window = cx.add_window(|window, cx| SplitDiff::new(&diff, window, cx));
        cx.run_until_parked();
        let (old, new) = window.update(cx, |s, _, cx| (highlighted(&s.old, cx), highlighted(&s.new, cx))).unwrap();
        assert_csharp(&old, "\"hi\"", "old");
        assert_csharp(&new, "\"hello\"", "new");
    }
}
