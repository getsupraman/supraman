use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{anyhow, bail};
use foreman_claude::{Health, list_sessions, transcript::slug};
use gpui::{AsyncWindowContext, Entity, MouseButton};

use crate::selftest::{click, sleep};
use crate::selftest_closed::{OLD, RECENT, claude_dir, transcript};
use crate::selftest_manage::{menu_open, press_painted};
use crate::view::FleetView;

pub const DELETE_ME: &str = "de1e7e00-0000-4000-8000-00000000de1e";
const TITLE: &str = "selftest delete me";
const WITHIN: Duration = Duration::from_secs(20);

fn project(home: &Path, cwd: &Path) -> PathBuf {
    claude_dir(home).join("projects").join(slug(cwd))
}

fn plant(home: &Path, cwd: &Path, id: &str, title: &str) -> anyhow::Result<(PathBuf, PathBuf)> {
    let dir = project(home, cwd);
    std::fs::create_dir_all(dir.join(id).join("subagents"))?;
    std::fs::write(dir.join(id).join("subagents").join("agent-1.jsonl"), "{}\n")?;
    let file = dir.join(format!("{id}.jsonl"));
    std::fs::write(&file, transcript(id, cwd, title))?;
    std::fs::OpenOptions::new().write(true).open(&file)?.set_modified(SystemTime::now() - Duration::from_secs(1800))?;
    Ok((file, dir.join(id)))
}

pub fn prepare(home: &Path, main_repo: &Path) -> anyhow::Result<()> {
    plant(home, main_repo, DELETE_ME, TITLE)?;
    println!("step 1o: fake transcript {DELETE_ME} ({TITLE}) with its {DELETE_ME}/ dir");
    Ok(())
}

async fn until<T>(view: &Entity<FleetView>, cx: &mut AsyncWindowContext, what: &str, f: impl Fn(&FleetView) -> Option<T>) -> anyhow::Result<T> {
    let result = crate::selftest::until(view, cx, WITHIN, what, f).await;
    result.map_err(|e| {
        let state = view.read_with(cx, |v, _| format!("confirm {:?}, error {:?}", v.confirm.as_ref().map(|c| c.text.clone()), v.action_error));
        e.context(state)
    })
}

fn row(v: &FleetView, session_id: &str) -> Option<(String, Health)> {
    v.search_tree()
        .workspaces
        .into_iter()
        .flat_map(|w| w.supervised.into_iter().chain(w.repos.into_iter().flat_map(|r| r.sessions)))
        .find(|r| r.session.session_id == session_id)
        .map(|r| (r.session.name, r.health))
}

async fn delete_from_menu(view: &Entity<FleetView>, session_id: &str, name: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<Duration> {
    press_painted(view, &format!("session-row-{session_id}"), MouseButton::Right, cx).await?;
    let keys = menu_open(view, name, cx).await?;
    if keys.last() != Some(&"delete") {
        bail!("the menu of {name} has {keys:?}, Delete… not last");
    }
    click(view, "menu-delete", cx).await?;
    let text = until(view, cx, "the delete question", |v| v.confirm.as_ref().map(|c| c.text.clone())).await?;
    if !text.contains(name) || !text.starts_with("Delete ") {
        bail!("the question does not name {name}: {text}");
    }
    let asked = Instant::now();
    click(view, "confirm-yes", cx).await?;
    until(view, cx, &format!("{name} leaves the tree"), |v| row(v, session_id).is_none().then_some(())).await?;
    Ok(asked.elapsed())
}

pub async fn run(view: &Entity<FleetView>, home: &Path, own: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let saved = view.update(cx, |v, _| {
        v.transcripts_nudge.wake();
        std::mem::replace(&mut v.show_stopped, true)
    });
    let result = check(view, home, own, cx).await;
    view.update(cx, |v, cx| {
        v.show_stopped = saved;
        v.confirm = None;
        cx.notify();
    });
    result
}

async fn check(view: &Entity<FleetView>, home: &Path, own: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let main = crate::selftest::main_repo_of_gate()?;
    let dir = project(home, &main);
    let (file, sub) = (dir.join(format!("{DELETE_ME}.jsonl")), dir.join(DELETE_ME));
    until(view, cx, "the closed session to delete", |v| row(v, DELETE_ME).filter(|(n, _)| n == TITLE)).await?;
    let interactive = delete_from_menu(view, DELETE_ME, TITLE, cx).await?;
    if file.exists() || sub.exists() {
        bail!("after Delete {} exists: {}, {} exists: {}", file.display(), file.exists(), sub.display(), sub.exists());
    }
    for kept in [RECENT, OLD] {
        if !dir.join(format!("{kept}.jsonl")).exists() {
            bail!("Delete of {DELETE_ME} took {kept}'s transcript too");
        }
    }
    sleep(cx, Duration::from_millis(1500)).await;
    if view.read_with(cx, |v, _| row(v, DELETE_ME)).is_some() {
        bail!("{DELETE_ME} came back after a scan");
    }

    let session = until(view, cx, &format!("{own}'s row"), |v| {
        v.search_tree()
            .workspaces
            .into_iter()
            .flat_map(|w| w.repos.into_iter().flat_map(|r| r.sessions))
            .find(|r| r.session.name == own)
            .map(|r| r.session)
    })
    .await?;
    let sid = session.session_id.clone();
    crate::selftest::forget_own_transcript(&sid, &session.cwd);
    press_painted(view, &format!("session-row-{sid}"), MouseButton::Right, cx).await?;
    let keys = menu_open(view, own, cx).await?;
    if keys.contains(&"delete") || !keys.contains(&"stop-session") {
        bail!("the live {own}'s menu has {keys:?}");
    }
    click(view, "menu-stop-session", cx).await?;
    until(view, cx, &format!("{own} stopped"), |v| row(v, &sid).filter(|(_, h)| *h == Health::Stopped)).await?;
    let (fixture, fixture_dir) = plant(home, &session.cwd, &sid, own)?;
    let background = delete_from_menu(view, &sid, own, cx).await?;
    let listed = cx.background_executor().spawn(async { list_sessions() }).await.map_err(|e| anyhow!("claude agents: {e}"))?;
    if let Some(s) = listed.iter().find(|s| s.session_id == sid) {
        bail!("{own} is still listed by claude agents after Delete: {}", crate::selftest::raw(s));
    }
    if fixture.exists() || fixture_dir.exists() {
        bail!("{own}'s transcript {} or its dir is still there", fixture.display());
    }
    let error = view.read_with(cx, |v, _| v.action_error.clone());
    if let Some(e) = error {
        bail!("Delete left an error: {e}");
    }
    Ok(format!(
        "closed {DELETE_ME} deleted {} ms after Delete (transcript and {DELETE_ME}/ gone, {RECENT} and {OLD} kept); \
         live {own} offered no Delete, stopped from its menu, then rm + transcript and out of the tree {} ms after Delete, not in claude agents",
        interactive.as_millis(),
        background.as_millis()
    ))
}
