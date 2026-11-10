//! `--selftest` step 7i: a new shell's folder taken from the active tab, and its program's OSC 2 title on the tab and its hover card, read from a real pwsh.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context as _, anyhow, bail};
use supraman_ui::hover_card::LAST_CARD;
use gpui::{AsyncWindowContext, Entity, Modifiers, MouseMoveEvent, PlatformInput, point, px};

use crate::{
    selftest::{hex6, sleep, wait_for_tab_terminal},
    selftest_manage::shot,
    selftest_work::WORKSPACE,
    view::FleetView,
};

const WITHIN: Duration = Duration::from_secs(15);
const MARK: &str = "fm-cwd=";

pub(crate) async fn run(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let saved = view.update(cx, |v, _| std::mem::replace(&mut v.shell_args, vec!["-NoLogo".into(), "-NoProfile".into()]));
    let result = check(view, cx).await;
    view.update(cx, |v, _| v.shell_args = saved);
    let ids: Vec<u64> = view.read_with(cx, |v, _| v.tabs.iter().map(|t| t.id).collect());
    view.update(cx, |v, cx| ids.into_iter().for_each(|id| v.close_tab(id, cx)));
    result
}

/// A new shell opened with a Work tab active starts in that tab's workspace, and with no tab in the first workspace;
/// on the first one, an OSC 2 title becomes its label, its hover card names where it runs, and an empty one restores it.
async fn check(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    close_all(view, cx);
    let (root, first) = view.read_with(cx, |v, _| {
        let home = |w: &supraman_core::Workspace| w.home().map(Path::to_path_buf);
        (v.workspaces.iter().find(|w| w.name == WORKSPACE).and_then(home), v.workspaces.first().and_then(home))
    });
    let (root, first) = (root.ok_or_else(|| anyhow!("the registry has no {WORKSPACE}"))?, first.ok_or_else(|| anyhow!("no workspace"))?);
    if same(&root, &first) {
        bail!("{WORKSPACE} is the first workspace, so this step could not tell the two rules apart");
    }

    view.update(cx, |v, cx| v.open_work(WORKSPACE, cx));
    let id = cx.update(|window, cx| view.update(cx, |v, cx| v.new_shell(window, cx)))?;
    let terminal = wait_for_tab_terminal(view, id, WITHIN, cx).await?;
    // ConPTY's own title (pwsh's path) is no title: the label stays the tab's own
    until(cx, "the pwsh prompt", |cx| terminal.read_with(cx, |t, _| t.get_content()).contains("PS ").then_some(())).await?;
    let fallback = label(view, id, cx).ok_or_else(|| anyhow!("tab {id} has no label"))?;
    let in_work = cwd_of(&terminal, id, cx).await.context("with the Work tab active")?;
    if !same(Path::new(&in_work), &root) {
        bail!("with Work · {WORKSPACE} active the new shell is in {in_work}, not {}", root.display());
    }
    if label(view, id, cx).as_deref() != Some(fallback.as_str()) {
        bail!("before any OSC title the tab reads {:?}, not its own {fallback:?}", label(view, id, cx));
    }

    let name = format!("fm-osc-{}", hex6());
    let title = format!("\u{2733} {name}");
    terminal.update(cx, |t, _| {
        t.input(format!("Write-Host -NoNewline \"$([char]27)]2;$([char]0x2733)  {name}$([char]9)$([char]7)\"\r").into_bytes())
    });
    until(cx, &format!("the tab label turns to {title:?}"), |cx| (label(view, id, cx).as_deref() == Some(title.as_str())).then_some(())).await?;

    let card = hover_card(view, id, &title, cx).await?;
    let lines: Vec<&str> = card.text.lines().collect();
    let want = [
        format!("workspace  {WORKSPACE}"),
        format!("folder  {}", root.display()),
        "kind  shell".to_string(),
    ];
    for line in &want {
        if !lines.contains(&line.as_str()) {
            bail!("the card has no line {line:?}: {:?}", card.text);
        }
    }
    if !lines.iter().any(|l| l.starts_with("started  ")) {
        bail!("the card has no start time: {:?}", card.text);
    }
    if let Some(branch) = supraman_core::branch_at(&root) {
        if !lines.contains(&format!("branch  {branch}").as_str()) {
            bail!("{} is on {branch}, the card does not say so: {:?}", root.display(), card.text);
        }
    }
    shot("m5-29-tab-card", cx).await;

    terminal.update(cx, |t, _| t.input("Write-Host -NoNewline \"$([char]27)]2;$([char]7)\"\r".to_string().into_bytes()));
    until(cx, "an empty OSC 2 brings back the tab's own label", |cx| (label(view, id, cx).as_deref() == Some(fallback.as_str())).then_some(())).await?;

    close_all(view, cx);
    let no_tab = cx.update(|window, cx| view.update(cx, |v, cx| v.new_shell(window, cx)))?;
    let no_tab_terminal = wait_for_tab_terminal(view, no_tab, WITHIN, cx).await?;
    let in_first = cwd_of(&no_tab_terminal, no_tab, cx).await.context("with no tab")?;
    if !same(Path::new(&in_first), &first) {
        bail!("with no tab the new shell is in {in_first}, not the first workspace's {}", first.display());
    }
    let exe_dir = supraman_core::install::exe_dir().ok();
    if exe_dir.is_some_and(|d| same(&d, Path::new(&in_first)) || same(&d, Path::new(&in_work))) {
        bail!("a new shell started in the exe's folder");
    }

    Ok(format!(
        "a new shell with Work · {WORKSPACE} active in {in_work}, with no tab in the first workspace's {in_first}; OSC 2 {title:?} became the label; \
         hovered, the card showed after {} ms with {} lines; an empty title restored {fallback:?}",
        card.after.as_millis(),
        lines.len()
    ))
}

