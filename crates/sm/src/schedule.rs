//! `sm schedule …`: nothing here approves a schedule or registers a task; see C100.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, TimeZone};
use supraman_claude::{LaunchSpec, mcp_config_path};
use supraman_core::schedule::{
    self, Action, ActionKind, CatchUp, Clock, Day, Overlap, Profile, Prompt, Schedule, Span, Status, Trigger,
};
use supraman_core::{CoreError, find_plugin_dir, folders, sm_allowed_tools, paths, slugify};
use supraman_supervisor::{BgSession, LaunchConfig};

use crate::{Failure, value};

/// The `SUPRAMAN_ROLE` a run's session carries; [`supraman_supervisor::workman_role`] mints it an orchestrator.
pub(crate) const RUN_ROLE: &str = "repo";

pub fn schedule(home: &Path, args: &[&str]) -> Result<(), Failure> {
    migrate(home);
    let (folder, args) = folder_arg(args)?;
    let at = || folder_of(home, folder);
    match args.as_slice() {
        ["add", rest @ ..] => add(home, &at()?, rest),
        ["list", rest @ ..] => list(home, &at()?, rest),
        ["show", rest @ ..] => show(home, &at()?, rest),
        ["edit", rest @ ..] => edit(home, &at()?, rest),
        ["pause", rest @ ..] => set_status(home, &at()?, Status::Paused, rest),
        ["resume", rest @ ..] => set_status(home, &at()?, Status::Active, rest),
        ["remove", rest @ ..] => remove(home, &at()?, rest),
        ["dry-run", rest @ ..] => dry_run(home, &at()?, rest),
        ["run", rest @ ..] => run_now(home, &at()?, rest),
        ["secret", "set", rest @ ..] => secret_set(home, &at()?, rest, &mut std::io::stdin().lock()),
        ["report", rest @ ..] => report(home, rest),
        _ => Err(Failure::Usage("schedule needs add, list, show, edit, pause, resume, remove, dry-run, run, report or secret set".into())),
    }
}

/// The earlier per-workspace schedule files moved to their folders' once, what moved said on stderr.
pub(crate) fn migrate(home: &Path) {
    match schedule::migrate(home, &*supraman_core::secret::default_store(home)) {
        Ok(m) => {
            for (ws, id, folder) in &m.moved {
                eprintln!("moved schedule {id} of the former workspace {ws} to {}: pending until Approve in the Supraman window", folder.display());
            }
            for line in &m.unplaced {
                eprintln!("warning: {line}");
            }
        }
        Err(e) => eprintln!("warning: cannot move the earlier schedule files to their folders: {e}"),
    }
}

/// `--folder <dir>` taken out of `args`, wherever it stands.
fn folder_arg<'a>(args: &[&'a str]) -> Result<(Option<&'a str>, Vec<&'a str>), Failure> {
    let (mut folder, mut rest) = (None, Vec::new());
    let mut it = args.iter();
    while let Some(&arg) = it.next() {
        if arg == "--folder" {
            folder = Some(value(&mut it, arg)?);
        } else {
            rest.push(arg);
        }
    }
    Ok((folder, rest))
}

/// The deepest listed folder holding `--folder <dir>`, else the current folder (`assign::holding`).
fn folder_of(home: &Path, given: Option<&str>) -> Result<PathBuf, Failure> {
    let dir = match given {
        Some(d) => std::path::absolute(d).map_err(|e| Failure::Usage(format!("--folder {d}: {e}")))?,
        None => std::env::current_dir().map_err(|e| Failure::Usage(format!("cannot read the current folder: {e}")))?,
    };
    let entries = folders::load(home)?;
    supraman_claude::holding(&dir, &entries, supraman_claude::main_repo).map(|f| f.path.clone()).ok_or_else(|| {
        Failure::Refused(format!("error: no listed folder holds {}; give --folder <dir> of a listed folder, or list it with sm folder add <dir>", dir.display()))
    })
}

/// `<id>`.
fn one_id<'a>(positional: &[&'a str], verb: &str) -> Result<&'a str, Failure> {
    match positional {
        [id] => Ok(id),
        _ => Err(Failure::Usage(format!("schedule {verb} needs <id> [--folder <dir>]"))),
    }
}

fn find<'a>(schedules: &'a mut [Schedule], folder: &Path, id: &str) -> supraman_core::Result<&'a mut Schedule> {
    schedules
        .iter_mut()
        .find(|s| s.id == id)
        .ok_or_else(|| CoreError::NoSchedule { folder: folder.display().to_string(), id: id.to_string() })
}

fn load_one(home: &Path, folder: &Path, id: &str) -> Result<Schedule, Failure> {
    let mut all = schedule::load(home, folder)?;
    Ok(find(&mut all, folder, id)?.clone())
}

