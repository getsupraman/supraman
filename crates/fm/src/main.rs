use std::collections::HashSet;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use foreman_claude::{ClaudeError, Health, Session, assign, check_supported, list_sessions, now_ms};
use foreman_core::{CoreError, DEFAULT_DEPTH, Platform, Registry, scan, slugify, validate_name};

const USAGE: &str = "\
usage:
  fm platform add <dir> [--name <slug>] [--depth N] [--dry-run] [--force]
  fm platform list
  fm platform show <name>
  fm platform remove <name>
  fm sessions [--platform <name>] [--all]
  fm probe launch
  fm probe hooks
  fm plugin install <plugin-dir>  (copies this fm.exe to <plugin-dir>/bin, where the hooks run it)
  fm hook <event>                 (run by plugins/foreman; reads the hook payload on stdin)";

mod hook;
mod plugin;
mod probe;
mod probe_hooks;

enum Failure {
    Usage(String),
    Core(CoreError),
    Claude(ClaudeError),
    Probe(&'static str),
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
        Err(Failure::Probe(what)) => {
            eprintln!("error: {what} (see the probe lines above)");
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
        ["platform", "add", rest @ ..] => add(rest),
        ["platform", "list"] => list(),
        ["platform", "show", name] => show(name),
        ["platform", "remove", name] => remove(name),
        ["sessions", rest @ ..] => sessions(rest),
        ["probe", "launch"] => {
            check_supported()?;
            if probe::launch() {
                Ok(())
            } else {
                Err(Failure::Probe("the decided token route did not work"))
            }
        }
        ["probe", "hooks"] => {
            check_supported()?;
            if probe_hooks::run() {
                Ok(())
            } else {
                Err(Failure::Probe("a hook signal was not observed"))
            }
        }
        ["plugin", "install", dir] => {
            let exe = std::env::current_exe().map_err(|e| Failure::Usage(format!("cannot locate fm.exe: {e}")))?;
            match plugin::install(std::path::Path::new(dir), &exe) {
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
    let dir = dir.ok_or_else(|| Failure::Usage("platform add needs a <dir>".into()))?;
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
    let platform = Platform::new(name, root, repos);

    let registry = Registry::from_env()?;
    if !args.force && registry.exists(&platform.name)? {
        return Err(CoreError::AlreadyExists(platform.name).into());
    }
    if args.dry_run {
        println!("dry run: would register platform {} (nothing written)", platform.name);
        print_platform(&platform, false);
        return Ok(());
    }
    let path = registry.save(&platform, args.force)?;
    println!("registered platform {} at {}", platform.name, path.display());
    print_platform(&platform, true);
    Ok(())
}

fn list() -> Result<(), Failure> {
    let platforms = Registry::from_env()?.list()?;
    if platforms.is_empty() {
        println!("no platforms registered");
    }
    for p in platforms {
        println!("{}\trepos={}\troot={}", p.name, p.repos.len(), p.root.display());
    }
    Ok(())
}

fn show(name: &str) -> Result<(), Failure> {
    let platform = Registry::from_env()?.load(name)?;
    print_platform(&platform, true);
    Ok(())
}

fn remove(name: &str) -> Result<(), Failure> {
    Registry::from_env()?.remove(name)?;
    println!("removed platform {name}");
    Ok(())
}

fn print_platform(p: &Platform, with_created: bool) {
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
/// `--platform` is given an "unassigned" group lists sessions outside every platform.
fn sessions(args: &[&str]) -> Result<(), Failure> {
    let mut platform = None;
    let mut all = false;
    let mut it = args.iter();
    while let Some(&arg) = it.next() {
        match arg {
            "--platform" => platform = Some(value(&mut it, "--platform")?),
            "--all" => all = true,
            other => return Err(Failure::Usage(format!("unexpected argument '{other}'"))),
        }
    }
    let registry = Registry::from_env()?;
    let platforms = match platform {
        Some(name) => vec![registry.load(name)?],
        None => registry.list()?,
    };
    check_supported()?;
    let sessions = list_sessions()?;
    print!("{}", render(&platforms, &sessions, all, platform.is_none() && all, now_ms()));
    Ok(())
}

fn render(platforms: &[Platform], sessions: &[Session], show_stopped: bool, show_unassigned: bool, now: i64) -> String {
    let mut visible: Vec<&Session> = sessions
        .iter()
        .filter(|s| show_stopped || s.health(now) != Health::Stopped)
        .collect();
    visible.sort_by(|a, b| b.started_at.cmp(&a.started_at).then_with(|| a.id.cmp(&b.id)));
    let visible: Vec<Session> = visible.into_iter().cloned().collect();
    let none = if show_stopped { "no sessions" } else { "no live sessions" };

    let mut out = String::new();
    let mut placed = HashSet::new();
    if platforms.is_empty() {
        out.push_str("no platforms registered\n");
    }
    for p in platforms {
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

    fn platform(name: &str, root: &str, repos: &[&str]) -> Platform {
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
        Platform::new(name, root, repos)
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
        let p = platform("demo", r"C:\demo", &["Alpha", "Beta", "Gamma"]);
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
        let p = platform("demo", r"C:\demo", &["Alpha", "Beta"]);
        let text = render(&[p], &rows(), true, true, NOW);
        assert!(text.contains("aaaa0003  stopped"), "{text}");
        assert!(text.contains("unassigned  (1 sessions)\n    bbbb0004  idle"), "{text}");
        assert!(text.contains(r"interactive  -  C:\other"), "{text}");
    }

    #[test]
    fn empty_platform_says_so() {
        let p = platform("empty", r"C:\nothing", &["X"]);
        assert_eq!(render(&[p], &rows(), false, false, NOW), "empty  C:\\nothing  (0 sessions)\n  no live sessions\n");
        assert_eq!(render(&[], &[], false, false, NOW), "no platforms registered\n");
    }
}
