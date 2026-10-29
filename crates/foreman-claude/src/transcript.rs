//! Closed sessions from `<claude dir>/projects/<slug>/<sessionId>.jsonl`, read only: `claude agents --json
//! --all` lists an interactive session only while it runs. The slug is lossy, so it only narrows the project
//! dirs ([`slug`] prefix); a session's cwd comes from its lines.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;

use foreman_core::time::epoch_ms;

use crate::session::{Kind, Session, SessionState};

/// What a transcript says about its session.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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
}

/// A name longer than this is cut, with `…`.
const TITLE_CHARS: usize = 80;
/// The line types read for a name.
const META: [&str; 5] = ["custom-title", "agent-name", "ai-title", "summary", "last-prompt"];
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
            None => line.len() <= SHORT_LINE && META.iter().any(|m| contains(line, format!(r#""type":"{m}""#).as_bytes())),
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
        // A user/assistant line can carry a whole tool result: parse it only while something is missing.
        let wants_prompt = self.first_prompt.is_none() && contains(line, br#""type":"user""#);
        let wants_cwd = self.cwd.is_none() && contains(line, br#""cwd":"#);
        if !wants_prompt && !wants_cwd {
            return;
        }
        let Ok(v) = serde_json::from_slice::<Value>(line) else {
            return;
        };
        if wants_cwd && let Some(cwd) = v.get("cwd").and_then(Value::as_str).filter(|c| !c.is_empty()) {
            self.cwd = Some(PathBuf::from(cwd));
        }
        if wants_prompt
            && v.get("type").and_then(Value::as_str) == Some("user")
            && let Some(prompt) = prompt_of(&v)
        {
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
    hay.windows(needle.len()).any(|w| w == needle)
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
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(offset))?;
    let mut reader = BufReader::with_capacity(64 * 1024, file);
    let mut at = offset;
    let mut line = Vec::new();
    loop {
        line.clear();
        let n = reader.read_until(b'\n', &mut line)?;
        if n == 0 || line.last() != Some(&b'\n') {
            return Ok(at);
        }
        at += n as u64;
        t.feed(&line);
    }
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

pub use foreman_core::paths::claude_dir;

struct Cached {
    len: u64,
    modified: SystemTime,
    /// Where the next read starts.
    offset: u64,
    transcript: Transcript,
}

struct Listed {
    modified: Option<SystemTime>,
    at: SystemTime,
    /// `(path, len, mtime)` of each `*.jsonl`.
    files: Vec<(PathBuf, u64, SystemTime)>,
}

/// The transcripts under one `projects` dir, cached: the dir list by its mtime, each project dir's
/// listing by its mtime (re-read anyway every [`Self::RELIST_EVERY`], or while it holds a recent, kept or
/// live session), each file by its length (an appended file is read from where the last read stopped).
pub struct Transcripts {
    projects: PathBuf,
    dirs: Option<(Option<SystemTime>, SystemTime, Vec<String>)>,
    listed: HashMap<PathBuf, Listed>,
    files: HashMap<PathBuf, Cached>,
}

impl Transcripts {
    pub const RELIST_EVERY: Duration = Duration::from_secs(300);

    pub fn new(projects: PathBuf) -> Self {
        Self {
            projects,
            dirs: None,
            listed: HashMap::new(),
            files: HashMap::new(),
        }
    }

    /// Under `<claude dir>/projects`.
    pub fn in_claude_dir(claude_dir: &Path) -> Self {
        Self::new(claude_dir.join("projects"))
    }

    /// The [`Transcript::listable`] transcripts of the project dirs whose name starts with the [`slug`]
    /// of one of `roots` (case-insensitive), modified since `since_ms` or with an id in `keep`. `live`
    /// (ids `claude agents` lists running) keeps their dirs re-listed: an old file resumed grows
    /// without touching its dir's mtime.
    pub fn scan(&mut self, roots: &[PathBuf], since_ms: i64, keep: &HashSet<String>, live: &HashSet<String>) -> Vec<Transcript> {
        let now = SystemTime::now();
        let prefixes: Vec<String> = roots.iter().map(|r| slug(r).to_ascii_lowercase()).collect();
        let names = self.dir_names(now);
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for name in names.iter().filter(|n| {
            let lower = n.to_ascii_lowercase();
            prefixes.iter().any(|p| lower.starts_with(p.as_str()))
        }) {
            let dir = self.projects.join(name);
            for (path, len, modified) in self.list(&dir, now, since_ms, keep, live) {
                let Some(id) = session_id_of(&path) else { continue };
                if epoch_ms(modified) < since_ms && !keep.contains(&id) {
                    continue;
                }
                seen.insert(path.clone());
                if let Some(t) = self.read_cached(&path, len, modified)
                    && t.listable()
                {
                    out.push(t);
                }
            }
        }
        self.files.retain(|p, _| seen.contains(p));
        out
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

    fn read_cached(&mut self, path: &Path, len: u64, modified: SystemTime) -> Option<Transcript> {
        if let Some(c) = self.files.get_mut(path) {
            if c.len == len && c.modified == modified {
                return Some(c.transcript.clone());
            }
            if len > c.len {
                c.offset = feed_from(path, c.offset, &mut c.transcript).ok()?;
                c.len = len;
                c.modified = modified;
                c.transcript.modified_ms = epoch_ms(modified);
                return Some(c.transcript.clone());
            }
        }
        let mut transcript = Transcript {
            session_id: session_id_of(path)?,
            modified_ms: epoch_ms(modified),
            ..Transcript::default()
        };
        let offset = feed_from(path, 0, &mut transcript).ok()?;
        self.files.insert(path.to_path_buf(), Cached { len, modified, offset, transcript: transcript.clone() });
        Some(transcript)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Sanitised from a real transcript (claude 2.1.306): its line shapes, none of its text.
    const FIXTURE: &str = include_str!("../tests/fixtures/transcript.jsonl");
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
    }
}
