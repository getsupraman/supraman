//! The ratchets (UI-KIT §11): per-file counts of what the kit replaces, held to `ratchet/ceilings.toml`, which may only go down.
//!
//! Each file is read without its comments, string and char literals, `tests.rs`, `tests/` and its `#[cfg(test)]` items and modules.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub const FEATURE_CRATES: [&str; 3] = ["supraman-app", "supraman-git-view", "supraman-work-view"];
pub const KIT_CRATES: [&str; 1] = ["supraman-ui"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// `overflow_y_scroll(`, `overflow_x_scroll(`, `overflow_scroll(`.
    OverflowScroll,
    /// A focus change while a `render*` fn runs: `window.focus(`, `.focus(window`, a `.focus(` on a line naming `focus_handle`, or a call
    /// passing `window` to a fn whose own body changes the focus; none inside an `on_*`, `listener`, `processor`, `spawn*`, `defer*`,
    /// `subscribe*` or `observe*` call's arguments, which run later.
    RenderFocus,
    /// `.hover(`: a feature's own hover fill.
    Hover,
    /// `px(` in a feature crate (not the `.px(` padding method).
    Px,
    /// `px(` in the kit's own sources.
    KitPx,
    /// `ink::` or a `use` of `ink` in a feature crate: a mark beside text is a `label::Label`.
    Ink,
}

impl Kind {
    pub const ALL: [Kind; 6] = [Kind::OverflowScroll, Kind::RenderFocus, Kind::Hover, Kind::Px, Kind::KitPx, Kind::Ink];

    /// The table of `ceilings.toml` holding this kind.
    pub fn key(self) -> &'static str {
        match self {
            Kind::OverflowScroll => "overflow_scroll",
            Kind::RenderFocus => "render_focus",
            Kind::Hover => "hover",
            Kind::Px => "px",
            Kind::KitPx => "kit_px",
            Kind::Ink => "ink",
        }
    }

    fn words(self) -> &'static str {
        match self {
            Kind::OverflowScroll => "overflow_*scroll calls",
            Kind::RenderFocus => "render-time focus changes",
            Kind::Hover => ".hover( calls",
            Kind::Px => "px( literals",
            Kind::KitPx => "px( literals",
            Kind::Ink => "ink:: uses",
        }
    }

    pub fn crates(self) -> &'static [&'static str] {
        match self {
            Kind::KitPx => &KIT_CRATES,
            _ => &FEATURE_CRATES,
        }
    }
}

/// Where each kind was found: `crate/src/path.rs` and its lines, one per occurrence.
pub type Found = BTreeMap<Kind, BTreeMap<String, Vec<usize>>>;

/// The ceilings: kind's table key → file → ceiling.
pub type Ceilings = BTreeMap<String, BTreeMap<String, usize>>;

pub const CEILINGS: &str = include_str!("ratchet/ceilings.toml");

pub fn ceilings(text: &str) -> Ceilings {
    toml::from_str(text).expect("ratchet/ceilings.toml parses")
}

/// A scanned source: its key, and its code with every stripped character a space, lines kept.
struct Source {
    key: String,
    code: String,
}

/// Every kind's occurrences in the counted sources under `crates`.
pub fn scan(crates: &Path) -> Found {
    let mut found = Found::new();
    let mut members: BTreeSet<&str> = BTreeSet::new();
    for kind in Kind::ALL {
        members.extend(kind.crates());
        found.entry(kind).or_default();
    }
    let sources: BTreeMap<&str, Vec<Source>> = members.iter().map(|m| (*m, read_crate(crates, m))).collect();
    let focusing = Focusing::new(sources.values().flatten());
    for (member, files) in &sources {
        for source in files {
            for kind in Kind::ALL.into_iter().filter(|k| k.crates().contains(member)) {
                let lines = match kind {
                    Kind::OverflowScroll => calls(&source.code, &["overflow_y_scroll(", "overflow_x_scroll(", "overflow_scroll("], |before| !before.is_some_and(is_ident)),
                    Kind::Hover => calls(&source.code, &[".hover("], |_| true),
                    Kind::Px | Kind::KitPx => calls(&source.code, &["px("], |before| !before.is_some_and(|c| c == '.' || is_ident(c))),
                    Kind::RenderFocus => render_focus(&source.code, &focusing),
                    Kind::Ink => calls(&source.code, &["ink::", "ink;", "ink,", "ink}"], |before| !before.is_some_and(is_ident)),
                };
                if !lines.is_empty() {
                    found.entry(kind).or_default().insert(source.key.clone(), lines);
                }
            }
        }
    }
    found
}

