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

/// Each `(text, key)` of `wanted` that is no node of `sample` coloured as `key` (or a key below it).
fn misses(path: &str, sample: &str, wanted: &[(&str, &str)]) -> Vec<String> {
    let language = language_for(path).unwrap_or_else(|| panic!("{path} has a language"));
    let coloured = coloured(&language, sample);
    let hit = |text: &str, key: &str| coloured.iter().any(|(t, k)| t == text && (k == key || k.starts_with(&format!("{key}."))));
    let missed: Vec<_> = wanted.iter().filter(|(text, key)| !hit(text, key)).map(|(text, key)| format!("{path}: {text:?} as {key}")).collect();
    if !missed.is_empty() {
        eprintln!("{path} colours {coloured:?}");
    }
    missed
}

/// First parts of capture names no theme needs to colour: names Zed's own queries leave to a theme, text a
/// document shows plain, and the helpers a query's predicates match on.
const UNTHEMED: &[&str] =
    &["module", "namespace", "none", "spell", "nospell", "conceal", "lifetime", "label", "text", "error", "array", "assignvalue", "clean", "selector"];

#[test]
fn every_compiled_capture_has_a_themed_name() {
    let theme = theme();
    let mut uncoloured = Vec::new();
    for compiled in COMPILED {
        let language = language_for(compiled.suffixes[0]).unwrap_or_else(|| panic!("{} builds", compiled.name));
        let query = &language.grammar().unwrap().highlights_config.as_ref().unwrap().query;
        for name in query.capture_names() {
            let first = name.split('.').next().unwrap_or(name);
            if !(name.starts_with('_') || UNTHEMED.contains(&first) || theme.highlight_id(name).is_some()) {
                uncoloured.push(format!("{}: @{name}", compiled.name));
            }
        }
    }
    assert!(uncoloured.is_empty(), "captures with no colour in Zed's themes: {uncoloured:#?}");
}

#[test]
fn csharp_highlights() {
    let sample = "// note\nusing System;\nnamespace Demo {\n    public class Greeter {\n        int count = 42;\n        public string Greet() { return \"hi\"; }\n    }\n}\n";
    let wanted = [("// note", "comment"), ("using", "keyword"), ("public", "keyword"), ("class", "keyword"), ("\"hi\"", "string"), ("42", "number"), ("Greeter", "type"), ("Greet", "function")];
    assert_eq!(misses("src/Greeter.cs", sample, &wanted), Vec::<String>::new());
}

