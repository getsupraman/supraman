//! Closed sessions from `<claude dir>/projects/<slug>/<sessionId>.jsonl`, read only: `claude agents --json
//! --all` lists an interactive session only while it runs. The slug is lossy, so it only narrows the project
//! dirs ([`slug`] prefix); a session's cwd comes from its lines.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use memchr::memmem;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use supraman_core::time::epoch_ms;

use crate::session::{Kind, Session, SessionState};

/// What a transcript says about its session.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transcript {
    pub session_id: String,
    /// The first `cwd` of its lines: where it ran, and where `claude --resume` finds it again.
    pub cwd: Option<PathBuf>,
    pub custom_title: Option<String>,
    pub agent_name: Option<String>,
    pub ai_title: Option<String>,
    pub summary: Option<String>,
    pub first_prompt: Option<String>,
    pub last_prompt: Option<String>,
    /// The file's mtime, epoch milliseconds: its last activity.
    pub modified_ms: i64,
    /// The session claude moved it on to (its `continued-in` record), unless it ran on after that.
    #[serde(default)]
    pub continued_in: Option<String>,
}

/// A name longer than this is cut, with `…`.
const TITLE_CHARS: usize = 80;
/// The line types read for a name.
const META: [&str; 6] = ["custom-title", "agent-name", "ai-title", "summary", "last-prompt", "continued-in"];
/// A meta line not opening with its type is looked for only in lines up to this long.
const SHORT_LINE: usize = 16 * 1024;

impl Transcript {
    /// As claude names it: its custom title, its agent name, its summary, its first prompt, else its id.
    pub fn title(&self) -> String {
        [&self.custom_title, &self.agent_name, &self.ai_title, &self.summary, &self.first_prompt]
            .into_iter()
            .flatten()
            .map(|t| one_line(t))
            .find(|t| !t.is_empty())
            .unwrap_or_else(|| self.session_id.get(..8).unwrap_or(&self.session_id).to_string())
    }

    /// Worth a row: it ran somewhere, and was given a prompt or a name.
    pub fn listable(&self) -> bool {
        self.cwd.is_some()
            && (self.first_prompt.is_some() || self.custom_title.is_some() || self.agent_name.is_some() || self.ai_title.is_some())
    }

    /// As a stopped interactive row of `claude agents`, named by [`Self::title`], started at its last activity.
    pub fn to_session(&self) -> Session {
        Session {
            id: self.session_id.get(..8).unwrap_or(&self.session_id).to_string(),
            session_id: self.session_id.clone(),
            name: self.title(),
            cwd: self.cwd.clone().unwrap_or_default(),
            kind: Kind::Interactive,
            state: Some(SessionState::Stopped),
            status: None,
            waiting_for: None,
            pid: None,
            started_at: Some(self.modified_ms),
        }
    }

    /// One line of the file, without its newline. Anything unreadable is skipped.
    pub fn feed(&mut self, line: &[u8]) {
        let line = trim(line);
        if line.is_empty() {
            return;
        }
        // claude writes the meta lines type first; a short line naming one is read too, whatever its key order.
        let lead = leading_type(line);
        let meta = match lead {
            Some(kind) => META.contains(&kind),
            None => line.len() <= SHORT_LINE && types_in(line).any(|t| META.contains(&t)),
        };
        if meta {
            let Ok(v) = serde_json::from_slice::<Value>(line) else {
                return;
            };
            let (key, slot) = match v.get("type").and_then(Value::as_str) {
                Some("custom-title") => ("customTitle", &mut self.custom_title),
                Some("agent-name") => ("agentName", &mut self.agent_name),
                Some("ai-title") => ("aiTitle", &mut self.ai_title),
                Some("summary") => ("summary", &mut self.summary),
                Some("last-prompt") => ("lastPrompt", &mut self.last_prompt),
                Some("continued-in") => ("continuedInSessionId", &mut self.continued_in),
                _ => return,
            };
            if let Some(text) = v.get(key).and_then(Value::as_str).filter(|t| !t.trim().is_empty()) {
                *slot = Some(text.to_string());
            }
            return;
        }
        // the other type-first lines (mode, api-request, cost-state, …) carry neither a cwd nor a prompt
        if lead.is_some_and(|k| !matches!(k, "user" | "assistant" | "system")) {
            return;
        }
        if self.continued_in.is_some() && lead.map_or_else(|| line_type(line), |k| Some(k.to_string())).is_some_and(|k| k == "user" || k == "assistant") {
            self.continued_in = None;
        }
        // A user/assistant line can carry a whole tool result: parse it only while something is missing.
        let wants_prompt = self.first_prompt.is_none() && lead.map_or_else(|| line_type(line).as_deref() == Some("user"), |k| k == "user");
        let wants_cwd = self.cwd.is_none() && contains(line, br#""cwd""#);
        if !wants_prompt && !wants_cwd {
            return;
        }
        let Ok(v) = serde_json::from_slice::<Value>(line) else {
            return;
        };
        if wants_cwd && let Some(cwd) = v.get("cwd").and_then(Value::as_str).filter(|c| !c.is_empty()) {
            self.cwd = Some(PathBuf::from(cwd));
        }
        if wants_prompt && let Some(prompt) = prompt_of(&v) {
            self.first_prompt = Some(prompt);
        }
    }
}

/// The text a person typed, from a user line: not a tool result, a meta line, a compaction summary or
/// a `<command…>` wrapper.
fn prompt_of(v: &Value) -> Option<String> {
    let flag = |k: &str| v.get(k).and_then(Value::as_bool).unwrap_or(false);
    if flag("isMeta") || flag("isCompactSummary") || flag("isSidechain") || v.get("toolUseResult").is_some() {
        return None;
    }
    let content = v.get("message")?.get("content")?;
    let text = match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .find(|b| b.get("type").and_then(Value::as_str) == Some("text"))
            .and_then(|b| b.get("text")?.as_str())?
            .to_string(),
        _ => return None,
    };
    let text = text.trim();
    (!text.is_empty() && !text.starts_with('<')).then(|| text.to_string())
}

fn one_line(text: &str) -> String {
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if joined.chars().count() > TITLE_CHARS {
        format!("{}…", joined.chars().take(TITLE_CHARS - 1).collect::<String>())
    } else {
        joined
    }
}

fn trim(line: &[u8]) -> &[u8] {
    let mut l = line;
    while let [rest @ .., b'\n' | b'\r'] = l {
        l = rest;
    }
    l
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    memmem::find(hay, needle).is_some()
}

