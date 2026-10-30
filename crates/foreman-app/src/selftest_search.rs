use std::time::{Duration, Instant};

use anyhow::bail;
use gpui::{AsyncWindowContext, Entity, KeyDownEvent, Keystroke, Modifiers, PlatformInput};

use crate::{
    selftest::{WORKSPACE, sleep, until_app},
    selftest_closed::{OLD, RECENT},
    selftest_manage::{post_keys, shot},
    view::FleetView,
};

const WITHIN: Duration = Duration::from_secs(5);

fn result_rows(v: &FleetView) -> Vec<String> {
    let mut rows: Vec<String> = v
        .hits
        .borrow()
        .keys()
        .filter(|k| k.starts_with("result-ws-") || k.starts_with("result-repo-") || k.starts_with("session-row-"))
        .cloned()
        .collect();
    rows.sort();
    rows
}

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

/// The search box (`render_search_box`: 8 px beside, 4 px above) holds the field, whose text starts past its border and padding.
fn text_inset(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    use foreman_ui::text_input::{PAD_Y, TEXT_LEFT};
    let (outer, text) = view.read_with(cx, |v, cx| (v.hits.borrow().get("search-box").map(|h| h.bounds), v.search.read(cx).text_bounds()));
    let (Some(outer), Some(text)) = (outer, text) else {
        bail!("the search box or its text was not painted");
    };
    let (left, top) = (f32::from(text.left() - outer.left()), f32::from(text.top() - outer.top()));
    let (want_left, want_top) = (8. + f32::from(TEXT_LEFT), 4. + 1. + f32::from(PAD_Y));
    if left + 1. < want_left || top + 1. < want_top {
        bail!("the search text starts {left} px in and {top} px down in the search box, not {want_left} and {want_top}");
    }
    Ok(format!("the search text starts {left} px in and {top} px down in the search box"))
}

fn select_all(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    cx.update(|window, cx| view.update(cx, |v, cx| v.focus_search(window, cx)))?;
    Ok(())
}

pub async fn run(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let mut out = Vec::new();
    view.update(cx, |v, cx| {
        v.show_stopped = false;
        v.clear_search(cx);
    });
    let tree_row = format!("ws-row-{WORKSPACE}");
    until_app(view, cx, WITHIN, "the tree before the search", |v, _| v.hits.borrow().contains_key(&tree_row).then_some(())).await?;

    // dispatched: a posted Ctrl is not in the keyboard state Windows reads
    let keystroke = Keystroke { modifiers: Modifiers { control: true, shift: true, ..Default::default() }, key: "e".into(), key_char: None };
    cx.update(|window, cx| window.dispatch_event(PlatformInput::KeyDown(KeyDownEvent { keystroke, is_held: false, prefer_character_input: false }), cx))?;
    until_focused(view, true, "the search box focused by Ctrl+Shift+E", cx).await?;
    out.push("Ctrl+Shift+E focuses the box".to_string());

    // an id prefix: RECENT only (OLD = 01d05e1f… has the digits inside, not at its start)
    let prefix = &RECENT[..4];
    let started = Instant::now();
    type_query(view, prefix, cx).await?;
    let want = vec![format!("result-repo-{WORKSPACE}/"), format!("result-ws-{WORKSPACE}"), format!("session-row-{RECENT}")];
    let rows = until_app(view, cx, WITHIN, &format!("the results of {prefix:?}"), |v, _| {
        let rows = result_rows(v);
        let shape: Vec<String> = rows.iter().map(|r| if r.starts_with("result-repo-") { format!("result-repo-{WORKSPACE}/") } else { r.clone() }).collect();
        (shape.len() == 3 && shape.iter().all(|r| want.contains(r))).then_some(rows)
    })
    .await
    .map_err(|e| e.context(format!("rows painted {:?}", view.read_with(cx, |v, _| result_rows(v)))))?;
    if rows.iter().any(|r| r.contains(OLD)) {
        bail!("{prefix:?} found {OLD}, whose id only holds it further in: {rows:?}");
    }
    let summary = view.read_with(cx, |v, cx| v.search_results(cx).summary());
    out.push(format!("{prefix:?} typed by posted keys: {rows:?} ({summary}) in {} ms, its id marked on the row, stopped sessions hidden in the tree", started.elapsed().as_millis()));
    let id_key = format!("session-id-{}", &RECENT[..8]);
    until_app(view, cx, WITHIN, "the matched id shown on the result row", |v, _| v.hits.borrow().contains_key(&id_key).then_some(())).await?;
    shot("m5-30-search-id", cx).await;
    out.push(text_inset(view, cx)?);

    select_all(view, cx)?;
    type_query(view, "closed session", cx).await?;
    let recent = format!("session-row-{RECENT}");
    until_app(view, cx, WITHIN, "the closed session found by its title", |v, _| {
        let rows = result_rows(v);
        (rows.contains(&recent) && rows.iter().filter(|r| r.starts_with("session-row-")).count() == 1).then_some(())
    })
    .await
    .map_err(|e| e.context(format!("rows painted {:?}", view.read_with(cx, |v, _| result_rows(v)))))?;
    out.push("\"closed session\" finds it by its title".into());

    select_all(view, cx)?;
    type_query(view, WORKSPACE, cx).await?;
    let ws = format!("result-ws-{WORKSPACE}");
    let marks = until_app(view, cx, WITHIN, "the workspace found by its name", |v, cx| {
        let found = v.hits.borrow().contains_key(&ws);
        let r = v.search_results(cx);
        found.then(|| r.workspaces.iter().find(|w| w.name == WORKSPACE).map(|w| w.name_marks.clone())).flatten()
    })
    .await?;
    if marks != [0..WORKSPACE.len()] {
        bail!("{WORKSPACE} found with its name marked at {marks:?}");
    }
    out.push(format!("{WORKSPACE:?} finds the workspace, its name marked"));

    select_all(view, cx)?;
    type_query(view, "zzqx9", cx).await?;
    until_app(view, cx, WITHIN, "No results", |v, _| (v.hits.borrow().contains_key("search-none") && result_rows(v).is_empty()).then_some(())).await?;
    out.push("\"zzqx9\": No results".into());

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
