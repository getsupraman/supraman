//! `fm schedule …`: the schedules of a workspace through [`foreman_core::schedule`]. Nothing here approves a
//! schedule or registers a task (C100): an added or changed schedule stays pending until René's Approve.

use std::path::Path;

use chrono::{DateTime, Local};
use foreman_claude::{LaunchSpec, mcp_config_path};
use foreman_core::schedule::{
    self, Action, ActionKind, CatchUp, Clock, Day, Overlap, Profile, Prompt, Schedule, Span, Status, Trigger,
};
use foreman_core::{CoreError, Registry, Workspace, find_plugin_dir, fm_allowed_tools, paths, slugify};
use foreman_supervisor::LaunchConfig;

use crate::{Failure, value};

/// The `FOREMAN_ROLE` a run's session carries: it works in one folder, as a repo session does.
const RUN_ROLE: &str = "repo";

pub fn schedule(registry: &Registry, args: &[&str]) -> Result<(), Failure> {
    match args {
        ["add", rest @ ..] => add(registry, rest),
        ["list", rest @ ..] => list(registry, rest),
        ["show", rest @ ..] => show(registry, rest),
        ["edit", rest @ ..] => edit(registry, rest),
        ["pause", rest @ ..] => set_status(registry, Status::Paused, rest),
        ["resume", rest @ ..] => set_status(registry, Status::Active, rest),
        ["remove", rest @ ..] => remove(registry, rest),
        ["dry-run", rest @ ..] => dry_run(registry, rest),
        ["run", rest @ ..] => run_now(registry, rest),
        ["secret", "set", rest @ ..] => secret_set(registry, rest, &mut std::io::stdin().lock()),
        _ => Err(Failure::Usage("schedule needs add, list, show, edit, pause, resume, remove, dry-run, run or secret set".into())),
    }
}

/// The workspace named, else the one whose folders contain the cwd (`Workspace::contains`).
fn workspace(registry: &Registry, given: Option<&str>) -> Result<Workspace, Failure> {
    if let Some(name) = given {
        return Ok(registry.load(name)?);
    }
    let cwd = std::env::current_dir().map_err(|e| Failure::Usage(format!("cannot read the current folder: {e}")))?;
    let (all, _) = registry.load_all()?;
    let in_folder: Vec<&Workspace> = all.iter().filter(|w| w.folders.iter().any(|f| paths::within(&cwd, &f.path))).collect();
    let found: Vec<&Workspace> = if in_folder.is_empty() { all.iter().filter(|w| w.contains(&cwd)).collect() } else { in_folder };
    match found.as_slice() {
        [one] => Ok((*one).clone()),
        [] => Err(Failure::Refused(format!(
            "error: no workspace contains {}; name the workspace (fm schedule … <workspace> …)",
            cwd.display()
        ))),
        many => Err(Failure::Refused(format!(
            "error: {} lies in workspaces {}; name the one meant",
            cwd.display(),
            many.iter().map(|w| w.name.as_str()).collect::<Vec<_>>().join(", ")
        ))),
    }
}

/// `[<workspace>] <id>`.
fn target<'a>(registry: &Registry, positional: &[&'a str], verb: &str) -> Result<(Workspace, &'a str), Failure> {
    match positional {
        [id] => Ok((workspace(registry, None)?, id)),
        [w, id] => Ok((workspace(registry, Some(w))?, id)),
        _ => Err(Failure::Usage(format!("schedule {verb} needs [<workspace>] <id>"))),
    }
}

fn find<'a>(schedules: &'a mut [Schedule], workspace: &str, id: &str) -> foreman_core::Result<&'a mut Schedule> {
    schedules
        .iter_mut()
        .find(|s| s.id == id)
        .ok_or_else(|| CoreError::NoSchedule { workspace: workspace.to_string(), id: id.to_string() })
}

fn load_one(registry: &Registry, w: &Workspace, id: &str) -> Result<Schedule, Failure> {
    let mut all = schedule::load(registry.home(), &w.name)?;
    Ok(find(&mut all, &w.name, id)?.clone())
}