/// Every `x` of a `"type":"x"` in the line.
fn types_in(line: &[u8]) -> impl Iterator<Item = &str> {
    const KEY: &[u8] = br#""type":""#;
    memmem::find_iter(line, KEY).filter_map(move |at| {
        let rest = &line[at + KEY.len()..];
        let end = memchr::memchr(b'"', rest)?;
        std::str::from_utf8(&rest[..end]).ok()
    })
}

/// The line's top-level `type`, whatever its key order and spacing.
fn line_type(line: &[u8]) -> Option<String> {
    #[derive(Deserialize)]
    struct Kind {
        #[serde(rename = "type")]
        kind: Option<String>,
    }
    serde_json::from_slice::<Kind>(line).ok()?.kind
}

/// `meta` of a line opening with `{"type":"meta"`.
fn leading_type(line: &[u8]) -> Option<&str> {
    let rest = line.strip_prefix(br#"{"type":""#)?;
    let end = rest.iter().position(|&b| b == b'"')?;
    std::str::from_utf8(&rest[..end]).ok()
}

pub fn read(path: &Path) -> std::io::Result<Transcript> {
    let mut t = Transcript {
        session_id: session_id_of(path).unwrap_or_default(),
        modified_ms: std::fs::metadata(path).and_then(|m| m.modified()).map(epoch_ms).unwrap_or(0),
        ..Transcript::default()
    };
    feed_from(path, 0, &mut t)?;
    Ok(t)
}

/// Feeds every complete line from byte `offset` on; returns the offset after the last one fed (a line
/// still being written is left for the next read).
fn feed_from(path: &Path, offset: u64, t: &mut Transcript) -> std::io::Result<u64> {
    feed_lines(path, offset, false, t, |_| false)
}

/// [`feed_from`] until `enough`; with `mid_line`, `offset` may fall inside a line, which is skipped.
fn feed_lines(path: &Path, offset: u64, mid_line: bool, t: &mut Transcript, enough: impl Fn(&Transcript) -> bool) -> std::io::Result<u64> {
    let mut file = File::open(path)?;
    let mut at = offset;
    let mut line = Vec::new();
    let mut reader = if mid_line && offset > 0 {
        file.seek(SeekFrom::Start(offset - 1))?;
        let mut reader = BufReader::with_capacity(64 * 1024, file);
        let n = reader.read_until(b'\n', &mut line)?;
        if line.last() != Some(&b'\n') {
            return Ok(offset);
        }
        at = offset - 1 + n as u64;
        reader
    } else {
        file.seek(SeekFrom::Start(offset))?;
        BufReader::with_capacity(64 * 1024, file)
    };
    while !enough(t) {
        line.clear();
        let n = reader.read_until(b'\n', &mut line)?;
        if n == 0 || line.last() != Some(&b'\n') {
            break;
        }
        at += n as u64;
        t.feed(&line);
    }
    #[cfg(test)]
    tests::READ_BYTES.fetch_add(at - offset, Ordering::Relaxed);
    Ok(at)
}

/// A file this small is read whole.
const WHOLE_UNDER: u64 = 256 * 1024;
/// The first tail read; each next one is four times longer.
const TAIL_BLOCK: u64 = 64 * 1024;

/// What a whole read of the file gives for [`Transcript::title`], `cwd`, `first_prompt` and
/// [`Transcript::listable`], reading only its head (to the cwd and first prompt) and its tail: claude appends
/// the meta lines again near the end, so the tail holds the latest `custom-title`, the name that outranks
/// the rest. A tail without one grows until it meets the head. Returns the offset after the last line fed.
fn read_head_and_tail(path: &Path, len: u64, t: &mut Transcript) -> std::io::Result<u64> {
    if len <= WHOLE_UNDER {
        return feed_from(path, 0, t);
    }
    let head = feed_lines(path, 0, false, t, |t| t.cwd.is_some() && t.first_prompt.is_some())?;
    let mut block = TAIL_BLOCK;
    loop {
        let start = len.saturating_sub(block).max(head);
        let mut tail = Transcript { cwd: t.cwd.clone(), first_prompt: t.first_prompt.clone(), ..Transcript::default() };
        let end = feed_lines(path, start, start > head, &mut tail, |_| false)?;
        if tail.custom_title.is_some() || start == head {
            for (later, slot) in [
                (tail.custom_title, &mut t.custom_title),
                (tail.agent_name, &mut t.agent_name),
                (tail.ai_title, &mut t.ai_title),
                (tail.summary, &mut t.summary),
                (tail.last_prompt, &mut t.last_prompt),
            ] {
                if later.is_some() {
                    *slot = later;
                }
            }
            t.continued_in = tail.continued_in;
            return Ok(end.max(head));
        }
        block *= 4;
    }
}

/// The session `session_id` continues in, from its transcript's tail; `None` when it has no transcript or ran on.
pub fn continued_in(claude_dir: &Path, session_id: &str) -> Option<String> {
    let path = crate::find_transcript(claude_dir, session_id, None)?;
    let start = std::fs::metadata(&path).ok()?.len().saturating_sub(TAIL_BLOCK);
    let mut t = Transcript::default();
    feed_lines(&path, start, start > 0, &mut t, |_| false).ok()?;
    t.continued_in
}

/// The newest session of `session_id`'s `continued-in` chain, one step at a time by `next`; itself when it continued in none.
pub fn newest(session_id: &str, mut next: impl FnMut(&str) -> Option<String>) -> String {
    let mut seen = HashSet::new();
    let mut at = session_id.to_string();
    while seen.insert(at.clone()) {
        match next(&at) {
            Some(later) => at = later,
            None => break,
        }
    }
    at
}

/// [`newest`] read from the transcripts under `claude_dir`.
pub fn newest_in(claude_dir: &Path, session_id: &str) -> String {
    newest(session_id, |sid| continued_in(claude_dir, sid))
}

/// Each continued session among `transcripts`, to the newest session of its chain.
pub fn chains(transcripts: &[Transcript]) -> HashMap<String, String> {
    let next: HashMap<&str, &str> = transcripts.iter().filter_map(|t| Some((t.session_id.as_str(), t.continued_in.as_deref()?))).collect();
    next.keys().map(|sid| (sid.to_string(), newest(sid, |at| next.get(at).map(|n| n.to_string())))).collect()
}

/// The first prompt of a transcript, read only as far as it (at most [`FIRST_PROMPT_WITHIN`] bytes).
pub fn first_prompt(path: &Path) -> std::io::Result<Option<String>> {
    let file = File::open(path)?;
    let mut reader = BufReader::with_capacity(64 * 1024, file);
    let mut t = Transcript::default();
    let (mut line, mut read) = (Vec::new(), 0u64);
    while t.first_prompt.is_none() && read < FIRST_PROMPT_WITHIN {
        line.clear();
        let n = reader.read_until(b'\n', &mut line)?;
        if n == 0 {
            break;
        }
        read += n as u64;
        t.feed(&line);
    }
    Ok(t.first_prompt)
}

const FIRST_PROMPT_WITHIN: u64 = 8 * 1024 * 1024;

/// A session's tokens from its transcript's assistant `usage` lines, read on from where the last
/// [`Usage::read`] stopped. claude repeats a message's usage on each of its content-block lines, so each
/// message id counts once, with its last line's numbers.
#[derive(Debug, Clone, Default)]
pub struct Usage {
    offset: u64,
    by_message: HashMap<String, Tokens>,
    unnamed: Tokens,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub cache_creation: u64,
    pub cache_read: u64,
}

impl Tokens {
    fn add(&mut self, o: &Tokens) {
        self.input += o.input;
        self.output += o.output;
        self.cache_creation += o.cache_creation;
        self.cache_read += o.cache_read;
    }
}

impl Usage {
    /// Reads the complete lines appended to `path` since the last call.
    pub fn read(&mut self, path: &Path) -> std::io::Result<()> {
        let mut file = File::open(path)?;
        if file.metadata()?.len() < self.offset {
            *self = Self::default();
        }
        file.seek(SeekFrom::Start(self.offset))?;
        let mut reader = BufReader::with_capacity(64 * 1024, file);
        let mut line = Vec::new();
        loop {
            line.clear();
            let n = reader.read_until(b'\n', &mut line)?;
            if n == 0 || line.last() != Some(&b'\n') {
                return Ok(());
            }
            self.offset += n as u64;
            self.feed(trim(&line));
        }
    }

    fn feed(&mut self, line: &[u8]) {
        if !contains(line, br#""usage""#) {
            return;
        }
        let Ok(v) = serde_json::from_slice::<Value>(line) else {
            return;
        };
        if v.get("type").and_then(Value::as_str) != Some("assistant") {
            return;
        }
        let Some(message) = v.get("message") else { return };
        let Some(u) = message.get("usage") else { return };
        let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
        let tokens = Tokens {
            input: n("input_tokens"),
            output: n("output_tokens"),
            cache_creation: n("cache_creation_input_tokens"),
            cache_read: n("cache_read_input_tokens"),
        };
        match message.get("id").and_then(Value::as_str) {
            Some(id) => {
                self.by_message.insert(id.to_string(), tokens);
            }
            None => self.unnamed.add(&tokens),
        }
    }

    pub fn total(&self) -> Tokens {
        let mut sum = self.unnamed;
        for t in self.by_message.values() {
            sum.add(t);
        }
        sum
    }
}

/// How rlarch's nested-claude opens every brief (`header.mjs` `protocolHeader`).
pub const CONTROLLER_HEADER: &str = "Your controller is the Claude Code session '";

/// Who started a session, read from its first prompt: the controller header, or a pointer to a
/// brief under `.claude/briefs/` (whose own text holds the header).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launcher {
    /// As the header names it, `name [ref]`.
    pub controller: Option<String>,
    pub brief: Option<PathBuf>,
}

