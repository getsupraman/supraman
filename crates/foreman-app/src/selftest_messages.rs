//! `--selftest` step 7n (m5/36): a Messages row's ×. Two `fm tell` notes from one sender are dropped into
//! the scratch workspace's outbox; × on the older hides it and the count drops by one, the newer stays,
//! `messages-dismissed.json` holds it (read back as a restart does) and the outbox file is still there.
//! Dismissing what is left hides the section; a newer note from the same sender shows it again, and is
//! dismissed in turn so later steps see no section.

use std::{
    path::Path,
    time::{Duration, Instant},
};

use anyhow::{anyhow, bail};
use foreman_core::outbox_dir;
use gpui::{AsyncWindowContext, Entity};

use crate::{
    dismissed_messages::{DismissedMessages, FILE},
    selftest::{WORKSPACE, click, sleep},
    selftest_manage::shot,
    view::FleetView,
};

const SENDER: &str = "fm-selftest-dismiss";
/// A poll reads the outbox every `REFRESH_EVERY` plus `claude agents`.
const WITHIN: Duration = Duration::from_secs(15);

async fn until<T>(view: &Entity<FleetView>, cx: &mut AsyncWindowContext, what: &str, f: impl Fn(&FleetView) -> Option<T>) -> anyhow::Result<T> {
    let started = Instant::now();
    loop {
        if let Some(t) = view.read_with(cx, |v, _| f(v)) {
            return Ok(t);
        }
        if started.elapsed() > WITHIN {
            bail!("{what}: not within {} s", WITHIN.as_secs());
        }
        sleep(cx, Duration::from_millis(50)).await;
    }
}

fn painted(v: &FleetView, key: &str) -> bool {
    v.hits.borrow().contains_key(key)
}

fn dismiss_key(file: &str) -> String {
    format!("msg-dismiss-{WORKSPACE}/{file}")
}

/// Off the panel, and until the m5/18 hold is over, so a row that arrives is drawn at once.
pub(crate) async fn pointer_off(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let right = view.read_with(cx, |v, _| v.left_width) + gpui::px(200.);
    crate::selftest_left::move_to(gpui::point(right, gpui::px(300.)), cx)?;
    until(view, cx, "the left panel's hold to end", |v| (!v.left_held()).then_some(())).await
}

fn tell(outbox: &Path, tag: &str, body: &str) -> anyhow::Result<String> {
    let file = format!("{:013}-selftest-dismiss-{tag}.json", foreman_claude::now_ms());
    let text = serde_json::json!({"from": "uds:selftest", "from_name": SENDER, "body": body}).to_string();
    std::fs::write(outbox.join(format!("{file}.tmp")), text)?;
    std::fs::rename(outbox.join(format!("{file}.tmp")), outbox.join(&file))?;
    Ok(file)
}