/// Every option of `add` and `edit`; `None` (or empty) leaves the field as it is.
#[derive(Default)]
struct Opts {
    id: Option<String>,
    name: Option<String>,
    description: Option<String>,
    folder: Option<String>,
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
            "--folder" => o.folder = Some(v(arg)?.to_string()),
            "--every" => o.every = Some(parse_every(v(arg)?)?),
            "--command" => set_prompt(&mut o, PromptArg::Command(v(arg)?.to_string()))?,
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
                return Err(Failure::Refused(format!("error: {arg} is not a schedule option; a schedule's mode comes from --profile read|worktree-edit|auto")));
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
    use Day::*;
    [Mon, Tue, Wed, Thu, Fri, Sat, Sun].into_iter().find(|day| d.eq_ignore_ascii_case(day_name(*day)))
}

fn day_name(d: Day) -> &'static str {
    ["mon", "tue", "wed", "thu", "fri", "sat", "sun"][d as usize]
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

/// Applies `o` to `s`; files and the slash command are checked against the schedule's folder.
fn apply(w: &Workspace, s: &mut Schedule, o: Opts) -> Result<(), Failure> {
    if let Some(name) = o.name {
        s.name = name;
    }
    if let Some(d) = o.description {
        s.description = d;
    }
    if let Some(f) = o.folder {
        s.folder = w.find_folder(&f).map(|f| f.name.clone()).ok_or_else(|| no_folder(w, &f))?;
    }
    let folder = w.folder(&s.folder).ok_or_else(|| no_folder(w, &s.folder))?.path.clone();
    if let Some(t) = o.every {
        s.trigger = t;
    }
    let a = &mut s.action;
    if let Some(p) = o.prompt {
        a.prompt = Some(match p {
            PromptArg::Command(c) => Prompt::Command(c),
            PromptArg::File(f) => Prompt::File(in_folder(&folder, &f, "prompt file")?),
            PromptArg::Text(t) => Prompt::Text(t),
        });
    }
    if o.no_script {
        a.script = None;
        a.script_args.clear();
    }
    if let Some(script) = o.script {
        a.script = Some(in_folder(&folder, &script, "script")?);
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
        a.worktree = profile != Profile::Read;
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
        schedule::base_prompt(&folder, command)?;
    }
    Ok(())
}

fn no_folder(w: &Workspace, folder: &str) -> Failure {
    CoreError::NoFolder {
        workspace: w.name.clone(),
        folder: folder.to_string(),
        known: w.folders.iter().map(|f| f.name.as_str()).collect::<Vec<_>>().join(", "),
    }
    .into()
}

