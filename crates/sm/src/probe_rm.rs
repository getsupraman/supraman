//! `sm probe rm-transcript`: what `claude rm` leaves of a stopped background session; see `docs/LEARNINGS.md` m5/38.

use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use supraman_claude::{Claude, Health, LaunchSpec, Removal, forget_transcript, is_session_id, now_ms};
use supraman_core::paths::claude_dir;
use supraman_core::process::{self, Kill};

use crate::probe::{Fleet, cargo_root, describe, hex6, mkdir, wait_until_stopped, yes};

const TURN_WITHIN: Duration = Duration::from_secs(180);
const RESUME_WITHIN: Duration = Duration::from_secs(180);

/// What a Claude Code tool shell exports: a child claude must not run as its session.
pub(crate) const PARENT_VARS: [&str; 12] = [
    "CLAUDECODE",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_PID",
    "CLAUDE_CODE_SESSION_ATTENDED",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_MESSAGING_TOKEN",
    "CLAUDE_CODE_EXECPATH",
    "CLAUDE_CODE_ALT_SCREEN_FULL_REPAINT",
    "CLAUDE_EFFORT",
    "CLAUDE_JOB_DIR",
];

pub fn run() -> bool {
    // single-threaded still: nothing has been spawned
    for var in PARENT_VARS {
        unsafe { std::env::remove_var(var) };
    }
    let claude = Claude::default();
    let hex = hex6();
    let mut fleet = Fleet { claude: claude.clone(), names: Vec::new() };
    let Some(claude_home) = claude_dir() else {
        println!("probe error: no claude dir");
        return false;
    };
    let root = cargo_root().join("target").join(format!("aw-m2-probe-{hex}"));
    let mut project: Option<PathBuf> = None;
    let ok = match probe(&claude, &mut fleet, &claude_home, &root, &hex, &mut project) {
        Ok(ok) => ok,
        Err(e) => {
            println!("probe error: {e}");
            false
        }
    };
    drop(fleet);
    if let Some(dir) = project {
        cleanup(&claude_home, &dir);
    }
    let _ = fs::remove_dir_all(&root);
    ok
}

fn probe(claude: &Claude, fleet: &mut Fleet, home: &Path, root: &Path, hex: &str, project: &mut Option<PathBuf>) -> Result<bool, String> {
    let cwd = mkdir(root, "rm")?;
    let name = format!("aw-m2-probe-{hex}-rm");
    let mut spec = LaunchSpec::new(&name, &cwd, "This is an automated probe. Reply with the word ready and stop.");
    spec.strict_mcp_config = true;
    spec.extra_args = vec!["--settings".into(), r#"{"worktree":{"bgIsolation":"none"}}"#.into()];
    println!("launch {name} from {}", cwd.display());
    let out = fleet.launch(&spec)?;
    println!("  {out}");
    let row = fleet.row(&name)?;
    let sid = row.session_id.clone();
    if !is_session_id(&sid) {
        return Err(format!("row has no session uuid: {}", describe(&row)));
    }
    let dir = home.join("projects").join(supraman_claude::transcript::slug(&cwd));
    *project = Some(dir.clone());
    let transcript = dir.join(format!("{sid}.jsonl"));
    let sibling = dir.join(&sid);

    let deadline = Instant::now() + TURN_WITHIN;
    loop {
        let row = fleet.row(&name)?;
        let ended = matches!(row.health(now_ms()), Health::Idle | Health::Stopped);
        if ended && transcript.exists() {
            println!("turn ended: {}", describe(&row));
            break;
        }
        if Instant::now() > deadline {
            return Err(format!("no turn end within {}s: {}", TURN_WITHIN.as_secs(), describe(&row)));
        }
        thread::sleep(Duration::from_millis(1000));
    }
    println!("before stop: transcript {} ({} bytes), sibling dir {}", yes(transcript.exists()), len(&transcript), yes(sibling.exists()));

    let stopped = claude.stop(&row.id).map_err(|e| format!("stop: {e}"))?;
    println!("stop: {}", stopped.trim());
    wait_until_stopped(claude, &name)?;
    let row = fleet.row(&name)?;
    println!("stopped row: {}", describe(&row));
    println!("after stop: transcript {} ({} bytes), sibling dir {}", yes(transcript.exists()), len(&transcript), yes(sibling.exists()));

    match claude.remove(&row.id, None).map_err(|e| format!("rm: {e}"))? {
        Removal::Removed(out) => println!("rm: {out}"),
        Removal::Kept(refusal) => return Err(format!("rm kept it: {}", refusal.text)),
    }
    thread::sleep(Duration::from_secs(2));
    let listed = claude.list_sessions().map_err(|e| e.to_string())?.into_iter().any(|s| s.session_id == sid);
    let files_after_rm = listing(&dir);
    println!(
        "after rm: row listed {} | transcript {} ({} bytes) | sibling dir {} | project dir holds {files_after_rm:?}",
        yes(listed),
        yes(transcript.exists()),
        len(&transcript),
        yes(sibling.exists())
    );

    let started = Instant::now();
    let mut command = process::command(&claude.program);
    command
        .args(["--resume", &sid, "-p", "--strict-mcp-config", "Without using any tool: what single word did you reply with earlier in this conversation? Reply with only that word, or UNKNOWN."])
        .current_dir(&cwd);
    let output = process::run(&mut command, RESUME_WITHIN, Kill::Child).map_err(|e| format!("resume -p: {e}"))?;
    println!(
        "resume -p after rm: exit {:?} in {} ms | stdout {:?} | stderr {:?}",
        output.code,
        started.elapsed().as_millis(),
        first(&output.stdout),
        first(&output.stderr)
    );
    println!(
        "after resume: transcript {} ({} bytes) | project dir holds {:?}",
        yes(transcript.exists()),
        len(&transcript),
        listing(&dir)
    );
    let rows: Vec<String> = claude.list_sessions().map_err(|e| e.to_string())?.into_iter().filter(|s| s.cwd == cwd).map(|s| describe(&s)).collect();
    println!("rows with the probe's cwd after resume: {rows:?}");
    Ok(true)
}

/// Deletes every session transcript in the probe's own project dir through the app's confined delete.
pub(crate) fn cleanup(home: &Path, dir: &Path) {
    for file in listing(dir) {
        let Some(sid) = file.strip_suffix(".jsonl").filter(|s| is_session_id(s)) else { continue };
        match forget_transcript(home, sid, None) {
            Ok(gone) => println!("cleanup: forgot {sid}: {} transcript(s), {} dir(s)", gone.transcripts.len(), gone.dirs.len()),
            Err(e) => println!("cleanup: cannot forget {sid}: {e}"),
        }
    }
    for name in listing(dir).into_iter().filter(|n| is_session_id(n)) {
        match forget_transcript(home, &name, None) {
            Ok(gone) => println!("cleanup: forgot the dir of {name}: {} dir(s)", gone.dirs.len()),
            Err(e) => println!("cleanup: cannot forget {name}: {e}"),
        }
    }
    println!("cleanup: project dir {} holds {:?}", dir.display(), listing(dir));
}

fn listing(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir).map(|r| r.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
    names.sort();
    names
}

fn len(path: &Path) -> u64 {
    fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

fn first(text: &str) -> String {
    text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default().chars().take(200).collect()
}
