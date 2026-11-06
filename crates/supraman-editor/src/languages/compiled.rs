use tree_sitter::Language;

/// A grammar compiled into Supraman beside Zed's, with its own highlights query.
pub(crate) struct Compiled {
    pub name: &'static str,
    /// Extensions and whole file names, as Zed's `path_suffixes`.
    pub suffixes: &'static [&'static str],
    pub grammar: fn() -> Language,
    pub highlights: &'static str,
    /// The Zed language whose highlights query `highlights` adds to.
    pub inherits: Option<&'static str>,
    /// Injection queries, joined.
    pub injections: &'static [&'static str],
}

/// The HTML around a PHP file's `<?php … ?>` blocks, one document across them.
const PHP_TEXT_IS_HTML: &str = "((text) @injection.content (#set! injection.language \"html\") (#set! injection.combined))";

pub(crate) const COMPILED: &[Compiled] = &[
    Compiled { name: "C#", suffixes: &["cs", "csx"], grammar: || tree_sitter_c_sharp::LANGUAGE.into(), highlights: tree_sitter_c_sharp::HIGHLIGHTS_QUERY, inherits: None, injections: &[] },
    Compiled { name: "Java", suffixes: &["java"], grammar: || tree_sitter_java::LANGUAGE.into(), highlights: tree_sitter_java::HIGHLIGHTS_QUERY, inherits: None, injections: &[] },
    Compiled { name: "Kotlin", suffixes: &["kt", "kts"], grammar: || tree_sitter_kotlin_sg::LANGUAGE.into(), highlights: tree_sitter_kotlin_sg::HIGHLIGHTS_QUERY, inherits: None, injections: &[] },
    Compiled { name: "PHP", suffixes: &["php", "phtml"], grammar: || tree_sitter_php::LANGUAGE_PHP.into(), highlights: tree_sitter_php::HIGHLIGHTS_QUERY, inherits: None, injections: &[tree_sitter_php::INJECTIONS_QUERY, PHP_TEXT_IS_HTML] },
    Compiled {
        name: "Ruby",
        suffixes: &["rb", "rake", "gemspec", "ru", "Gemfile", "Rakefile"],
        grammar: || tree_sitter_ruby::LANGUAGE.into(),
        highlights: tree_sitter_ruby::HIGHLIGHTS_QUERY, inherits: None, injections: &[],
    },
    Compiled { name: "HTML", suffixes: &["html", "htm"], grammar: || tree_sitter_html::LANGUAGE.into(), highlights: tree_sitter_html::HIGHLIGHTS_QUERY, inherits: None, injections: &[tree_sitter_html::INJECTIONS_QUERY] },
    Compiled {
        name: "XML",
        suffixes: &["xml", "xsd", "xsl", "xslt", "csproj", "fsproj", "vbproj", "props", "targets", "nuspec", "xaml", "axaml", "resx", "config", "svg", "slnx"],
        grammar: || tree_sitter_xml::LANGUAGE_XML.into(),
        highlights: tree_sitter_xml::XML_HIGHLIGHT_QUERY, inherits: None, injections: &[],
    },
    Compiled { name: "SQL", suffixes: &["sql"], grammar: || tree_sitter_sequel::LANGUAGE.into(), highlights: tree_sitter_sequel::HIGHLIGHTS_QUERY, inherits: None, injections: &[] },
    Compiled { name: "TOML", suffixes: &["toml", "Cargo.lock"], grammar: || tree_sitter_toml_ng::LANGUAGE.into(), highlights: tree_sitter_toml_ng::HIGHLIGHTS_QUERY, inherits: None, injections: &[] },
    Compiled {
        name: "PowerShell",
        suffixes: &["ps1", "psm1", "psd1"],
        grammar: || tree_sitter_powershell::LANGUAGE.into(),
        highlights: tree_sitter_powershell::HIGHLIGHTS_QUERY, inherits: None, injections: &[],
    },
    Compiled { name: "Batch", suffixes: &["bat", "cmd"], grammar: || tree_sitter_batch::LANGUAGE.into(), highlights: tree_sitter_batch::HIGHLIGHTS_QUERY, inherits: None, injections: &[] },
    Compiled {
        name: "Dockerfile",
        suffixes: &["Dockerfile", "dockerfile", "Containerfile", "containerfile"],
        grammar: || tree_sitter_containerfile::LANGUAGE.into(),
        highlights: tree_sitter_containerfile::HIGHLIGHTS_QUERY, inherits: None, injections: &[tree_sitter_containerfile::INJECTIONS_QUERY],
    },
    Compiled { name: "SCSS", suffixes: &["scss"], grammar: tree_sitter_scss::language, highlights: tree_sitter_scss::HIGHLIGHTS_QUERY, inherits: Some("css"), injections: &[] },
    Compiled { name: "Less", suffixes: &["less"], grammar: tree_sitter_less::language, highlights: tree_sitter_less::HIGHLIGHTS_QUERY, inherits: Some("css"), injections: &[] },
    Compiled { name: "Lua", suffixes: &["lua"], grammar: || tree_sitter_lua::LANGUAGE.into(), highlights: tree_sitter_lua::HIGHLIGHTS_QUERY, inherits: None, injections: &[] },
    Compiled { name: "Scala", suffixes: &["scala", "sc", "sbt"], grammar: || tree_sitter_scala::LANGUAGE.into(), highlights: tree_sitter_scala::HIGHLIGHTS_QUERY, inherits: None, injections: &[] },
    Compiled { name: "Dart", suffixes: &["dart"], grammar: || tree_sitter_dart::LANGUAGE.into(), highlights: tree_sitter_dart::HIGHLIGHTS_QUERY, inherits: None, injections: &[] },
    Compiled { name: "Swift", suffixes: &["swift"], grammar: || tree_sitter_swift::LANGUAGE.into(), highlights: tree_sitter_swift::HIGHLIGHTS_QUERY, inherits: None, injections: &[] },
    Compiled { name: "F#", suffixes: &["fs", "fsi", "fsx"], grammar: || tree_sitter_fsharp::LANGUAGE_FSHARP.into(), highlights: tree_sitter_fsharp::HIGHLIGHTS_QUERY, inherits: None, injections: &[] },
    Compiled { name: "R", suffixes: &["r", "R"], grammar: || tree_sitter_r::LANGUAGE.into(), highlights: tree_sitter_r::HIGHLIGHTS_QUERY, inherits: None, injections: &[] },
    Compiled {
        name: "GraphQL",
        suffixes: &["graphql", "graphqls", "gql"],
        grammar: || tree_sitter_graphql::LANGUAGE.into(),
        highlights: include_str!("../../queries/graphql/highlights.scm"), inherits: None, injections: &[],
    },
    Compiled { name: "Protocol Buffers", suffixes: &["proto"], grammar: || tree_sitter_proto::LANGUAGE.into(), highlights: include_str!("../../queries/proto/highlights.scm"), inherits: None, injections: &[] },
    Compiled { name: "HCL", suffixes: &["tf", "tfvars", "hcl"], grammar: || tree_sitter_hcl::LANGUAGE.into(), highlights: include_str!("../../queries/hcl/highlights.scm"), inherits: None, injections: &[] },
    Compiled { name: "INI", suffixes: &["ini", "editorconfig", "gitconfig", "gitmodules"], grammar: || tree_sitter_ini::LANGUAGE.into(), highlights: tree_sitter_ini::HIGHLIGHTS_QUERY, inherits: None, injections: &[] },
    Compiled {
        name: "Makefile",
        suffixes: &["Makefile", "makefile", "GNUmakefile", "mk"],
        grammar: || tree_sitter_make::LANGUAGE.into(),
        highlights: tree_sitter_make::HIGHLIGHTS_QUERY, inherits: None, injections: &[],
    },
];