/// Every option of `add` and `edit`; `None` (or empty) leaves the field as it is.
#[derive(Default)]
struct Opts {
    id: Option<String>,
    name: Option<String>,
    description: Option<String>,
    every: Option<Trigger>,
    prompt: Option<PromptArg>,
    script: Option<String>,
    no_script: bool,
    script_args: Option<Vec<String>>,
    script_timeout: Option<Span>,
    profile: Option<Profile>,
    allow_tools: Option<Vec<String>>,
    deny_tools: Option<Vec<String>>,
    add_dirs: Option<Vec<String>>,
    secrets: Option<Vec<String>>,
    model: Option<String>,
    max_runtime: Option<Span>,
    max_tokens: Option<u64>,
    overlap: Option<Overlap>,
    catch_up: Option<CatchUp>,
    wake: Option<bool>,
}

enum PromptArg {
    Command(String),
    File(String),
    Text(String),
}

fn push(list: &mut Option<Vec<String>>, v: &str) {
    list.get_or_insert_with(Vec::new).push(v.to_string());
}

fn parsed<T: std::str::FromStr<Err = String>>(flag: &str, raw: &str) -> Result<T, Failure> {
    raw.parse().map_err(|e: String| Failure::Usage(format!("{flag}: {e}")))
}

fn parse_opts<'a>(args: &[&'a str]) -> Result<(Opts, Vec<&'a str>), Failure> {
    let mut o = Opts::default();
    let mut positional = Vec::new();
    let mut it = args.iter();
    while let Some(&arg) = it.next() {
        let mut v = |flag: &str| value(&mut it, flag);
        match arg {
            "--id" => o.id = Some(v(arg)?.to_string()),
            "--name" => o.name = Some(v(arg)?.to_string()),
            "--description" => o.description = Some(v(arg)?.to_string()),
            "--every" => o.every = Some(parse_every(v(arg)?)?),
            "--command" => {
                let c = v(arg)?;
                if let Some(name) = git_bash_rewrite(c, &supraman_core::platform::shell::git_roots(), supraman_core::platform::shell::in_git_bash()) {
                    return Err(Failure::Refused(format!("error: Git Bash rewrote /{name} into {c}; write //{name} (or set MSYS_NO_PATHCONV=1)")));
                }
                set_prompt(&mut o, PromptArg::Command(c.to_string()))?
            }
            "--prompt-file" => set_prompt(&mut o, PromptArg::File(v(arg)?.to_string()))?,
            "--prompt" => set_prompt(&mut o, PromptArg::Text(v(arg)?.to_string()))?,
            "--script" => o.script = Some(v(arg)?.to_string()),
            "--no-script" => o.no_script = true,
            "--script-arg" => push(&mut o.script_args, v(arg)?),
            "--script-timeout" => o.script_timeout = Some(parsed(arg, v(arg)?)?),
            "--profile" => o.profile = Some(parsed(arg, v(arg)?).map_err(|e| match e {
                Failure::Usage(m) => Failure::Refused(format!("error: {m}")),
                e => e,
            })?),
            "--allow-tools" => push(&mut o.allow_tools, v(arg)?),
            "--deny-tools" => push(&mut o.deny_tools, v(arg)?),
            "--add-dir" => push(&mut o.add_dirs, v(arg)?),
            "--secret" => push(&mut o.secrets, v(arg)?),
            "--model" => o.model = Some(v(arg)?.to_string()),
            "--max-runtime" => o.max_runtime = Some(parsed(arg, v(arg)?)?),
            "--max-tokens" => {
                let raw = v(arg)?;
                o.max_tokens = Some(raw.replace('_', "").parse().map_err(|_| Failure::Usage(format!("--max-tokens: '{raw}' is not a whole number")))?);
            }
            "--max-cost" => return Err(Failure::Usage("the cap is in tokens: use --max-tokens".into())),
            "--overlap" => {
                o.overlap = Some(match v(arg)? {
                    "skip" => Overlap::Skip,
                    "queue" => Overlap::Queue,
                    "restart" => Overlap::Restart,
                    other => return Err(Failure::Usage(format!("--overlap is skip, queue or restart, not '{other}'"))),
                })
            }
            "--catch-up" => {
                o.catch_up = Some(match v(arg)? {
                    "once" => CatchUp::Once,
                    "skip" => CatchUp::Skip,
                    other => return Err(Failure::Usage(format!("--catch-up is once or skip, not '{other}'"))),
                })
            }
            "--wake" => o.wake = Some(true),
            "--no-wake" => o.wake = Some(false),
            "--permission-mode" | "--dangerously-skip-permissions" => {
                return Err(Failure::Refused(format!("error: {arg} is not a schedule option; a schedule's mode comes from --profile read|edit|auto")));
            }
            flag if flag.starts_with("--") => return Err(Failure::Usage(format!("unknown option {flag}"))),
            p => positional.push(p),
        }
    }
    if o.script.is_some() && o.no_script {
        return Err(Failure::Usage("--script and --no-script exclude each other".into()));
    }
    Ok((o, positional))
}

