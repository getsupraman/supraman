mod captures;
mod compiled;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use gpui::{App, AppContext as _};
use language::{Language, LanguageConfig, LanguageMatcher, LanguageRegistry};
use theme::{ActiveTheme as _, Theme};

use compiled::{COMPILED, Compiled};

/// The grammars Zed bundles natively, by their folder in the `grammars` crate; `javascript` and `jsonc`
/// reuse another's grammar, and the last three have no file of their own: only injections reach them.
const ZED: &[&str] = &[
    "bash", "c", "cpp", "css", "diff", "go", "gomod", "gowork", "javascript", "json", "jsonc", "markdown", "python", "rust", "tsx", "typescript", "yaml",
    "markdown-inline", "jsdoc", "regex",
];

/// Names a fenced block's info string uses beside a language's name and suffixes, by the language's key.
const ALIASES: &[(&str, &[&str])] = &[
    ("C#", &["csharp", "c-sharp"]),
    ("bash", &["shell", "shellscript", "zsh"]),
    ("PowerShell", &["pwsh", "ps"]),
    ("Dockerfile", &["docker"]),
    ("Makefile", &["make"]),
    ("Protocol Buffers", &["protobuf"]),
    ("HCL", &["terraform"]),
    ("go", &["golang"]),
    ("javascript", &["node"]),
    ("F#", &["fsharp"]),
];

struct Known {
    /// `(key, suffixes)` of every language: Zed's from their configs, read once, then `COMPILED`'s.
    suffixes: Vec<(&'static str, Vec<String>)>,
}

fn known() -> &'static Known {
    static KNOWN: OnceLock<Known> = OnceLock::new();
    KNOWN.get_or_init(|| {
        let zed = ZED.iter().map(|name| (*name, grammars::load_config(name).matcher.path_suffixes.iter().map(|s| s.to_string()).collect()));
        let compiled = COMPILED.iter().map(|c| (c.name, c.suffixes.iter().map(|s| s.to_string()).collect()));
        Known { suffixes: zed.chain(compiled).collect() }
    })
}

/// Every language of `known`, built once: the first call compiles all their queries, a thread each.
fn built() -> &'static HashMap<&'static str, Arc<Language>> {
    static BUILT: OnceLock<HashMap<&'static str, Arc<Language>>> = OnceLock::new();
    BUILT.get_or_init(|| {
        std::thread::scope(|s| {
            let names = known().suffixes.iter().map(|(name, _)| *name);
            let threads: Vec<_> = names.map(|name| (name, s.spawn(move || build(name)))).collect();
            threads.into_iter().filter_map(|(name, t)| Some((name, t.join().ok()??))).collect()
        })
    })
}

/// The language a path is highlighted as, by its file name or extension. The first call builds every
/// language, so call it off the UI thread.
pub fn language_for(path: &str) -> Option<Arc<Language>> {
    let file = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let ext = file.rsplit_once('.').map(|(_, e)| e);
    let known = known();
    let by = |hit: &dyn Fn(&str) -> bool| known.suffixes.iter().find(|(_, s)| s.iter().any(|s| hit(s))).map(|(n, _)| *n);
    let name = by(&|s| s == file).or_else(|| by(&|s| ext.is_some_and(|e| e.eq_ignore_ascii_case(s))))?;
    built().get(name).cloned()
}

/// Builds every language off the UI thread, so the first file shown does not wait for it.
pub(crate) fn warm(cx: &App) {
    cx.background_spawn(async { built() }).detach();
}

static REGISTRY: OnceLock<Arc<LanguageRegistry>> = OnceLock::new();

/// The registry of every language, where a buffer's injections resolve, coloured by the active theme.
/// Only call it once `language_for` has returned a language: before, it would build them all here.
pub(crate) fn registry(cx: &App) -> Arc<LanguageRegistry> {
    let registry = REGISTRY.get_or_init(|| {
        let registry = Arc::new(LanguageRegistry::new(cx.background_executor().clone()));
        built().values().for_each(|language| registry.add(language.clone()));
        registry
    });
    retheme(registry, cx);
    registry.clone()
}

/// Each theme change re-colours every language, so open buffers paint in the new theme's syntax colours.
pub(crate) fn follow_theme(cx: &mut App) {
    cx.observe_global::<theme::GlobalTheme>(|cx| {
        if let Some(registry) = REGISTRY.get() {
            retheme(registry, cx);
        }
    })
    .detach();
}

fn retheme(registry: &LanguageRegistry, cx: &App) {
    static THEMED: Mutex<Option<Arc<Theme>>> = Mutex::new(None);
    let theme = cx.theme();
    let mut themed = THEMED.lock().unwrap_or_else(|e| e.into_inner());
    if !themed.as_ref().is_some_and(|t| Arc::ptr_eq(t, theme)) {
        registry.set_theme(theme.clone());
        *themed = Some(theme.clone());
    }
}

fn aliases(name: &str) -> impl Iterator<Item = gpui::SharedString> {
    ALIASES.iter().filter(move |(n, _)| *n == name).flat_map(|(_, a)| a.iter().map(|a| (*a).into()))
}

fn build(name: &str) -> Option<Arc<Language>> {
    let language = match COMPILED.iter().find(|c| c.name == name) {
        Some(compiled) => build_compiled(compiled, true).or_else(|e| {
            log::warn!("the {name} grammar's injections do not load: {e:#}");
            build_compiled(compiled, false)
        }),
        None => {
            let mut config = grammars::load_config(name);
            Arc::get_mut(&mut config.matcher).into_iter().for_each(|m| m.path_suffixes.extend(aliases(name)));
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

fn build_compiled(compiled: &Compiled, injections: bool) -> anyhow::Result<Language> {
    let path_suffixes = compiled.suffixes.iter().map(|s| (*s).into()).chain(aliases(compiled.name)).collect();
    let config = LanguageConfig { name: compiled.name.into(), matcher: LanguageMatcher { path_suffixes, ..Default::default() }.into(), ..Default::default() };
    let inherited = compiled.inherits.and_then(|name| grammars::load_queries(name).highlights).unwrap_or_default();
    let query = format!("{inherited}\n{}", captures::themed(compiled.highlights));
    let language = Language::new(config, Some((compiled.grammar)())).with_highlights_query(&query)?;
    match injections && !compiled.injections.is_empty() {
        true => language.with_injection_query(&compiled.injections.join("\n")),
        false => Ok(language),
    }
}

#[cfg(test)]
mod tests;
