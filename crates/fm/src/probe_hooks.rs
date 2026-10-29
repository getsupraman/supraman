//! `fm probe hooks`: which hooks a parked and an auto-compacting session fire. Answers: `docs/LEARNINGS.md` m2/2.

use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use foreman_claude::{Claude, LaunchSpec};
use foreman_core::process::{self, Kill};
use foreman_core::{Event, EventReader, role_tag};

use crate::probe::{Fleet, ROW_WITHIN, describe, hex6, mkdir, wait_until_stopped, cargo_root, write, yes};

const ROLE: &str = "probe";
const WINDOW: &str = "100000";
/// The output reserve comes off the window: at the default (128k for this model) a 100k window
/// compacts on every call, in a loop.
const MAX_OUTPUT: &str = "8000";
const PCT: &str = "80";
/// One big file per turn: more than one lets the next `Read` refill the context right after a
/// compaction, which loops until "rapid-refill breaker tripped" and the turn ends with no Stop.
const BIG_FILES: usize = 1;
const BIG_LINES: usize = 800;
const ROUNDS: usize = 5;
const EVENT_WITHIN: Duration = Duration::from_secs(90);
const TURN_WITHIN: Duration = Duration::from_secs(480);

const PROMPT_PARK: &str = "This is an automated probe. Run exactly this one Bash command and nothing else: \
`echo parked > parked.txt`. Then stop.";

/// Answered in the reply text, read back from the transcript: a file is not a reliable channel, because a
/// `--bg` session in a main checkout moves into `.claude/worktrees/<name>` before its first write.
const PROMPT_TAGS: &str = "This is an automated probe. Do not use any tool and do not write any file. Look through \
everything in your context (hook output, system reminders, the conversation summary) for lines that start with \
FOREMAN-ROLE. Your whole reply must be exactly those lines, as written, one per line, in the order they appear, and \
nothing else; if there are none, reply NONE.";

fn prompt_read(round: usize) -> String {
    let files: Vec<String> = (1..=BIG_FILES).map(|i| format!("big-{round}-{i}.txt")).collect();
    format!(
        "This is an automated probe. Use the Read tool to read each of these files in the current directory in full, \
one after another, without summarising them: {}. Then reply with the single word done.",
        files.join(", ")
    )
}

/// `repo`: the checkout whose `target/` holds the scratch cwd (default: this build's cargo workspace). The
/// plugin always comes from this build's cargo workspace, so nothing is written into `repo` outside `target/`.
pub fn run(repo: Option<PathBuf>) -> bool {
    let claude = Claude::default();
    let hex = hex6();
    let root = repo.unwrap_or_else(cargo_root).join("target").join(format!("aw-m2-probe-{hex}"));
    let mut fleet = Fleet {
        claude: claude.clone(),
        names: Vec::new(),
    };
    println!("scratch {}", root.display());
    let ok = match probe(&claude, &mut fleet, &root, &hex) {
        Ok(ok) => ok,
        Err(e) => {
            println!("probe error: {e}");
            false
        }
    };
    drop(fleet);
    ok
}

struct Setup {
    plugin: PathBuf,
    home: PathBuf,
}

impl Setup {
    fn role_file(&self) -> PathBuf {
        self.home.join("roles").join(format!("{ROLE}.md"))
    }

    /// Writes role text version `v` and returns its tag line.
    fn set_role(&self, hex: &str, v: u32) -> Result<String, String> {
        let text = format!("You are the Foreman m2/2 probe role, version {v} (run {hex}). Follow the user's probe instructions exactly.");
        fs::write(self.role_file(), format!("{text}\n")).map_err(|e| format!("write role: {e}"))?;
        Ok(role_tag(ROLE, &text))
    }

    fn spec(&self, name: String, cwd: &Path, prompt: &str, extra_env: &[(&str, &str)]) -> LaunchSpec {
        let mut env = serde_json::Map::new();
        env.insert("FOREMAN_HOME".into(), self.home.display().to_string().into());
        env.insert("FOREMAN_ROLE".into(), ROLE.into());
        for (k, v) in extra_env {
            env.insert((*k).into(), (*v).into());
        }
        let mut spec = LaunchSpec::new(name, cwd, prompt);
        // no MCP server: the probe needs none, and none can put a token into a debug log
        spec.strict_mcp_config = true;
        spec.plugin_dirs = vec![self.plugin.clone()];
        spec.extra_args = vec!["--settings".into(), serde_json::json!({ "env": env }).to_string()];
        spec
    }
}