/// The cwd the shell in `terminal` prints.
async fn cwd_of(terminal: &Entity<supraman_term::Terminal>, id: u64, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    terminal.update(cx, |t, _| t.input(format!("Write-Host ('{MARK}' + (Get-Location).Path)\r").into_bytes()));
    until(cx, &format!("tab {id} printing its cwd"), |cx| {
        let content = terminal.read_with(cx, |t, _| t.get_content());
        // the echoed command has `'` right after the mark; the printed line has the path
        content.lines().filter_map(|l| l.trim().strip_prefix(MARK)).find(|p| !p.starts_with('\'')).map(|p| p.trim().to_string())
    })
    .await
}

fn close_all(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) {
    let ids: Vec<u64> = view.read_with(cx, |v, _| v.tabs.iter().map(|t| t.id).collect());
    view.update(cx, |v, cx| ids.into_iter().for_each(|id| v.close_tab(id, cx)));
}

fn same(a: &Path, b: &Path) -> bool {
    let key = |p: &Path| p.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_lowercase();
    key(a) == key(b)
}

struct Shown {
    text: String,
    after: Duration,
}

async fn hover_card(view: &Entity<FleetView>, id: u64, title: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<Shown> {
    let bounds = view
        .read_with(cx, |v, _| v.hits.borrow().get(&format!("tab-{id}")).map(|h| h.bounds))
        .ok_or_else(|| anyhow!("tab {id} was not painted"))?;
    // left of centre: off the close button
    let position = point(bounds.left() + px(12.), bounds.center().y);
    // away first, so the hover starts now
    move_to(point(px(1.), px(1.)), cx)?;
    crate::selftest::drawn(cx).await?;
    LAST_CARD.with(|l| l.borrow_mut().take());
    let started = Instant::now();
    loop {
        move_to(position, cx)?;
        sleep(cx, Duration::from_millis(50)).await;
        if let Some(text) = LAST_CARD.with(|l| l.borrow().clone()) {
            let after = started.elapsed();
            if text.lines().next() != Some(title) {
                bail!("the card's first line is not the title {title:?}: {text:?}");
            }
            return Ok(Shown { text, after });
        }
        if started.elapsed() > Duration::from_secs(5) {
            bail!("hovering tab {id} at {position:?} for 5 s showed no card");
        }
    }
}

fn move_to(position: gpui::Point<gpui::Pixels>, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    cx.update(|window, cx| {
        let event = MouseMoveEvent { position, pressed_button: None, modifiers: Modifiers::default() };
        window.dispatch_event(PlatformInput::MouseMove(event), cx);
    })
}

fn label(view: &Entity<FleetView>, id: u64, cx: &mut AsyncWindowContext) -> Option<String> {
    view.read_with(cx, |v, _| v.tab(id).map(|t| t.label()))
}

async fn until<T>(cx: &mut AsyncWindowContext, what: &str, check: impl FnMut(&mut AsyncWindowContext) -> Option<T>) -> anyhow::Result<T> {
    crate::selftest::until_with(cx, WITHIN, what, check).await
}
