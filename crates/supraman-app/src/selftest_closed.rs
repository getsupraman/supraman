use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{anyhow, bail};
use gpui::{AsyncWindowContext, Entity};

use crate::{
    selftest::{click, double_click, sleep, until},
    stars::Stars,
    view::FleetView,
};

pub const RECENT: &str = "5e1f7e57-0000-4000-8000-00000000c105";
pub const OLD: &str = "01d05e1f-0000-4000-8000-0000000001d0";
const RECENT_TITLE: &str = "selftest closed session";
const OLD_TITLE: &str = "selftest starred old session";
const WITHIN: Duration = Duration::from_secs(15);

pub fn claude_dir(home: &Path) -> PathBuf {
    home.join("fake-claude")
}

pub(crate) fn seen_file(home: &Path) -> PathBuf {
    claude_dir(home).join("resumed.txt")
}

pub(crate) fn fake_claude(home: &Path) -> PathBuf {
    claude_dir(home).join("claude.cmd")
}

/// claude's key order.
pub(crate) fn transcript(id: &str, cwd: &Path, title: &str) -> String {
    let user = serde_json::json!({
        "parentUuid": null, "isSidechain": false, "type": "user",
        "message": {"role": "user", "content": "selftest prompt"},
        "uuid": "u1", "timestamp": "2026-10-27T10:00:00.000Z", "userType": "external", "entrypoint": "cli",
        "cwd": cwd.display().to_string(), "sessionId": id, "version": "2.1.306", "gitBranch": "master",
    });
    format!(
        "{{\"type\":\"mode\",\"mode\":\"normal\",\"sessionId\":\"{id}\"}}\n{user}\n{{\"type\":\"ai-title\",\"aiTitle\":\"not the name\",\"sessionId\":\"{id}\"}}\n{{\"type\":\"custom-title\",\"customTitle\":\"{title}\",\"sessionId\":\"{id}\"}}\n"
    )
}

pub fn prepare(home: &Path, main_repo: &Path) -> anyhow::Result<()> {
    let dir = claude_dir(home).join("projects").join(supraman_claude::transcript::slug(main_repo));
    std::fs::create_dir_all(&dir)?;
    let write = |id: &str, title: &str, age: Duration| -> anyhow::Result<()> {
        let path = dir.join(format!("{id}.jsonl"));
        std::fs::write(&path, transcript(id, main_repo, title))?;
        let file = std::fs::OpenOptions::new().write(true).open(&path)?;
        file.set_modified(SystemTime::now() - age)?;
        Ok(())
    };
    write(RECENT, RECENT_TITLE, Duration::from_secs(3600))?;
    write(OLD, OLD_TITLE, Duration::from_secs(20 * 24 * 3600))?;
    let seen = seen_file(home);
    std::fs::write(
        fake_claude(home),
        format!("@echo %*> \"{}\"\r\n@cd>> \"{}\"\r\n@echo fake claude resumed\r\n", seen.display(), seen.display()),
    )?;
    let (mut stars, _) = Stars::load(Some(home));
    stars.toggle(OLD).map_err(|e| anyhow!("cannot star {OLD}: {e}"))?;
    println!("step 1h: fake claude dir {} with transcripts {RECENT} (1 h old) and {OLD} (20 days, starred)", claude_dir(home).display());
    Ok(())
}

fn row(v: &FleetView, id: &str) -> Option<(String, bool, bool)> {
    v.tree()
        .workspaces
        .iter()
        .flat_map(|w| w.repos.iter().flat_map(|r| &r.sessions))
        .find(|r| r.session.session_id == id)
        .map(|r| (r.session.name.clone(), r.closed, r.starred))
}

fn painted(v: &FleetView, key: &str) -> bool {
    v.shown(key)
}

pub async fn run(view: &Entity<FleetView>, home: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let saved = view.update(cx, |v, _| {
        let saved = (v.show_stopped, std::mem::replace(&mut v.claude_command, format!("& '{}'", fake_claude(home).display())));
        v.show_stopped = false;
        v.transcripts_nudge.wake();
        saved
    });
    let result = check(view, home, cx).await;
    let ids: Vec<u64> = view.read_with(cx, |v, _| v.tabs.iter().map(|t| t.id).collect());
    view.update(cx, |v, cx| {
        ids.into_iter().for_each(|id| v.close_tab(id, cx));
        v.show_stopped = saved.0;
        v.claude_command = saved.1;
        v.hand_notices.clear();
        cx.notify();
    });
    result
}

