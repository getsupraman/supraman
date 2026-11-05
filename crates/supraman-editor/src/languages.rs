use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use language::Language;

/// The grammars Zed bundles natively, by their folder in the `grammars` crate; `javascript` and `jsonc`
/// reuse another's grammar.
const NAMES: &[&str] = &[
    "bash", "c", "cpp", "css", "diff", "go", "gomod", "gowork", "javascript", "json", "jsonc", "markdown", "python", "rust", "tsx", "typescript", "yaml",
];

struct Known {
    /// `(name, suffixes)` from each config, read once.
    suffixes: Vec<(&'static str, Vec<String>)>,
    built: Mutex<HashMap<&'static str, Option<Arc<Language>>>>,
}

fn known() -> &'static Known {
    static KNOWN: OnceLock<Known> = OnceLock::new();
    KNOWN.get_or_init(|| Known {
        suffixes: NAMES.iter().map(|name| (*name, grammars::load_config(name).matcher.path_suffixes.iter().map(|s| s.to_string()).collect())).collect(),
        built: Mutex::new(HashMap::new()),
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
    let config = grammars::load_config(name);
    let grammar = grammars::native_grammars().into_iter().find(|(g, _)| Some(*g) == config.grammar.as_deref()).map(|(_, g)| g)?;
    let language = Language::new(config, Some(grammar)).with_queries(grammars::load_queries(name));
    match language {
        Ok(language) => Some(Arc::new(language)),
        Err(e) => {
            log::warn!("the {name} grammar's queries do not load: {e:#}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages_by_extension_and_name() {
        assert_eq!(language_for("src/main.rs").map(|l| l.name().to_string()).as_deref(), Some("Rust"));
        assert_eq!(language_for("a/b/app.tsx").map(|l| l.name().to_string()).as_deref(), Some("TSX"));
        assert_eq!(language_for("x.json").map(|l| l.name().to_string()).as_deref(), Some("JSON"));
        assert!(language_for("notes.unknownext").is_none());
        assert!(language_for("bin.dat").is_none());
        assert_eq!(language_for("tsconfig.json").map(|l| l.name().to_string()).as_deref(), Some("JSONC"), "a whole file name before its extension");
        let a = language_for("one.py").unwrap();
        let b = language_for("two.py").unwrap();
        assert!(Arc::ptr_eq(&a, &b), "built once");
    }
}
