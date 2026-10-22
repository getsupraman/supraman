use std::path::PathBuf;
use std::process::ExitCode;

use foreman_core::{CoreError, DEFAULT_DEPTH, Platform, Registry, scan, slugify, validate_name};

const USAGE: &str = "\
usage:
  fm platform add <dir> [--name <slug>] [--depth N] [--dry-run] [--force]
  fm platform list
  fm platform show <name>
  fm platform remove <name>";

enum Failure {
    Usage(String),
    Core(CoreError),
}

impl From<CoreError> for Failure {
    fn from(e: CoreError) -> Self {
        Self::Core(e)
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
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
