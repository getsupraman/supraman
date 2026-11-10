//! `--selftest` step 7n's messages: each note's × painted where a click lands on it, and the hold that keeps a note
//! back under a resting pointer (real mouse moves over the real sidebar); what a dismissal keeps and when a note shows
//! again are `dismissed_messages`' tests.

use std::{
    path::Path,
    time::{Duration, Instant},
};

use anyhow::{anyhow, bail};
use supraman_core::outbox_dir;
use gpui::{AsyncWindowContext, Bounds, Entity, Pixels};

use crate::{
    selftest::{WORKSPACE, click, scrolled_into_view, sleep, until},
    selftest_left::{bounds, move_to},
    selftest_manage::shot,
    view::FleetView,
};

const SENDER: &str = "fm-selftest-dismiss";
/// A poll reads the outbox every `REFRESH_EVERY` plus `claude agents`.
const WITHIN: Duration = Duration::from_secs(15);

fn painted(v: &FleetView, key: &str) -> bool {
    v.hits.borrow().contains_key(key)
}

fn dismiss_key(file: &str) -> String {
    format!("msg-dismiss-{WORKSPACE}/{file}")
}

/// Off the panel and past the hold, so a row that arrives is drawn at once.
pub(crate) async fn pointer_off(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let right = view.read_with(cx, |v, _| v.left_width) + gpui::px(200.);
    crate::selftest_left::move_to(gpui::point(right, gpui::px(300.)), cx)?;
    until(view, cx, WITHIN, "the left panel's hold to end", |v| (!v.left_held()).then_some(())).await
}

fn tell(outbox: &Path, tag: &str, body: &str) -> anyhow::Result<String> {
    crate::selftest::drop_outbox(outbox, SENDER, &format!("dismiss-{tag}"), body)
}