/// × on every Messages row left, until the section is gone.
async fn dismiss_all(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<usize> {
    let mut clicked = 0;
    loop {
        let next = view.read_with(cx, |v, _| v.hits.borrow().keys().find(|k| k.starts_with("msg-dismiss-")).cloned());
        let Some(key) = next else { break };
        if clicked == 30 {
            bail!("30 × clicked and the Messages rows keep coming");
        }
        click(view, &key, cx).await?;
        until(view, cx, &format!("{key} gone after its ×"), |v| (!painted(v, &key)).then_some(())).await?;
        clicked += 1;
    }
    Ok(clicked)
}

/// m5/36 c5: the empty search box painted with the placeholder colour Dark Modern's file gives
/// (`#FFFFFF56`, painted as is before m5/36) and with the fitted one, screenshotted with `FOREMAN_SHOTS`.
pub async fn placeholder(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    use foreman_ui::themes::{hex, parse_color, placeholder_contrast};
    use theme::{ActiveTheme as _, GlobalTheme};
    crate::selftest::scrolled_into_view(view, "search-box", cx).await?;
    let paint = |color: gpui::Hsla, cx: &mut AsyncWindowContext| {
        cx.update(|_, cx| {
            let mut theme = (**cx.theme()).clone();
            theme.styles.colors.text_placeholder = color;
            GlobalTheme::update_theme(cx, std::sync::Arc::new(theme));
            cx.refresh_windows();
        })
        .map_err(|e| anyhow!("{e}"))
    };
    let (fitted, ratios) = cx
        .update(|_, cx| {
            let theme = cx.theme();
            (theme.colors().text_placeholder, placeholder_contrast(theme, theme.colors().text_placeholder))
        })
        .map_err(|e| anyhow!("{e}"))?;
    let before = parse_color("#FFFFFF56").ok_or_else(|| anyhow!("#FFFFFF56"))?;
    paint(before, cx)?;
    shot("m5-36-search-placeholder-before", cx).await;
    paint(fitted, cx)?;
    shot("m5-36-search-placeholder-after", cx).await;
    Ok(format!("search box placeholder {} ({:.2}..{:.2}:1 on panel/editor), shot before (#FFFFFF56) and after", hex(fitted), ratios.0, ratios.1))
}

pub async fn run(view: &Entity<FleetView>, home: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let outbox = outbox_dir(home, WORKSPACE)?;
    std::fs::create_dir_all(&outbox)?;
    pointer_off(view, cx).await?;
    let older = tell(&outbox, "a", "fm-selftest ready — nothing in flight")?;
    sleep(cx, Duration::from_millis(20)).await;
    let newer = tell(&outbox, "b", "fm-selftest ready — still nothing in flight")?;
    let (older_x, newer_x) = (dismiss_key(&older), dismiss_key(&newer));
    let before = until(view, cx, "both notes painted with a ×", |v| {
        (painted(v, &older_x) && painted(v, &newer_x)).then(|| v.messages_count())
    })
    .await?;
    shot("m5-36-messages-before", cx).await;

    click(view, &older_x, cx).await?;
    let after = until(view, cx, "the older note hidden by its ×", |v| (!painted(v, &older_x)).then(|| v.messages_count())).await?;
    if after != before - 1 {
        bail!("× on {older}: the count went from {before} to {after}, not {}", before - 1);
    }
    if !view.read_with(cx, |v, _| painted(v, &newer_x)) {
        bail!("× on {older} also hid {newer}");
    }
    if !outbox.join(&older).is_file() {
        bail!("× on {older} removed its outbox file");
    }
    let (reloaded, notice) = DismissedMessages::load(Some(home));
    if let Some(notice) = notice {
        bail!("{FILE}: {notice}");
    }
    if !reloaded.hides(WORKSPACE, &older) || reloaded.hides(WORKSPACE, &newer) {
        bail!("{FILE} read back as a restart does: {:?}", std::fs::read_to_string(home.join(FILE)).unwrap_or_default());
    }
    // a poll later the dismissal holds
    let ticks = view.read_with(cx, |v, _| v.ticks);
    until(view, cx, "a poll after the ×", |v| (v.ticks > ticks).then_some(())).await?;
    if view.read_with(cx, |v, _| painted(v, &older_x)) {
        bail!("{older} came back on the next poll");
    }

    let rest = dismiss_all(view, cx).await?;
    until(view, cx, "the Messages section hidden once empty", |v| (!painted(v, "messages-area") && v.messages_count() == 0).then_some(())).await?;
    if !outbox.join(&older).is_file() || !outbox.join(&newer).is_file() {
        bail!("the dismissed notes' outbox files are gone");
    }

    pointer_off(view, cx).await?;
    let again = tell(&outbox, "c", "fm-selftest ready — a newer note")?;
    let again_x = dismiss_key(&again);
    let shown = until(view, cx, "a newer note from the same sender", |v| (painted(v, &again_x) && painted(v, "messages-area")).then(|| v.messages_count())).await?;
    if shown != 1 {
        bail!("after the newer note the count is {shown}, not 1");
    }
    shot("m5-36-messages-newer", cx).await;
    click(view, &again_x, cx).await?;
    until(view, cx, "the section hidden again", |v| (!painted(v, "messages-area")).then_some(())).await?;
    let on_disk = std::fs::read_to_string(home.join(FILE)).map_err(|e| anyhow!("{FILE}: {e}"))?;
    if !on_disk.contains(&again) {
        bail!("{FILE} does not hold {again}: {on_disk:?}");
    }

    Ok(format!(
        "× on the older of 2 notes from {SENDER}: count {before} → {after}, the newer still shown, {FILE} holds it after a reload and a poll, \
         outbox file kept; {rest} more × emptied the section and hid it; a newer note showed it again (count 1) and its × hid it"
    ))
}
