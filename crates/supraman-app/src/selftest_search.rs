//! `--selftest` step 7k: the sidebar search driven by Ctrl+Shift+E, keys posted through the window procedure and Esc, its rows painted; which sessions match is `search`'s tests.

use std::time::{Duration, Instant};

use anyhow::bail;
use gpui::{AsyncWindowContext, Entity, KeyDownEvent, Keystroke, Modifiers, PlatformInput};

use crate::{
    selftest::{WORKSPACE, sleep, until_app},
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
    let tree_row = format!("ws-row-{WORKSPACE}");
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
    Ok(out.join("; "))
}