/// The name in `/name` when `value` is what Git Bash makes of it: `<Git root>/name`.
fn git_bash_rewrite<'a>(value: &'a str, roots: &[PathBuf], msystem: bool) -> Option<&'a str> {
    let slashed = value.replace('\\', "/");
    let b = slashed.as_bytes();
    if b.len() < 4 || !b[0].is_ascii_alphabetic() || b[1] != b':' || b[2] != b'/' {
        return None;
    }
    let (parent, name) = slashed.rsplit_once('/')?;
    if name.is_empty() || name.contains('.') {
        return None;
    }
    let key = paths::path_key(Path::new(parent));
    let ours = roots.iter().any(|r| paths::path_key(r).trim_end_matches('/') == key.trim_end_matches('/'));
    let git_like = msystem && parent.rsplit('/').next().is_some_and(|last| last.eq_ignore_ascii_case("git"));
    (ours || git_like).then(|| &value[value.len() - name.len()..])
}

fn set_prompt(o: &mut Opts, p: PromptArg) -> Result<(), Failure> {
    if o.prompt.replace(p).is_some() {
        return Err(Failure::Usage("give one of --command, --prompt-file and --prompt".into()));
    }
    Ok(())
}

/// `weekly mon,fri 09:00`, `daily 09:00`, `interval 2h` or `interval 2h from 08:00`.
fn parse_every(raw: &str) -> Result<Trigger, Failure> {
    let bad = |why: String| Failure::Usage(format!("--every '{raw}': {why}; use 'weekly mon,fri 09:00', 'daily 09:00' or 'interval 2h [from 08:00]'"));
    let words: Vec<&str> = raw.split_whitespace().collect();
    match words.as_slice() {
        ["weekly", days, at] => {
            let days = days.split(',').map(|d| parse_day(d).ok_or_else(|| bad(format!("'{d}' is not a day mon..sun")))).collect::<Result<Vec<_>, _>>()?;
            Ok(Trigger::Weekly { days, at: at.parse().map_err(bad)? })
        }
        ["daily", at] => Ok(Trigger::Daily { at: at.parse().map_err(bad)? }),
        ["interval", every] => Ok(Trigger::Interval { at: Clock::default(), every: every.parse().map_err(bad)? }),
        ["interval", every, "from", at] => Ok(Trigger::Interval { at: at.parse().map_err(bad)?, every: every.parse().map_err(bad)? }),
        _ => Err(bad("not a trigger".into())),
    }
}

fn parse_day(d: &str) -> Option<Day> {
    Day::ALL.into_iter().find(|day| d.eq_ignore_ascii_case(day.as_str()))
}

/// `path` as the folder-relative path the TOML keeps; it must lie in the folder and exist.
fn in_folder(folder: &Path, path: &str, what: &str) -> Result<String, Failure> {
    let p = Path::new(path);
    let rel = if p.is_absolute() {
        p.strip_prefix(folder)
            .map_err(|_| Failure::Refused(format!("error: the {what} {path} is not inside folder {}", folder.display())))?
            .to_path_buf()
    } else {
        p.to_path_buf()
    };
    let shown = rel.to_string_lossy().replace('\\', "/");
    if !folder.join(&rel).is_file() {
        return Err(Failure::Refused(format!("error: no {what} {shown} in folder {}", folder.display())));
    }
    Ok(shown)
}

/// Applies `o` to `s`; files and the slash command are checked against `folder`.
fn apply(folder: &Path, s: &mut Schedule, o: Opts) -> Result<(), Failure> {
    if let Some(name) = o.name {
        s.name = name;
    }
    if let Some(d) = o.description {
        s.description = d;
    }
    if let Some(t) = o.every {
        s.trigger = t;
    }
    let a = &mut s.action;
    if let Some(p) = o.prompt {
        a.prompt = Some(match p {
            PromptArg::Command(c) => Prompt::Command(c),
            PromptArg::File(f) => Prompt::File(in_folder(folder, &f, "prompt file")?),
            PromptArg::Text(t) => Prompt::Text(t),
        });
    }
    if o.no_script {
        a.script = None;
        a.script_args.clear();
    }
    if let Some(script) = o.script {
        a.script = Some(in_folder(folder, &script, "script")?);
    }
    if let Some(args) = o.script_args {
        a.script_args = args;
    }
    if let Some(t) = o.script_timeout {
        a.script_timeout = t;
    }
    if let Some(m) = o.model {
        a.model = m;
    }
    a.kind = match (a.script.is_some(), a.prompt.is_some()) {
        (true, true) => ActionKind::ScriptThenSession,
        (true, false) => ActionKind::Script,
        (false, _) => ActionKind::Session,
    };
    let p = &mut s.permissions;
    if let Some(profile) = o.profile {
        p.profile = profile;
    }
    for (field, given) in [(&mut p.allow_tools, o.allow_tools), (&mut p.deny_tools, o.deny_tools), (&mut p.add_dirs, o.add_dirs), (&mut p.secrets, o.secrets)] {
        if let Some(given) = given {
            *field = given;
        }
    }
    if let Some(r) = o.max_runtime {
        s.caps.max_runtime = r;
    }
    if let Some(t) = o.max_tokens {
        s.caps.max_tokens = t;
    }
    if let Some(v) = o.overlap {
        s.policy.overlap = v;
    }
    if let Some(v) = o.catch_up {
        s.policy.catch_up = v;
    }
    if let Some(v) = o.wake {
        s.policy.wake = v;
    }
    if let Some(command @ Prompt::Command(_)) = &s.action.prompt {
        schedule::base_prompt(folder, command)?;
    }
    Ok(())
}