impl Launcher {
    /// `None` when the prompt shows neither.
    pub fn of(prompt: &str) -> Option<Self> {
        let controller = controller_in(prompt);
        let brief = prompt.split_whitespace().find_map(|word| {
            let word = word.trim_matches(|c: char| matches!(c, '`' | '"' | '\'' | '“' | '”' | '(' | ')' | ',' | ';' | ':' | '.') );
            let lower = word.to_ascii_lowercase().replace('\\', "/");
            (lower.contains(".claude/briefs/") && lower.ends_with(".txt")).then(|| PathBuf::from(word))
        });
        (controller.is_some() || brief.is_some()).then_some(Self { controller, brief })
    }

    /// The controller from the brief's header when the prompt only pointed at the brief.
    pub fn resolve(mut self) -> Self {
        if self.controller.is_none()
            && let Some(brief) = &self.brief
            && let Ok(file) = File::open(brief)
        {
            let mut head = Vec::new();
            let _ = std::io::Read::read_to_end(&mut std::io::Read::take(file, 4096), &mut head);
            self.controller = controller_in(&String::from_utf8_lossy(&head));
        }
        self
    }

    /// The controller's session name, without its ` [ref]`.
    pub fn controller_name(&self) -> Option<&str> {
        let c = self.controller.as_deref()?;
        Some(match c.rsplit_once(" [") {
            Some((name, r)) if r.ends_with(']') => name,
            _ => c,
        })
    }
}

fn controller_in(text: &str) -> Option<String> {
    let rest = text.split_once(CONTROLLER_HEADER)?.1;
    let name = rest.split_once('\'')?.0.trim();
    (!name.is_empty() && !name.contains('\n')).then(|| name.to_string())
}

fn session_id_of(path: &Path) -> Option<String> {
    (path.extension()? == "jsonl").then_some(())?;
    Some(path.file_stem()?.to_string_lossy().into_owned())
}

/// claude's project dir name for `path`: every character but an ASCII letter or digit becomes `-`.
pub fn slug(path: &Path) -> String {
    path.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

#[derive(Clone, Serialize, Deserialize)]
struct Cached {
    len: u64,
    #[serde(with = "nanos")]
    modified: SystemTime,
    /// Where the next read starts.
    offset: u64,
    transcript: Transcript,
}

mod nanos {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    pub fn serialize<S: serde::Serializer>(t: &SystemTime, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64))
    }

    pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<SystemTime, D::Error> {
        Ok(UNIX_EPOCH + Duration::from_nanos(serde::Deserialize::deserialize(d)?))
    }
}

/// The on-disk form of [`Transcripts`]' file cache.
#[derive(Serialize, Deserialize)]
struct CacheFile {
    version: u32,
    projects: PathBuf,
    files: Vec<(PathBuf, Cached)>,
}

/// Bump whenever [`Transcript::feed`] reads something a cached transcript lacks.
const CACHE_VERSION: u32 = 2;