fn probe(claude: &Claude, fleet: &mut Fleet, root: &Path, hex: &str) -> Result<bool, String> {
    let plugin = cargo_root().join("plugins").join("foreman");
    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let (_, copied) = foreman_core::install_hook_exe(&plugin, &exe).map_err(|e| format!("plugin install: {e}"))?;
    println!("plugin {} (fm.exe {})", plugin.display(), if copied { "installed" } else { "up to date" });
    let setup = Setup {
        plugin,
        home: mkdir(root, "home")?,
    };
    mkdir(&setup.home, "roles")?;
    let tag_v1 = setup.set_role(hex, 1)?;

    let dir_park = mkdir(root, "park")?;
    let mut spec_park = setup.spec(format!("aw-m2-probe-{hex}-park"), &dir_park, PROMPT_PARK, &[]);
    spec_park.permission_mode = "default".into();

    let dir_compact = mkdir(root, "compact")?;
    for round in 1..=ROUNDS {
        for i in 1..=BIG_FILES {
            write(&dir_compact, &format!("big-{round}-{i}.txt"), &big_text(round, i))?;
        }
    }
    let mut spec_compact = setup.spec(
        format!("aw-m2-probe-{hex}-compact"),
        &dir_compact,
        &prompt_read(1),
        &[
            ("CLAUDE_AUTOCOMPACT_PCT_OVERRIDE", PCT),
            ("CLAUDE_CODE_AUTO_COMPACT_WINDOW", WINDOW),
            ("CLAUDE_CODE_MAX_OUTPUT_TOKENS", MAX_OUTPUT),
        ],
    );
    let debug_log = root.join("compact-debug.log");
    spec_compact.extra_args.extend(["--autocompact".to_string(), "100k".to_string(), "--debug-file".to_string(), debug_log.display().to_string()]);

    fleet.launch(&spec_park)?;
    fleet.launch(&spec_compact)?;
    let park = fleet.row(&spec_park.name)?;
    let compact = fleet.row(&spec_compact.name)?;
    let mut log = Log::new(&setup.home);

    // the compact session's startup injection was v1; everything after it sees v2
    let started = log.wait(&compact.session_id, EVENT_WITHIN, |es| any(es, "SessionStart", Some("startup")));
    let tag_v2 = setup.set_role(hex, 2)?;
    println!("compact: startup SessionStart seen={} → role file now v2", yes(started));

    let park_ok = log.wait(&park.session_id, Duration::from_secs(180), |es| {
        any(es, "SessionStart", Some("startup")) && any(es, "UserPromptSubmit", None) && any(es, "Notification", Some("permission_prompt"))
    });
    println!("park ({}): {}", describe(&park), summary(log.of(&park.session_id)));
    fleet.remove(&spec_park.name);

    // rounds of big reads until an auto-compaction has happened; resume between rounds
    let mut stop_seen = false;
    let mut compacted = false;
    for round in 1..=ROUNDS {
        if round > 1 {
            claude.stop(&compact.id).map_err(|e| e.to_string())?;
            wait_until_stopped(claude, &spec_compact.name)?;
            claude
                .resume(&compact.session_id, &dir_compact, Some(&prompt_read(round)))
                .map_err(|e| format!("resume: {e}"))?;
        }
        let before = log.of(&compact.session_id).iter().filter(|e| e.event == "Stop").count();
        let turn_ended = log.wait(&compact.session_id, TURN_WITHIN, |es| es.iter().filter(|e| e.event == "Stop").count() > before);
        stop_seen |= turn_ended;
        compacted = auto_compacted(log.of(&compact.session_id));
        println!(
            "compact round {round}: turn ended={} | PreCompact(auto)+SessionStart(compact)={} | {}",
            yes(turn_ended),
            yes(compacted),
            summary(log.of(&compact.session_id))
        );
        if compacted || !turn_ended {
            break;
        }
    }

    // the proof turn: v3 on disk, so a v2 tag can only be the compact-time injection
    let tag_v3 = setup.set_role(hex, 3)?;
    let mut tags = None;
    if compacted {
        claude.stop(&compact.id).map_err(|e| e.to_string())?;
        wait_until_stopped(claude, &spec_compact.name)?;
        let n = log.of(&compact.session_id).len();
        let stops = log.of(&compact.session_id).iter().filter(|e| e.event == "Stop").count();
        let out = claude.resume(&compact.session_id, &dir_compact, Some(PROMPT_TAGS)).map_err(|e| format!("resume: {e}"))?;
        println!("compact: resumed for the tag question: {}", out.lines().next().unwrap_or(""));
        let answered = log.wait(&compact.session_id, TURN_WITHIN, |es| es.iter().filter(|e| e.event == "Stop").count() > stops);
        let all = log.of(&compact.session_id);
        let transcript = all.iter().rev().find_map(|e| e.transcript_path.clone());
        if answered && let Some(path) = &transcript {
            tags = last_reply(Path::new(path));
        }
        println!(
            "compact: question turn ended={} | reply read from the transcript {}",
            yes(answered),
            transcript.as_deref().unwrap_or("(no transcript_path)")
        );
        println!(
            "compact: hooks after --bg --resume: {}",
            all[n.min(all.len())..].iter().map(label).collect::<Vec<_>>().join(", ")
        );
    } else {
        println!("compact: no auto-compaction within {ROUNDS} rounds; manual fallback:");
        manual_compact(&setup, &dir_compact, &compact.session_id, claude, &spec_compact.name)?;
    }

    let has = |tag: &str| tags.as_deref().is_some_and(|t| t.lines().any(|l| l.trim() == tag));
    let role_ok = has(&tag_v2);
    println!(
        "role re-injection: reply lists v1={} v2(compact)={} v3(resume)={} [{}]",
        yes(has(&tag_v1)),
        yes(role_ok),
        yes(has(&tag_v3)),
        tags.as_deref().map(|t| t.lines().map(str::trim).collect::<Vec<_>>().join(" / ")).unwrap_or_else(|| "no answer".into())
    );
    println!("compact: autocompact debug lines:\n{}", autocompact_lines(&debug_log));
    let row = fleet.row(&spec_compact.name).map(|r| describe(&r)).unwrap_or_default();
    println!("compact row: {row}");

    let ok = park_ok && stop_seen && compacted && role_ok;
    println!(
        "hooks: park signals={} | Stop={} | auto-compact={} | role after compact={} → {}",
        yes(park_ok),
        yes(stop_seen),
        yes(compacted),
        yes(role_ok),
        if ok { "PASS" } else { "FAIL" }
    );
    Ok(ok)
}