async fn check(view: &Entity<FleetView>, home: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let started = Instant::now();
    let (name, closed, starred) = until(view, cx, WITHIN, "the starred 20-day-old transcript in the tree with stopped hidden", |v| row(v, OLD)).await?;
    if name != OLD_TITLE || !closed || !starred {
        bail!("{OLD} shows as {name:?} closed={closed} starred={starred}");
    }
    let first = started.elapsed();
    if view.read_with(cx, |v, _| row(v, RECENT)).is_some() {
        bail!("{RECENT} is shown although stopped sessions are hidden and it is not starred");
    }

    click(view, "toggle-stopped", cx).await?;
    let (name, closed, _) = until(view, cx, WITHIN, "the closed session once stopped sessions are shown", |v| row(v, RECENT)).await?;
    if name != RECENT_TITLE || !closed {
        bail!("{RECENT} shows as {name:?} closed={closed}: not named by its custom title");
    }
    until(view, cx, WITHIN, "its resume and star painted", |v| {
        (painted(v, &format!("resume-{}", &RECENT[..8])) && painted(v, &format!("star-{RECENT}"))).then_some(())
    })
    .await?;

    let tabs = view.read_with(cx, |v, _| v.tabs.len());
    double_click(view, &format!("session-row-{RECENT}"), cx).await?;
    until(view, cx, WITHIN, "a double-click on its row resumes it in a tab", |v| (v.tabs.len() > tabs).then_some(())).await?;
    sleep(cx, Duration::from_millis(500)).await;
    let opened = view.read_with(cx, |v, _| v.tabs.len()) - tabs;
    if opened != 1 {
        bail!("a double-click on {RECENT}'s row opened {opened} tabs, expected 1");
    }
    let seen = seen_file(home);
    let deadline = Instant::now() + WITHIN;
    let text = loop {
        let text = std::fs::read_to_string(&seen).unwrap_or_default();
        if text.lines().count() >= 2 {
            break text;
        }
        if Instant::now() > deadline {
            bail!("the fake claude never ran (no {} within {} s)", seen.display(), WITHIN.as_secs());
        }
        sleep(cx, Duration::from_millis(100)).await;
    };
    let mut lines = text.lines();
    let (args, cwd) = (lines.next().unwrap_or_default().trim(), lines.next().unwrap_or_default().trim());
    if !args.starts_with(&format!("--resume {RECENT}")) {
        bail!("the fake claude got `{args}`, not --resume {RECENT}");
    }
    let main = crate::selftest::main_repo_of_gate()?;
    if !cwd.eq_ignore_ascii_case(main.to_string_lossy().replace('/', "\\").trim_end_matches('\\')) {
        bail!("the fake claude ran in {cwd}, not the session's cwd {}", main.display());
    }
    let minted = args.contains("--mcp-config");

    click(view, &format!("star-{RECENT}"), cx).await?;
    until(view, cx, WITHIN, "the star sets", |v| row(v, RECENT).filter(|r| r.2).map(|_| ())).await?;
    click(view, "toggle-stopped", cx).await?;
    until(view, cx, WITHIN, "stopped hidden again", |v| (!v.show_stopped).then_some(())).await?;
    sleep(cx, Duration::from_millis(300)).await;
    if view.read_with(cx, |v, _| row(v, RECENT)).is_none() {
        bail!("the starred {RECENT} dropped out with stopped sessions hidden");
    }

    click(view, &format!("star-{RECENT}"), cx).await?;
    until(view, cx, WITHIN, "unstar hides it with stopped hidden", |v| row(v, RECENT).is_none().then_some(())).await?;
    let (on_disk, _) = Stars::load(Some(home));
    if on_disk.ids().contains(RECENT) {
        bail!("unstar left {RECENT} in starred.json");
    }
    Ok(format!(
        "starred 20-day-old {OLD} shown with stopped hidden {} ms in; {RECENT} only with stopped shown, as \"{RECENT_TITLE}\"; \
         a double-click on its row resumed it: the fake claude ran `{args}` in {cwd} ({}); star kept it with stopped hidden; unstar hid it and left starred.json",
        first.as_millis(),
        if minted { "with its own token config" } else { "plain: no Workman token to mint" }
    ))
}