fn add(home: &Path, folder: &Path, args: &[&str]) -> Result<(), Failure> {
    let (mut o, positional) = parse_opts(args)?;
    if !positional.is_empty() {
        return Err(Failure::Usage("schedule add takes options only; the folder is --folder <dir> or the current one".into()));
    }
    let name = o.name.clone().ok_or_else(|| Failure::Usage("schedule add needs --name".into()))?;
    let trigger = o.every.take().ok_or_else(|| Failure::Usage("schedule add needs --every".into()))?;
    if o.prompt.is_none() && o.script.is_none() {
        return Err(Failure::Usage("schedule add needs --command, --prompt-file or --prompt, or a --script".into()));
    }
    let id = match o.id.take() {
        Some(id) => id,
        None => slugify(&name).chars().take(schedule::MAX_ID_LEN).collect::<String>().trim_end_matches('-').to_string(),
    };
    let mut s = Schedule {
        id,
        name: String::new(),
        description: String::new(),
        status: Status::Pending,
        created_by: schedule::CreatedBy::Fm,
        created_at: 0,
        trigger,
        action: Action {
            kind: ActionKind::Session,
            prompt: None,
            script: None,
            script_args: Vec::new(),
            script_timeout: Span(10 * 60),
            worktree: false,
            model: String::new(),
        },
        permissions: Default::default(),
        caps: Default::default(),
        policy: Default::default(),
        approval: None,
    };
    apply(folder, &mut s, o)?;
    let id = s.id.clone();
    let saved = schedule::edit(home, folder, |all| {
        if all.iter().any(|o| o.id == id) {
            return Err(CoreError::InvalidSchedule { id: id.clone(), why: format!("the id is taken in {}; give another --id", folder.display()) });
        }
        all.push(s);
        Ok(all.last().cloned().expect("just pushed"))
    })?;
    println!("added schedule {} to {}", saved.id, folder.display());
    print_schedule(home, folder, &saved);
    print_pending(&saved);
    Ok(())
}

fn edit(home: &Path, folder: &Path, args: &[&str]) -> Result<(), Failure> {
    let (o, positional) = parse_opts(args)?;
    if o.id.is_some() {
        return Err(Failure::Usage("an id cannot change; remove the schedule and add it again".into()));
    }
    let id = one_id(&positional, "edit")?;
    let saved = schedule::edit(home, folder, |all| {
        let s = find(all, folder, id)?;
        apply(folder, s, o).map_err(|e| match e {
            Failure::Core(e) => e,
            Failure::Usage(m) | Failure::Refused(m) => CoreError::InvalidSchedule { id: id.to_string(), why: m.trim_start_matches("error: ").to_string() },
            _ => CoreError::InvalidSchedule { id: id.to_string(), why: "cannot apply the options".into() },
        })?;
        Ok(s.clone())
    })?;
    println!("edited schedule {} in {}", saved.id, folder.display());
    print_schedule(home, folder, &saved);
    print_pending(&saved);
    Ok(())
}

fn set_status(home: &Path, folder: &Path, to: Status, args: &[&str]) -> Result<(), Failure> {
    let verb = if to == Status::Paused { "pause" } else { "resume" };
    let id = one_id(args, verb)?;
    let saved = schedule::edit(home, folder, |all| {
        let s = find(all, folder, id)?;
        s.status = to;
        Ok(s.clone())
    })?;
    let shown = schedule::effective_status(&*supraman_core::secret::default_store(home), folder, &saved);
    println!("schedule {} in {}: {}", saved.id, folder.display(), status_text(shown));
    if to == Status::Active && shown != Status::Active {
        print_pending(&saved);
    }
    Ok(())
}

fn remove(home: &Path, folder: &Path, args: &[&str]) -> Result<(), Failure> {
    let id = one_id(args, "remove")?;
    schedule::edit(home, folder, |all| {
        find(all, folder, id)?;
        all.retain(|s| s.id != id);
        Ok(())
    })?;
    println!("removed schedule {id} from {}; the Supraman window removes its task", folder.display());
    Ok(())
}