/// `(path, sample, [(text, key)])`: each compiled language's keyword, string and comment, or its equivalents.
const SAMPLES: &[(&str, &str, &[(&str, &str)])] = &[
    ("Main.java", "// note\nclass Main { String s = \"hi\"; int n = 42; }\n", &[("// note", "comment"), ("class", "keyword"), ("\"hi\"", "string"), ("42", "number")]),
    ("Main.kt", "// note\nfun main() { val s = \"hi\" }\n", &[("// note", "comment"), ("fun", "keyword"), ("val", "keyword"), ("\"hi\"", "string")]),
    ("index.php", "<?php\n// note\nfunction f() { return 'hi'; }\n", &[("// note", "comment"), ("function", "keyword"), ("return", "keyword"), ("'hi'", "string")]),
    ("app.rb", "# note\ndef greet\n  \"hi\"\nend\n", &[("# note", "comment"), ("def", "keyword"), ("end", "keyword"), ("\"hi\"", "string")]),
    ("index.html", "<!-- note -->\n<p class=\"x\">hi</p>\n", &[("<!-- note -->", "comment"), ("p", "tag"), ("class", "attribute"), ("x", "string")]),
    ("App.csproj", "<?xml version=\"1.0\"?>\n<!-- note -->\n<Project Sdk=\"Microsoft.NET.Sdk\"></Project>\n", &[("<!-- note -->", "comment"), ("Project", "tag"), ("Sdk", "property"), ("\"Microsoft.NET.Sdk\"", "string")]),
    ("q.sql", "-- note\nSELECT name FROM users WHERE id = 'x';\n", &[("-- note", "comment"), ("SELECT", "keyword"), ("FROM", "keyword"), ("'x'", "string")]),
    ("Cargo.toml", "# note\n[package]\nname = \"x\"\nversion = 1\n", &[("# note", "comment"), ("package", "type"), ("\"x\"", "string"), ("1", "number")]),
    ("build.ps1", "# note\nfunction Get-X { if ($true) { return \"hi\" } }\n", &[("# note", "comment"), ("function", "keyword"), ("if", "keyword"), ("\"hi\"", "string")]),
    ("run.cmd", "REM note\n@echo off\nset NAME=x\nif exist a.txt echo \"hi\"\n", &[("REM note", "comment"), ("set", "keyword"), ("echo", "function"), ("\"hi\"", "string")]),
    ("Dockerfile", "# note\nFROM alpine:3\nRUN echo \"hi\"\n", &[("# note\n", "comment"), ("FROM", "keyword"), ("RUN", "keyword")]),
    ("site.scss", "// note\n@mixin m { color: red; }\n.a { content: \"hi\"; }\n", &[("// note", "comment"), ("@mixin", "keyword"), ("\"hi\"", "string"), ("color", "property")]),
    ("site.less", "// note\n@import \"x\";\n.a { color: red; }\n", &[("// note", "comment"), ("@import", "keyword"), ("\"x\"", "string"), ("color", "property")]),
    ("init.lua", "-- note\nlocal function f() return \"hi\" end\n", &[("-- note", "comment"), ("local", "keyword"), ("return", "keyword"), ("\"hi\"", "string")]),
    ("Main.scala", "// note\nobject Main { def f = \"hi\" }\n", &[("// note", "comment"), ("object", "keyword"), ("def", "keyword"), ("\"hi\"", "string")]),
    ("main.dart", "// note\nclass A { String f() { return 'hi'; } }\n", &[("// note", "comment"), ("class", "keyword"), ("return", "keyword"), ("'hi'", "string")]),
    ("main.swift", "// note\nfunc f() -> String { return \"hi\" }\n", &[("// note", "comment"), ("func", "keyword"), ("return", "keyword"), ("hi", "string")]),
    ("Lib.fs", "// note\nlet f x = if x then \"hi\" else \"no\"\n", &[("// note", "comment"), ("let", "keyword"), ("if", "keyword"), ("\"hi\"", "string")]),
    ("plot.R", "# note\nf <- function(x) { if (x) \"hi\" }\n", &[("# note", "comment"), ("function", "keyword"), ("if", "keyword"), ("\"hi\"", "string")]),
    ("schema.graphql", "# note\ntype Query { name(id: ID): String @deprecated(reason: \"hi\") }\n", &[("# note", "comment"), ("type", "keyword"), ("\"hi\"", "string"), ("Query", "type")]),
    ("a.proto", "// note\nsyntax = \"proto3\";\nmessage A { string name = 1; }\n", &[("// note", "comment"), ("message", "keyword"), ("\"proto3\"", "string"), ("1", "number")]),
    ("main.tf", "# note\nresource \"aws_s3_bucket\" \"b\" {\n  count = 2\n  name = true\n}\n", &[("# note", "comment"), ("resource", "keyword"), ("aws_s3_bucket", "string"), ("2", "number"), ("true", "boolean")]),
    ("app.ini", "; note\n[section]\nkey = value\n", &[("; note\n", "comment"), ("section", "type"), ("key", "property")]),
    ("Makefile", "# note\ninclude rules.mk\nifeq ($(X),1)\nall: build\nendif\n", &[("# note", "comment"), ("include", "keyword"), ("ifeq", "keyword")]),
];

#[test]
fn compiled_languages_highlight() {
    let missed: Vec<_> = SAMPLES.iter().flat_map(|(path, sample, wanted)| misses(path, sample, wanted)).collect();
    assert!(missed.is_empty(), "not coloured: {missed:#?}");
    let covered: Vec<_> = SAMPLES.iter().map(|(path, ..)| name(path).unwrap()).chain(["C#".to_string()]).collect();
    for compiled in COMPILED {
        assert!(covered.iter().any(|n| n == compiled.name), "{} has a sample", compiled.name);
    }
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
    for compiled in COMPILED {
        for suffix in compiled.suffixes {
            if !suffix.contains('.') {
                assert_eq!(name(&format!("a/b.{suffix}")).as_deref(), Some(compiled.name), "*.{suffix}");
            }
            assert_eq!(name(&format!("a\\{suffix}")).as_deref(), Some(compiled.name), "a file named {suffix}");
        }
    }
    let named = [
        ("src\\Program.cs", "C#"),
        ("build.csx", "C#"),
        ("App.csproj", "XML"),
        ("Directory.Build.props", "XML"),
        ("Directory.Build.targets", "XML"),
        ("MainWindow.xaml", "XML"),
        ("Strings.resx", "XML"),
        ("web.config", "XML"),
        ("icon.svg", "XML"),
        ("scripts/heavy-lock.ps1", "PowerShell"),
        ("Module.psm1", "PowerShell"),
        ("Module.psd1", "PowerShell"),
        ("build.bat", "Batch"),
        ("build.cmd", "Batch"),
        ("Dockerfile", "Dockerfile"),
        ("web.dockerfile", "Dockerfile"),
        ("site.scss", "SCSS"),
        ("site.less", "Less"),
        ("Index.cshtml", "-"),
        ("App.vb", "-"),
        ("infra/main.tf", "HCL"),
        ("config.hcl", "HCL"),
        ("setup.ini", "INI"),
        (".editorconfig", "INI"),
        (".gitconfig", "INI"),
        ("Makefile", "Makefile"),
        ("Cargo.lock", "TOML"),
        ("Gemfile", "Ruby"),
    ];
    for (path, want) in named {
        assert_eq!(name(path).as_deref().unwrap_or("-"), want, "{path}");
    }
    assert!(language_for("Program.cs.unknownext").is_none(), "only the last extension counts");
    assert!(language_for("Dockerfile.unknownext").is_none(), "a known name before an unknown extension stays plain");
}

