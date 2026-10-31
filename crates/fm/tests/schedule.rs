use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use foreman_core::schedule::{self, Status};
use foreman_core::{Registry, secret};

struct Fixture {
    _tmp: tempfile::TempDir,
    home: PathBuf,
    claude: PathBuf,
    alpha: PathBuf,
    beta: PathBuf,
    outside: PathBuf,
}

/// Workspace `demo` with plain folders Alpha (with `/fix`) and Beta, in a temp home.
fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let (home, claude, alpha, beta, outside) =
        (root.join("home"), root.join("claude"), root.join("ws").join("Alpha"), root.join("ws").join("Beta"), root.join("outside"));
    for d in [&home, &claude, &beta, &outside, &alpha.join(".claude").join("commands"), &alpha.join("scripts"), &alpha.join("sub")] {
        fs::create_dir_all(d).unwrap();
    }
    fs::write(alpha.join(".claude").join("commands").join("fix.md"), "Fix the bugs.\n").unwrap();
    fs::write(alpha.join("prompt.md"), "Triage the inbox.\n").unwrap();
    fs::write(alpha.join("scripts").join("gate.ps1"), "exit 10\n").unwrap();
    fs::write(alpha.join("scripts").join("gate.py"), "print(1)\n").unwrap();
    let f = Fixture { _tmp: tmp, home, claude, alpha, beta, outside };
    let out = f.fm_in(&f.outside, &["workspace", "new", "demo", "--folder", s(&f.alpha), "--folder", s(&f.beta)]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    f
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

impl Fixture {
    fn fm(&self, args: &[&str]) -> Output {
        self.fm_in(&self.outside, args)
    }

    fn fm_in(&self, cwd: &Path, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_fm"))
            .args(args)
            .current_dir(cwd)
            .env("FOREMAN_HOME", &self.home)
            .env("CLAUDE_CONFIG_DIR", &self.claude)
            .env_remove("APPDATA")
            .env_remove("FOREMAN_ROLE")
            .output()
            .unwrap()
    }

    fn ok(&self, args: &[&str]) -> String {
        let out = self.fm(args);
        assert_eq!(out.status.code(), Some(0), "fm {args:?}: {out:?}");
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn refused(&self, args: &[&str]) -> String {
        let out = self.fm(args);
        assert_eq!(out.status.code(), Some(1), "fm {args:?} must be refused: {out:?}");
        String::from_utf8_lossy(&out.stderr).into_owned()
    }

    fn workspace(&self) -> foreman_core::Workspace {
        Registry::at(self.home.clone()).load("demo").unwrap()
    }

    fn stored(&self, id: &str) -> schedule::Schedule {
        schedule::load(&self.home, "demo").unwrap().into_iter().find(|s| s.id == id).unwrap()
    }

    /// René's Approve, which only the window makes; here it stands in for it to test what fm does after.
    fn approve(&self, id: &str) {
        let store = secret::default_store(&self.home);
        schedule::approve(&self.home, &*store, &self.workspace(), id, 1).unwrap();
        assert_eq!(self.stored(id).status, Status::Active);
    }
}

const ADD: [&str; 9] = ["schedule", "add", "demo", "--name", "Fix bugs", "--folder", "Alpha", "--every", "weekly mon,fri 09:00"];

fn add_fix(f: &Fixture, extra: &[&str]) -> String {
    let mut args: Vec<&str> = ADD.to_vec();
    args.extend(["--command", "/fix"]);
    args.extend(extra);
    f.ok(&args)
}

#[test]
fn add_saves_a_pending_schedule_through_the_store() {
    let f = fixture();
    let text = add_fix(&f, &["--profile", "worktree-edit", "--allow-tools", "Bash(dotnet test:*)", "--secret", "SENTRY_TOKEN", "--max-tokens", "500000", "--overlap", "queue", "--wake"]);
    assert!(text.contains("added schedule fix-bugs to workspace demo"), "{text}");
    assert!(text.contains("status: pending\n"), "{text}");
    assert!(text.contains("pending: it runs only after Approve in the Supraman window"), "{text}");

    let saved = f.stored("fix-bugs");
    assert_eq!(saved.status, Status::Pending);
    assert!(saved.approval.is_none());
    assert_eq!(saved.folder, "Alpha");
    assert_eq!(saved.action.prompt, Some(schedule::Prompt::Command("/fix".into())));
    assert!(saved.action.worktree, "worktree-edit runs in a worktree");
    assert_eq!(saved.permissions.secrets, ["SENTRY_TOKEN"]);
    assert_eq!(saved.caps.max_tokens, 500_000);
    assert!(saved.policy.wake);
    let toml = fs::read_to_string(schedule::file_path(&f.home, "demo").unwrap()).unwrap();
    assert!(toml.contains("secrets = [\"SENTRY_TOKEN\"]") && !toml.contains("[schedule.approval]"), "{toml}");

    let list = f.ok(&["schedule", "list", "demo"]);
    assert!(list.starts_with("fix-bugs\tpending\tfolder=Alpha\tevery=weekly mon,fri 09:00\tnext="), "{list}");
    assert!(f.ok(&["schedule", "show", "demo", "fix-bugs"]).contains("profile: worktree-edit\n"));

    let taken = f.refused(&ADD.iter().copied().chain(["--command", "/fix"]).collect::<Vec<_>>());
    assert!(taken.contains("the id is taken"), "{taken}");
}

#[test]
fn prompt_file_and_script_are_kept_relative_to_the_folder() {
    let f = fixture();
    let prompt = f.alpha.join("prompt.md");
    f.ok(&["schedule", "add", "demo", "--name", "Triage", "--folder", "Alpha", "--every", "daily 07:30", "--prompt-file", s(&prompt), "--script", "scripts/gate.ps1", "--script-arg", "-Quick"]);
    let saved = f.stored("triage");
    assert_eq!(saved.action.prompt, Some(schedule::Prompt::File("prompt.md".into())));
    assert_eq!(saved.action.kind, schedule::ActionKind::ScriptThenSession);
    assert_eq!(saved.action.script_args, ["-Quick"]);

    f.ok(&["schedule", "add", "demo", "--name", "Gate only", "--folder", "Alpha", "--every", "interval 2h from 08:00", "--script", "scripts/gate.ps1"]);
    assert_eq!(f.stored("gate-only").action.kind, schedule::ActionKind::Script);
    let dry = f.ok(&["schedule", "dry-run", "demo", "gate-only"]);
    assert!(dry.contains("session: none (a script action)"), "{dry}");

    let missing = f.refused(&["schedule", "add", "demo", "--name", "Nope", "--folder", "Beta", "--every", "daily 07:30", "--prompt-file", "nope.md"]);
    assert!(missing.contains("no prompt file nope.md in folder") && missing.contains("Beta"), "{missing}");
}

#[test]
fn a_missing_slash_command_names_the_folder() {
    let f = fixture();
    let err = f.refused(&["schedule", "add", "demo", "--name", "X", "--folder", "Beta", "--every", "daily 09:00", "--command", "/fix"]);
    assert!(err.contains("no slash command /fix in folder") && err.contains(s(&f.beta)) && err.contains(".claude/commands/fix.md"), "{err}");
    assert!(schedule::load(&f.home, "demo").unwrap().is_empty(), "nothing is written");

    add_fix(&f, &[]);
    fs::remove_file(f.alpha.join(".claude").join("commands").join("fix.md")).unwrap();
    let err = f.refused(&["schedule", "dry-run", "demo", "fix-bugs"]);
    assert!(err.contains("no slash command /fix in folder") && err.contains(s(&f.alpha)), "{err}");
}

#[test]
fn refuses_bypass_permissions_unknown_scripts_and_flag_shaped_rules() {
    let f = fixture();
    fn with<'a>(extra: &[&'a str]) -> Vec<&'a str> {
        ADD.iter().copied().chain(["--command", "/fix"]).chain(extra.iter().copied()).collect()
    }
    let err = f.refused(&with(&["--profile", "bypassPermissions"]));
    assert!(err.contains("bypassPermissions is never a schedule's mode"), "{err}");
    let err = f.refused(&with(&["--permission-mode", "bypassPermissions"]));
    assert!(err.contains("--permission-mode is not a schedule option"), "{err}");
    let err = f.refused(&with(&["--script", "scripts/gate.py"]));
    assert!(err.contains("a script is .ps1, .cmd, .bat or .exe"), "{err}");
    let err = f.refused(&with(&["--allow-tools", "--dangerously-skip-permissions"]));
    assert!(err.contains("is not a tool rule"), "{err}");
    let err = f.refused(&with(&["--max-cost", "5"]));
    assert!(err.contains("use --max-tokens"), "{err}");
    let err = f.refused(&with(&["--every", "interval 5m"]));
    assert!(err.contains("an interval is at least 15m"), "{err}");
    assert!(schedule::load(&f.home, "demo").unwrap().is_empty(), "nothing is written");
}