fn print_pending(s: &Schedule) {
    if s.status == Status::Pending {
        println!("pending: it runs only after Approve in the Supraman window (sm never approves)");
    }
}

fn status_text(s: Status) -> &'static str {
    match s {
        Status::Pending => "pending",
        Status::Active => "active",
        Status::Paused => "paused",
    }
}

fn list(home: &Path, folder: &Path, args: &[&str]) -> Result<(), Failure> {
    if !args.is_empty() {
        return Err(Failure::Usage("schedule list takes [--folder <dir>] only".into()));
    }
    let all = schedule::load(home, folder)?;
    if all.is_empty() {
        println!("no schedules in {}", folder.display());
    }
    let store = supraman_core::secret::default_store(home);
    for s in &all {
        let next = next_fires(&s.trigger, Local::now(), 1).first().map(local_text).unwrap_or_else(|| "-".into());
        println!("{}\t{}\tevery={}\tnext={next}\t{}", s.id, status_text(schedule::effective_status(&*store, folder, s)), s.trigger, s.name);
    }
    Ok(())
}

fn show(home: &Path, folder: &Path, args: &[&str]) -> Result<(), Failure> {
    let id = one_id(args, "show")?;
    print_schedule(home, folder, &load_one(home, folder, id)?);
    let runs: Vec<schedule::Run> = schedule::runs(home, &schedule::folder_key(folder))?.into_iter().filter(|r| r.schedule == id).collect();
    if runs.is_empty() {
        println!("last runs: none");
        return Ok(());
    }
    println!("last runs (newest first):");
    for r in runs.iter().rev().take(SHOWN_RUNS) {
        println!("  {}", run_text(r));
    }
    Ok(())
}

const SHOWN_RUNS: usize = 5;

/// `<run>  <started>  <fired>  <outcome>[: why]`; a run with no end is `running`.
fn run_text(r: &schedule::Run) -> String {
    let snake = |v: serde_json::Result<serde_json::Value>| v.ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_else(|| "?".into());
    let (started, fired) = match r.started {
        Some((ts, how)) => (
            Local.timestamp_millis_opt(ts).single().map(|t| local_text(&t)).unwrap_or_else(|| ts.to_string()),
            snake(serde_json::to_value(how)),
        ),
        None => ("-".into(), "-".into()),
    };
    let ended = match &r.ended {
        Some((_, outcome, why)) if why.is_empty() => snake(serde_json::to_value(outcome)),
        Some((_, outcome, why)) => format!("{}: {why}", snake(serde_json::to_value(outcome))),
        None => "running".into(),
    };
    format!("{}  {started}  {fired}  {ended}", r.run)
}

fn print_schedule(home: &Path, folder: &Path, s: &Schedule) {
    let store = supraman_core::secret::default_store(home);
    let shown = schedule::effective_status(&*store, folder, s);
    let changed = s.approval.is_some() && shown != Status::Active && s.status != Status::Paused;
    println!("id: {}", s.id);
    println!("folder: {}", folder.display());
    println!("name: {}", s.name);
    if !s.description.is_empty() {
        println!("description: {}", s.description);
    }
    println!("status: {}{}", status_text(shown), if changed { " (changed since approval)" } else { "" });
    println!("every: {}", s.trigger);
    let a = &s.action;
    println!("action: {}", match a.kind {
        ActionKind::Session => "session",
        ActionKind::ScriptThenSession => "script_then_session",
        ActionKind::Script => "script",
    });
    match &a.prompt {
        Some(Prompt::Command(c)) => println!("prompt: command {c}"),
        Some(Prompt::File(f)) => println!("prompt: file {f}"),
        Some(Prompt::Text(t)) => println!("prompt: text {t:?}"),
        None => {}
    }
    if let Some(script) = &a.script {
        println!("script: {script} {}\ttimeout={}", a.script_args.join(" "), a.script_timeout);
    }
    if a.worktree {
        println!("worktree: yes (saved by an earlier release; the next covered edit runs it in the folder's checkout)");
    }
    if !a.model.is_empty() {
        println!("model: {}", a.model);
    }
    let p = &s.permissions;
    println!("profile: {}", p.profile.as_str());
    println!("allow_tools: {}", joined(&p.allow_tools));
    println!("deny_tools: {}", joined(&p.deny_tools));
    if !p.add_dirs.is_empty() {
        println!("add_dirs: {}", p.add_dirs.join(" "));
    }
    println!("secrets: {}", joined(&p.secrets));
    println!("max_runtime: {}\tmax_tokens: {}", s.caps.max_runtime, s.caps.max_tokens);
    let policy = &s.policy;
    println!(
        "overlap: {}\tcatch_up: {}\twake: {}",
        match policy.overlap {
            Overlap::Skip => "skip",
            Overlap::Queue => "queue",
            Overlap::Restart => "restart",
        },
        match policy.catch_up {
            CatchUp::Once => "once",
            CatchUp::Skip => "skip",
        },
        if policy.wake { "yes" } else { "no" }
    );
}

