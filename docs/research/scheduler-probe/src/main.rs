//! sched/1 probe: what a process started by Task Scheduler (logged-on user, limited rights, no
//! console) can reach. `serve <home>` runs interactively; `task <home> <scratch> <repo>` runs from
//! the task and writes `<home>/task-<stamp>.log`. Never prints a token.
#![windows_subsystem = "windows"]

use std::fmt::Write as _;
use std::path::Path;
use std::time::Duration;

use foreman_claude::{Claude, LaunchSpec};
use foreman_core::secret::{FileStore, SecretStore};
use foreman_workstate::{Auth, fake::FakeWorkState};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["serve", home] => serve(Path::new(home)),
        ["task", home, scratch, repo] => task(Path::new(home), Path::new(scratch), Path::new(repo)),
        _ => std::process::exit(2),
    }
}

/// A fake WorkState with its OAuth server; signs in once through the DPAPI FileStore of `home`, then
/// serves refreshes until `<home>/stop` exists.
fn serve(home: &Path) {
    std::fs::create_dir_all(home).unwrap();
    let ws = FakeWorkState::start(Vec::new());
    ws.enable_oauth();
    foreman_core::Settings::update(home, |s| s.workstate.url = Some(ws.base().to_string())).unwrap();
    let auth = Auth::open(home);
    let status = auth.sign_in(ws.base(), &foreman_workstate::auth::http_browser());
    let secrets = std::fs::read_dir(home.join("oauth").join("secrets")).map(|d| d.count()).unwrap_or(0);
    std::fs::write(home.join("ready"), format!("{} sign_in={status:?} secret_files={secrets}\n", ws.base())).unwrap();
    while !home.join("stop").exists() {
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn task(home: &Path, scratch: &Path, repo: &Path) {
    let stamp = foreman_core::time::now_ms();
    let mut log = String::new();
    context(&mut log);
    probe_a(&mut log, scratch);
    probe_b(&mut log, home);
    probe_c(&mut log, repo);
    std::fs::write(home.join(format!("task-{stamp}.log")), log).unwrap();
}

fn context(log: &mut String) {
    use windows::Win32::System::Console::GetConsoleWindow;
    use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
    let console = unsafe { GetConsoleWindow() };
    let mut session = 0u32;
    let _ = unsafe { ProcessIdToSessionId(std::process::id(), &mut session) };
    let _ = writeln!(log, "== context");
    let _ = writeln!(log, "user={} session={session} elevated={} console_window={}", std::env::var("USERNAME").unwrap_or_default(), foreman_claude::is_elevated(), !console.is_invalid());
    let _ = writeln!(log, "cwd={}", std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_default());
    let path = std::env::var("PATH").unwrap_or_default();
    let _ = writeln!(log, "PATH has .local\\bin: {}", path.to_lowercase().contains(r".local\bin"));
    let _ = writeln!(log, "user_home={:?} claude_dir={:?}", foreman_core::paths::user_home(), foreman_claude::claude_dir());
}

fn probe_a(log: &mut String, scratch: &Path) {
    let _ = writeln!(log, "== (a) claude");
    let claude = Claude::default();
    match claude.run_in(&["--version"], None, Duration::from_secs(30)) {
        Ok(v) => { let _ = writeln!(log, "claude --version: {}", v.trim()); }
        Err(e) => { let _ = writeln!(log, "claude --version FAILED: {e}"); }
    }
    let _ = writeln!(log, "claude daemon status: {:?}", foreman_claude::daemon_status());
    match claude.list_sessions() {
        Ok(rows) => { let _ = writeln!(log, "claude agents --json --all: ok, {} rows", rows.len()); }
        Err(e) => { let _ = writeln!(log, "claude agents --json --all FAILED: {e}"); }
    }
    std::fs::create_dir_all(scratch).unwrap();
    let name = format!("probe-sched-1-{}", foreman_core::time::hex6());
    let mut spec = LaunchSpec::new(&name, scratch, "This is a probe started by a scheduled task. Reply with the word OK and nothing else.");
    spec.permission_mode = "plan".into();
    match claude.launch(&spec) {
        Ok(out) => { let _ = writeln!(log, "claude --bg --name {name}: {out}"); }
        Err(e) => {
            let _ = writeln!(log, "claude --bg --name {name} FAILED: {e}");
            return;
        }
    }
    let rows = claude.find_by_name(&name, Duration::from_secs(30)).unwrap_or_default();
    let Some(row) = rows.first() else {
        let _ = writeln!(log, "no row named {name} within 30 s");
        return;
    };
    let _ = writeln!(log, "row: id={} sessionId={} pid={:?} state={:?} status={:?} cwd={}", row.id, row.session_id, row.pid, row.state, row.status, row.cwd.display());
    std::thread::sleep(Duration::from_secs(15));
    match claude.stop(&row.id) {
        Ok(out) => { let _ = writeln!(log, "claude stop {}: {}", row.id, out.trim()); }
        Err(e) => { let _ = writeln!(log, "claude stop {} FAILED: {e}", row.id); }
    }
    std::thread::sleep(Duration::from_secs(3));
    if let Some(after) = claude.list_sessions().ok().and_then(|r| r.into_iter().find(|s| s.id == row.id)) {
        let _ = writeln!(log, "after stop: pid={:?} state={:?} status={:?}", after.pid, after.state, after.status);
    }
}

fn probe_b(log: &mut String, home: &Path) {
    let _ = writeln!(log, "== (b) DPAPI FileStore and OAuth refresh");
    let auth = Auth::open(home);
    let origin = auth.configured_origin();
    let _ = writeln!(log, "configured origin: {origin}");
    if let Some(issuer) = auth.issuer_of(&origin) {
        match FileStore::new(home).get(&issuer) {
            Ok(Some(rt)) => { let _ = writeln!(log, "FileStore get: refresh token read, {} chars", rt.len()); }
            Ok(None) => { let _ = writeln!(log, "FileStore get: none"); }
            Err(e) => { let _ = writeln!(log, "FileStore get FAILED: {e}"); }
        }
    }
    match auth.access_token(&origin) {
        Ok(token) => { let _ = writeln!(log, "access_token: token obtained, {} chars", token.len()); }
        Err(e) => { let _ = writeln!(log, "access_token FAILED: {e}"); }
    }
}

fn probe_c(log: &mut String, repo: &Path) {
    let _ = writeln!(log, "== (c) git status in {}", repo.display());
    let entries = foreman_core::process::git(repo, &["config", "--get-all", "safe.directory"]).map(|o| o.stdout).unwrap_or_default();
    let wanted = repo.display().to_string().replace('\\', "/").to_lowercase();
    let listed = entries.lines().any(|l| l.trim().to_lowercase() == wanted || l.trim() == "*");
    let _ = writeln!(log, "safe.directory: {} entries, this repo (or *) listed: {listed}", entries.lines().count());
    for args in [&["--no-optional-locks", "status", "--short", "--branch"][..]] {
        match foreman_core::process::git(repo, args) {
            Ok(out) => {
                let _ = writeln!(log, "git {}: exit {:?}", args.join(" "), out.code);
                let first = |s: &str| s.lines().take(4).collect::<Vec<_>>().join(" | ");
                let _ = writeln!(log, "  stdout: {}", first(&out.stdout));
                let _ = writeln!(log, "  stderr: {}", first(&out.stderr));
            }
            Err(e) => { let _ = writeln!(log, "git {} FAILED: {e}", args.join(" ")); }
        }
    }
}
