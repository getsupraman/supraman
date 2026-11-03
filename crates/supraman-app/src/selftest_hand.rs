//! A [+] session in a workspace folder, run by a fake claude that records its env and plays its hooks
//! with the `sm` it finds on PATH; a real claude's skill listing is not read here.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context as _, anyhow, bail};
use gpui::{AsyncWindowContext, Entity};

use crate::{
    selftest::{WORKSPACE, main_repo_of_gate, sleep, until, wait_for_tab_terminal},
    selftest_closed::claude_dir,
    view::FleetView,
};

const WITHIN: Duration = Duration::from_secs(20);
const SID: &str = "5e1f7a2d-0000-4000-8000-000000000a1d";
const DONE: &str = "fake hand claude done";

fn seen_file(home: &Path) -> PathBuf {
    claude_dir(home).join("hand-seen.txt")
}

fn write_fake(home: &Path) -> anyhow::Result<PathBuf> {
    let (fake, seen) = (claude_dir(home).join("hand-claude.cmd"), seen_file(home));
    std::fs::create_dir_all(claude_dir(home))?;
    let seen = seen.display();
    let script = [
        format!("@set SUPRAMAN_> \"{seen}\""),
        format!("@set SUPRAMAN_HOME>> \"{seen}\""),
        format!("@where sm>> \"{seen}\""),
        format!("@sm --version>> \"{seen}\" 2>&1"),
        format!("@echo {{\"session_id\":\"{SID}\",\"source\":\"startup\"}}| sm hook SessionStart>> \"{seen}\""),
        format!("@echo {{\"session_id\":\"{SID}\",\"notification_type\":\"permission_prompt\"}}| sm hook Notification"),
        format!("@echo {DONE}>> \"{seen}\""),
    ];
    std::fs::write(&fake, script.join("\r\n") + "\r\n")?;
    Ok(fake)
}

pub(crate) async fn run(view: &Entity<FleetView>, home: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    std::fs::remove_file(seen_file(home)).ok();
    let fake = write_fake(home)?;
    let saved = view.update(cx, |v, _| std::mem::replace(&mut v.claude_command, format!("& '{}'", fake.display())));
    let result = check(view, home, cx).await;
    let mine: Vec<u64> = view.read_with(cx, |v, _| v.tabs.iter().filter(|t| t.title == "hand selftest").map(|t| t.id).collect());
    view.update(cx, |v, cx| {
        v.claude_command = saved;
        mine.into_iter().for_each(|id| v.close_tab(id, cx));
        v.injected.lock().unwrap_or_else(|e| e.into_inner()).clear();
        cx.notify();
    });
    result
}

async fn check(view: &Entity<FleetView>, home: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let folder = main_repo_of_gate()?;
    let repo = folder.file_name().context("the main repo has no folder name")?.to_string_lossy().into_owned();
    let plugin = view.read_with(cx, |v, _| v.plugin_dir.clone()).context("this build found no plugins/supraman")?;
    let id = cx.update(|window, cx| view.update(cx, |v, cx| v.new_claude(folder.clone(), "hand selftest".into(), window, cx)))?;
    wait_for_tab_terminal(view, id, WITHIN, cx).await?;
    let seen = seen_file(home);
    let deadline = std::time::Instant::now() + WITHIN;
    let text = loop {
        let text = std::fs::read_to_string(&seen).unwrap_or_default();
        if text.contains(DONE) {
            break text;
        }
        if std::time::Instant::now() > deadline {
            bail!("the fake claude did not finish within {} s; {} holds {text:?}", WITHIN.as_secs(), seen.display());
        }
        sleep(cx, Duration::from_millis(100)).await;
    };
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    let session = crate::hand::plain_name(&folder);
    let home_line = format!("SUPRAMAN_HOME={}", home.display());
    let env = [
        "SUPRAMAN_ROLE=hand".to_string(),
        format!("SUPRAMAN_WORKSPACE={WORKSPACE}"),
        format!("SUPRAMAN_SESSION={session}"),
        format!("SUPRAMAN_REPO={repo}"),
        home_line,
    ];
    for want in &env {
        if !lines.iter().any(|l| l.eq_ignore_ascii_case(want)) {
            bail!("the [+] claude's env has no {want}: {text}");
        }
    }
    let sm = supraman_core::install::sm_exe()?;
    let found = lines.iter().find(|l| l.to_ascii_lowercase().ends_with("sm.exe")).copied().unwrap_or_default();
    if !found.eq_ignore_ascii_case(&sm.display().to_string()) {
        bail!("`where sm` found `{found}` first, not {}: {text}", sm.display());
    }
    let version = lines.iter().find(|l| l.starts_with("sm ")).copied().ok_or_else(|| anyhow!("`sm --version` printed nothing: {text}"))?;
    let tag = lines.iter().find(|l| l.starts_with("SUPRAMAN-ROLE hand v")).copied().ok_or_else(|| anyhow!("SessionStart injected no hand role: {text}"))?;
    if !text.contains(&format!("Supraman workspace {WORKSPACE}")) || !text.contains("/supraman:supraman-scheduler") {
        bail!("the hand role is not rendered for {WORKSPACE}: {text}");
    }
    let notices = view.read_with(cx, |v, _| v.hand_notices.clone());
    if notices.iter().any(|n| n.contains("plugins/supraman")) {
        bail!("a missing-plugin notice although the plugin is at {}: {notices:?}", plugin.display());
    }

    until(view, cx, WITHIN, "the hand session's permission prompt among the hook signals", |v| {
        v.hooks.get(SID).filter(|h| h.is_permission_prompt()).map(drop)
    })
    .await?;
    let row = serde_json::json!([{
        "sessionId": SID,
        "name": session,
        "cwd": folder.display().to_string(),
        "kind": "interactive",
        "startedAt": supraman_claude::now_ms() - 60_000,
        "status": "idle",
        "pid": std::process::id(),
    }]);
    let rows = supraman_claude::parse(&row.to_string())?;
    view.read_with(cx, |v, _| v.injected.lock().unwrap_or_else(|e| e.into_inner()).extend(rows));
    let label = until(view, cx, WITHIN, "the hand session's row with its hook signal", |v| {
        v.tree()
            .workspaces
            .iter()
            .filter(|w| w.name == WORKSPACE)
            .flat_map(|w| w.repos.iter().filter(|r| r.name == repo).flat_map(|r| &r.sessions))
            .find(|r| r.session.session_id == SID)
            .and_then(|r| r.hook.as_ref().map(|h| h.label(supraman_claude::now_ms())))
    })
    .await?;
    if !label.starts_with("permission prompt") {
        bail!("the hand session's row shows `{label}`, not its permission prompt");
    }
    Ok(format!(
        "[+] in {WORKSPACE}/{repo}: SUPRAMAN_ROLE=hand SUPRAMAN_SESSION={session} and the home in its env, no missing-plugin notice, \
         {} first on PATH ({version}), SessionStart injected `{tag}`, its row under {WORKSPACE}/{repo} shows `{label}`",
        sm.display()
    ))
}