/// Each kind's counts against `ceilings`: a count over its file's ceiling (0 for a file not listed) or under it.
pub fn problems(found: &Found, ceilings: &Ceilings, kind: Kind) -> Vec<String> {
    let empty = BTreeMap::new();
    let table = ceilings.get(kind.key()).unwrap_or(&empty);
    let counts = found.get(&kind).cloned().unwrap_or_default();
    let files: BTreeSet<&String> = table.keys().chain(counts.keys()).collect();
    let mut out = Vec::new();
    for file in files {
        let lines = counts.get(file).map(Vec::as_slice).unwrap_or_default();
        let (n, ceiling) = (lines.len(), table.get(file).copied().unwrap_or(0));
        if n > ceiling {
            out.push(format!("{file}: {n} {}, over its ceiling {ceiling} (lines {lines:?})", kind.words()));
        } else if n < ceiling {
            out.push(format!("{file}: {n} {}, below its ceiling {ceiling}: lower the ceiling to {n} in [{}]", kind.words(), kind.key()));
        }
    }
    out
}

/// Today's counts as `ceilings.toml`'s text.
pub fn table(found: &Found) -> String {
    let mut out = String::new();
    for kind in Kind::ALL {
        out.push_str(&format!("[{}]\n", kind.key()));
        for (file, lines) in found.get(&kind).into_iter().flatten() {
            out.push_str(&format!("\"{file}\" = {}\n", lines.len()));
        }
        out.push('\n');
    }
    out
}

fn read_crate(crates: &Path, member: &str) -> Vec<Source> {
    let src = crates.join(member).join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    let mut excluded: Vec<PathBuf> = Vec::new();
    let mut read = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        let (code, test_mods) = without_tests(&code_of(&text));
        let base = match file.file_stem().and_then(|s| s.to_str()) {
            Some("lib" | "main" | "mod") => file.parent().map(Path::to_path_buf).unwrap_or_default(),
            _ => file.with_extension(""),
        };
        for name in test_mods {
            excluded.push(base.join(format!("{name}.rs")));
            excluded.push(base.join(name));
        }
        read.push((file, code));
    }
    read.into_iter()
        .filter(|(file, _)| !excluded.iter().any(|x| file.starts_with(x)))
        .map(|(file, code)| {
            let key = file.strip_prefix(crates).unwrap_or(&file).to_string_lossy().replace('\\', "/");
            Source { key, code }
        })
        .collect()
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        if path.is_dir() {
            if name != "tests" {
                rust_files(&path, out);
            }
        } else if name.ends_with(".rs") && name != "tests.rs" {
            out.push(path);
        }
    }
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `text` with its comments and string and char literals blanked, lines kept.
pub fn code_of(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        match blanked_end(&chars, i) {
            Some(end) => {
                out.extend(chars[i..end].iter().map(|c| if *c == '\n' { '\n' } else { ' ' }));
                i = end;
            }
            None => {
                out.push(chars[i]);
                i += 1;
            }
        }
    }
    out
}

/// Where the comment or literal starting at `i` ends, if one does.
fn blanked_end(chars: &[char], i: usize) -> Option<usize> {
    let (c, next) = (chars[i], chars.get(i + 1).copied());
    let after = |from: usize, close: &[char]| (from..chars.len()).find(|j| chars[*j..].starts_with(close)).map_or(chars.len(), |j| j + close.len());
    match (c, next) {
        ('/', Some('/')) => Some((i..chars.len()).find(|j| chars[*j] == '\n').unwrap_or(chars.len())),
        ('/', Some('*')) => Some(block_comment_end(chars, i)),
        ('"', _) => Some(string_end(chars, i + 1)),
        ('\'', Some('\\')) => Some(after(i + 3, &['\''])),
        ('\'', _) if chars.get(i + 2) == Some(&'\'') => Some(i + 3),
        ('r', _) | ('b', Some('r')) if i == 0 || !is_ident(chars[i - 1]) => {
            let start = if c == 'b' { i + 2 } else { i + 1 };
            let hashes = chars[start..].iter().take_while(|c| **c == '#').count();
            let close: Vec<char> = std::iter::once('"').chain(std::iter::repeat_n('#', hashes)).collect();
            (chars.get(start + hashes) == Some(&'"')).then(|| after(start + hashes + 1, &close))
        }
        _ => None,
    }
}

