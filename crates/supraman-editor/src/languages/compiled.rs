use tree_sitter_language::LanguageFn;

/// A grammar compiled into Supraman beside Zed's, with its own highlights query.
pub(crate) struct Compiled {
    pub name: &'static str,
    /// Extensions and whole file names, as Zed's `path_suffixes`.
    pub suffixes: &'static [&'static str],
    pub grammar: LanguageFn,
    pub highlights: &'static str,
}

pub(crate) const COMPILED: &[Compiled] = &[Compiled {
    name: "C#",
    suffixes: &["cs", "csx"],
    grammar: tree_sitter_c_sharp::LANGUAGE,
    highlights: tree_sitter_c_sharp::HIGHLIGHTS_QUERY,
}];