#[test]
fn the_workspace_and_folder_come_from_the_cwd() {
    let f = fixture();
    let out = f.fm_in(&f.alpha.join("sub"), &["schedule", "add", "--name", "Here", "--every", "daily 09:00", "--command", "/fix"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(f.stored("here").folder, "Alpha");
    let out = f.fm_in(&f.alpha, &["schedule", "show", "here"]);
    assert!(String::from_utf8_lossy(&out.stdout).contains("workspace: demo\n"), "{out:?}");

    let out = f.fm(&["schedule", "list"]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("no workspace contains") && err.contains(s(&f.outside)), "{err}");
}

#[test]
fn covered_edits_pause_and_resume_never_make_a_schedule_active_without_approval() {
    let f = fixture();
    add_fix(&f, &[]);
    let resumed = f.ok(&["schedule", "resume", "demo", "fix-bugs"]);
    assert!(resumed.contains("fix-bugs in workspace demo: pending"), "{resumed}");
    assert_eq!(f.stored("fix-bugs").status, Status::Pending);

    f.approve("fix-bugs");
    let approval = f.stored("fix-bugs").approval;
    let text = f.ok(&["schedule", "edit", "demo", "fix-bugs", "--name", "Fix all bugs", "--description", "weekly"]);
    assert!(text.contains("status: active\n"), "an uncovered edit keeps the approval: {text}");

    let text = f.ok(&["schedule", "edit", "demo", "fix-bugs", "--every", "daily 10:00"]);
    assert!(text.contains("status: pending (changed since approval)"), "{text}");
    let saved = f.stored("fix-bugs");
    assert_eq!((saved.status, saved.approval), (Status::Pending, approval), "the edit leaves the approval for Approve to diff");
    assert!(f.ok(&["schedule", "resume", "demo", "fix-bugs"]).contains(": pending"));
    assert_eq!(f.stored("fix-bugs").status, Status::Pending);

    f.approve("fix-bugs");
    assert!(f.ok(&["schedule", "pause", "demo", "fix-bugs"]).contains(": paused"));
    assert!(f.ok(&["schedule", "resume", "demo", "fix-bugs"]).contains(": active"), "an approved schedule resumes");

    for edit in [["--profile", "auto"], ["--max-runtime", "2h"], ["--overlap", "restart"], ["--allow-tools", "Bash(ls:*)"]] {
        f.approve("fix-bugs");
        let args: Vec<&str> = ["schedule", "edit", "demo", "fix-bugs"].into_iter().chain(edit).collect();
        f.ok(&args);
        assert_eq!(f.stored("fix-bugs").status, Status::Pending, "{edit:?}");
    }

    let text = f.ok(&["schedule", "remove", "demo", "fix-bugs"]);
    assert!(text.contains("removed schedule fix-bugs"), "{text}");
    assert!(f.ok(&["schedule", "list", "demo"]).contains("no schedules in workspace demo"));
    assert!(f.refused(&["schedule", "show", "demo", "fix-bugs"]).contains("has no schedule 'fix-bugs'"));
}

#[test]
fn dry_run_prints_the_prompt_the_argv_the_permissions_and_three_fires() {
    let f = fixture();
    add_fix(&f, &["--profile", "worktree-edit", "--allow-tools", "Bash(dotnet test:*)", "--deny-tools", "WebFetch", "--secret", "SENTRY_TOKEN", "--model", "opus"]);
    let text = f.ok(&["schedule", "dry-run", "demo", "fix-bugs"]);
    assert!(text.contains("nothing launched, nothing written"), "{text}");
    assert!(text.contains("This is an unattended scheduled run `fix-bugs-") && text.contains(" schedule report fix-bugs-"), "{text}");
    assert!(text.contains("` with the Bash tool, exactly as written"), "a PowerShell `fm` parks the run: {text}");
    assert!(text.contains("\n  /fix\n"), "the base prompt follows the preamble: {text}");
    let argv = text.split("argv:\n").nth(1).unwrap();
    for arg in ["  claude\n", "  --bg\n", "  --name\n  fm-demo-sched-fix-bugs-", "  --permission-mode\n  acceptEdits\n", "  --strict-mcp-config\n", "  --allowedTools\n", "  Bash(fm:*)\n", "  Edit\n", "  \"Bash(dotnet test:*)\"\n", "  --disallowedTools\n", "  \"Bash(git push --force:*)\"\n", "  Bash(schtasks:*)\n", "  WebFetch\n", "  --model\n  opus\n", "  --\n"] {
        assert!(argv.contains(arg), "argv lacks {arg:?}: {text}");
    }
    assert!(text.contains(&format!("cwd: {}", f.alpha.join(".claude").join("worktrees").join("fm-sched-fix-bugs-").display())), "{text}");
    assert!(text.contains("permissions: profile worktree-edit (--permission-mode acceptEdits)"), "{text}");
    assert!(text.contains("  allow: Bash(fm:*)") && text.contains("Read  Glob  Grep  Edit  Write  Bash(git status:*)"), "{text}");
    assert!(text.contains("Bash(git commit:*)  Bash(git push -u origin fm-sched/fix-bugs/") && text.contains(")  Bash(dotnet test:*)"), "{text}");
    assert!(text.contains("needs those commands as --allow-tools"), "dry-run says what a fix needs: {text}");
    assert!(text.contains("  deny: Bash(git push --force:*)") && text.contains("WebFetch"), "{text}");
    assert!(text.contains("secrets: SENTRY_TOKEN (names only"), "{text}");
    let fires: Vec<&str> = text.split("next fires:\n").nth(1).unwrap().lines().collect();
    assert_eq!(fires.len(), 3, "{text}");
    assert!(fires.iter().all(|l| (l.starts_with("  Mon ") || l.starts_with("  Fri ")) && l.contains(" 09:00 ")), "{fires:?}");
    assert_eq!(f.stored("fix-bugs").status, Status::Pending, "a dry run changes nothing");
    assert!(!f.home.join("mcp").exists(), "a dry run writes no mcp config");
}

#[test]
fn show_lists_the_schedules_last_runs_newest_first() {
    use schedule::{Fired, Outcome, RunEvent, RunRecord};
    let f = fixture();
    add_fix(&f, &[]);
    assert!(f.ok(&["schedule", "show", "demo", "fix-bugs"]).ends_with("last runs: none\n"));
    let record = |run: &str, schedule: &str, ts: i64, event: RunEvent| RunRecord { ts, run: run.into(), schedule: schedule.into(), event };
    let started = |ts, run: &str, schedule: &str| record(run, schedule, ts, RunEvent::Started { trigger: Fired::Task, fingerprint: "f".into(), fm_pid: 1 });
    let ended = |ts, run: &str, outcome, why: &str| record(run, "fix-bugs", ts, RunEvent::Ended { outcome, why: why.into(), tokens: None, ms: 1 });
    for r in [
        started(1_000, "fix-bugs-1", "fix-bugs"),
        ended(2_000, "fix-bugs-1", Outcome::Done, ""),
        started(3_000, "other-1", "other"),
        started(4_000, "fix-bugs-2", "fix-bugs"),
        ended(5_000, "fix-bugs-2", Outcome::CappedTokens, "3000001 tokens"),
        started(6_000, "fix-bugs-3", "fix-bugs"),
    ] {
        schedule::append_run(&f.home, "demo", &r).unwrap();
    }
    let text = f.ok(&["schedule", "show", "demo", "fix-bugs"]);
    let runs: Vec<&str> = text.split("last runs (newest first):\n").nth(1).expect(&text).lines().collect();
    assert_eq!(runs.len(), 3, "{text}");
    assert!(runs[0].starts_with("  fix-bugs-3  ") && runs[0].ends_with("  task  running"), "{runs:?}");
    assert!(runs[1].starts_with("  fix-bugs-2  ") && runs[1].ends_with("  task  capped_tokens: 3000001 tokens"), "{runs:?}");
    assert!(runs[2].starts_with("  fix-bugs-1  ") && runs[2].ends_with("  task  done"), "{runs:?}");
}

#[test]
fn read_runs_in_the_main_checkout_with_the_default_mode() {
    let f = fixture();
    add_fix(&f, &[]);
    let text = f.ok(&["schedule", "dry-run", "demo", "fix-bugs"]);
    assert!(text.contains(&format!("cwd: {}\n", f.alpha.display())), "{text}");
    assert!(text.contains("  --permission-mode\n  default\n") && text.contains("allow: Bash(fm:*)") && !text.contains("  Edit\n"), "{text}");
}

/// C100: fm has no path that approves a schedule or registers a task.
#[test]
fn fm_never_approves_and_never_registers_a_task() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let forbidden = ["approve(", "APPROVAL_KEY", "Approval {", "approval = Some", ".approval =", "Registrar", "schtasks", "ScheduledTask", "ITaskService", "TaskScheduler"];
    let mut files = 0;
    let mut dirs = vec![src.clone()];
    while let Some(dir) = dirs.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
                continue;
            }
            let text = fs::read_to_string(&path).unwrap();
            files += 1;
            for word in forbidden {
                assert!(!text.contains(word), "{} names {word}", path.display());
            }
        }
    }
    assert!(files > 5, "read the sources at {}", src.display());
    let manifest = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap();
    assert!(!manifest.contains("registrar") && !manifest.contains("windows"), "{manifest}");
}