async fn dismiss_all(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<usize> {
    let mut clicked = 0;
    loop {
        let next = view.read_with(cx, |v, _| v.hits.borrow().keys().find(|k| k.starts_with("msg-dismiss-")).cloned());
        let Some(key) = next else { break };
        if clicked == 30 {
            bail!("30 × clicked and the Messages rows keep coming");
        }
        click(view, &key, cx).await?;
        until(view, cx, WITHIN, &format!("{key} gone after its ×"), |v| (!painted(v, &key)).then_some(())).await?;
        clicked += 1;
    }
    Ok(clicked)
}

pub async fn run(view: &Entity<FleetView>, home: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let outbox = outbox_dir(home, WORKSPACE)?;
    std::fs::create_dir_all(&outbox)?;
    pointer_off(view, cx).await?;
    let older = tell(&outbox, "a", "fm-selftest ready — nothing in flight")?;
    sleep(cx, Duration::from_millis(20)).await;
    let newer = tell(&outbox, "b", "fm-selftest ready — still nothing in flight")?;
    let (older_x, newer_x) = (dismiss_key(&older), dismiss_key(&newer));
    let before = until(view, cx, WITHIN, "both notes painted with a ×", |v| {
        (painted(v, &older_x) && painted(v, &newer_x)).then(|| v.messages_count())
    })
    .await?;
    shot("m5-36-messages-before", cx).await;

    click(view, &older_x, cx).await?;
    let after = until(view, cx, WITHIN, "the older note hidden by its ×", |v| (!painted(v, &older_x)).then(|| v.messages_count())).await?;
    if after != before - 1 {
        bail!("× on {older}: the count went from {before} to {after}, not {}", before - 1);
    }
    if !view.read_with(cx, |v, _| painted(v, &newer_x)) {
        bail!("× on {older} also hid {newer}");
    }
    let rest = dismiss_all(view, cx).await?;
    until(view, cx, WITHIN, "the Messages section hidden once empty", |v| (!painted(v, "messages-area") && v.messages_count() == 0).then_some(())).await?;

    Ok(format!("× on the older of 2 notes from {SENDER}: count {before} → {after}, the newer still shown; {rest} more × emptied the section and hid it"))
}

const TABLE: [&str; 5] = [
    "selftest\tproj\topen\tselftest\t—\t0/6 done",
    "  selftest-shout-textkit     run    open    shout: textkit.shout(text)    —    0/1 done",
    "    selftest-shout-textkit-fn  task  open  Add textkit.shout(text) with unit tests  ✗0/3",
    "  selftest-shout-textcli     run    open    shout: textcli greet --shout    —    0/1 done",
    "    selftest-shout-textcli-flag  task  open  greet --shout prints textkit.shout(greet(name))  ✗0/3",
];

pub async fn held(view: &Entity<FleetView>, home: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let outbox = outbox_dir(home, WORKSPACE)?;
    let key = format!("ws-row-{WORKSPACE}");
    let mut void = Vec::new();
    let (said, file, aimed) = loop {
        pointer_off(view, cx).await?;
        match held_arrival(view, &key, &outbox, cx).await? {
            Ok(done) => break done,
            Err(why) if void.len() < 2 => void.push(why),
            Err(why) => {
                void.push(why);
                bail!("the hold under the pointer was released and armed again before the note was read, in each of 3 tries: {}", void.join("; "));
            }
        }
    };
    let retaken = if void.is_empty() { String::new() } else { format!(" (retaken: {})", void.join("; ")) };

    let right = view.read_with(cx, |v, _| v.left_width) + gpui::px(200.);
    move_to(gpui::point(right, gpui::px(300.)), cx)?;
    let msg_key = until(view, cx, WITHIN, "the held note once the pointer left", |v| {
        (!v.left_held()).then(|| v.messages.iter().find_map(|(w, msgs)| msgs.iter().any(|m| m.file == file).then(|| format!("{w}/{file}")))).flatten()
    })
    .await?;
    let x = dismiss_key(&file);
    until(view, cx, WITHIN, "the held note painted once the pointer left", |v| painted(v, &x).then_some(())).await?;
    let moved = view.read_with(cx, |v, _| bounds(v, &key)).ok_or_else(|| anyhow!("{key} gone once the pointer left"))?;
    if moved == aimed {
        bail!("{key} stayed at y {:.1} after the held note appeared too: the arrival never moved it, so the hold was not tested", f32::from(aimed.top()));
    }
    let table = table_lines(view, &msg_key, cx).await?;

    click(view, &x, cx).await?;
    until(view, cx, WITHIN, "the section hidden after the table note's ×", |v| (!painted(v, "messages-area")).then_some(())).await?;
    pointer_off(view, cx).await?;
    Ok(format!("{said}{retaken} | pointer left: the note appeared and moved {key} to y {:.1}; {table}; its × hid the section", f32::from(moved.top())))
}

/// `Err` when the hold was armed again meanwhile: the panel saw the pointer leave (an OS mouse message, or a rest), which shows held rows by design.
async fn held_arrival(
    view: &Entity<FleetView>,
    key: &str,
    outbox: &Path,
    cx: &mut AsyncWindowContext,
) -> anyhow::Result<Result<(String, String, Bounds<Pixels>), String>> {
    let aimed = scrolled_into_view(view, key, cx).await?;
    let target = aimed.center();
    move_to(target, cx)?;
    let (armed, count) = view.read_with(cx, |v, _| (v.left_holds, v.messages_count()));
    let mut body = vec!["Plan for the selftest (a work_tree, not a question):".to_string()];
    body.extend(TABLE.iter().map(|l| l.to_string()));
    let file = tell(outbox, "table", &body.join("\n"))?;
    // a hand still aiming moves a little; the hold lasts while it does (view/left.rs HOLD_STILL)
    let started = Instant::now();
    let mut read_after = None;
    let mut nudge = 0.;
    while read_after.is_none_or(|at: Duration| started.elapsed() < at + Duration::from_millis(500)) {
        let (read, now, held, holds) =
            view.read_with(cx, |v, _| (v.messages_count() > count && v.messages.values().flatten().any(|m| m.file == file), bounds(v, key), v.left_held(), v.left_holds));
        if holds != armed {
            return Ok(Err(format!("{file} read {:?} into the wait, the hold armed {} more time(s)", read_after, holds - armed)));
        }
        let now = now.ok_or_else(|| anyhow!("{key} is no longer painted while the pointer rests on it"))?;
        if now != aimed {
            bail!(
                "{file} (read: {read}) moved {key} from y {:.1} to y {:.1} under the pointer (held: {held})",
                f32::from(aimed.top()),
                f32::from(now.top())
            );
        }
        if read {
            read_after.get_or_insert(started.elapsed());
        }
        if started.elapsed() > WITHIN {
            bail!("the app did not read {file} from the outbox within {} s", WITHIN.as_secs());
        }
        nudge = if nudge == 0. { 1. } else { 0. };
        move_to(gpui::point(target.x + gpui::px(nudge), target.y), cx)?;
        sleep(cx, Duration::from_millis(100)).await;
    }
    shot("m5-18-hover-hold", cx).await;
    let header = view.read_with(cx, |v, _| painted(v, "messages-area"));
    let said = format!(
        "pointer on {key} at y {:.1}; {file} read {} ms later and held back (Messages section {}), {key} still at y {:.1} 500 ms after",
        f32::from(aimed.top()),
        read_after.unwrap_or_default().as_millis(),
        if header { "showing older rows only" } else { "not shown" },
        f32::from(aimed.top())
    );
    Ok(Ok((said, file, aimed)))
}

async fn table_lines(view: &Entity<FleetView>, key: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    view.update(cx, |v, cx| {
        v.expanded.insert(key.to_string());
        cx.notify();
    });
    let prefix = format!("msg-table-{key}#");
    let lines = until(view, cx, WITHIN, "the table's lines painted", |v| {
        let hits = v.hits.borrow();
        let mut lines: Vec<(String, Bounds<Pixels>)> = hits.iter().filter(|(k, _)| k.starts_with(&prefix)).map(|(k, h)| (k.clone(), h.bounds)).collect();
        (lines.len() >= TABLE.len()).then(|| {
            lines.sort_by(|a, b| a.1.top().partial_cmp(&b.1.top()).unwrap_or(std::cmp::Ordering::Equal));
            lines
        })
    })
    .await?;
    scrolled_into_view(view, &lines[0].0, cx).await?;
    shot("m5-18-messages-table", cx).await;
    if lines.len() != TABLE.len() {
        bail!("{} table lines painted for {} rows: {:?}", lines.len(), TABLE.len(), lines.iter().map(|l| &l.0).collect::<Vec<_>>());
    }
    let heights: Vec<f32> = lines.iter().map(|(_, b)| f32::from(b.size.height)).collect();
    let (low, high) = heights.iter().fold((f32::MAX, 0f32), |(l, h), x| (l.min(*x), h.max(*x)));
    if high - low > 0.5 || high > 24. {
        bail!("table rows painted {heights:?} px high: a row wrapped");
    }
    for pair in lines.windows(2) {
        if (f32::from(pair[1].1.top() - pair[0].1.bottom())).abs() > 0.5 {
            bail!("table rows {} and {} are not consecutive lines", pair[0].0, pair[1].0);
        }
    }
    Ok(format!("its {}-row table painted {} lines, each {low:.1} px, unwrapped", TABLE.len(), lines.len()))
}
