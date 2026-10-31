use std::time::{Duration, Instant};

use anyhow::{anyhow, bail};
use supraman_ui::hover_card::LAST_CARD;
use gpui::{AsyncWindowContext, Entity, Modifiers, MouseMoveEvent, PlatformInput, point, px};

use crate::{
    selftest::{hex6, sleep, wait_for_tab_terminal},
    selftest_manage::shot,
    selftest_work::WORKSPACE,
    view::FleetView,
};

const WITHIN: Duration = Duration::from_secs(15);

pub(crate) async fn run(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let saved = view.update(cx, |v, _| std::mem::replace(&mut v.shell_args, vec!["-NoLogo".into(), "-NoProfile".into()]));
    let result = check(view, cx).await;
    view.update(cx, |v, _| v.shell_args = saved);
    let ids: Vec<u64> = view.read_with(cx, |v, _| v.tabs.iter().map(|t| t.id).collect());
    view.update(cx, |v, cx| ids.into_iter().for_each(|id| v.close_tab(id, cx)));
    result
}

async fn check(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let root = view
        .read_with(cx, |v, _| v.workspaces.iter().find(|w| w.name == WORKSPACE).and_then(|w| w.home().map(std::path::Path::to_path_buf)))
        .ok_or_else(|| anyhow!("the registry has no {WORKSPACE}"))?;
    let fallback = format!("fm-title-{}", hex6());
    let id = cx.update(|window, cx| view.update(cx, |v, cx| v.open_shell_in(root.clone(), fallback.clone(), window, cx)))?;
    let terminal = wait_for_tab_terminal(view, id, WITHIN, cx).await?;

    // pwsh's prompt is up once it echoes; ConPTY's own title (pwsh's path) is no title
    sleep(cx, Duration::from_millis(1500)).await;
    if let Some(shown) = label(view, id, cx).filter(|l| *l != fallback) {
        bail!("before any OSC title the tab reads {shown:?}, not its own {fallback:?}");
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

    Ok(format!(
        "OSC 2 {title:?} became the label; hovered, the card showed after {} ms with {} lines; an empty title restored {fallback:?}",
        card.after.as_millis(),
        lines.len()
    ))
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
    sleep(cx, Duration::from_millis(100)).await;
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
            if after < Duration::from_millis(400) {
                bail!("the card showed {} ms after the pointer came, not after ~500 ms", after.as_millis());
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