struct Listed {
    modified: Option<SystemTime>,
    at: SystemTime,
    /// `(path, len, mtime)` of each `*.jsonl`.
    files: Vec<(PathBuf, u64, SystemTime)>,
}

type Job = (PathBuf, u64, SystemTime, Option<Cached>);

/// The transcripts under one `projects` dir, cached: the dir list by its mtime, each project dir's
/// listing by its mtime (re-read anyway every [`Self::RELIST_EVERY`], or while it holds a recent, kept or
/// live session), each file by its length (an appended file is read from where the last read stopped).
/// With [`Self::with_cache`] the file cache outlives the process.
pub struct Transcripts {
    projects: PathBuf,
    dirs: Option<(Option<SystemTime>, SystemTime, Vec<String>)>,
    listed: HashMap<PathBuf, Listed>,
    files: HashMap<PathBuf, Cached>,
    cache: Option<PathBuf>,
    unsaved: bool,
    saved_at: Option<Instant>,
}

impl Transcripts {
    pub const RELIST_EVERY: Duration = Duration::from_secs(300);
    /// A cache that only grew is saved at most this often; one holding a file read from its start, at once.
    pub const SAVE_EVERY: Duration = Duration::from_secs(300);

    pub fn new(projects: PathBuf) -> Self {
        Self {
            projects,
            dirs: None,
            listed: HashMap::new(),
            files: HashMap::new(),
            cache: None,
            unsaved: false,
            saved_at: None,
        }
    }

    /// Under `<claude dir>/projects`.
    pub fn in_claude_dir(claude_dir: &Path) -> Self {
        Self::new(claude_dir.join("projects"))
    }

    /// Starts from the file cache saved at `file` (an unreadable or older one is ignored) and saves it there.
    pub fn with_cache(mut self, file: PathBuf) -> Self {
        let loaded = std::fs::read(&file).ok().and_then(|bytes| serde_json::from_slice::<CacheFile>(&bytes).ok());
        if let Some(c) = loaded.filter(|c| c.version == CACHE_VERSION && c.projects == self.projects) {
            self.files = c.files.into_iter().collect();
        }
        self.cache = Some(file);
        self
    }

    /// A cache was loaded by [`Self::with_cache`] and holds files.
    pub fn has_cache(&self) -> bool {
        !self.files.is_empty()
    }

    /// What [`Self::scan`] gave when the cache was saved, reading no transcript: the cached ones it would
    /// list now, whether or not their files changed since.
    pub fn cached(&self, roots: &[PathBuf], since_ms: i64, keep: &HashSet<String>) -> Vec<Transcript> {
        let prefixes = prefixes(roots);
        self.files
            .iter()
            .filter(|(path, _)| path.parent().and_then(Path::file_name).is_some_and(|d| under(&d.to_string_lossy(), &prefixes)))
            .map(|(_, c)| &c.transcript)
            .filter(|t| (t.modified_ms >= since_ms || keep.contains(&t.session_id)) && t.listable())
            .cloned()
            .collect()
    }

    pub fn scan(&mut self, roots: &[PathBuf], since_ms: i64, keep: &HashSet<String>, live: &HashSet<String>) -> Vec<Transcript> {
        self.scan_with(roots, since_ms, keep, live, &|_, _| {})
    }

    /// The [`Transcript::listable`] transcripts of the project dirs whose name starts with the [`slug`]
    /// of one of `roots` (case-insensitive), modified since `since_ms` or with an id in `keep`. `live`
    /// (ids `claude agents` lists running) keeps their dirs re-listed: an old file resumed grows
    /// without touching its dir's mtime. The files not cached are read in parallel; `progress(read, of)`
    /// is called as each is done.
    pub fn scan_with(
        &mut self,
        roots: &[PathBuf],
        since_ms: i64,
        keep: &HashSet<String>,
        live: &HashSet<String>,
        progress: &(dyn Fn(usize, usize) + Sync),
    ) -> Vec<Transcript> {
        let now = SystemTime::now();
        let prefixes = prefixes(roots);
        let names = self.dir_names(now);
        self.listed.retain(|dir, _| dir.file_name().is_some_and(|d| names.iter().any(|n| d == n.as_str())));
        let mut wanted = Vec::new();
        for name in names.iter().filter(|n| under(n, &prefixes)) {
            let dir = self.projects.join(name);
            for (path, len, modified) in self.list(&dir, now, since_ms, keep, live) {
                let Some(id) = session_id_of(&path) else { continue };
                if epoch_ms(modified) < since_ms && !keep.contains(&id) {
                    continue;
                }
                wanted.push((path, len, modified));
            }
        }
        let seen: HashSet<&PathBuf> = wanted.iter().map(|(p, _, _)| p).collect();
        let before = self.files.len();
        self.files.retain(|p, _| seen.contains(p));
        let mut unsaved = self.files.len() != before;
        let mut jobs: Vec<Job> = wanted
            .iter()
            .filter_map(|(path, len, modified)| match self.files.get(path) {
                Some(c) if c.len == *len && c.modified == *modified => None,
                Some(c) if *len > c.len => Some((path.clone(), *len, *modified, Some(c.clone()))),
                _ => Some((path.clone(), *len, *modified, None)),
            })
            .collect();
        let fresh = jobs.iter().any(|j| j.3.is_none());
        jobs.sort_by(|a, b| b.1.cmp(&a.1));
        for (path, read) in read_all(jobs, progress) {
            match read {
                Some(c) => self.files.insert(path, c),
                None => self.files.remove(&path),
            };
            unsaved = true;
        }
        self.unsaved |= unsaved;
        if self.unsaved && (fresh || self.saved_at.is_none_or(|at| at.elapsed() >= Self::SAVE_EVERY)) {
            self.save();
        }
        wanted
            .iter()
            .filter_map(|(path, _, _)| self.files.get(path))
            .map(|c| &c.transcript)
            .filter(|t| t.listable())
            .cloned()
            .collect()
    }

    fn save(&mut self) {
        let Some(file) = &self.cache else { return };
        let snapshot = CacheFile {
            version: CACHE_VERSION,
            projects: self.projects.clone(),
            files: self.files.iter().map(|(p, c)| (p.clone(), c.clone())).collect(),
        };
        let Ok(json) = serde_json::to_vec(&snapshot) else { return };
        if supraman_core::fs::write_atomic(file, json).is_ok() {
            self.unsaved = false;
            self.saved_at = Some(Instant::now());
        }
    }

