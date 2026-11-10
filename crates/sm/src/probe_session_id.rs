//! `sm probe session-id`: whether `--session-id` and `--resume` keep a `--bg` session's uuid; see `docs/LEARNINGS.md` m5/40.

use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use supraman_claude::{Claude, Health, LaunchSpec, Removal, is_session_id, new_session_id, now_ms};
use supraman_core::paths::claude_dir;
use supraman_core::process::{self, Kill};

use crate::probe::{Fleet, cargo_root, describe, hex6, mkdir, wait_until_stopped, yes};
use crate::probe_rm::PARENT_VARS;

const TURN_WITHIN: Duration = Duration::from_secs(180);

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
    let target = std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from).unwrap_or_else(|| cargo_root().join("target"));
    let root = target.join(format!("aw-m2-probe-{hex}"));
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
        crate::probe_rm::cleanup(&claude_home, &dir);
    }
    let _ = fs::remove_dir_all(&root);
    ok
}

fn probe(claude: &Claude, fleet: &mut Fleet, home: &Path, root: &Path, hex: &str, project: &mut Option<PathBuf>) -> Result<bool, String> {
    let cwd = mkdir(root, "sid")?;
    let name = format!("aw-m2-probe-{hex}-sid");
    let wanted = new_session_id();
    let mut spec = LaunchSpec::new(&name, &cwd, "This is an automated probe. Reply with the word ready and stop.");
    spec.strict_mcp_config = true;
    spec.extra_args = vec![
        "--session-id".into(),
        wanted.clone(),
        "--settings".into(),
        r#"{"worktree":{"bgIsolation":"none"}}"#.into(),
    ];
    println!("launch {name} --session-id {wanted} from {}", cwd.display());
    *project = Some(home.join("projects").join(supraman_claude::transcript::slug(&cwd)));
    let out = fleet.launch(&spec)?;
    println!("  {out}");
    let row = fleet.row(&name)?;
    println!("row: {}", describe(&row));
    let launched_as = row.session_id.clone();
    println!("launch kept the id: {} (row sessionId {launched_as})", yes(launched_as == wanted));
    if !is_session_id(&launched_as) {
        return Err(format!("row has no session uuid: {}", describe(&row)));
    }
    let transcript = project.as_ref().unwrap().join(format!("{launched_as}.jsonl"));
    turn_end(fleet, &name, &transcript)?;

    let stopped = claude.stop(&row.id).map_err(|e| format!("stop: {e}"))?;
    println!("stop: {}", stopped.trim());
    wait_until_stopped(claude, &name)?;

    let before = fs::metadata(&transcript).map(|m| m.len()).unwrap_or(0);
    let out = claude
        .resume(&launched_as, &cwd, Some("This is an automated probe. Reply with the word again and stop."))
        .map_err(|e| format!("resume: {e}"))?;
    println!("resume --bg --resume {launched_as}: {out}");
    let rows: Vec<_> = claude.list_sessions().map_err(|e| e.to_string())?.into_iter().filter(|s| s.name == name).collect();
    for r in &rows {
        println!("row after resume: {}", describe(r));
    }
    let resumed_as = rows.iter().map(|r| r.session_id.clone()).collect::<Vec<_>>();
    println!("resume kept the id: {} (rows {resumed_as:?})", yes(resumed_as == [launched_as.clone()]));
    turn_end(fleet, &name, &transcript)?;
    let after = fs::metadata(&transcript).map(|m| m.len()).unwrap_or(0);
    println!("transcript {launched_as}.jsonl grew {before} -> {after} bytes");

    let row = fleet.row(&name)?;
    let stopped = claude.stop(&row.id).map_err(|e| format!("stop: {e}"))?;
    println!("stop: {}", stopped.trim());
    wait_until_stopped(claude, &name)?;
    match claude.remove(&row.id, None).map_err(|e| format!("rm: {e}"))? {
        Removal::Removed(out) => println!("rm: {out}"),
        Removal::Kept(refusal) => return Err(format!("rm kept it: {}", refusal.text)),
    }
    println!("FINDING --bg --session-id kept: {}; --bg --resume kept: {}", yes(launched_as == wanted), yes(resumed_as == [launched_as]));

    // the `+` tab's shape without --bg: does a foreground claude take the id?
    let headless = new_session_id();
    let started = Instant::now();
    let mut command = process::command(&claude.program);
    command
        .args(["-p", "--session-id", &headless, "--strict-mcp-config", "This is an automated probe. Reply with the word ready and stop."])
        .current_dir(&cwd);
    let output = process::run(&mut command, TURN_WITHIN, Kill::Child).map_err(|e| format!("-p: {e}"))?;
    let file = project.as_ref().unwrap().join(format!("{headless}.jsonl"));
    println!(
        "-p --session-id {headless}: exit {:?} in {} ms | stdout {:?} | transcript {headless}.jsonl {}",
        output.code,
        started.elapsed().as_millis(),
        output.stdout.trim(),
        yes(file.exists())
    );
    println!("FINDING foreground --session-id kept: {}", yes(file.exists()));
    Ok(true)
}

fn turn_end(fleet: &Fleet, name: &str, transcript: &Path) -> Result<(), String> {
    let deadline = Instant::now() + TURN_WITHIN;
    // a resumed row may still read idle from before its new turn
    thread::sleep(Duration::from_secs(3));
    loop {
        let row = fleet.row(name)?;
        if matches!(row.health(now_ms()), Health::Idle | Health::Stopped) && transcript.exists() {
            println!("turn ended: {}", describe(&row));
            return Ok(());
        }
        if Instant::now() > deadline {
            return Err(format!("no turn end within {}s: {}", TURN_WITHIN.as_secs(), describe(&row)));
        }
        thread::sleep(Duration::from_millis(1000));
    }
}