fn joined(list: &[String]) -> String {
    if list.is_empty() { "-".into() } else { list.join(" ") }
}

fn next_fires(t: &Trigger, mut after: DateTime<Local>, n: usize) -> Vec<DateTime<Local>> {
    let mut out = Vec::new();
    while out.len() < n {
        let Some(next) = schedule::next_fire(t, &after) else { break };
        out.push(next);
        after = next;
    }
    out
}

fn local_text(t: &DateTime<Local>) -> String {
    t.format("%a %Y-%m-%d %H:%M %:z").to_string()
}

/// The `claude --bg` a run of `s` stamped `stamp` launches, before its Workman token.
pub(crate) fn run_spec(home: &Path, folder: &Path, s: &Schedule, stamp: &str, plugin_dir: Option<&Path>) -> Result<LaunchSpec, Failure> {
    let prompt = s.action.prompt.as_ref().ok_or_else(|| Failure::Refused(format!("error: schedule {} runs a script alone; it has no session", s.id)))?;
    let run = schedule::run_id(&s.id, stamp);
    let name = schedule::session_name(&schedule::folder_key(folder), &s.id, stamp);
    let run_branch = s.action.worktree.then(|| schedule::run_branch(&s.id, stamp));
    let cwd = if s.action.worktree { schedule::run_worktree(folder, &s.id, stamp) } else { folder.to_path_buf() };
    let preamble = schedule::preamble(&run, &sm_command(plugin_dir), &cwd, run_branch.as_deref());
    let session = BgSession {
        name: name.clone(),
        role: RUN_ROLE.into(),
        cwd,
        repo: Some(paths::folder_name(folder)),
        prompt: format!("{preamble}\n\n{}", schedule::base_prompt(folder, prompt)?),
        permission_mode: s.permissions.profile.permission_mode().into(),
        add_dirs: s.permissions.add_dirs.iter().map(|d| folder.join(d)).collect(),
    };
    let always = plugin_dir.map(sm_allowed_tools).unwrap_or_else(|| vec!["Bash(sm:*)".into()]);
    let config = LaunchConfig {
        mcp_dir: home.join("mcp"),
        plugin_dirs: plugin_dir.map(Path::to_path_buf).into_iter().collect(),
        supraman_home: Some(home.to_path_buf()),
        bg_isolation_none: true,
        allowed_tools: s.permissions.allowed(&always, run_branch.as_deref()),
        claude_dir: supraman_core::paths::claude_dir(),
    };
    let mut spec = config.launch_spec(&session, &[("SUPRAMAN_SCHEDULE", &s.id), ("SUPRAMAN_RUN", &run)]);
    spec.mcp_config = Some(mcp_config_path(&config.mcp_dir, &name).map_err(|e| Failure::Refused(format!("error: {e}")))?);
    spec.extra_args.push("--disallowedTools".into());
    spec.extra_args.extend(s.permissions.denied());
    if !s.action.model.is_empty() {
        spec.extra_args.extend(["--model".to_string(), s.action.model.clone()]);
    }
    Ok(spec)
}

/// The plugin's `sm.exe` by the path its `--allowedTools` rule names, else `sm` on the session's PATH.
fn sm_command(plugin_dir: Option<&Path>) -> String {
    plugin_dir
        .map(|d| d.join("bin").join(supraman_core::hook_exe()))
        .filter(|exe| exe.is_file())
        .map(|exe| paths::normalize(&exe))
        .unwrap_or_else(|| "sm".into())
}

/// What a profile leaves to `--allow-tools`, said where the permissions are shown.
fn tools_hint(profile: Profile) -> Option<&'static str> {
    match profile {
        Profile::Read => None,
        Profile::Edit => Some(
            "a fix that builds or tests needs those commands as --allow-tools (e.g. --allow-tools \"Bash(cargo test:*)\"); git status/diff/log/add/commit are allowed, git push is not",
        ),
        Profile::Auto => Some("the classifier decides; --allow-tools pre-approves what it would ask about"),
    }
}

