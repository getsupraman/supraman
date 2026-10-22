//! `fm probe isolation`: where does a `--bg` session write, and what does `claude rm` leave behind?
//! Three `aw-m2-probe-<hex>-iso-*` sessions each `Write` one file into their cwd under `target/`:
//! - `main`: cwd in the main checkout (default settings);
//! - `none`: the same, with `--settings {"worktree":{"bgIsolation":"none"}}`;
//! - `linked`: cwd in this build's linked worktree (skipped when that is the main checkout).
//!
//! Then each is stopped and `claude rm`-ed once, and what is left (row, worktree dir, branch) is
//! reported before [`Fleet`]'s cleanup removes it. Answers are in `docs/LEARNINGS.md` (m2/2).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use foreman_claude::{Claude, LaunchSpec};

use crate::plugin;
use crate::probe::{Fleet, claude_worktree, git, hex6, mkdir, wait_until_stopped, workspace_root, yes};
use crate::probe_hooks::{Log, last_reply};

const TURN_WITHIN: Duration = Duration::from_secs(300);

fn prompt(case: &str) -> String {
    format!(
        "This is an automated probe. Use the Write tool to create the file iso.txt in the current directory containing \
exactly one line: ISO {case}. Then reply with only the absolute path of the file you wrote."
    )
}

/// `repo`: the main checkout to probe (default: the main checkout of this build's workspace).
pub fn run(repo: Option<PathBuf>) -> bool {
    let claude = Claude::default();
    let hex = hex6();
    let mut fleet = Fleet {
        claude: claude.clone(),
        names: Vec::new(),
    };
    let ok = match probe(&claude, &mut fleet, repo, &hex) {
        Ok(ok) => ok,
        Err(e) => {
            println!("probe error: {e}");
            false
        }
    };
    drop(fleet);
    ok
}

struct Case {
    label: &'static str,
    repo: PathBuf,
    spec: LaunchSpec,
}

