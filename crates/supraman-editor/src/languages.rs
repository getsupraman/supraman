mod captures;
mod compiled;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use language::{Language, LanguageConfig, LanguageMatcher};

use compiled::{COMPILED, Compiled};

/// The grammars Zed bundles natively, by their folder in the `grammars` crate; `javascript` and `jsonc`
/// reuse another's grammar.
const ZED: &[&str] = &[
    "bash", "c", "cpp", "css", "diff", "go", "gomod", "gowork", "javascript", "json", "jsonc", "markdown", "python", "rust", "tsx", "typescript", "yaml",
];

struct Known {
    /// `(key, suffixes)` of every language: Zed's from their configs, read once, then `COMPILED`'s.
    suffixes: Vec<(&'static str, Vec<String>)>,
    built: Mutex<HashMap<&'static str, Option<Arc<Language>>>>,
}

fn known() -> &'static Known {
    static KNOWN: OnceLock<Known> = OnceLock::new();
    KNOWN.get_or_init(|| {
        let zed = ZED.iter().map(|name| (*name, grammars::load_config(name).matcher.path_suffixes.iter().map(|s| s.to_string()).collect()));
        let compiled = COMPILED.iter().map(|c| (c.name, c.suffixes.iter().map(|s| s.to_string()).collect()));
        Known { suffixes: zed.chain(compiled).collect(), built: Mutex::new(HashMap::new()) }
    })
}

/// The language a path is highlighted as, by its file name or extension; built on first use (its
/// queries compile), so call it off the UI thread.
pub fn language_for(path: &str) -> Option<Arc<Language>> {
    let file = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let ext = file.rsplit_once('.').map(|(_, e)| e);
    let known = known();
    let by = |hit: &dyn Fn(&str) -> bool| known.suffixes.iter().find(|(_, s)| s.iter().any(|s| hit(s))).map(|(n, _)| *n);
    let name = by(&|s| s == file).or_else(|| by(&|s| Some(s) == ext))?;
    let mut built = known.built.lock().unwrap_or_else(|e| e.into_inner());
    built.entry(name).or_insert_with(|| build(name)).clone()
}

fn build(name: &str) -> Option<Arc<Language>> {
    let language = match COMPILED.iter().find(|c| c.name == name) {
        Some(compiled) => build_compiled(compiled),
        None => {
            let config = grammars::load_config(name);
            let grammar = grammars::native_grammars().into_iter().find(|(g, _)| Some(*g) == config.grammar.as_deref()).map(|(_, g)| g)?;
            Language::new(config, Some(grammar)).with_queries(grammars::load_queries(name))
        }
    };
    match language {
        Ok(language) => Some(Arc::new(language)),
        Err(e) => {
            log::warn!("the {name} grammar's queries do not load: {e:#}");
            None
        }
    }
}

fn build_compiled(compiled: &Compiled) -> anyhow::Result<Language> {
    let config = LanguageConfig {
        name: compiled.name.into(),
        matcher: LanguageMatcher { path_suffixes: compiled.suffixes.iter().map(|s| (*s).into()).collect(), ..Default::default() }.into(),
        ..Default::default()
    };
    let grammar = tree_sitter::Language::new(compiled.grammar);
    Language::new(config, Some(grammar)).with_highlights_query(&captures::themed(compiled.highlights))
}

#[cfg(test)]
mod tests;