fn block_comment_end(chars: &[char], i: usize) -> usize {
    let mut depth = 0;
    let mut j = i;
    while j < chars.len() {
        if chars[j..].starts_with(&['/', '*']) {
            depth += 1;
            j += 2;
        } else if chars[j..].starts_with(&['*', '/']) {
            depth -= 1;
            j += 2;
            if depth == 0 {
                return j;
            }
        } else {
            j += 1;
        }
    }
    chars.len()
}

/// The end of a `"…"` string whose text starts at `from`, past its closing quote.
fn string_end(chars: &[char], from: usize) -> usize {
    let mut j = from;
    while j < chars.len() && chars[j] != '"' {
        j += if chars[j] == '\\' { 2 } else { 1 };
    }
    (j + 1).min(chars.len())
}

/// `code` with every [`test_only`] item blanked, and the names of the out-of-line modules it gated.
pub fn without_tests(code: &str) -> (String, Vec<String>) {
    let mut chars: Vec<char> = code.chars().collect();
    let mut mods = Vec::new();
    let mut i = 0;
    while let Some(at) = find(&chars, i, "#[cfg(") {
        let Some(attr_end) = closing(&chars, at + 1) else { break };
        let attr: String = chars[at..=attr_end].iter().collect();
        if !test_only(&attr) {
            i = attr_end + 1;
            continue;
        }
        let mut j = attr_end + 1;
        loop {
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if chars.get(j) == Some(&'#') && chars.get(j + 1) == Some(&'[') {
                j = closing(&chars, j + 1).map_or(chars.len(), |e| e + 1);
            } else {
                break;
            }
        }
        let item_start = j;
        let mut depth = 0i32;
        let mut end = chars.len();
        while j < chars.len() {
            match chars[j] {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' => depth -= 1,
                '}' => {
                    depth -= 1;
                    if depth <= 0 {
                        end = j + 1;
                        break;
                    }
                }
                ';' if depth == 0 => {
                    end = j + 1;
                    break;
                }
                _ => {}
            }
            j += 1;
        }
        let item: String = chars[item_start..end].iter().collect();
        let words: Vec<&str> = item.split(|c: char| !is_ident(c)).filter(|w| !w.is_empty()).collect();
        if item.trim_end().ends_with(';')
            && let Some(m) = words.iter().position(|w| *w == "mod")
            && let Some(name) = words.get(m + 1)
        {
            mods.push(name.to_string());
        }
        for c in &mut chars[at..end] {
            if *c != '\n' {
                *c = ' ';
            }
        }
        i = end;
    }
    (chars.into_iter().collect(), mods)
}

/// A `#[cfg(…)]` that builds only under test: `test`, or `all(…)` naming it (not `any(test, …)`, not `not(test)`).
fn test_only(attr: &str) -> bool {
    let inner: String = attr.chars().filter(|c| !c.is_whitespace()).collect();
    inner == "#[cfg(test)]" || (inner.starts_with("#[cfg(all(") && has_word(&inner, "test") && !inner.contains("not(test"))
}

fn find(chars: &[char], from: usize, pat: &str) -> Option<usize> {
    let pat: Vec<char> = pat.chars().collect();
    (from..chars.len()).find(|i| chars[*i..].starts_with(&pat))
}

/// The bracket closing the one at `open`.
fn closing(chars: &[char], open: usize) -> Option<usize> {
    let (o, c) = match chars.get(open)? {
        '(' => ('(', ')'),
        '[' => ('[', ']'),
        '{' => ('{', '}'),
        _ => return None,
    };
    let mut depth = 0;
    for (j, ch) in chars.iter().enumerate().skip(open) {
        if *ch == o {
            depth += 1;
        } else if *ch == c {
            depth -= 1;
            if depth == 0 {
                return Some(j);
            }
        }
    }
    None
}

