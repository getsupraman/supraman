use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use supraman_claude::{ClaudeError, Health, Session, assign, check_supported, list_sessions, now_ms};
use supraman_core::{CoreError, DEFAULT_DEPTH, FolderKind, Registry, Workspace, discover, owners};

const USAGE: &str = "\
usage:
  sm workspace add <dir> [--name <slug>] [--depth N] [--dry-run] [--force]   (a parent folder, scanned)
  sm workspace new <name> [--home <dir>] [--folder <dir>]...   (a workspace of the folders given, or empty)
  sm workspace add-folder <name> <dir> [--enable]   (git or plain; --enable: it may have a supervised session)
  sm workspace remove-folder <name> <folder>       (stops its supervised session; stop, never rm)
  sm workspace enable <name> <folder>   |   sm workspace disable <name> <folder>
  sm workspace set-home <name> <dir>               (where the workspace and relay sessions run)
  sm workspace add-scan <name> <dir> [--depth N]   (adds the repos below <dir>, now and on every rescan)
  sm workspace remove-scan <name> <dir>            (the folders only it found go too)
  sm workspace list
  sm workspace show <name>                         (home, scans, folders with kind and owner)
  sm workspace remove <name>
  sm sessions [--workspace <name>] [--all]
  sm want <workspace> <repo> [--prompt-file <path>]   (start the repo's session fm-<workspace>-<repo>)
  sm unwant <workspace> <repo>         (stop it; stop, never rm)
  sm roster <workspace> [--json]       (the workspace's sessions: name, role, health, id, cwd)
  sm send <workspace> <to> <body>      (drop a message for the workspace's relay to send)
  sm send <workspace> <to> --file <path>  (into the session's pipe, else through the relay)
  sm tell <body>                       (inside a Supraman session: a message to the human)
  sm tell --file <path>
  sm messages <workspace>              (messages the relay received, newest last)
  sm inbox --dir <inbox>               (the relay's watcher: one JSON line per drop, forever)
  sm schedule add [<workspace>] --name <text> --every <when> (--command /<name> | --prompt-file <path> | --prompt <text>)
      [--id <id>] [--folder <folder>] [--script <path> [--script-arg <a>]... [--script-timeout 10m]]
      [--profile read|edit|auto] [--allow-tools <rule>]... [--deny-tools <rule>]... [--add-dir <dir>]...
      [--secret NAME]... [--model <model>] [--max-runtime 1h] [--max-tokens N] [--overlap skip|queue|restart]
      [--catch-up once|skip] [--wake] [--description <text>]
      <when>: 'weekly mon,fri 09:00' | 'daily 09:00' | 'interval 2h [from 08:00]'; a --script alone runs no session
  sm schedule edit [<workspace>] <id> <add's options>   (a list option replaces the list; --no-script, --no-wake)
  sm schedule list [<workspace>]  |  sm schedule show [<workspace>] <id>
  sm schedule pause|resume|remove [<workspace>] <id>
  sm schedule dry-run [<workspace>] <id>   (the prompt, the claude argv, the permissions, the next three fires)
  sm schedule run [<workspace>] <id>       (Run now: an active, approved schedule fires at once)
  sm schedule secret set [<workspace>] <id> <NAME>   (the value on stdin; the script gets it as $NAME)
  sm schedule report <run> done|needs-you <one line>   (inside a scheduled run: its outcome, to the run's owner)
  sm schedule fire --home <home> <workspace> <id> --fingerprint <hex>   (what the schedule's task runs, as smw.exe)
      <workspace>: default the one whose folders hold the current folder. An added or changed schedule is
      pending until Approve in the Supraman window; sm never approves and never registers a task.
  sm work overview <workspace>         (its Workman project: runs with rollups, who works, the inbox)
  sm work board <workspace> <run-ref>  (one run's items: status, criteria n/m, claim, verification)
  sm probe hooks [--root <repo>]       (scratch cwd under <repo>/target; default: this build's checkout)
  sm probe isolation [--root <repo>]   (<repo>: a main checkout; default: this build's main checkout)
  sm probe rm-transcript               (what claude rm leaves of a stopped --bg session; resume after it)
  sm probe session-id                  (does --bg --session-id <uuid> keep the uuid, and --bg --resume)
  sm release manifest <zip> --version <semver> --commit <sha> [--notes-url <url>]
      (writes supraman-update.json and SHA256SUMS beside the zip; the zip is Supraman-<semver>.zip)
  sm release sign <manifest>           (the secret key from UPDATE_SIGNING_KEY; writes <manifest>.sig)
  sm release verify <manifest> [--public-key <file>]   (the signature against update.pub, then the zip)
  sm release keygen --public-out <file>   (the secret key to stdout only, never to a console)
  sm update apply --from <release dir> --to <install dir> --log <file> [--wait-pid <pid>] [--wait-secs 60]
      [--restart <exe> [--restart-arg <a>]...]   (what Update runs from a copy of the new sm.exe)
  sm plugin install <plugin-dir>  (copies this sm.exe to <plugin-dir>/bin, where the hooks run it)
  sm hook <event>                 (run by plugins/supraman; reads the hook payload on stdin)";

mod edit;
pub mod fire;
mod hook;
mod probe;
mod probe_hooks;
mod probe_isolation;
mod probe_rm;
mod probe_session_id;
mod relay;
mod release;
mod repos;
pub mod run;
mod schedule;
mod update;
mod work;

enum Failure {
    Usage(String),
    Core(CoreError),
    Claude(ClaudeError),
    Supervisor(Box<supraman_supervisor::SupervisorError>),
    Probe(&'static str),
    /// Printed as is, exit 1.
    Refused(String),
    /// Already reported; the exit code alone.
    Exit(u8),
}

impl From<supraman_supervisor::SupervisorError> for Failure {
    fn from(e: supraman_supervisor::SupervisorError) -> Self {
        match e {
            supraman_supervisor::SupervisorError::Core(e) => Self::Core(e),
            e => Self::Supervisor(Box::new(e)),
        }
    }
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
        Failure::Supervisor(e) => e.to_string(),
        Failure::Probe(what) => what.to_string(),
        Failure::Refused(line) => line.trim_start_matches("error: ").to_string(),
        Failure::Exit(code) => format!("exit {code}"),
    }
}

/// `sm`'s command line, without the program name.
pub fn cli(args: Vec<String>) -> ExitCode {
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
        Err(Failure::Supervisor(e)) => {
            eprintln!("error: {e}");
            ExitCode::from(2)
        }
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
            println!("{USAGE}");
            Ok(())
        }
        ["-V" | "--version"] => {
            println!("sm {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        ["workspace", "add", rest @ ..] => add(rest),
        ["workspace", "new", rest @ ..] => edit::new(&registry()?, rest),
        ["workspace", "add-folder", rest @ ..] => edit::add_folder(&registry()?, rest),
        ["workspace", "remove-folder", rest @ ..] => edit::remove_folder(&registry()?, rest),
        ["workspace", verb @ ("enable" | "disable"), rest @ ..] => edit::set_enabled(&registry()?, *verb == "enable", rest),
        ["workspace", "set-home", rest @ ..] => edit::set_home(&registry()?, rest),
        ["workspace", "add-scan", rest @ ..] => edit::add_scan(&registry()?, rest),
        ["workspace", "remove-scan", rest @ ..] => edit::remove_scan(&registry()?, rest),
        ["workspace", "list"] => list(),
        ["workspace", "show", name] => show(name),
        ["workspace", "remove", name] => remove(name),
        ["sessions", rest @ ..] => sessions(rest),
        ["inbox", rest @ ..] => relay::inbox(rest),
        ["send", rest @ ..] => {
            relay::refuse_inside_foreman()?;
            relay::send_cmd(registry()?.home(), rest)
        }
        ["tell", rest @ ..] => relay::tell_cmd(rest),
        ["messages", rest @ ..] => relay::messages(&registry()?, rest),
        ["want", rest @ ..] => repos::want(&registry()?, rest),
        ["unwant", rest @ ..] => repos::unwant(&registry()?, rest),
        ["roster", rest @ ..] => repos::roster_cmd(&registry()?, rest),
        ["schedule", "fire", rest @ ..] => match fire::fire_cmd(rest, true) {
            0 => Ok(()),
            code => Err(Failure::Exit(code)),
        },
        ["schedule", rest @ ..] => schedule::schedule(&registry()?, rest),
        ["work", rest @ ..] => work::work(registry()?.home(), rest),
        ["probe", "hooks", rest @ ..] => {
            let root = probe_root(rest)?;
            check_claude()?;
            if probe_hooks::run(root) {
                Ok(())
            } else {
                Err(Failure::Probe("a hook signal was not observed"))
            }
        }
        ["probe", "isolation", rest @ ..] => {
            let root = probe_root(rest)?;
            check_claude()?;
            if probe_isolation::run(root) {
                Ok(())
            } else {
                Err(Failure::Probe("an isolation session did not finish its turn"))
            }
        }
        ["probe", "rm-transcript"] => {
            check_claude()?;
            if probe_rm::run() {
                Ok(())
            } else {
                Err(Failure::Probe("the rm-transcript probe did not finish"))
            }
        }
        ["probe", "session-id"] => {
            check_claude()?;
            if probe_session_id::run() {
                Ok(())
            } else {
                Err(Failure::Probe("the session-id probe did not finish"))
            }
        }
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

/// [`check_supported`], warning on stderr about a claude newer than tested; only one too old fails.
pub(crate) fn check_claude() -> Result<(), Failure> {
    if let Some(warning) = check_supported()?.untested() {
        eprintln!("sm: warning: {warning}");
    }
    Ok(())
}

fn probe_root(args: &[&str]) -> Result<Option<PathBuf>, Failure> {
    match args {
        [] => Ok(None),
        ["--root", dir] => std::path::absolute(dir)
            .map(Some)
            .map_err(|e| Failure::Usage(format!("--root {dir}: {e}"))),
        _ => Err(Failure::Usage(format!("unexpected arguments: {}", args.join(" ")))),
    }
}

struct AddArgs {
    dir: PathBuf,
    name: Option<String>,
    depth: usize,
    dry_run: bool,
    force: bool,
}

fn parse_add(args: &[&str]) -> Result<AddArgs, Failure> {
    let mut dir = None;
    let mut name = None;
    let mut depth = DEFAULT_DEPTH;
    let mut dry_run = false;
    let mut force = false;
    let mut it = args.iter();
    while let Some(&arg) = it.next() {
        match arg {
            "--name" => {
                name = Some(value(&mut it, "--name")?.to_string());
            }
            "--depth" => {
                depth = parse_depth(value(&mut it, "--depth")?)?;
            }
            "--dry-run" => dry_run = true,
            "--force" => force = true,
            flag if flag.starts_with("--") => {
                return Err(Failure::Usage(format!("unknown option {flag}")));
            }
            positional if dir.is_none() => dir = Some(PathBuf::from(positional)),
            extra => return Err(Failure::Usage(format!("unexpected argument '{extra}'"))),
        }
    }
    let dir = dir.ok_or_else(|| Failure::Usage("workspace add needs a <dir>".into()))?;
    Ok(AddArgs {
        dir,
        name,
        depth,
        dry_run,
        force,
    })
}

pub(crate) fn parse_depth(raw: &str) -> Result<usize, Failure> {
    raw.parse()
        .ok()
        .filter(|d| *d >= 1)
        .ok_or_else(|| Failure::Usage(format!("--depth must be a whole number >= 1, got '{raw}'")))
}

pub(crate) fn value<'a>(it: &mut std::slice::Iter<'_, &'a str>, flag: &str) -> Result<&'a str, Failure> {
    it.next()
        .copied()
        .ok_or_else(|| Failure::Usage(format!("{flag} needs a value")))
}

fn add(args: &[&str]) -> Result<(), Failure> {
    let args = parse_add(args)?;
    let mut workspace = discover(&args.dir, args.name.as_deref(), args.depth)?;

    let registry = registry()?;
    if !args.force {
        registry.check_new(&workspace)?;
    }
    for (folder, owner) in registry.claim_folders(&mut workspace, |_| true)? {
        println!("notice: {folder} is supervised in workspace {owner}; disabled here");
    }
    if args.dry_run {
        println!("dry run: would register workspace {} (nothing written)", workspace.name);
        print_workspace(&workspace, false, &with_owners(&registry, &workspace)?);
        return Ok(());
    }
    let path = registry.save(&workspace, args.force)?;
    println!("registered workspace {} at {}", workspace.name, path.display());
    print_workspace(&workspace, true, &with_owners(&registry, &workspace)?);
    Ok(())
}

pub(crate) fn registry() -> Result<Registry, Failure> {
    Ok(Registry::from_env()?)
}

fn list() -> Result<(), Failure> {
    let workspaces = registry()?.list()?;
    if workspaces.is_empty() {
        println!("no workspaces registered");
    }
    for p in workspaces {
        println!("{}\tfolders={}\thome={}", p.name, p.folders.len(), home_text(&p));
    }
    Ok(())
}

fn show(name: &str) -> Result<(), Failure> {
    let registry = registry()?;
    let workspace = registry.load(name)?;
    print_workspace(&workspace, true, &with_owners(&registry, &workspace)?);
    Ok(())
}

fn remove(name: &str) -> Result<(), Failure> {
    registry()?.remove(name)?;
    println!("removed workspace {name}");
    Ok(())
}

fn home_text(p: &Workspace) -> String {
    p.home().map(|h| h.display().to_string()).unwrap_or_else(|| "(none)".into())
}

/// [`owners`] of the registered workspaces with `workspace` as it is now in place of its file.
pub(crate) fn with_owners(registry: &Registry, workspace: &Workspace) -> Result<HashMap<String, String>, Failure> {
    let mut all: Vec<Workspace> = registry.list()?.into_iter().filter(|w| w.name != workspace.name).collect();
    let at = all.partition_point(|w| w.name < workspace.name);
    all.insert(at, workspace.clone());
    Ok(owners(&all))
}

/// `owner=` names the workspace that supervises the folder's session ([`owners`]), `-` for none.
pub(crate) fn print_workspace(p: &Workspace, with_created: bool, owned: &HashMap<String, String>) {
    println!("name: {}", p.name);
    println!("home: {}", home_text(p));
    if with_created {
        println!("created: {}", p.created_at.to_rfc3339());
    }
    println!("scans: {}", p.scans.len());
    for s in &p.scans {
        let skipped: Vec<String> = s.exclude.iter().map(|x| x.display().to_string()).collect();
        let skipped = if skipped.is_empty() { String::new() } else { format!("\tskips={}", skipped.join(";")) };
        println!("  {}\tdepth={}{skipped}", s.path.display(), s.depth);
    }
    println!("folders: {}", p.folders.len());
    let yes = |b: bool| if b { "yes" } else { "no" };
    for f in &p.folders {
        let branch = match (f.kind, f.branch.as_deref()) {
            (FolderKind::Plain, _) => "-",
            (FolderKind::Git, b) => b.unwrap_or("(detached)"),
        };
        let owner = owned.get(&supraman_core::paths::path_key(&f.path)).map(String::as_str).unwrap_or("-");
        println!(
            "  {}\tkind={}\tbranch={branch}\tclaude_md={}\tagents_md={}\tenabled={}\towner={owner}\tpath={}",
            f.name,
            if f.is_git() { "git" } else { "plain" },
            yes(f.has_claude_md),
            yes(f.has_agents_md),
            yes(f.enabled),
            f.path.display(),
        );
    }
}

/// Without `--all`, stopped sessions are hidden; with it, they are shown, and when no
/// `--workspace` is given an "unassigned" group lists sessions outside every workspace.
fn sessions(args: &[&str]) -> Result<(), Failure> {
    let mut workspace = None;
    let mut all = false;
    let mut it = args.iter();
    while let Some(&arg) = it.next() {
        match arg {
            "--workspace" => workspace = Some(value(&mut it, "--workspace")?),
            "--all" => all = true,
            other => return Err(Failure::Usage(format!("unexpected argument '{other}'"))),
        }
    }
    let registry = registry()?;
    let workspaces = match workspace {
        Some(name) => vec![registry.load(name)?],
        None => registry.list()?,
    };
    check_claude()?;
    let sessions = list_sessions()?;
    print!("{}", render(&workspaces, &sessions, all, workspace.is_none() && all, now_ms()));
    Ok(())
}

fn render(workspaces: &[Workspace], sessions: &[Session], show_stopped: bool, show_unassigned: bool, now: i64) -> String {
    let mut visible: Vec<&Session> = sessions
        .iter()
        .filter(|s| show_stopped || s.health(now) != Health::Stopped)
        .collect();
    visible.sort_by(|a, b| b.started_at.cmp(&a.started_at).then_with(|| a.id.cmp(&b.id)));
    let visible: Vec<Session> = visible.into_iter().cloned().collect();
    let none = if show_stopped { "no sessions" } else { "no live sessions" };

    let mut out = String::new();
    let mut placed = HashSet::new();
    if workspaces.is_empty() {
        out.push_str("no workspaces registered\n");
    }
    for p in workspaces {
        let assignment = assign(&visible, p);
        let count: usize = assignment.by_repo.values().map(Vec::len).sum::<usize>() + assignment.in_home.len();
        let _ = writeln!(out, "{}  {}  ({count} sessions)", p.name, home_text(p));
        if count == 0 {
            let _ = writeln!(out, "  {none}");
        }
        for (repo, group) in &assignment.by_repo {
            let _ = writeln!(out, "  {repo}");
            for s in group {
                placed.insert(s.session_id.clone());
                write_session(&mut out, s, now);
            }
        }
        if !assignment.in_home.is_empty() {
            let _ = writeln!(out, "  (home)");
            for s in &assignment.in_home {
                placed.insert(s.session_id.clone());
                write_session(&mut out, s, now);
            }
        }
    }
    if show_unassigned {
        let rest: Vec<&Session> = visible.iter().filter(|s| !placed.contains(&s.session_id)).collect();
        let _ = writeln!(out, "unassigned  ({} sessions)", rest.len());
        for s in rest {
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
    use supraman_claude::parse;
    use supraman_core::Folder;

    use super::*;

    const NOW: i64 = 1_800_000_000_000;

    fn workspace(name: &str, root: &str, repos: &[&str]) -> Workspace {
        let repos = repos
            .iter()
            .map(|r| Folder::new(*r, PathBuf::from(root).join(r)))
            .collect();
        Workspace::new(name, root, repos)
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
    fn groups_by_repo_and_hides_stopped_by_default() {
        let p = workspace("demo", r"C:\demo", &["Alpha", "Beta", "Gamma"]);
        let text = render(&[p], &rows(), false, false, NOW);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], r"demo  C:\demo  (2 sessions)");
        assert_eq!(lines[1], "  Alpha");
        assert!(lines[2].starts_with("    aaaa0001  running "), "{text}");
        assert!(lines[2].ends_with(r"background   fm-demo-a  C:\demo\Alpha"), "{text}");
        assert_eq!(lines[3], "  Beta");
        assert!(lines[4].contains("blocked (permission prompt)"), "{text}");
        assert_eq!(lines.len(), 5, "{text}");
    }

    #[test]
    fn all_shows_stopped_and_unassigned() {
        let p = workspace("demo", r"C:\demo", &["Alpha", "Beta"]);
        let text = render(&[p], &rows(), true, true, NOW);
        assert!(text.contains("aaaa0003  stopped"), "{text}");
        assert!(text.contains("unassigned  (1 sessions)\n    bbbb0004  idle"), "{text}");
        assert!(text.contains(r"interactive  -  C:\other"), "{text}");
    }

    #[test]
    fn empty_workspace_says_so() {
        let p = workspace("empty", r"C:\nothing", &["X"]);
        assert_eq!(render(&[p], &rows(), false, false, NOW), "empty  C:\\nothing  (0 sessions)\n  no live sessions\n");
        assert_eq!(render(&[], &[], false, false, NOW), "no workspaces registered\n");
    }
}