    fn dir_names(&mut self, now: SystemTime) -> Vec<String> {
        let modified = std::fs::metadata(&self.projects).and_then(|m| m.modified()).ok();
        if let Some((m, at, names)) = &self.dirs
            && *m == modified
            && modified.is_some()
            && now.duration_since(*at).unwrap_or_default() < Self::RELIST_EVERY
        {
            return names.clone();
        }
        let names: Vec<String> = std::fs::read_dir(&self.projects)
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        self.dirs = Some((modified, now, names.clone()));
        names
    }

    fn list(&mut self, dir: &Path, now: SystemTime, since_ms: i64, keep: &HashSet<String>, live: &HashSet<String>) -> Vec<(PathBuf, u64, SystemTime)> {
        let modified = std::fs::metadata(dir).and_then(|m| m.modified()).ok();
        if let Some(l) = self.listed.get(dir) {
            let busy = l.files.iter().any(|(p, _, m)| {
                epoch_ms(*m) >= since_ms || session_id_of(p).is_some_and(|id| keep.contains(&id) || live.contains(&id))
            });
            if !busy && l.modified == modified && modified.is_some() && now.duration_since(l.at).unwrap_or_default() < Self::RELIST_EVERY {
                return l.files.clone();
            }
        }
        let files: Vec<(PathBuf, u64, SystemTime)> = std::fs::read_dir(dir)
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
                    .filter_map(|e| {
                        let m = e.metadata().ok()?;
                        m.is_file().then(|| (e.path(), m.len(), m.modified().unwrap_or(UNIX_EPOCH)))
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.listed.insert(dir.to_path_buf(), Listed { modified, at: now, files: files.clone() });
        files
    }
}

fn prefixes(roots: &[PathBuf]) -> Vec<String> {
    roots.iter().map(|r| slug(r).to_ascii_lowercase()).collect()
}

/// A project dir named for one of the roots or a folder below it.
fn under(dir: &str, prefixes: &[String]) -> bool {
    let lower = dir.to_ascii_lowercase();
    prefixes.iter().any(|p| lower.starts_with(p.as_str()))
}

/// One file: from where `cached` stopped when it grew, else from its start. `None` when it cannot be read.
fn read_one(path: &Path, len: u64, modified: SystemTime, cached: Option<Cached>) -> Option<Cached> {
    let mut c = match cached {
        Some(mut c) => {
            c.offset = feed_from(path, c.offset, &mut c.transcript).ok()?;
            c
        }
        None => {
            let mut transcript = Transcript { session_id: session_id_of(path)?, ..Transcript::default() };
            let offset = read_head_and_tail(path, len, &mut transcript).ok()?;
            Cached { len, modified, offset, transcript }
        }
    };
    c.len = len;
    c.modified = modified;
    c.transcript.modified_ms = epoch_ms(modified);
    Some(c)
}

/// [`read_one`] each job, on up to [`READERS`] threads.
fn read_all(jobs: Vec<Job>, progress: &(dyn Fn(usize, usize) + Sync)) -> Vec<(PathBuf, Option<Cached>)> {
    let total = jobs.len();
    let done = AtomicUsize::new(0);
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(READERS);
    supraman_core::pool::map(jobs, threads, |(path, len, modified, cached)| {
        let read = read_one(&path, len, modified, cached);
        progress(done.fetch_add(1, Ordering::Relaxed) + 1, total);
        (path, read)
    })
}

const READERS: usize = 8;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    pub(super) static READ_BYTES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    /// Sanitised from a real transcript (claude 2.1.306): its line shapes, none of its text.
    const FIXTURE: &str = include_str!("../tests/fixtures/transcript.jsonl");
    /// The end of a session claude 2.1.317 moved to the background: its line shapes, none of its text.
    const CONTINUED: &str = include_str!("../tests/fixtures/continued.jsonl");
    const FIRST: &str = "0a000000-0000-4000-8000-00000000000a";
    const SECOND: &str = "0b000000-0000-4000-8000-00000000000b";
    const THIRD: &str = "0c000000-0000-4000-8000-00000000000c";

    #[test]
    fn a_transcript_ending_in_continued_in_resolves_through_its_chain_to_the_newest_session() {
        let claude = tempfile::tempdir().unwrap();
        let dir = claude.path().join("projects").join(slug(Path::new(r"C:\demo\alpha")));
        std::fs::create_dir_all(&dir).unwrap();
        let first = write(&dir, &format!("{FIRST}.jsonl"), CONTINUED);
        let t = read(&first).unwrap();
        assert_eq!((t.continued_in.as_deref(), t.title().as_str()), (Some(SECOND), "demo"));
        assert_eq!(continued_in(claude.path(), FIRST).as_deref(), Some(SECOND));

        let second = CONTINUED.replace(FIRST, SECOND).replace(&format!(r#""continuedInSessionId":"{SECOND}""#), &format!(r#""continuedInSessionId":"{THIRD}""#));
        write(&dir, &format!("{SECOND}.jsonl"), &second);
        assert_eq!(newest_in(claude.path(), FIRST), THIRD, "through the chain");
        assert_eq!(newest_in(claude.path(), THIRD), THIRD, "a session without a transcript is its own newest");
        assert_eq!(newest(FIRST, |at| Some(if at == FIRST { SECOND } else { FIRST }.to_string())), FIRST, "a loop ends");

        let (a, b) = (read(&first).unwrap(), read(&dir.join(format!("{SECOND}.jsonl"))).unwrap());
        let map = chains(&[a, b, Transcript { session_id: THIRD.into(), ..Transcript::default() }]);
        assert_eq!(map, HashMap::from([(FIRST.to_string(), THIRD.to_string()), (SECOND.to_string(), THIRD.to_string())]));
    }

    #[test]
    fn a_session_resumed_after_its_continued_in_runs_on_and_a_grown_read_sees_the_record() {
        let tmp = tempfile::tempdir().unwrap();
        let ran_on = format!(
            "{CONTINUED}{}\n",
            r#"{"parentUuid":"t-1","isSidechain":false,"type":"user","message":{"role":"user","content":"again"},"uuid":"u-2","cwd":"C:\\demo\\alpha","sessionId":"0a000000-0000-4000-8000-00000000000a"}"#
        );
        assert_eq!(read(&write(tmp.path(), &format!("{FIRST}.jsonl"), &ran_on)).unwrap().continued_in, None);

        let (head, tail) = CONTINUED.split_at(CONTINUED.find(r#"{"type":"continued-in""#).unwrap());
        let path = write(tmp.path(), &format!("{FIRST}.jsonl"), head);
        let mut t = Transcript::default();
        let offset = feed_from(&path, 0, &mut t).unwrap();
        assert_eq!(t.continued_in, None);
        std::fs::write(&path, CONTINUED).unwrap();
        feed_from(&path, offset, &mut t).unwrap();
        assert_eq!((t.continued_in.as_deref(), tail.is_empty()), (Some(SECOND), false));
    }

    #[test]
    fn the_head_and_tail_read_of_a_long_transcript_finds_its_continued_in() {
        let tmp = tempfile::tempdir().unwrap();
        let (head, tail) = CONTINUED.split_at(CONTINUED.find(r#"{"type":"last-prompt""#).unwrap());
        let filler = r#"{"parentUuid":"a-1","isSidechain":false,"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"x"}]},"sessionId":"0a000000-0000-4000-8000-00000000000a"}"#;
        let body = format!("{head}{}{tail}", format!("{filler}\n").repeat((WHOLE_UNDER as usize / filler.len()) + 10));
        let path = write(tmp.path(), &format!("{FIRST}.jsonl"), &body);
        let len = std::fs::metadata(&path).unwrap().len();
        assert!(len > WHOLE_UNDER);
        let mut t = Transcript { session_id: FIRST.into(), ..Transcript::default() };
        read_head_and_tail(&path, len, &mut t).unwrap();
        assert_eq!(t.continued_in.as_deref(), Some(SECOND));
    }
    const ID: &str = "0f3e8b52-6c1d-4a7e-9b20-5d4c3a2b1e0f";

    fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn a_launcher_is_read_from_the_header_or_the_brief_the_prompt_points_at() {
        let header = format!("{CONTROLLER_HEADER}autowork-b0 [408d24]'. It started you, it is watching you, and it\nis who you report to");
        let l = Launcher::of(&header).unwrap();
        assert_eq!(l.controller.as_deref(), Some("autowork-b0 [408d24]"));
        assert_eq!(l.controller_name(), Some("autowork-b0"));
        assert_eq!(l.brief, None);

        let dir = tempfile::tempdir().unwrap();
        let briefs = dir.path().join(".claude").join("briefs");
        std::fs::create_dir_all(&briefs).unwrap();
        let brief = write(&briefs, "brief-aw-x.txt", &format!("{header}\n# Unit x\n"));
        let prompt = format!("Read the file {} and follow it exactly. It is your brief. Start now.", brief.display());
        let l = Launcher::of(&prompt).unwrap();
        assert_eq!(l.brief.as_deref(), Some(brief.as_path()), "the trailing dot is not part of the path");
        assert_eq!(l.controller, None);
        assert_eq!(l.clone().resolve().controller.as_deref(), Some("autowork-b0 [408d24]"));
        let gone = Launcher::of(r"Read the file `P:\x\.claude\briefs\brief-gone.txt`.").unwrap();
        assert_eq!(gone.resolve().controller, None, "a brief that is gone names no controller, it still marks the launch");

        assert_eq!(Launcher::of("fix the tree, then run the tests"), None);
        assert_eq!(Launcher::of("look at .claude/briefs/ for ideas"), None);
    }

    #[test]
    fn the_first_prompt_is_read_without_reading_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let mut body = String::from("{\"type\":\"mode\",\"mode\":\"normal\"}\n");
        body.push_str(&format!("{}\n", serde_json::json!({"type":"user","message":{"role":"user","content":"first one"},"cwd":"C:\\x"})));
        body.push_str(&format!("{}\n", serde_json::json!({"type":"user","message":{"role":"user","content":"second"},"cwd":"C:\\x"})));
        let path = write(dir.path(), &format!("{ID}.jsonl"), &body);
        assert_eq!(first_prompt(&path).unwrap().as_deref(), Some("first one"));
        let none = write(dir.path(), "empty.jsonl", "{\"type\":\"mode\"}\n");
        assert_eq!(first_prompt(&none).unwrap(), None);
    }

    #[test]
    fn a_spaced_user_line_gives_the_prompt_and_cwd_and_a_nested_user_type_does_not() {
        let mut t = Transcript::default();
        t.feed(br#"{"parentUuid": null, "message": {"role": "user", "content": [{"type": "user", "text": "hi"}]}, "type": "assistant"}"#);
        assert_eq!(t.first_prompt, None, "an assistant line with a nested user type");
        t.feed(br#"{"parentUuid": null, "type": "user", "message": {"role": "user", "content": "spaced prompt"}, "cwd": "C:\\x"}"#);
        assert_eq!(t.first_prompt.as_deref(), Some("spaced prompt"));
        assert_eq!(t.cwd.as_deref(), Some(Path::new(r"C:\x")));
    }

    #[test]
    fn reads_the_last_title_the_first_cwd_and_the_first_prompt() {
        let dir = tempfile::tempdir().unwrap();
        let t = read(&write(dir.path(), &format!("{ID}.jsonl"), FIXTURE)).unwrap();
        assert_eq!(t.session_id, ID);
        assert_eq!(t.cwd.as_deref(), Some(Path::new(r"C:\demo\Gaia")), "the first cwd, not the later worktree");
        assert_eq!(t.custom_title.as_deref(), Some("pipeline-two"), "the last custom-title wins");
        assert_eq!(t.agent_name.as_deref(), Some("pipeline-two"));
        assert_eq!(t.ai_title.as_deref(), Some("Fix the deploy step"));
        assert_eq!(t.first_prompt.as_deref(), Some("first prompt text"), "not the meta line, the command or the tool result");
        assert_eq!(t.last_prompt.as_deref(), Some("latest prompt text"));
        assert_eq!(t.title(), "pipeline-two");
        assert!(t.listable());
    }

    #[test]
    fn the_name_falls_back_custom_title_agent_name_summary_first_prompt_id() {
        let mut t = Transcript { session_id: ID.into(), ..Transcript::default() };
        assert_eq!(t.title(), "0f3e8b52", "nothing else: the id");
        t.first_prompt = Some("  fix\n the   build  ".into());
        assert_eq!(t.title(), "fix the build", "one line");
        t.summary = Some("Old summary".into());
        assert_eq!(t.title(), "Old summary");
        t.ai_title = Some("AI title".into());
        assert_eq!(t.title(), "AI title");
        t.agent_name = Some("agent".into());
        assert_eq!(t.title(), "agent");
        t.custom_title = Some("custom".into());
        assert_eq!(t.title(), "custom");
        t.custom_title = Some("x".repeat(200));
        assert_eq!(t.title().chars().count(), TITLE_CHARS);
        assert!(t.title().ends_with('…'));
    }

    #[test]
    fn a_malformed_line_is_skipped_and_an_empty_file_lists_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let body = format!(
            "{{\"type\":\"custom-title\",\"customTitle\":\"broken\n{{not json at all}}\n\n{}\n",
            r#"{"type":"user","message":{"role":"user","content":"after the bad lines"},"cwd":"C:\\demo\\Gaia","sessionId":"x"}"#
        );
        let t = read(&write(dir.path(), &format!("{ID}.jsonl"), &body)).unwrap();
        assert_eq!(t.custom_title, None, "the cut custom-title line");
        assert_eq!(t.first_prompt.as_deref(), Some("after the bad lines"));
        assert_eq!(t.cwd.as_deref(), Some(Path::new(r"C:\demo\Gaia")));

        let empty = read(&write(dir.path(), "1e2d3c4b-0000-0000-0000-000000000000.jsonl", "")).unwrap();
        assert_eq!(empty.cwd, None);
        assert!(!empty.listable());
        assert_eq!(empty.title(), "1e2d3c4b");
    }

    #[test]
    fn slugs_like_claude() {
        assert_eq!(slug(Path::new(r"P:\Projects\LarchSys\Gaia")), "P--Projects-LarchSys-Gaia");
        assert_eq!(slug(Path::new(r"P:\a.worktrees\m5-27")), "P--a-worktrees-m5-27");
    }

    #[test]
    fn scan_takes_only_the_repos_dirs_and_reads_a_grown_file_from_where_it_stopped() {
        let claude = tempfile::tempdir().unwrap();
        let projects = claude.path().join("projects");
        let gaia = projects.join("C--demo-Gaia");
        let worktree = projects.join("C--demo-Gaia--claude-worktrees-x");
        let other = projects.join("C--demo-Other");
        for d in [&gaia, &worktree, &other] {
            std::fs::create_dir_all(d).unwrap();
        }
        let prompt = |cwd: &str, text: &str| {
            format!("{}\n", serde_json::json!({"type":"user","message":{"role":"user","content":text},"cwd":cwd,"sessionId":"s"}))
        };
        let a = write(&gaia, "aaaaaaaa-0000-0000-0000-000000000001.jsonl", &prompt(r"C:\demo\Gaia", "in gaia"));
        write(&worktree, "bbbbbbbb-0000-0000-0000-000000000002.jsonl", &prompt(r"C:\demo\Gaia\.claude\worktrees\x", "in a worktree"));
        write(&other, "cccccccc-0000-0000-0000-000000000003.jsonl", &prompt(r"C:\demo\Other", "elsewhere"));
        write(&gaia, "dddddddd-0000-0000-0000-000000000004.jsonl", "");

        let mut scanner = Transcripts::in_claude_dir(claude.path());
        let none = HashSet::new();
        let mut found = scanner.scan(&[PathBuf::from(r"C:\demo\Gaia")], 0, &none, &none);
        found.sort_by(|x, y| x.session_id.cmp(&y.session_id));
        let ids: Vec<&str> = found.iter().map(|t| &t.session_id[..8]).collect();
        assert_eq!(ids, ["aaaaaaaa", "bbbbbbbb"], "not Other's dir, not the empty file");

        let mut f = std::fs::OpenOptions::new().append(true).open(&a).unwrap();
        write!(f, "{}\n{{\"type\":\"custom-title\",\"customTitle\":\"half", serde_json::json!({"type":"custom-title","customTitle":"renamed"})).unwrap();
        drop(f);
        let found = scanner.scan(&[PathBuf::from(r"C:\demo\Gaia")], 0, &none, &none);
        let a_now = found.iter().find(|t| t.session_id.starts_with("aaaaaaaa")).unwrap();
        assert_eq!(a_now.title(), "renamed", "the appended line, not the half-written one");
        assert_eq!(a_now.first_prompt.as_deref(), Some("in gaia"), "kept from the first read");

        let future = epoch_ms(SystemTime::now()) + 3_600_000;
        assert!(scanner.scan(&[PathBuf::from(r"C:\demo\Gaia")], future, &none, &none).is_empty(), "all older than the window");
        let keep = HashSet::from(["aaaaaaaa-0000-0000-0000-000000000001".to_string()]);
        let kept = scanner.scan(&[PathBuf::from(r"C:\demo\Gaia")], future, &keep, &none);
        assert_eq!(kept.len(), 1, "a kept id past the window");
        assert_eq!(kept[0].to_session().name, "renamed");
        assert_eq!(kept[0].to_session().kind, Kind::Interactive);

        assert_eq!(scanner.listed.len(), 2, "gaia and its worktree");
        std::fs::remove_dir_all(&worktree).unwrap();
        scanner.scan(&[PathBuf::from(r"C:\demo\Gaia")], 0, &none, &none);
        assert_eq!(scanner.listed.keys().collect::<Vec<_>>(), [&gaia], "a removed project dir's listing is dropped");
    }

    /// Transcripts shaped like claude's (type-first meta lines among message lines, long tool results), sized
    /// by `size(i)`. Their names vary: no custom title, the latest one near the end, or one only early; some
    /// open with a command before the first prompt, some end in a half-written line.
    fn transcripts(projects: &Path, count: usize, size: impl Fn(usize) -> usize) -> Vec<PathBuf> {
        let dir = projects.join("C--bench-Repo");
        std::fs::create_dir_all(&dir).unwrap();
        let filler = "x".repeat(2000);
        let tool = "y".repeat(60_000);
        let meta = |kind: &str, key: &str, value: String| format!("{}\n", serde_json::json!({"type":kind,key:value,"sessionId":"s"}));
        let user = |content: String| {
            format!("{}\n", serde_json::json!({"parentUuid":"u","isSidechain":false,"type":"user","message":{"role":"user","content":content},"cwd":r"C:\bench\Repo","uuid":"u"}))
        };
        (0..count)
            .map(|i| {
                let target = size(i);
                let mut body = String::with_capacity(target + 100_000);
                body.push_str(&meta("mode", "mode", "normal".into()));
                if i % 4 == 0 {
                    body.push_str(&user("<command-name>/model</command-name>".into()));
                }
                body.push_str(&user(format!("prompt {i}")));
                let mut n = 0;
                while body.len() < target {
                    n += 1;
                    let line = if n % 7 == 0 {
                        serde_json::json!({"parentUuid":"u","isSidechain":false,"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t","content":tool}]},"toolUseResult":{"stdout":"z"},"cwd":r"C:\bench\Repo","uuid":"u"})
                    } else {
                        serde_json::json!({"parentUuid":"u","isSidechain":false,"message":{"role":"assistant","content":[{"type":"text","text":filler}]},"type":"assistant","cwd":r"C:\bench\Repo","uuid":"a"})
                    };
                    body.push_str(&format!("{line}\n"));
                    if n % 50 == 0 {
                        body.push_str(&meta("ai-title", "aiTitle", format!("title {i}.{n}")));
                        body.push_str(&meta("last-prompt", "lastPrompt", format!("asked {i}.{n}")));
                        if i % 3 == 1 || (i % 3 == 2 && n == 50) {
                            body.push_str(&meta("custom-title", "customTitle", format!("named {i}.{n}")));
                            body.push_str(&meta("agent-name", "agentName", format!("named {i}.{n}")));
                        }
                    }
                }
                if i % 3 == 1 {
                    body.push_str(&meta("custom-title", "customTitle", format!("renamed {i}")));
                }
                if i % 5 == 4 {
                    body.push_str(r#"{"type":"custom-title","customTitle":"half"#);
                }
                write(&dir, &format!("{i:08x}-0000-0000-0000-000000000000.jsonl"), &body)
            })
            .collect()
    }

    fn named(t: &Transcript) -> (String, Option<PathBuf>, Option<String>, bool) {
        (t.title(), t.cwd.clone(), t.first_prompt.clone(), t.listable())
    }

    #[test]
    fn the_head_and_tail_read_names_each_session_as_the_whole_read_does() {
        let claude = tempfile::tempdir().unwrap();
        let files = transcripts(&claude.path().join("projects"), 24, |i| [100_000, 400_000, 1_500_000][(i / 3) % 3]);
        let found = Transcripts::in_claude_dir(claude.path()).scan(&[PathBuf::from(r"C:\bench\Repo")], 0, &HashSet::new(), &HashSet::new());
        assert_eq!(found.len(), files.len());
        for path in &files {
            let whole = read(path).unwrap();
            let t = found.iter().find(|t| t.session_id == whole.session_id).unwrap();
            assert_eq!(named(t), named(&whole), "{}", path.display());
        }
    }

    #[test]
    fn the_file_cache_outlives_the_scanner_and_a_grown_file_is_read_from_where_it_stopped() {
        let claude = tempfile::tempdir().unwrap();
        let dir = claude.path().join("projects").join("C--demo-Gaia");
        std::fs::create_dir_all(&dir).unwrap();
        let line = |text: &str| format!("{}\n", serde_json::json!({"type":"user","message":{"role":"user","content":text},"cwd":r"C:\demo\Gaia"}));
        let a = write(&dir, "aaaaaaaa-0000-0000-0000-000000000001.jsonl", &line("first"));
        write(&dir, "bbbbbbbb-0000-0000-0000-000000000002.jsonl", &line("other"));
        let cache = claude.path().join("cache").join("transcripts.json");
        let roots = [PathBuf::from(r"C:\demo\Gaia")];
        let none = HashSet::new();
        let reads = std::sync::atomic::AtomicUsize::new(0);
        let count = |_: usize, _: usize| {
            reads.fetch_add(1, Ordering::Relaxed);
        };
        assert_eq!(Transcripts::in_claude_dir(claude.path()).with_cache(cache.clone()).scan_with(&roots, 0, &none, &none, &count).len(), 2);
        assert_eq!(reads.swap(0, Ordering::Relaxed), 2);
        assert!(cache.is_file(), "saved after reading files from their start");

        let mut f = std::fs::OpenOptions::new().append(true).open(&a).unwrap();
        writeln!(f, "{}", serde_json::json!({"type":"custom-title","customTitle":"renamed"})).unwrap();
        drop(f);
        let mut again = Transcripts::in_claude_dir(claude.path()).with_cache(cache.clone());
        let found = again.scan_with(&roots, 0, &none, &none, &count);
        assert_eq!(reads.swap(0, Ordering::Relaxed), 1, "only the grown file is read");
        let a_now = found.iter().find(|t| t.session_id.starts_with("aaaaaaaa")).unwrap();
        assert_eq!((a_now.title(), a_now.first_prompt.as_deref()), ("renamed".to_string(), Some("first")));

        let shown = Transcripts::in_claude_dir(claude.path()).with_cache(cache.clone());
        assert!(shown.has_cache());
        let mut cached: Vec<String> = shown.cached(&roots, 0, &none).iter().map(Transcript::title).collect();
        cached.sort();
        assert_eq!(cached, ["other", "renamed"], "as the last scan saved them");
        assert!(shown.cached(&[PathBuf::from(r"C:\demo\Other")], 0, &none).is_empty(), "another root's");

        std::fs::write(&cache, "not json").unwrap();
        assert_eq!(Transcripts::in_claude_dir(claude.path()).with_cache(cache).scan_with(&roots, 0, &none, &none, &count).len(), 2);
        assert_eq!(reads.load(Ordering::Relaxed), 2, "an unreadable cache is read again from the files");
    }

    #[test]
    fn usage_counts_each_message_once_and_reads_on_from_where_it_stopped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");
        let line = |id: &str, out: u64| {
            format!(
                r#"{{"type":"assistant","message":{{"id":"{id}","usage":{{"input_tokens":10,"output_tokens":{out},"cache_creation_input_tokens":100,"cache_read_input_tokens":1000}}}}}}"#
            )
        };
        let user = r#"{"type":"user","message":{"content":"say \"usage\""}}"#;
        std::fs::write(&path, format!("{}\n{}\n{user}\n", line("m1", 1), line("m1", 5))).unwrap();
        let mut u = Usage::default();
        u.read(&path).unwrap();
        assert_eq!(u.total(), Tokens { input: 10, output: 5, cache_creation: 100, cache_read: 1000 }, "m1 once, its last line");

        let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        std::io::Write::write_all(&mut f, format!("{}\n{}", line("m2", 7), line("m3", 9)).as_bytes()).unwrap();
        u.read(&path).unwrap();
        assert_eq!(u.total().output, 12, "m2 added; m3 is still being written");
        std::io::Write::write_all(&mut f, b"\n").unwrap();
        u.read(&path).unwrap();
        assert_eq!(u.total(), Tokens { input: 30, output: 21, cache_creation: 300, cache_read: 3000 });
    }
}