fn has_word(text: &str, word: &str) -> bool {
    text.split(|c: char| !is_ident(c)).any(|w| w == word)
}

fn line_of(chars: &[char], at: usize) -> usize {
    chars[..at].iter().filter(|c| **c == '\n').count() + 1
}

/// The lines of each occurrence of any of `pats` whose preceding character `keep` accepts.
fn calls(code: &str, pats: &[&str], keep: impl Fn(Option<char>) -> bool) -> Vec<usize> {
    let chars: Vec<char> = code.chars().collect();
    let pats: Vec<Vec<char>> = pats.iter().map(|p| p.chars().collect()).collect();
    let mut lines = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if let Some(p) = pats.iter().find(|p| chars[i..].starts_with(p))
            && keep(i.checked_sub(1).map(|j| chars[j]))
        {
            lines.push(line_of(&chars, i));
            i += p.len();
            continue;
        }
        i += 1;
    }
    lines
}

/// A fn: its name, the type of the `impl` holding it, whether it takes `self` and names `Window`, and its body's braces.
struct FnDef {
    name: String,
    owner: Option<String>,
    takes_self: bool,
    window: bool,
    body: (usize, usize),
}

impl FnDef {
    fn key(&self) -> String {
        match &self.owner {
            Some(owner) => format!("{owner}::{}", self.name),
            None => self.name.clone(),
        }
    }
}

/// Every `impl` block: its type's last path segment and its braces.
fn impls(chars: &[char]) -> Vec<(String, (usize, usize))> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(at) = find(chars, i, "impl") {
        i = at + 4;
        if (at > 0 && is_ident(chars[at - 1])) || chars.get(at + 4).is_some_and(|c| is_ident(*c)) {
            continue;
        }
        let Some((open, close)) = header_and_body(chars, at + 4) else { continue };
        let header: String = chars[at + 4..open].iter().collect();
        let mut depth = 0;
        let mut plain = String::new();
        for c in header.chars() {
            match c {
                '<' => depth += 1,
                '>' => depth -= 1,
                _ if depth == 0 => plain.push(c),
                _ => {}
            }
        }
        let target = plain.rsplit(" for ").next().unwrap_or(&plain);
        let target = target.split(" where ").next().unwrap_or(target);
        let name = target.split(|c: char| !is_ident(c)).filter(|w| !w.is_empty()).last().unwrap_or_default();
        if !name.is_empty() {
            out.push((name.to_string(), (open, close)));
        }
    }
    out
}

/// From `from`, the item's body braces: the first `{` outside brackets and its match; `None` at a `;` first.
fn header_and_body(chars: &[char], from: usize) -> Option<(usize, usize)> {
    let mut depth = 0i32;
    let mut j = from;
    loop {
        match chars.get(j)? {
            '(' | '<' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            '>' if j == 0 || chars[j - 1] != '-' => depth -= 1,
            ';' if depth <= 0 => return None,
            '{' if depth <= 0 => return closing(chars, j).map(|close| (j, close)),
            _ => {}
        }
        j += 1;
    }
}

fn fns(chars: &[char]) -> Vec<FnDef> {
    let impls = impls(chars);
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(at) = find(chars, i, "fn ") {
        i = at + 3;
        if at > 0 && is_ident(chars[at - 1]) {
            continue;
        }
        let name: String = chars[at + 3..].iter().take_while(|c| is_ident(**c)).collect();
        if name.is_empty() {
            continue;
        }
        let Some((open, close)) = header_and_body(chars, at + 3 + name.len()) else { continue };
        let signature: String = chars[at..open].iter().collect();
        let owner = impls.iter().filter(|(_, (o, c))| *o < at && at < *c).max_by_key(|(_, (o, _))| *o).map(|(n, _)| n.clone());
        let params = signature.split_once('(').map(|(_, p)| p).unwrap_or_default();
        let takes_self = params.split(',').next().is_some_and(|first| has_word(first, "self"));
        out.push(FnDef { name, owner, takes_self, window: signature.contains("Window"), body: (open, close) });
    }
    out
}