fn dry_run(home: &Path, folder: &Path, args: &[&str]) -> Result<(), Failure> {
    let id = one_id(args, "dry-run")?;
    let s = load_one(home, folder, id)?;
    let fires = next_fires(&s.trigger, Local::now(), 3);
    let stamp = schedule::run_stamp(fires.first().unwrap_or(&Local::now()));
    let store = supraman_core::secret::default_store(home);
    println!("dry run of schedule {} in {} (nothing launched, nothing written)", s.id, folder.display());
    println!("status: {}", status_text(schedule::effective_status(&*store, folder, &s)));
    println!("run: {}", schedule::run_id(&s.id, &stamp));
    if let Some(script) = &s.action.script {
        println!("script: {script} {}\t(in the folder's main checkout, timeout {})", s.action.script_args.join(" "), s.action.script_timeout);
    }
    if s.action.kind == ActionKind::Script {
        println!("session: none (a script action)");
    } else {
        let plugin_dir = std::env::current_exe().ok().and_then(|exe| find_plugin_dir(&exe).ok());
        let spec = run_spec(home, folder, &s, &stamp, plugin_dir.as_deref())?;
        println!("prompt:");
        for line in spec.prompt.lines() {
            println!("  {line}");
        }
        if s.action.kind == ActionKind::ScriptThenSession {
            println!("  <gate-output> the script's stdout, when it exits 10 </gate-output>");
        }
        println!("cwd: {}", spec.cwd.display());
        println!("argv:");
        println!("  claude");
        for arg in spec.argv() {
            println!("  {}", quoted(&arg.to_string_lossy()));
        }
        println!("mcp config: written at fire with the run's minted token; absent when Supraman is not signed in");
        let always = plugin_dir.as_deref().map(sm_allowed_tools).unwrap_or_else(|| vec!["Bash(sm:*)".into()]);
        println!("permissions: profile {} (--permission-mode {})", s.permissions.profile.as_str(), s.permissions.profile.permission_mode());
        let allowed = s.permissions.allowed(&always, s.action.worktree.then(|| schedule::run_branch(&s.id, &stamp)).as_deref());
        println!("  allow: {}", allowed.join("  "));
        println!("  deny: {}", s.permissions.denied().join("  "));
        if let Some(needs) = tools_hint(s.permissions.profile) {
            println!("  {needs}");
        }
        for tool in uncovered_tools(&prompt_text(folder, s.action.prompt.as_ref()), &allowed) {
            println!("hint: the run calls {tool}; add --allow-tools \"Bash({tool}:*)\"");
        }
    }
    if !s.permissions.secrets.is_empty() {
        println!("secrets: {} (names only; values from the secret store at fire)", s.permissions.secrets.join(" "));
    }
    println!("next fires:");
    for f in &fires {
        println!("  {}", local_text(f));
    }
    Ok(())
}

/// Build and run tools a prompt commonly calls, which no profile allows by itself.
const HINTED_TOOLS: [&str; 6] = ["dotnet", "docker", "npm", "pnpm", "cargo", "python"];

/// The text a run's prompt is made from: the slash command's file, the prompt file or the text; empty where unreadable.
fn prompt_text(folder: &Path, prompt: Option<&Prompt>) -> String {
    match prompt {
        Some(Prompt::Command(name)) => std::fs::read_to_string(schedule::command_file(folder, name).1).unwrap_or_default(),
        Some(Prompt::File(rel)) => std::fs::read_to_string(folder.join(rel)).unwrap_or_default(),
        Some(Prompt::Text(text)) => text.clone(),
        None => String::new(),
    }
}

/// Each of [`HINTED_TOOLS`] that `text` names as a word and no rule in `allowed` covers.
fn uncovered_tools(text: &str, allowed: &[String]) -> Vec<&'static str> {
    let names = |tool: &str| {
        text.match_indices(tool).any(|(i, _)| {
            let before = text.as_bytes()[..i].last().is_none_or(|&b| !(b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.')));
            let after = text.as_bytes().get(i + tool.len()).is_none_or(|&b| !(b.is_ascii_alphabetic() || matches!(b, b'_' | b'-')));
            before && after
        })
    };
    let covered = |tool: &str| {
        allowed.iter().any(|rule| {
            rule == "Bash"
                || ["Bash(", "PowerShell("]
                    .iter()
                    .any(|shell| rule.strip_prefix(shell).and_then(|r| r.strip_prefix(tool)).is_some_and(|rest| rest.starts_with([':', ' ', ')'])))
        })
    };
    HINTED_TOOLS.into_iter().filter(|t| names(t) && !covered(t)).collect()
}

/// Run now: the task's path without its fingerprint (checks 1 to 3 still hold).
fn run_now(home: &Path, folder: &Path, args: &[&str]) -> Result<(), Failure> {
    let id = one_id(args, "run")?;
    let key = schedule::folder_key(folder);
    let req = crate::fire::Request { home, key: &key, id, trigger: schedule::Fired::RunNow, fingerprint: None };
    let report = crate::fire::fire(&req, &mut crate::fire::RealHost::new(home));
    crate::fire::print_report(&report);
    match report.exit {
        0 => Ok(()),
        code => Err(Failure::Exit(code)),
    }
}

/// `<run> done|needs-you "<one line>"` from the run's session, left for the sm that owns the run (scheduler.md, step 9).
fn report(home: &Path, args: &[&str]) -> Result<(), Failure> {
    let usage = || Failure::Usage("schedule report needs <run> done|needs-you \"<one line>\"".into());
    let (run, outcome, why) = match args {
        [run, outcome, why @ ..] => (*run, *outcome, why.join(" ")),
        _ => return Err(usage()),
    };
    let outcome = match outcome {
        "done" => schedule::Outcome::Done,
        "needs-you" | "needs_you" => schedule::Outcome::NeedsYou,
        _ => return Err(usage()),
    };
    let (all, _) = schedule::load_all(home);
    for scheduled in &all {
        let Ok(dir) = schedule::live_dir(home, &scheduled.key) else { continue };
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            let Some(live) = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice::<schedule::Live>(&b).ok()) else { continue };
            if live.run != run {
                continue;
            }
            let id = path.file_stem().and_then(|x| x.to_str()).unwrap_or_default();
            let body = schedule::Reported { run: run.to_string(), outcome, why: why.clone() };
            let file = crate::run::report_path(&dir, id);
            supraman_core::fs::write_atomic(&file, serde_json::to_vec(&body).expect("plain data serializes"))
                .map_err(|e| Failure::Refused(format!("error: cannot write {}: {e}", file.display())))?;
            println!("reported run {run}: {}; sm ends the run", if outcome == schedule::Outcome::Done { "done" } else { "needs you" });
            return Ok(());
        }
    }
    Err(Failure::Refused(format!("error: no run {run} is in flight")))
}