#[test]
fn every_injection_query_loads() {
    for compiled in COMPILED.iter().filter(|c| !c.injections.is_empty()) {
        let language = language_for(compiled.suffixes[0]).unwrap();
        assert!(language.grammar().unwrap().injection_config.is_some(), "{}'s injections", compiled.name);
    }
    for name in ["Markdown", "Markdown-Inline", "JSDoc", "Regex"] {
        assert!(built().values().any(|l| l.name().as_ref() == name), "{name} is built");
    }
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

    fn runs_of(path: &str, text: &str, cx: &mut TestAppContext) -> Vec<(String, String)> {
        cx.update(|cx| crate::init("Cascadia Mono", 13., cx));
        let cx: &mut VisualTestContext = cx.add_empty_window();
        let editor = cx.update(|window, cx| {
            let buffer = read_only_buffer(text.into(), language_for(path), cx);
            file_editor(buffer, window, cx)
        });
        cx.run_until_parked();
        cx.update(|_, cx| highlighted(&editor, cx))
    }

    /// Each `(text, key)` of `wanted` no run of `runs` paints: a run holding `text` whose theme key is `key` or below it.
    fn unpainted(runs: &[(String, String)], wanted: &[(&str, &str)]) -> Vec<String> {
        let hit = |text: &str, key: &str| runs.iter().any(|(t, k)| t.contains(text) && (k == key || k.starts_with(&format!("{key}."))));
        wanted.iter().filter(|(t, k)| !hit(t, k)).map(|(t, k)| format!("{t:?} as {k}")).collect()
    }

    const MARKDOWN: &str = "# Title

- an item with `a code span`, **bold**, *italic* and [a link](https://example.com)

| a | b |
|---|---|
| 1 | 2 |
";

    #[gpui::test]
    fn markdown_colours_its_inline_text(cx: &mut TestAppContext) {
        let runs = runs_of("README.md", MARKDOWN, cx);
        let wanted = [
            ("# Title", "title"),
            ("-", "punctuation.list_marker"),
            ("`a code span`", "text.literal"),
            ("**bold**", "emphasis.strong"),
            ("*italic*", "emphasis"),
            ("a link", "link_text"),
            ("https://example.com", "link_uri"),
            ("|", "punctuation"),
        ];
        assert_eq!(unpainted(&runs, &wanted), Vec::<String>::new(), "runs {runs:?}");
    }

    #[gpui::test]
    fn fenced_code_takes_its_language(cx: &mut TestAppContext) {
        let text = "Some code:

```rust
fn main() { let s = \"hi\"; }
```

```csharp
public class A { }
```

```toml
[package]
name = \"x\"
```

```powershell
function Get-X { return 1 }
```
";
        let runs = runs_of("notes.md", text, cx);
        let wanted = [("fn", "keyword"), ("\"hi\"", "string"), ("public", "keyword"), ("A", "type"), ("package", "type"), ("\"x\"", "string"), ("function", "keyword")];
        assert_eq!(unpainted(&runs, &wanted), Vec::<String>::new(), "runs {runs:?}");
    }

    #[gpui::test]
    fn html_colours_its_script_and_style(cx: &mut TestAppContext) {
        let text = "<p>hi</p>
<script>
const n = 42;
function f() { return \"s\"; }
</script>
<style>
a { color: red; }
</style>
";
        let runs = runs_of("index.html", text, cx);
        let wanted = [("p", "tag"), ("const", "keyword"), ("function", "keyword"), ("42", "number"), ("\"s\"", "string"), ("color", "property")];
        assert_eq!(unpainted(&runs, &wanted), Vec::<String>::new(), "runs {runs:?}");
    }

    #[gpui::test]
    fn php_colours_the_html_around_it(cx: &mut TestAppContext) {
        let runs = runs_of("index.php", "<p class=\"x\">hi</p>
<?php echo 'hi'; ?>
<b>bye</b>
", cx);
        let wanted = [("p", "tag"), ("class", "attribute"), ("b", "tag"), ("echo", "keyword"), ("'hi'", "string")];
        assert_eq!(unpainted(&runs, &wanted), Vec::<String>::new(), "runs {runs:?}");
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