/// The documented fallback when auto-compaction cannot be triggered cheaply: `claude --resume <id> -p
/// /compact`, with the plugin and env passed again because `-p` starts a fresh process.
fn manual_compact(setup: &Setup, cwd: &Path, session_id: &str, claude: &Claude, name: &str) -> Result<(), String> {
    let _ = claude.find_by_name(name, ROW_WITHIN).ok().and_then(|r| r.into_iter().next()).map(|r| claude.stop(&r.id));
    wait_until_stopped(claude, name)?;
    let spec = setup.spec(name.to_string(), cwd, "", &[]);
    let started = Instant::now();
    let out = process::run(
        process::command(&claude.program)
            .args(["--resume", session_id, "-p", "/compact", "--plugin-dir"])
            .arg(&setup.plugin)
            .args(&spec.extra_args)
            .current_dir(cwd),
        TURN_WITHIN,
        Kill::Child,
    )
    .map_err(|e| format!("claude -p /compact: {e}"))?;
    let mut log = Log::new(&setup.home);
    log.wait(session_id, Duration::from_secs(5), |es| auto_or_manual(es, "manual"));
    println!(
        "  manual /compact: exit {:?} in {:.1}s | {}",
        out.code,
        started.elapsed().as_secs_f32(),
        summary(log.of(session_id))
    );
    Ok(())
}

/// The text of the last assistant message in a transcript `.jsonl` that has any.
pub(crate) fn last_reply(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    text.lines().rev().find_map(|line| {
        let v: serde_json::Value = serde_json::from_str(line).ok()?;
        if v.get("type")?.as_str()? != "assistant" {
            return None;
        }
        let parts: Vec<&str> = v["message"]["content"]
            .as_array()?
            .iter()
            .filter(|c| c["type"] == "text")
            .filter_map(|c| c["text"].as_str())
            .collect();
        (!parts.is_empty()).then(|| parts.join("\n"))
    })
}