/// `<id> <NAME>`, the value on stdin (one line, its line end dropped), into the secret store.
fn secret_set(home: &Path, folder: &Path, args: &[&str], stdin: &mut dyn std::io::BufRead) -> Result<(), Failure> {
    let [id, name] = args else {
        return Err(Failure::Usage("schedule secret set needs <id> <NAME> [--folder <dir>], the value on stdin".into()));
    };
    let (id, name) = (*id, *name);
    let s = load_one(home, folder, id)?;
    if !s.permissions.secrets.iter().any(|n| n == name) {
        return Err(Failure::Refused(format!(
            "error: schedule {id} has no secret {name}; its secrets are {} (sm schedule edit … --secret {name})",
            joined(&s.permissions.secrets)
        )));
    }
    let mut value = String::new();
    stdin.read_line(&mut value).map_err(|e| Failure::Usage(format!("cannot read the value from stdin: {e}")))?;
    let value = value.trim_end_matches(['\r', '\n']);
    if value.is_empty() {
        return Err(Failure::Refused("error: the value on stdin is empty".into()));
    }
    let store = supraman_core::secret::default_store(home);
    store.set(&schedule::secret_key(&schedule::folder_key(folder), id, name), value).map_err(|e| Failure::Refused(format!("error: {e}")))?;
    println!("secret {name} of schedule {id} in {} is set; fire passes it to the script's environment", folder.display());
    Ok(())
}

fn quoted(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains(|c: char| c.is_whitespace() || c == '"') {
        return arg.to_string();
    }
    format!("{arg:?}")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{git_bash_rewrite, uncovered_tools};

    #[test]
    fn a_command_git_bash_turned_into_a_path_under_its_root_is_named() {
        let root = [PathBuf::from(r"D:\Fake Tools\Git")];
        assert_eq!(git_bash_rewrite("D:/Fake Tools/Git/replay-failures", &root, false), Some("replay-failures"));
        assert_eq!(git_bash_rewrite(r"d:\fake tools\git\fix", &root, false), Some("fix"));
        assert_eq!(git_bash_rewrite("C:/Program Files/Git/fix", &root, false), None, "not under a known root");
        assert_eq!(git_bash_rewrite("C:/Program Files/Git/fix", &root, true), Some("fix"), "MSYSTEM and a Git folder");
        assert_eq!(git_bash_rewrite("C:/Program Files/Other/fix", &root, true), None);
        assert_eq!(git_bash_rewrite("/replay-failures", &root, true), None);
        assert_eq!(git_bash_rewrite("D:/Fake Tools/Git/sub/fix", &root, false), None);
        assert_eq!(git_bash_rewrite("D:/Fake Tools/Git/fix.md", &root, false), None);
    }

    #[test]
    fn a_tool_is_hinted_when_named_as_a_word_and_no_rule_covers_it() {
        let text = "Run `docker compose up -d`, then dotnet run; pnpm i. python3 x.py";
        assert_eq!(uncovered_tools(text, &[]), ["dotnet", "docker", "pnpm", "python"]);
        let allowed = ["Bash(docker:*)".to_string(), "Bash(dotnet run:*)".into(), "PowerShell(pnpm:*)".into(), "Bash(dockerx:*)".into()];
        assert_eq!(uncovered_tools(text, &allowed), ["python"]);
        assert!(uncovered_tools(text, &["Bash".to_string()]).is_empty());
        assert!(uncovered_tools("docker-compose up; the npmrc; cargo_home; mydotnet", &[]).is_empty());
    }
}
