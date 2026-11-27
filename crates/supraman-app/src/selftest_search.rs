//! `--selftest` step 7k: the sidebar search driven by Ctrl+Shift+E, keys posted through the window procedure and Esc, its rows painted, Up/Down and Enter picking a result; which sessions match is `search`'s tests.

use std::time::{Duration, Instant};

use anyhow::bail;
use gpui::{AsyncWindowContext, Entity, KeyDownEvent, Keystroke, Modifiers, PlatformInput};

use crate::{
    selftest::{GROUP, sleep, until_app},
    selftest_closed::RECENT,
    selftest_manage::post_keys,
    view::FleetView,
};

const WITHIN: Duration = Duration::from_secs(5);

fn query(v: &FleetView, cx: &gpui::App) -> String {
    v.search.read(cx).text().to_string()
}

async fn type_query(view: &Entity<FleetView>, text: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let keys: Vec<String> = text.chars().map(String::from).collect();
    post_keys(&keys.iter().map(String::as_str).collect::<Vec<_>>(), cx).await?;
    until_app(view, cx, WITHIN, &format!("the box reading {text:?}"), |v, cx| (query(v, cx) == text).then_some(()))
        .await
        .map_err(|e| e.context(format!("it reads {:?}", view.read_with(cx, |v, cx| query(v, cx)))))
}

async fn until_focused(view: &Entity<FleetView>, want: bool, what: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let started = Instant::now();
    while cx.update(|window, cx| view.read(cx).search.read(cx).is_focused(window))? != want {
        if started.elapsed() > Duration::from_secs(5) {
            bail!("{what}: not within 5 s");
        }
        sleep(cx, Duration::from_millis(50)).await;
    }
    Ok(())
}

pub async fn run(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let mut out = Vec::new();
    view.update(cx, |v, cx| {
        v.show_stopped = false;
        v.clear_search(cx);
    });
    let tree_row = format!("group-row-{GROUP}");
    until_app(view, cx, WITHIN, "the tree before the search", |v, _| v.shown(&tree_row).then_some(())).await?;

    // dispatched: a posted Ctrl is not in the keyboard state Windows reads
    let keystroke = Keystroke { modifiers: Modifiers { control: true, shift: true, ..Default::default() }, key: "e".into(), key_char: None };
    cx.update(|window, cx| window.dispatch_event(PlatformInput::KeyDown(KeyDownEvent { keystroke, is_held: false, prefer_character_input: false }), cx))?;
    until_focused(view, true, "the search box focused by Ctrl+Shift+E", cx).await?;
    out.push("Ctrl+Shift+E focuses the box".to_string());

    let prefix = &RECENT[..4];
    type_query(view, prefix, cx).await?;
    let recent = format!("session-row-{RECENT}");
    until_app(view, cx, WITHIN, &format!("a result for {prefix:?}"), |v, _| v.shown(&recent).then_some(())).await?;
    out.push(format!("{prefix:?} typed by posted keys finds {RECENT}"));

    post_keys(&["escape"], cx).await?;
    until_app(view, cx, WITHIN, "the box cleared by Esc and the tree back", |v, cx| {
        let hits = v.hits.borrow();
        let results = hits.keys().any(|k| k.starts_with("result-") || k == "search-summary" || k == "search-none");
        (query(v, cx).is_empty() && hits.contains_key(&tree_row) && !results).then_some(())
    })
    .await
    .map_err(|e| e.context(format!("box {:?}", view.read_with(cx, |v, cx| query(v, cx)))))?;
    if let Some(k) = view.read_with(cx, |v, _| v.hits.borrow().keys().find(|k| k.starts_with("session-id-")).cloned()) {
        bail!("{k} painted in the tree: a row shows its id only while a search matched it");
    }
    out.push("Esc clears it, the tree is back, no row shows its id".into());
    post_keys(&["escape"], cx).await?;
    until_focused(view, false, "a second Esc on the empty box giving the keys back to the window", cx).await?;
    out.push("a second Esc gives the keys back".into());
    out.push(pick_by_keys(view, prefix, &tree_row, cx).await?);
    Ok(out.join("; "))
}

/// Down and Up move the results' selection while the box keeps the keys and its text; Enter opens the selected as a click does.
async fn pick_by_keys(view: &Entity<FleetView>, prefix: &str, tree_row: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let keystroke = Keystroke { modifiers: Modifiers { control: true, shift: true, ..Default::default() }, key: "e".into(), key_char: None };
    cx.update(|window, cx| window.dispatch_event(PlatformInput::KeyDown(KeyDownEvent { keystroke, is_held: false, prefer_character_input: false }), cx))?;
    until_focused(view, true, "the search box focused again", cx).await?;
    type_query(view, prefix, cx).await?;
    let rows = until_app(view, cx, WITHIN, "two result rows", |v, _| Some(v.result_picks().0).filter(|rows| rows.len() >= 2)).await?;
    let picked = |want: String| move |v: &FleetView, cx: &gpui::App| {
        let (_, selected, held) = v.result_picks();
        (selected.as_deref() == Some(want.as_str()) && held && query(v, cx) == prefix).then_some(())
    };
    post_keys(&["down"], cx).await?;
    until_app(view, cx, WITHIN, "Down selecting the first result", picked(rows[0].clone()))
        .await
        .map_err(|e| e.context(format!("rows, selected, held: {:?}; the box reads {:?}", view.read_with(cx, |v, _| v.result_picks()), view.read_with(cx, |v, cx| query(v, cx)))))?;
    post_keys(&["down"], cx).await?;
    until_app(view, cx, WITHIN, "Down selecting the second", picked(rows[1].clone())).await?;
    post_keys(&["up"], cx).await?;
    until_app(view, cx, WITHIN, "Up back to the first", picked(rows[0].clone())).await?;
    let Some(at) = rows.iter().position(|k| k.starts_with("result-group-") || k.starts_with("result-repo-")) else { bail!("no group or folder among the results {rows:?}") };
    for _ in 0..at {
        post_keys(&["down"], cx).await?;
    }
    until_app(view, cx, WITHIN, &format!("Down to {}", rows[at]), picked(rows[at].clone())).await?;
    post_keys(&["enter"], cx).await?;
    until_app(view, cx, WITHIN, &format!("Enter opening {}: the search cleared and the tree back", rows[at]), |v, cx| {
        (query(v, cx).is_empty() && v.hits.borrow().contains_key(tree_row)).then_some(())
    })
    .await?;
    post_keys(&["escape"], cx).await?;
    until_focused(view, false, "Esc on the empty box giving the keys back", cx).await?;
    Ok(format!("Down, Down, Up select {} with the box keeping the keys and its text; Enter on {} opens it", rows[0], rows[at]))
}