fn probe(claude: &Claude, fleet: &mut Fleet, repo: Option<PathBuf>, hex: &str) -> Result<bool, String> {
    let linked = workspace_root();
    let main = match repo {
        Some(r) => r,
        None => main_checkout(&linked)?,
    };
    println!("main checkout {} | this build's checkout {}", main.display(), linked.display());
    let plugin = linked.join("plugins").join("foreman");
    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    plugin::install(&plugin, &exe).map_err(|e| format!("plugin install: {e}"))?;
    let home = mkdir(&linked.join("target").join(format!("aw-m2-probe-{hex}")), "home")?;

    let spec = |label: &str, repo: &Path, settings: serde_json::Value| -> Result<LaunchSpec, String> {
        let cwd = mkdir(&repo.join("target").join(format!("aw-m2-probe-{hex}")), &format!("iso-{label}"))?;
        let mut settings = settings;
        settings["env"] = serde_json::json!({ "FOREMAN_HOME": home.display().to_string() });
        let mut spec = LaunchSpec::new(format!("aw-m2-probe-{hex}-iso-{label}"), cwd, prompt(label));
        spec.strict_mcp_config = true;
        spec.plugin_dirs = vec![plugin.clone()];
        spec.extra_args = vec!["--settings".into(), settings.to_string()];
        Ok(spec)
    };
    let mut cases = vec![
        Case {
            label: "main",
            repo: main.clone(),
            spec: spec("main", &main, serde_json::json!({}))?,
        },
        Case {
            label: "none",
            repo: main.clone(),
            spec: spec("none", &main, serde_json::json!({ "worktree": { "bgIsolation": "none" } }))?,
        },
    ];
    if linked != main {
        cases.push(Case {
            label: "linked",
            repo: linked.clone(),
            spec: spec("linked", &linked, serde_json::json!({}))?,
        });
    }

    for case in &cases {
        fleet.launch(&case.spec)?;
    }
    let mut log = Log::new(&home);
    let mut ok = true;
    for case in &cases {
        let row = fleet.row(&case.spec.name)?;
        let ended = log.wait(&row.session_id, TURN_WITHIN, |es| es.iter().any(|e| e.event == "Stop"));
        let events = log.of(&row.session_id);
        let reply = events
            .iter()
            .rev()
            .find_map(|e| e.transcript_path.clone())
            .and_then(|t| last_reply(Path::new(&t)))
            .unwrap_or_else(|| "none".into());
        let after = fleet.row(&case.spec.name)?;
        let worktree = claude_worktree(&after.cwd).map(|(_, w)| w);
        let in_cwd = case.spec.cwd.join("iso.txt").is_file();
        println!(
            "iso-{}: turn ended={} | row cwd {} → {} | iso.txt in launch cwd={} | reply {:?}",
            case.label,
            yes(ended),
            row.cwd.display(),
            after.cwd.display(),
            yes(in_cwd),
            reply.trim()
        );
        if let Some(wt) = &worktree {
            let branch = git(wt, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_default();
            let dirty = git(wt, &["status", "--short"]).unwrap_or_default();
            let ahead = git(wt, &["rev-list", "--count", "HEAD", "^HEAD@{upstream}"])
                .or_else(|_| git(&case.repo, &["rev-list", "--count", &branch, "^HEAD"]))
                .unwrap_or_default();
            let locked = git(&case.repo, &["worktree", "list", "--porcelain"])
                .unwrap_or_default()
                .split("\n\n")
                .any(|block| block.contains(&wt.display().to_string().replace('\\', "/")) && block.contains("locked"));
            println!(
                "  worktree {} | branch {branch} | commits not on the checkout's HEAD: {ahead} | dirty: {} | locked={}",
                wt.display(),
                if dirty.is_empty() { "no".to_string() } else { dirty.replace('\n', ", ") },
                yes(locked)
            );
        }

        // one plain `claude rm`, and what it leaves
        let _ = claude.stop(&after.id);
        wait_until_stopped(claude, &case.spec.name)?;
        let rm = claude.rm(&after.id).map_err(|e| e.to_string());
        let row_left = !claude.find_by_name(&case.spec.name, Duration::ZERO).map_err(|e| e.to_string())?.is_empty();
        let dir_left = worktree.as_ref().is_some_and(|w| w.exists());
        let branch_left = worktree.as_ref().is_some_and(|w| {
            let name = format!("worktree-{}", w.file_name().map(|n| n.to_string_lossy()).unwrap_or_default());
            git(&case.repo, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{name}")]).is_ok()
        });
        println!(
            "  claude rm: {} | left: row={} worktree dir={} worktree-* branch={}",
            match &rm {
                Ok(out) => format!("ok {:?}", out.trim()),
                Err(e) => format!("failed {e:?}"),
            },
            yes(row_left),
            yes(dir_left),
            yes(branch_left)
        );
        // a worktree whose row is already gone is beyond Fleet's reach: remove it here
        if !row_left && dir_left && let Some(wt) = &worktree {
            let branch = git(wt, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_default();
            let wt_s = wt.display().to_string();
            let _ = git(&case.repo, &["worktree", "unlock", &wt_s]);
            let removed = git(&case.repo, &["worktree", "remove", "--force", "--force", &wt_s]);
            let deleted = branch.starts_with("worktree-").then(|| git(&case.repo, &["branch", "-D", &branch]));
            println!("  cleanup: git worktree remove {removed:?}; branch -D {deleted:?}");
        }
        ok &= ended;
    }
    let _ = fs::remove_dir_all(main.join("target").join(format!("aw-m2-probe-{hex}")));
    Ok(ok)
}

/// The main working tree of the repository `dir` belongs to: the parent of `git-common-dir`.
fn main_checkout(dir: &Path) -> Result<PathBuf, String> {
    let common = git(dir, &["rev-parse", "--path-format=absolute", "--git-common-dir"])?;
    Path::new(&common)
        .parent()
        .map(|p| PathBuf::from(p.to_string_lossy().replace('/', "\\")))
        .ok_or_else(|| format!("no parent of {common}"))
}