/// Calls whose arguments run later than the call (an event's handler, a task).
fn deferred(name: &str) -> bool {
    name.starts_with("on_")
        || name.starts_with("spawn")
        || name.starts_with("defer")
        || name.starts_with("subscribe")
        || name.starts_with("observe")
        || matches!(name, "listener" | "processor")
}

/// Names gpui and std give every entity or element, never followed as a call into a focusing fn.
const COMMON: [&str; 12] = ["update", "update_in", "read", "read_with", "new", "when", "when_some", "map", "and_then", "child", "children", "focus"];

/// The positions of `body` that run when it runs: not inside a [`deferred`] call's arguments, nor in a closure taking its own `window`
/// (a handler gpui or the owner calls later).
fn immediate(chars: &[char], body: (usize, usize)) -> Vec<bool> {
    let mut now = vec![false; chars.len()];
    let mut stack: Vec<bool> = Vec::new();
    let mut handler_until = 0;
    for i in body.0..=body.1.min(chars.len() - 1) {
        let later = stack.last().copied().unwrap_or(false) || i < handler_until;
        match chars[i] {
            '(' => stack.push(stack.last().copied().unwrap_or(false) || deferred(&ident_before(chars, i))),
            ')' => {
                stack.pop();
            }
            '|' if i >= handler_until => handler_until = handler_end(chars, i).unwrap_or(handler_until),
            _ => {}
        }
        now[i] = !later;
    }
    now
}

/// Where the closure opening at `bar` ends, when its parameters name `window`.
fn handler_end(chars: &[char], bar: usize) -> Option<usize> {
    let before = (0..bar).rev().find(|j| !chars[*j].is_whitespace())?;
    let opens = matches!(chars[before], '(' | ',' | '=' | '{' | ';') || ident_before(chars, before + 1) == "move";
    if !opens {
        return None;
    }
    let close = (bar + 1..chars.len()).find(|j| chars[*j] == '|')?;
    let params: String = chars[bar + 1..close].iter().collect();
    if !has_word(&params, "window") {
        return None;
    }
    let start = (close + 1..chars.len()).find(|j| !chars[*j].is_whitespace())?;
    if chars[start] == '{' {
        return closing(chars, start).map(|e| e + 1);
    }
    let mut depth = 0i32;
    for (j, c) in chars.iter().enumerate().skip(start) {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' if depth == 0 => return Some(j),
            ')' | ']' | '}' => depth -= 1,
            ',' | ';' if depth == 0 => return Some(j),
            _ => {}
        }
    }
    Some(chars.len())
}

fn ident_before(chars: &[char], at: usize) -> String {
    let mut k = at;
    while k > 0 && chars[k - 1].is_whitespace() {
        k -= 1;
    }
    let start = (0..k).rev().take_while(|j| is_ident(chars[*j])).last().unwrap_or(k);
    chars[start..k].iter().collect()
}

/// The direct focus changes in `body` where `now` holds.
fn direct_focus(chars: &[char], body: (usize, usize), now: &[bool]) -> Vec<usize> {
    let mut found = Vec::new();
    let mut i = body.0;
    let starts = |i: usize, p: &str| chars[i..].starts_with(&p.chars().collect::<Vec<_>>());
    while i <= body.1 {
        if !now[i] {
            i += 1;
            continue;
        }
        if starts(i, "window.focus(") && !(i > 0 && is_ident(chars[i - 1])) {
            found.push(i);
            i += "window.focus(".len();
            continue;
        }
        if starts(i, ".focus(") {
            let after = i + ".focus(".len();
            let arg: String = chars[after..].iter().skip_while(|c| c.is_whitespace()).take(6).collect();
            let line_start = (0..i).rev().find(|j| chars[*j] == '\n').map_or(0, |j| j + 1);
            let line_end = (i..chars.len()).find(|j| chars[*j] == '\n').unwrap_or(chars.len());
            let line: String = chars[line_start..line_end].iter().collect();
            if arg.starts_with("window") || (!arg.starts_with('|') && line.contains("focus_handle")) {
                found.push(i);
                i = after;
                continue;
            }
        }
        i += 1;
    }
    found
}