/// The `autocompact:` lines of the debug log (token counts are redacted there), deduplicated.
fn autocompact_lines(path: &Path) -> String {
    let text = fs::read_to_string(path).unwrap_or_default();
    let mut lines: Vec<&str> = Vec::new();
    for l in text.lines().filter(|l| l.contains("autocompact:") || l.contains("Reactive compact") || l.contains("[reactive-compact] finished")) {
        let l = l.split_once("] ").map_or(l, |(_, rest)| rest);
        if !lines.contains(&l) && lines.len() < 30 {
            lines.push(l);
        }
    }
    lines.iter().map(|l| format!("  {l}")).collect::<Vec<_>>().join("
")
}

/// Every event read so far, per session.
pub(crate) struct Log {
    reader: EventReader,
    events: Vec<Event>,
}

impl Log {
    pub(crate) fn new(home: &Path) -> Self {
        Self {
            reader: EventReader::new(home),
            events: Vec::new(),
        }
    }

    pub(crate) fn of(&self, session_id: &str) -> Vec<Event> {
        self.events.iter().filter(|e| e.session_id == session_id).cloned().collect()
    }

    pub(crate) fn wait(&mut self, session_id: &str, within: Duration, done: impl Fn(&[Event]) -> bool) -> bool {
        let deadline = Instant::now() + within;
        loop {
            self.events.extend(self.reader.poll());
            if done(&self.of(session_id)) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            thread::sleep(Duration::from_millis(500));
        }
    }
}

fn any(events: &[Event], event: &str, detail: Option<&str>) -> bool {
    events.iter().any(|e| e.event == event && (detail.is_none() || detail_of(e) == detail))
}

fn detail_of(e: &Event) -> Option<&str> {
    e.source.as_deref().or(e.notification_type.as_deref()).or(e.trigger.as_deref())
}

fn auto_compacted(events: Vec<Event>) -> bool {
    auto_or_manual(&events, "auto")
}

/// PreCompact with `trigger` followed later by SessionStart(compact).
fn auto_or_manual(events: &[Event], trigger: &str) -> bool {
    let Some(pre) = events.iter().position(|e| e.event == "PreCompact" && e.trigger.as_deref() == Some(trigger)) else {
        return false;
    };
    events[pre..].iter().any(|e| e.event == "SessionStart" && e.source.as_deref() == Some("compact"))
}

fn label(e: &Event) -> String {
    match detail_of(e) {
        Some(d) => format!("{}({d})", e.event),
        None => e.event.clone(),
    }
}

fn summary(events: Vec<Event>) -> String {
    let Some(first) = events.first() else {
        return "no events".into();
    };
    let names = events
        .iter()
        .map(|e| format!("{}@+{:.1}s", label(e), (e.ts - first.ts) as f64 / 1000.0))
        .collect::<Vec<_>>()
        .join(" ");
    format!("{} event(s): {names}", events.len())
}

/// About 80 KB of plain words per file: ~20k tokens, under the Read tool's per-call cap.
fn big_text(round: usize, file: usize) -> String {
    const WORDS: [&str; 16] = [
        "foreman", "probe", "context", "window", "compaction", "session", "hook", "event", "ledger", "worker",
        "controller", "signal", "summary", "turn", "budget", "token",
    ];
    let mut out = String::with_capacity(BIG_LINES * 100);
    for line in 0..BIG_LINES {
        out.push_str(&format!("r{round}f{file} line {line:04}:"));
        for k in 0..12 {
            out.push(' ');
            out.push_str(WORDS[(line * 7 + k * 3 + file + round) % WORDS.len()]);
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(event: &str, source: Option<&str>, trigger: Option<&str>) -> Event {
        Event {
            ts: 1,
            event: event.into(),
            session_id: "s".into(),
            name: String::new(),
            source: source.map(Into::into),
            notification_type: None,
            trigger: trigger.map(Into::into),
            cwd: String::new(),
            transcript_path: None,
        }
    }

    #[test]
    fn last_reply_is_the_final_assistant_text() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        let lines = [
            r#"{"type":"user","message":{"role":"user","content":"q"}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"old"}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"A"},{"type":"text","text":"B"}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"x"}]}}"#,
            r#"{"type":"system","subtype":"x"}"#,
            "not json",
        ];
        fs::write(&path, lines.join("\n")).unwrap();
        assert_eq!(last_reply(&path).as_deref(), Some("A\nB"));
        assert_eq!(last_reply(&dir.path().join("missing")), None);
    }

    #[test]
    fn auto_compaction_needs_precompact_then_session_start() {
        let pre = e("PreCompact", None, Some("auto"));
        let start = e("SessionStart", Some("compact"), None);
        assert!(auto_compacted(vec![e("Stop", None, None), pre.clone(), start.clone()]));
        assert!(!auto_compacted(vec![start.clone(), pre.clone()]));
        assert!(!auto_compacted(vec![e("PreCompact", None, Some("manual")), start]));
    }

    #[test]
    fn big_text_is_about_eighty_kilobytes() {
        let n = big_text(1, 1).len();
        assert!((60_000..100_000).contains(&n), "{n}");
    }
}