fn add(registry: &Registry, args: &[&str]) -> Result<(), Failure> {
    let (mut o, positional) = parse_opts(args)?;
    let w = match positional.as_slice() {
        [] => workspace(registry, None)?,
        [name] => workspace(registry, Some(name))?,
        _ => return Err(Failure::Usage("schedule add takes one [<workspace>]".into())),
    };
    let name = o.name.clone().ok_or_else(|| Failure::Usage("schedule add needs --name".into()))?;
    let trigger = o.every.take().ok_or_else(|| Failure::Usage("schedule add needs --every".into()))?;
    if o.prompt.is_none() && o.script.is_none() {
        return Err(Failure::Usage("schedule add needs --command, --prompt-file or --prompt, or a --script".into()));
    }
    let id = match o.id.take() {
        Some(id) => id,
        None => slugify(&name).chars().take(schedule::MAX_ID_LEN).collect::<String>().trim_end_matches('-').to_string(),
    };
    let folder = match o.folder.take() {
        Some(f) => w.find_folder(&f).map(|f| f.name.clone()).ok_or_else(|| no_folder(&w, &f))?,
        None => default_folder(&w)?,
    };
    let mut s = Schedule {
        id,
        name: String::new(),
        description: String::new(),
        folder,
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
    apply(&w, &mut s, o)?;
    let id = s.id.clone();
    let saved = schedule::edit(registry.home(), &w, |all| {
        if all.iter().any(|o| o.id == id) {
            return Err(CoreError::InvalidSchedule { id: id.clone(), why: format!("the id is taken in workspace {}; give another --id", w.name) });
        }
        all.push(s);
        Ok(all.last().cloned().expect("just pushed"))
    })?;
    println!("added schedule {} to workspace {}", saved.id, w.name);
    print_schedule(registry, &w, &saved);
    print_pending(&saved);
    Ok(())
}

/// The folder that contains the cwd, else the workspace's only folder.
fn default_folder(w: &Workspace) -> Result<String, Failure> {
    let cwd = std::env::current_dir().ok();
    if let Some(f) = cwd.and_then(|cwd| w.folders.iter().filter(|f| paths::within(&cwd, &f.path)).max_by_key(|f| f.path.as_os_str().len())) {
        return Ok(f.name.clone());
    }
    match w.folders.as_slice() {
        [only] => Ok(only.name.clone()),
        _ => Err(Failure::Usage(format!(
            "schedule add needs --folder: workspace {} has folders {}",
            w.name,
            w.folders.iter().map(|f| f.name.as_str()).collect::<Vec<_>>().join(", ")
        ))),
    }
}

fn edit(registry: &Registry, args: &[&str]) -> Result<(), Failure> {
    let (o, positional) = parse_opts(args)?;
    if o.id.is_some() {
        return Err(Failure::Usage("an id cannot change; remove the schedule and add it again".into()));
    }
    let (w, id) = target(registry, &positional, "edit")?;
    let saved = schedule::edit(registry.home(), &w, |all| {
        let s = find(all, &w.name, id)?;
        apply(&w, s, o).map_err(|e| match e {
            Failure::Core(e) => e,
            Failure::Usage(m) | Failure::Refused(m) => CoreError::InvalidSchedule { id: id.to_string(), why: m.trim_start_matches("error: ").to_string() },
            _ => CoreError::InvalidSchedule { id: id.to_string(), why: "cannot apply the options".into() },
        })?;
        Ok(s.clone())
    })?;
    println!("edited schedule {} in workspace {}", saved.id, w.name);
    print_schedule(registry, &w, &saved);
    print_pending(&saved);
    Ok(())
}

fn set_status(registry: &Registry, to: Status, args: &[&str]) -> Result<(), Failure> {
    let verb = if to == Status::Paused { "pause" } else { "resume" };
    let (w, id) = target(registry, args, verb)?;
    let saved = schedule::edit(registry.home(), &w, |all| {
        let s = find(all, &w.name, id)?;
        s.status = to;
        Ok(s.clone())
    })?;
    let shown = schedule::effective_status(&*foreman_core::secret::default_store(registry.home()), &w, &saved);
    println!("schedule {} in workspace {}: {}", saved.id, w.name, status_text(shown));
    if to == Status::Active && shown != Status::Active {
        print_pending(&saved);
    }
    Ok(())
}

fn remove(registry: &Registry, args: &[&str]) -> Result<(), Failure> {
    let (w, id) = target(registry, args, "remove")?;
    schedule::edit(registry.home(), &w, |all| {
        find(all, &w.name, id)?;
        all.retain(|s| s.id != id);
        Ok(())
    })?;
    println!("removed schedule {id} from workspace {}; the Foreman window removes its task", w.name);
    Ok(())
}

fn print_pending(s: &Schedule) {
    if s.status == Status::Pending {
        println!("pending: it runs only after Approve in the Foreman window (fm never approves)");
    }
}

fn status_text(s: Status) -> &'static str {
    match s {
        Status::Pending => "pending",
        Status::Active => "active",
        Status::Paused => "paused",
    }
}

fn list(registry: &Registry, args: &[&str]) -> Result<(), Failure> {
    let w = match args {
        [] => workspace(registry, None)?,
        [name] => workspace(registry, Some(name))?,
        _ => return Err(Failure::Usage("schedule list takes one [<workspace>]".into())),
    };
    let all = schedule::load(registry.home(), &w.name)?;
    if all.is_empty() {
        println!("no schedules in workspace {}", w.name);
    }
    let store = foreman_core::secret::default_store(registry.home());
    for s in &all {
        let next = next_fires(&s.trigger, Local::now(), 1).first().map(local_text).unwrap_or_else(|| "-".into());
        println!(
            "{}\t{}\tfolder={}\tevery={}\tnext={next}\t{}",
            s.id,
            status_text(schedule::effective_status(&*store, &w, s)),
            s.folder,
            trigger_text(&s.trigger),
            s.name
        );
    }
    Ok(())
}