/// How a call names its fn: `Type::name(`, `.name(` or `name(`.
enum Call {
    Path(String, String),
    Method(String),
    Free(String),
}

/// The calls in `body` where `now` holds that pass `window` among their arguments.
fn calls_with_window(chars: &[char], body: (usize, usize), now: &[bool], owner: Option<&str>) -> Vec<(usize, Call)> {
    let mut out = Vec::new();
    let mut i = body.0;
    while i <= body.1 {
        if !now[i] || !is_ident(chars[i]) || (i > 0 && is_ident(chars[i - 1])) {
            i += 1;
            continue;
        }
        let name: String = chars[i..].iter().take_while(|c| is_ident(**c)).collect();
        let after = i + name.len();
        if chars.get(after) == Some(&'(') && !COMMON.contains(&name.as_str()) && !deferred(&name) && !name.starts_with("render") {
            let args: String = closing(chars, after).map(|e| chars[after..e].iter().collect()).unwrap_or_default();
            if has_word(&args, "window") {
                let call = if i >= 2 && chars[i - 1] == ':' && chars[i - 2] == ':' {
                    let path = ident_before(chars, i - 2);
                    Call::Path(if path == "Self" { owner.unwrap_or_default().to_string() } else { path }, name.clone())
                } else if i >= 1 && chars[i - 1] == '.' {
                    Call::Method(name.clone())
                } else {
                    Call::Free(name.clone())
                };
                out.push((i, call));
            }
        }
        i = after.max(i + 1);
    }
    out
}

/// Every scanned file's fns, and the keys of those that change the focus when they run, themselves or through a callee.
struct Focusing {
    files: Vec<(Vec<char>, Vec<FnDef>)>,
    keys: BTreeSet<String>,
    methods: BTreeSet<String>,
}

impl Focusing {
    fn new<'a>(sources: impl Iterator<Item = &'a Source>) -> Self {
        let files: Vec<(Vec<char>, Vec<FnDef>)> = sources
            .map(|s| {
                let chars: Vec<char> = s.code.chars().collect();
                let fns = fns(&chars);
                (chars, fns)
            })
            .collect();
        let mut this = Self { files, keys: BTreeSet::new(), methods: BTreeSet::new() };
        loop {
            let mut grown = Vec::new();
            for (chars, fns) in &this.files {
                for f in fns {
                    if !f.window || f.name.starts_with("render") || COMMON.contains(&f.name.as_str()) || deferred(&f.name) || this.keys.contains(&f.key()) {
                        continue;
                    }
                    let now = immediate(chars, f.body);
                    let focuses = !direct_focus(chars, f.body, &now).is_empty()
                        || calls_with_window(chars, f.body, &now, f.owner.as_deref()).iter().any(|(_, c)| this.resolves(c));
                    if focuses {
                        grown.push((f.key(), f.takes_self.then(|| f.name.clone())));
                    }
                }
            }
            if grown.is_empty() {
                return this;
            }
            for (key, method) in grown {
                this.keys.insert(key);
                this.methods.extend(method);
            }
        }
    }

    fn resolves(&self, call: &Call) -> bool {
        match call {
            Call::Path(path, name) => self.keys.contains(&format!("{path}::{name}")) || self.keys.contains(name),
            Call::Method(name) => self.methods.contains(name),
            Call::Free(name) => self.keys.contains(name),
        }
    }
}

/// The lines of focus changes while a `render*` fn runs: direct ones, and calls passing `window` into a fn that changes it.
fn render_focus(code: &str, focusing: &Focusing) -> Vec<usize> {
    let chars: Vec<char> = code.chars().collect();
    let mut found = BTreeSet::new();
    for f in fns(&chars).into_iter().filter(|f| f.name.starts_with("render")) {
        let now = immediate(&chars, f.body);
        found.extend(direct_focus(&chars, f.body, &now));
        found.extend(calls_with_window(&chars, f.body, &now, f.owner.as_deref()).into_iter().filter(|(_, c)| focusing.resolves(c)).map(|(at, _)| at));
    }
    found.into_iter().map(|at| line_of(&chars, at)).collect()
}

#[cfg(test)]
mod tests;
