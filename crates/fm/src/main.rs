use std::collections::HashSet;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use foreman_claude::{ClaudeError, Health, Session, assign, check_supported, list_sessions, now_ms};
use foreman_core::{CoreError, DEFAULT_DEPTH, Registry, Workspace, migrate_home, scan, slugify, validate_name};

const USAGE: &str = "\
usage:
  fm workspace add <dir> [--name <slug>] [--depth N] [--dry-run] [--force]
  fm workspace list
  fm workspace show <name>
  fm workspace remove <name>
  fm sessions [--workspace <name>] [--all]
  fm want <workspace> <repo> [--prompt-file <path>]   (start the repo's session fm-<workspace>-<repo>)
  fm unwant <workspace> <repo>         (stop it; stop, never rm)
  fm roster <workspace> [--json]       (the workspace's sessions: name, role, health, id, cwd)
  fm send <workspace> <to> <body>      (drop a message for the workspace's relay to send)
  fm send <workspace> <to> --file <path>
  fm messages <workspace>              (messages the relay received, newest last)
  fm inbox --dir <inbox>               (the relay's watcher: one JSON line per drop, forever)
  fm work overview <workspace>         (its WorkState project: runs with rollups, who works, the inbox)
  fm work board <workspace> <run-ref>  (one run's items: status, criteria n/m, claim, verification)
  fm probe launch
  fm probe hooks [--root <repo>]       (scratch cwd under <repo>/target; default: this build's checkout)
  fm probe isolation [--root <repo>]   (<repo>: a main checkout; default: this build's main checkout)
  fm plugin install <plugin-dir>  (copies this fm.exe to <plugin-dir>/bin, where the hooks run it)
  fm hook <event>                 (run by plugins/foreman; reads the hook payload on stdin)";

mod hook;
mod probe;
mod probe_hooks;
mod probe_isolation;
mod relay;
mod repos;
mod work;

enum Failure {
    Usage(String),
    Core(CoreError),
    Claude(ClaudeError),
    Supervisor(Box<foreman_supervisor::SupervisorError>),
    Probe(&'static str),
    /// Printed as is, exit 1.
    Refused(String),
}

impl From<foreman_supervisor::SupervisorError> for Failure {
    fn from(e: foreman_supervisor::SupervisorError) -> Self {
        match e {
            foreman_supervisor::SupervisorError::Core(e) => Self::Core(e),
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

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
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
            println!("fm {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        ["workspace", "add", rest @ ..] => add(rest),
        ["workspace", "list"] => list(),
        ["workspace", "show", name] => show(name),
        ["workspace", "remove", name] => remove(name),
        ["sessions", rest @ ..] => sessions(rest),
        ["inbox", rest @ ..] => relay::inbox(rest),
        ["send", rest @ ..] => {
            relay::refuse_inside_foreman()?;
            relay::send_cmd(registry()?.home(), rest)
        }
        ["messages", rest @ ..] => relay::messages(&registry()?, rest),
        ["want", rest @ ..] => repos::want(&registry()?, rest),
        ["unwant", rest @ ..] => repos::unwant(&registry()?, rest),
        ["roster", rest @ ..] => repos::roster_cmd(&registry()?, rest),
        ["work", rest @ ..] => work::work(registry()?.home(), rest),
        ["probe", "launch"] => {
            check_supported()?;
            if probe::launch() {
                Ok(())
            } else {
                Err(Failure::Probe("the decided token route did not work"))
            }
        }
        ["probe", "hooks", rest @ ..] => {
            let root = probe_root(rest)?;
            check_supported()?;
            if probe_hooks::run(root) {
                Ok(())
            } else {
                Err(Failure::Probe("a hook signal was not observed"))
            }
        }
        ["probe", "isolation", rest @ ..] => {
            let root = probe_root(rest)?;
            check_supported()?;
            if probe_isolation::run(root) {
                Ok(())
            } else {
                Err(Failure::Probe("an isolation session did not finish its turn"))
            }
        }
        ["plugin", "install", dir] => {
            let exe = std::env::current_exe().map_err(|e| Failure::Usage(format!("cannot locate fm.exe: {e}")))?;
            match foreman_core::install_hook_exe(std::path::Path::new(dir), &exe) {
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
                let raw = value(&mut it, "--depth")?;
                depth = raw
                    .parse()
                    .ok()
                    .filter(|d| *d >= 1)
                    .ok_or_else(|| Failure::Usage(format!("--depth must be a whole number >= 1, got '{raw}'")))?;
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

fn value<'a>(it: &mut std::slice::Iter<'_, &'a str>, flag: &str) -> Result<&'a str, Failure> {
    it.next()
        .copied()
        .ok_or_else(|| Failure::Usage(format!("{flag} needs a value")))
}

fn add(args: &[&str]) -> Result<(), Failure> {
    let args = parse_add(args)?;
    let root = std::path::absolute(&args.dir).map_err(|_| CoreError::NotADirectory(args.dir.clone()))?;
    let name = match args.name {
        Some(name) => name,
        None => {
            let folder = root.file_name().map(|f| f.to_string_lossy()).unwrap_or_default();
            slugify(&folder)
        }
    };
    validate_name(&name)?;
    let repos = scan(&root, args.depth)?;
    let workspace = Workspace::new(name, root, repos);

    let registry = registry()?;
    if !args.force && registry.exists(&workspace.name)? {
        return Err(CoreError::AlreadyExists(workspace.name).into());
    }
    if args.dry_run {
        println!("dry run: would register workspace {} (nothing written)", workspace.name);
        print_workspace(&workspace, false);
        return Ok(());
    }
    let path = registry.save(&workspace, args.force)?;
    println!("registered workspace {} at {}", workspace.name, path.display());
    print_workspace(&workspace, true);
    Ok(())
}

/// The registry of `FOREMAN_HOME`, after [`migrate_home`]; its notices go to stderr.
fn registry() -> Result<Registry, Failure> {
    let registry = Registry::from_env()?;
    for notice in migrate_home(registry.home())? {
        eprintln!("notice: {notice}");
    }
    Ok(registry)
}

fn list() -> Result<(), Failure> {
    let workspaces = registry()?.list()?;
    if workspaces.is_empty() {
        println!("no workspaces registered");
    }
    for p in workspaces {
        println!("{}\trepos={}\troot={}", p.name, p.repos.len(), p.root.display());
    }
    Ok(())
}

fn show(name: &str) -> Result<(), Failure> {
    let workspace = registry()?.load(name)?;
    print_workspace(&workspace, true);
    Ok(())
}

fn remove(name: &str) -> Result<(), Failure> {
    registry()?.remove(name)?;
    println!("removed workspace {name}");
    Ok(())
}

fn print_workspace(p: &Workspace, with_created: bool) {
    println!("name: {}", p.name);
    println!("root: {}", p.root.display());
    if with_created {
        println!("created: {}", p.created_at.to_rfc3339());
    }
    println!("repos: {}", p.repos.len());
    let yes = |b: bool| if b { "yes" } else { "no" };
    for r in &p.repos {
        println!(
            "  {}\tbranch={}\tclaude_md={}\tagents_md={}\tenabled={}",
            r.name,
            r.branch.as_deref().unwrap_or("(detached)"),
            yes(r.has_claude_md),
            yes(r.has_agents_md),
            yes(r.enabled),
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
    check_supported()?;
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
        let count: usize = assignment.by_repo.values().map(Vec::len).sum();
        let _ = writeln!(out, "{}  {}  ({count} sessions)", p.name, p.root.display());
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
    use foreman_claude::parse;
    use foreman_core::Repo;

    use super::*;

    const NOW: i64 = 1_800_000_000_000;

    fn workspace(name: &str, root: &str, repos: &[&str]) -> Workspace {
        let repos = repos
            .iter()
            .map(|r| Repo {
                name: (*r).into(),
                path: PathBuf::from(root).join(r),
                has_claude_md: false,
                has_agents_md: false,
                branch: None,
                enabled: true,
            })
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