fn show(registry: &Registry, args: &[&str]) -> Result<(), Failure> {
    let (w, id) = target(registry, args, "show")?;
    print_schedule(registry, &w, &load_one(registry, &w, id)?);
    Ok(())
}

fn print_schedule(registry: &Registry, w: &Workspace, s: &Schedule) {
    let store = foreman_core::secret::default_store(registry.home());
    let shown = schedule::effective_status(&*store, w, s);
    let changed = s.approval.is_some() && shown != Status::Active && s.status != Status::Paused;
    println!("id: {}", s.id);
    println!("workspace: {}", w.name);
    println!("name: {}", s.name);
    if !s.description.is_empty() {
        println!("description: {}", s.description);
    }
    println!("status: {}{}", status_text(shown), if changed { " (changed since approval)" } else { "" });
    println!("folder: {}", s.folder);
    println!("every: {}", trigger_text(&s.trigger));
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
    println!("worktree: {}", if a.worktree { "yes" } else { "no" });
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

fn trigger_text(t: &Trigger) -> String {
    match t {
        Trigger::Weekly { days, at } => format!("weekly {} {at}", days.iter().map(|d| day_name(*d)).collect::<Vec<_>>().join(",")),
        Trigger::Daily { at } => format!("daily {at}"),
        Trigger::Interval { at, every } => format!("interval {every} from {at}"),
    }
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

/// The `claude --bg` a run of `s` stamped `stamp` launches; `plugin_dir` is Foreman's plugin, when found.
pub(crate) fn run_spec(home: &Path, w: &Workspace, s: &Schedule, stamp: &str, plugin_dir: Option<&Path>) -> Result<LaunchSpec, Failure> {
    let folder = w.folder(&s.folder).ok_or_else(|| no_folder(w, &s.folder))?;
    let prompt = s.action.prompt.as_ref().ok_or_else(|| Failure::Refused(format!("error: schedule {} runs a script alone; it has no session", s.id)))?;
    let run = schedule::run_id(&s.id, stamp);
    let name = schedule::session_name(&w.name, &s.id, stamp);
    let cwd = if s.action.worktree { schedule::run_worktree(&folder.path, &s.id, stamp) } else { folder.path.clone() };
    let mut spec = LaunchSpec::new(&name, cwd, format!("{}\n\n{}", schedule::preamble(&run), schedule::base_prompt(&folder.path, prompt)?));
    spec.permission_mode = s.permissions.profile.permission_mode().into();
    spec.mcp_config = Some(mcp_config_path(&home.join("mcp"), &name).map_err(|e| Failure::Refused(format!("error: {e}")))?);
    spec.strict_mcp_config = true;
    spec.plugin_dirs = plugin_dir.map(Path::to_path_buf).into_iter().collect();
    spec.add_dirs = s.permissions.add_dirs.iter().map(|d| folder.path.join(d)).collect();
    let config = LaunchConfig { foreman_home: Some(home.to_path_buf()), bg_isolation_none: true, ..LaunchConfig::default() };
    if let Some(settings) = config.session_settings(&w.name, &name, RUN_ROLE, Some(&folder.name)) {
        spec.extra_args.extend(["--settings".to_string(), settings]);
    }
    let always = plugin_dir.map(fm_allowed_tools).unwrap_or_else(|| vec!["Bash(fm:*)".into()]);
    spec.extra_args.push("--allowedTools".into());
    spec.extra_args.extend(s.permissions.allowed(&always));
    spec.extra_args.push("--disallowedTools".into());
    spec.extra_args.extend(s.permissions.denied());
    if !s.action.model.is_empty() {
        spec.extra_args.extend(["--model".to_string(), s.action.model.clone()]);
    }
    Ok(spec)
}

fn dry_run(registry: &Registry, args: &[&str]) -> Result<(), Failure> {
    let (w, id) = target(registry, args, "dry-run")?;
    let s = load_one(registry, &w, id)?;
    let fires = next_fires(&s.trigger, Local::now(), 3);
    let stamp = schedule::run_stamp(fires.first().unwrap_or(&Local::now()));
    let store = foreman_core::secret::default_store(registry.home());
    println!("dry run of schedule {} in workspace {} (nothing launched, nothing written)", s.id, w.name);
    println!("status: {}", status_text(schedule::effective_status(&*store, &w, &s)));
    println!("run: {}", schedule::run_id(&s.id, &stamp));
    if let Some(script) = &s.action.script {
        println!("script: {script} {}\t(in the folder's main checkout, timeout {})", s.action.script_args.join(" "), s.action.script_timeout);
    }
    if s.action.kind == ActionKind::Script {
        println!("session: none (a script action)");
    } else {
        let plugin_dir = std::env::current_exe().ok().and_then(|exe| find_plugin_dir(&exe).ok());
        let spec = run_spec(registry.home(), &w, &s, &stamp, plugin_dir.as_deref())?;
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
        println!("mcp config: written at fire with the run's minted token; absent when Foreman is not signed in");
        let always = plugin_dir.as_deref().map(fm_allowed_tools).unwrap_or_else(|| vec!["Bash(fm:*)".into()]);
        println!("permissions: profile {} (--permission-mode {})", s.permissions.profile.as_str(), s.permissions.profile.permission_mode());
        println!("  allow: {}", s.permissions.allowed(&always).join("  "));
        println!("  deny: {}", s.permissions.denied().join("  "));
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

/// Run now: the task's path without its fingerprint (checks 1 to 3 still hold).
fn run_now(registry: &Registry, args: &[&str]) -> Result<(), Failure> {
    let (w, id) = target(registry, args, "run")?;
    let req = crate::fire::Request { home: registry.home(), workspace: &w.name, id, trigger: schedule::Fired::RunNow, fingerprint: None };
    let report = crate::fire::fire(&req, &mut crate::fire::RealHost::new(registry.home()));
    crate::fire::print_report(&report);
    match report.exit {
        0 => Ok(()),
        code => Err(Failure::Exit(code)),
    }
}

/// `[<workspace>] <id> <NAME>`, the value on stdin (one line, its line end dropped), into the secret store.
fn secret_set(registry: &Registry, args: &[&str], stdin: &mut dyn std::io::BufRead) -> Result<(), Failure> {
    let (positional, name) = match args.split_last() {
        Some((name, rest)) if !rest.is_empty() => (rest, *name),
        _ => return Err(Failure::Usage("schedule secret set needs [<workspace>] <id> <NAME>, the value on stdin".into())),
    };
    let (w, id) = target(registry, positional, "secret set")?;
    let s = load_one(registry, &w, id)?;
    if !s.permissions.secrets.iter().any(|n| n == name) {
        return Err(Failure::Refused(format!(
            "error: schedule {id} has no secret {name}; its secrets are {} (fm schedule edit … --secret {name})",
            joined(&s.permissions.secrets)
        )));
    }
    let mut value = String::new();
    stdin.read_line(&mut value).map_err(|e| Failure::Usage(format!("cannot read the value from stdin: {e}")))?;
    let value = value.trim_end_matches(['\r', '\n']);
    if value.is_empty() {
        return Err(Failure::Refused("error: the value on stdin is empty".into()));
    }
    let store = foreman_core::secret::default_store(registry.home());
    store.set(&schedule::secret_key(&w.name, id, name), value).map_err(|e| Failure::Refused(format!("error: {e}")))?;
    println!("secret {name} of schedule {id} in workspace {} is set; fire passes it to the script's environment", w.name);
    Ok(())
}

fn quoted(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains(|c: char| c.is_whitespace() || c == '"') {
        return arg.to_string();
    }
    format!("{arg:?}")
}
