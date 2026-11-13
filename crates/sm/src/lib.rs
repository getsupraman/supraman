use std::collections::HashSet;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use supraman_claude::{ClaudeError, Health, Session, check_supported, list_sessions, now_ms, place};
use supraman_core::folders::Entries;
use supraman_core::paths::path_key;
use supraman_core::CoreError;

const USAGE: &str = "\
usage:
  sm folder add <dir> [--group <name>] [--at N]   (git or plain; at the top level's end, or the group's)
  sm folder remove <dir> [--group <name>]          (from the top level, else the first group listing it)
  sm folder list
  sm group add <name> [--at N]  |  sm group rename <name> <new>
  sm group remove <name> [--with-folders]          (else its folders move to the top level)
  sm sessions [--folder <dir>] [--all]   (each under the deepest listed folder holding its cwd)
  sm schedule add [--folder <dir>] --name <text> --every <when> (--command /<name> | --prompt-file <path> | --prompt <text>)
      [--id <id>] [--script <path> [--script-arg <a>]... [--script-timeout 10m]]
      [--profile read|edit|auto] [--allow-tools <rule>]... [--deny-tools <rule>]... [--add-dir <dir>]...
      [--secret NAME]... [--model <model>] [--max-runtime 1h] [--max-tokens N] [--overlap skip|queue|restart]
      [--catch-up once|skip] [--wake] [--description <text>]
      <when>: 'weekly mon,fri 09:00' | 'daily 09:00' | 'interval 2h [from 08:00]'; a --script alone runs no session
  sm schedule edit <id> [--folder <dir>] <add's options>   (a list option replaces the list; --no-script, --no-wake)
  sm schedule list [--folder <dir>]  |  sm schedule show <id> [--folder <dir>]
  sm schedule pause|resume|remove <id> [--folder <dir>]
  sm schedule dry-run <id> [--folder <dir>]   (the prompt, the claude argv, the permissions, the next three fires)
  sm schedule run <id> [--folder <dir>]       (Run now: an active, approved schedule fires at once)
  sm schedule secret set <id> <NAME> [--folder <dir>]   (the value on stdin; the script gets it as $NAME)
  sm schedule report <run> done|needs-you <one line>   (inside a scheduled run: its outcome, to the run's owner)
  sm schedule fire --home <home> <folder key> <id> --fingerprint <hex>   (what the schedule's task runs, as smw.exe)
      --folder <dir>: the deepest listed folder holding it, default the current folder's. An added or changed
      schedule is pending until Approve in the Supraman window; sm never approves and never registers a task.
  sm release manifest <zip> --version <semver> --commit <sha> [--notes-url <url>]
      (writes supraman-update.json and SHA256SUMS beside the zip; the zip is Supraman-<semver>.zip)
  sm release sign <manifest>           (the secret key from UPDATE_SIGNING_KEY; writes <manifest>.sig)
  sm release verify <manifest> [--public-key <file>]   (the signature against update.pub, then the zip)
  sm release keygen --public-out <file>   (the secret key to stdout only, never to a console)
  sm update apply --from <release dir> --to <install dir> --log <file> [--wait-pid <pid>] [--wait-secs 60]
      [--restart <exe> [--restart-arg <a>]...]   (what Update runs from a copy of the new sm.exe)
  sm plugin install <plugin-dir>  (copies this sm.exe to <plugin-dir>/bin, where the hooks run it)
  sm hook <event>                 (run by plugins/supraman; reads the hook payload on stdin)";

#[cfg(feature = "probes")]
const PROBE_USAGE: &str = "\
  sm probe hooks [--root <repo>]       (scratch cwd under <repo>/target; default: this build's checkout)
  sm probe isolation [--root <repo>]   (<repo>: a main checkout; default: this build's main checkout)
  sm probe rm-transcript               (what claude rm leaves of a stopped --bg session; resume after it)
  sm probe session-id                  (does --bg --session-id <uuid> keep the uuid, and --bg --resume)";

#[cfg(feature = "probes")]
fn usage() -> String {
    format!("{USAGE}\n{PROBE_USAGE}")
}

#[cfg(not(feature = "probes"))]
fn usage() -> String {
    USAGE.to_string()
}

mod folders;
pub mod fire;
mod hook;
#[cfg(feature = "probes")]
mod probe;
#[cfg(feature = "probes")]
mod probe_hooks;
#[cfg(feature = "probes")]
mod probe_isolation;
#[cfg(feature = "probes")]
mod probe_rm;
#[cfg(feature = "probes")]
mod probe_session_id;
mod release;
pub mod run;
mod schedule;
mod update;

enum Failure {
    Usage(String),
    Core(CoreError),
    Claude(ClaudeError),
    #[cfg(feature = "probes")]
    Probe(&'static str),
    /// Printed as is, exit 1.
    Refused(String),
    /// Already reported; the exit code alone.
    Exit(u8),
}

impl From<CoreError> for Failure {
    fn from(e: CoreError) -> Self {
        Self::Core(e)
    }
}

impl From<ClaudeError> for Failure {
    fn from(e: ClaudeError) -> Self {
        Self::Claude(e)
    }
}

/// A failure as one line, for a run record.
fn failure_text(f: Failure) -> String {
    match f {
        Failure::Usage(m) => m,
        Failure::Core(e) => e.to_string(),
        Failure::Claude(e) => e.to_string(),
        #[cfg(feature = "probes")]
        Failure::Probe(what) => what.to_string(),
        Failure::Refused(line) => line.trim_start_matches("error: ").to_string(),
        Failure::Exit(code) => format!("exit {code}"),
    }
}

/// Hidden: ssh's `SSH_ASKPASS`; the app bridge's answer on stdout, else exit 1 (supraman_core::askpass).
fn askpass(args: &[String]) -> ExitCode {
    match supraman_core::askpass::ask(&args.join(" ")) {
        Some(answer) => {
            println!("{answer}");
            ExitCode::SUCCESS
        }
        None => ExitCode::from(1),
    }
}

/// `sm`'s command line, without the program name.
pub fn cli(args: Vec<String>) -> ExitCode {
    if args.first().map(String::as_str) == Some("mcp-header") {
        return mcp_header(&args[1..]);
    }
    if args.first().map(String::as_str) == Some("askpass") {
        return askpass(&args[1..]);
    }
    clean_install();
    // A hook must never break the session: exit 0 whatever the arguments or the outcome.
    if args.first().map(String::as_str) == Some("hook") {
        if let Some(event) = args.get(1) {
            std::panic::set_hook(Box::new(|_| {}));
            let _ = std::panic::catch_unwind(|| hook::run(event));
        }
        return ExitCode::SUCCESS;
    }
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure::Usage(msg)) => {
            eprintln!("error: {msg}\n{USAGE}");
            ExitCode::from(1)
        }
        Err(Failure::Core(e)) => {
            eprintln!("error: {e}");
            ExitCode::from(if e.is_user_error() { 1 } else { 2 })
        }
        Err(Failure::Claude(e)) => {
            eprintln!("error: {e}");
            ExitCode::from(2)
        }
        #[cfg(feature = "probes")]
        Err(Failure::Probe(what)) => {
            eprintln!("error: {what} (see the probe lines above)");
            ExitCode::from(1)
        }
        Err(Failure::Refused(line)) => {
            eprintln!("{line}");
            ExitCode::from(1)
        }
        Err(Failure::Exit(code)) => ExitCode::from(code),
    }
}

/// `smw.exe`: `schedule fire` and the window's Run now (`schedule run --home`), with no console; what it has to say goes to `schedules/fire.log`.
pub fn smw(args: Vec<String>) -> ExitCode {
    clean_install();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["schedule", "fire", rest @ ..] => ExitCode::from(fire::fire_cmd(rest, false)),
        ["schedule", "run", rest @ ..] => ExitCode::from(fire::run_now_cmd(rest)),
        _ => ExitCode::from(fire::EXIT_UNKNOWN),
    }
}

/// A session's `headersHelper`, before any start-up work: always one header line and exit 0 (`supraman_supervisor::header`).
fn mcp_header(args: &[String]) -> ExitCode {
    use std::io::Write as _;
    std::panic::set_hook(Box::new(|_| {}));
    let line = std::panic::catch_unwind(|| {
        let (home, name) = match args {
            [flag, home, name] if flag == "--home" => (Some(std::path::PathBuf::from(home)), name.as_str()),
            [flag, hex, name] if flag == "--home-hex" => (supraman_claude::mcp_header_home(hex), name.as_str()),
            _ => (None, ""),
        };
        supraman_supervisor::header(home.as_deref(), name)
    });
    let line = line.unwrap_or_else(|_| supraman_supervisor::header(None, ""));
    let _ = writeln!(std::io::stdout(), "{line}");
    ExitCode::SUCCESS
}

/// The leftovers of an update in the install this exe runs from (`supraman_core::install::cleanup`).
fn clean_install() {
    if let Ok(dir) = supraman_core::install::exe_dir() {
        supraman_core::install::cleanup(&supraman_core::install::install_dir(&dir));
    }
}

fn run(args: &[String]) -> Result<(), Failure> {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["-h" | "--help" | "help"] => {
            println!("{}", usage());
            Ok(())
        }
        ["-V" | "--version"] => {
            println!("sm {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        ["folder", rest @ ..] => folders::folder(rest),
        ["group", rest @ ..] => folders::group(rest),
        ["sessions", rest @ ..] => sessions(rest),
        ["schedule", "fire", rest @ ..] => match fire::fire_cmd(rest, true) {
            0 => Ok(()),
            code => Err(Failure::Exit(code)),
        },
        ["schedule", rest @ ..] => schedule::schedule(&home()?, rest),
        #[cfg(feature = "probes")]
        ["probe", rest @ ..] => probe_cmd(rest),
        ["release", rest @ ..] => release::release(rest),
        ["update", rest @ ..] => update::update(rest),
        ["plugin", "install", dir] => {
            let exe = std::env::current_exe().map_err(|e| Failure::Usage(format!("cannot locate sm.exe: {e}")))?;
            match supraman_core::install_hook_exe(std::path::Path::new(dir), &exe) {
                Ok((path, true)) => println!("installed {}", path.display()),
                Ok((path, false)) => println!("up to date {}", path.display()),
                Err(e) => return Err(Failure::Usage(format!("plugin install {dir}: {e}"))),
            }
            Ok(())
        }
        [] => Err(Failure::Usage("missing command".into())),
        _ => Err(Failure::Usage(format!("unrecognised arguments: {}", args.join(" ")))),
    }
}

#[cfg(feature = "probes")]
fn probe_cmd(args: &[&str]) -> Result<(), Failure> {
    match args {
        ["hooks", rest @ ..] => {
            let root = probe_root(rest)?;
            check_claude()?;
            if probe_hooks::run(root) {
                Ok(())
            } else {
                Err(Failure::Probe("a hook signal was not observed"))
            }
        }
        ["isolation", rest @ ..] => {
            let root = probe_root(rest)?;
            check_claude()?;
            if probe_isolation::run(root) {
                Ok(())
            } else {
                Err(Failure::Probe("an isolation session did not finish its turn"))
            }
        }
        ["rm-transcript"] => {
            check_claude()?;
            if probe_rm::run() {
                Ok(())
            } else {
                Err(Failure::Probe("the rm-transcript probe did not finish"))
            }
        }
        ["session-id"] => {
            check_claude()?;
            if probe_session_id::run() {
                Ok(())
            } else {
                Err(Failure::Probe("the session-id probe did not finish"))
            }
        }
        _ => Err(Failure::Usage(format!("unrecognised arguments: probe {}", args.join(" ")))),
    }
}

/// [`check_supported`], warning on stderr about a claude newer than tested; only one too old fails.
pub(crate) fn check_claude() -> Result<(), Failure> {
    if let Some(warning) = check_supported()?.untested() {
        eprintln!("sm: warning: {warning}");
    }
    Ok(())
}

#[cfg(feature = "probes")]
fn probe_root(args: &[&str]) -> Result<Option<PathBuf>, Failure> {
    match args {
        [] => Ok(None),
        ["--root", dir] => std::path::absolute(dir)
            .map(Some)
            .map_err(|e| Failure::Usage(format!("--root {dir}: {e}"))),
        _ => Err(Failure::Usage(format!("unexpected arguments: {}", args.join(" ")))),
    }
}

pub(crate) fn value<'a>(it: &mut std::slice::Iter<'_, &'a str>, flag: &str) -> Result<&'a str, Failure> {
    it.next()
        .copied()
        .ok_or_else(|| Failure::Usage(format!("{flag} needs a value")))
}



pub(crate) fn home() -> Result<PathBuf, Failure> {
    Ok(supraman_core::home_from_env().ok_or(CoreError::NoHome)?)
}

/// `--all` shows stopped sessions and, without `--folder`, an "unplaced" list of those under no listed folder.
fn sessions(args: &[&str]) -> Result<(), Failure> {
    let mut folder = None;
    let mut all = false;
    let mut it = args.iter();
    while let Some(&arg) = it.next() {
        match arg {
            "--folder" => folder = Some(std::path::absolute(value(&mut it, "--folder")?).map_err(|e| Failure::Usage(e.to_string()))?),
            "--all" => all = true,
            other => return Err(Failure::Usage(format!("unexpected argument '{other}'"))),
        }
    }
    let entries = supraman_core::folders::load(&home()?)?;
    check_claude()?;
    let sessions = list_sessions()?;
    print!("{}", render(&entries, folder.as_deref(), &sessions, all, now_ms()));
    Ok(())
}

/// Each listed folder once, in order, with its sessions; `only` keeps that folder alone.
fn render(entries: &Entries, only: Option<&std::path::Path>, sessions: &[Session], show_stopped: bool, now: i64) -> String {
    let mut visible: Vec<Session> = sessions.iter().filter(|s| show_stopped || s.health(now) != Health::Stopped).cloned().collect();
    visible.sort_by(|a, b| b.started_at.cmp(&a.started_at).then_with(|| a.id.cmp(&b.id)));
    let none = if show_stopped { "no sessions" } else { "no live sessions" };
    let placed = place(&visible, entries, &supraman_core::folders::Shown::new());
    let mut out = String::new();
    if entries.list.is_empty() {
        out.push_str("no folders listed\n");
    }
    let mut seen = HashSet::new();
    for f in entries.folders().filter(|f| only.is_none_or(|o| path_key(o) == path_key(&f.path))) {
        let key = path_key(&f.path);
        if !seen.insert(key.clone()) {
            continue;
        }
        let held = placed.by_folder.get(&key).map(Vec::as_slice).unwrap_or_default();
        let _ = writeln!(out, "{}  ({} sessions)", f.path.display(), held.len());
        if held.is_empty() {
            let _ = writeln!(out, "  {none}");
        }
        for s in held {
            write_session(&mut out, s, now);
        }
    }
    if only.is_none() && show_stopped {
        let _ = writeln!(out, "unplaced  ({} sessions)", placed.unplaced.len());
        for s in &placed.unplaced {
            write_session(&mut out, s, now);
        }
    }
    out
}

fn write_session(out: &mut String, s: &Session, now: i64) {
    let name = if s.name.is_empty() { "-" } else { &s.name };
    let _ = writeln!(
        out,
        "    {:<8}  {:<28}  {:<11}  {}  {}",
        s.id,
        s.health(now).to_string(),
        s.kind.as_str(),
        name,
        s.cwd.display()
    );
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use supraman_claude::parse;
    use supraman_core::folders::{Entry, FolderEntry};

    use super::*;

    const NOW: i64 = 1_800_000_000_000;

    fn entries(paths: &[&str]) -> Entries {
        Entries::new(vec![Entry::Group { name: "demo".into(), folders: paths.iter().map(|p| FolderEntry::new(*p)).collect() }])
    }

    fn rows() -> Vec<Session> {
        let old = NOW - 600_000;
        parse(&format!(
            r#"[
  {{"id":"aaaa0001","sessionId":"aaaa0001-x","name":"fm-demo-a","cwd":"C:\\demo\\Alpha","kind":"background","startedAt":{old},"state":"working","status":"busy","pid":1}},
  {{"id":"aaaa0002","sessionId":"aaaa0002-x","name":"fm-demo-b","cwd":"C:\\demo\\Beta\\src","kind":"background","startedAt":{},"state":"blocked","status":"waiting","waitingFor":"permission prompt","pid":2}},
  {{"id":"aaaa0003","sessionId":"aaaa0003-x","name":"fm-demo-c","cwd":"C:\\demo\\Alpha","kind":"background","startedAt":{old},"state":"stopped"}},
  {{"sessionId":"bbbb0004-x","cwd":"C:\\other","kind":"interactive","startedAt":{old},"status":"idle","pid":4}}
]"#,
            old + 1
        ))
        .unwrap()
    }

    #[test]
    fn groups_by_folder_and_hides_stopped_by_default() {
        let text = render(&entries(&[r"C:\demo\Alpha", r"C:\demo\Beta", r"C:\demo\Gamma"]), None, &rows(), false, NOW);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], r"C:\demo\Alpha  (1 sessions)");
        assert!(lines[1].starts_with("    aaaa0001  running "), "{text}");
        assert!(lines[1].ends_with(r"background   fm-demo-a  C:\demo\Alpha"), "{text}");
        assert_eq!(lines[2], r"C:\demo\Beta  (1 sessions)");
        assert!(lines[3].contains("blocked (permission prompt)"), "{text}");
        assert_eq!(lines[4..], [r"C:\demo\Gamma  (0 sessions)", "  no live sessions"], "{text}");
    }

    #[test]
    fn all_shows_stopped_and_unplaced_and_folder_keeps_one() {
        let e = entries(&[r"C:\demo\Alpha", r"C:\demo\Beta"]);
        let text = render(&e, None, &rows(), true, NOW);
        assert!(text.contains("aaaa0003  stopped"), "{text}");
        assert!(text.contains("unplaced  (1 sessions)\n    bbbb0004  idle"), "{text}");
        assert!(text.contains(r"interactive  -  C:\other"), "{text}");
        let one = render(&e, Some(Path::new(r"c:/demo/beta")), &rows(), true, NOW);
        assert!(one.starts_with(r"C:\demo\Beta  (1 sessions)") && !one.contains("Alpha") && !one.contains("unplaced"), "{one}");
        assert_eq!(render(&Entries::default(), None, &[], false, NOW), "no folders listed\n");
    }
}
