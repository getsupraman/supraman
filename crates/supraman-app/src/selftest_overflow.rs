//! 30 tabs overflow the strip: they shrink to the minimum before any is cut off, the first and the last are reached by
//! the wheel and by the ⌄ list, and a drag scrolls at the edge. A unit test cannot: it needs a real window's layout,
//! wheel and drag; the arithmetic itself is `supraman_ui::tabs::fit`'s tests.

use std::time::Duration;

use anyhow::{anyhow, bail};
use gpui::{
    AsyncWindowContext, Entity, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PlatformInput, Pixels, Point, ScrollDelta,
    ScrollEvent, point, px,
};
use supraman_ui::hit::Hit;
use supraman_ui::tabs::{LIST, LIST_BUTTON, STRIP, TAB_MIN, tab_width};

use crate::{
    saved_tabs::SavedTab,
    selftest::{click, sleep, until},
    selftest_supervise::type_text,
    selftest_tabs::press,
    view::FleetView,
};

const COUNT: usize = 30;
const WITHIN: Duration = Duration::from_secs(5);
const NOTCH: f32 = 120.;

pub(crate) async fn run(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    close_all(view, cx).await?;
    let result = async {
        let (ids, shared) = open(view, cx).await?;
        let pinned = pinned(view, cx)?;
        let wheel = wheel(view, &ids, cx).await?;
        let listed = list(view, &ids, cx).await?;
        let dragged = drag(view, &ids, cx).await?;
        Ok(format!("{shared}; {pinned}; {wheel}; {listed}; {dragged}"))
    }
    .await;
    close_all(view, cx).await?;
    result
}

async fn close_all(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let open: Vec<u64> = view.read_with(cx, |v, _| v.tabs.iter().map(|t| t.id).collect());
    view.update(cx, |v, cx| open.into_iter().for_each(|id| v.close_tab(id, cx)));
    until(view, cx, WITHIN, "every tab closed", |v| v.tabs.is_empty().then_some(())).await
}

fn hit(view: &Entity<FleetView>, key: &str, cx: &mut AsyncWindowContext) -> Option<Hit> {
    view.read_with(cx, |v, _| v.hits.borrow().get(key).copied())
}

fn forget(view: &Entity<FleetView>, key: &str, cx: &mut AsyncWindowContext) {
    view.read_with(cx, |v, _| v.hits.borrow_mut().remove(key));
}

/// Painted again within a few frames after [`forget`].
async fn painted(view: &Entity<FleetView>, key: &str, cx: &mut AsyncWindowContext) -> bool {
    view.update(cx, |_, cx| cx.notify());
    sleep(cx, Duration::from_millis(300)).await;
    hit(view, key, cx).is_some()
}

fn visible(view: &Entity<FleetView>, id: u64, cx: &mut AsyncWindowContext) -> bool {
    hit(view, &format!("tab-{id}"), cx).is_some_and(|h| h.fully_visible())
}

/// One tab at a time up to [`COUNT`]: each opened tab is whole in view, and while `n` tabs fit at the minimum
/// every one of them is, at the room's even share.
async fn open(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<(Vec<u64>, String)> {
    let cwd = crate::selftest::home();
    let mut ids = Vec::new();
    let mut fitted = 0;
    for n in 1..=COUNT {
        let saved = SavedTab::Shell { title: format!("overflow {n:02}"), cwd: cwd.clone(), shell: None };
        let id = view.update(cx, |v, cx| v.open_placeholder(saved, cx));
        ids.push(id);
        let key = format!("tab-{id}");
        let seen = std::cell::RefCell::new(None);
        let settled = until(view, cx, WITHIN, &format!("tab {n} of {COUNT} painted whole at its share"), |v| {
            let hits = v.hits.borrow();
            let (room, tab) = (hits.get(STRIP)?.bounds.size.width, hits.get(&key)?);
            let want = tab_width(room, n);
            *seen.borrow_mut() = Some((room, want, *tab));
            ((tab.bounds.size.width - want).abs() <= supraman_ui::hit::SNAP && tab.fully_visible()).then_some((room, want))
        })
        .await;
        let (room, width) = settled.map_err(|e| anyhow!("{e:#}; last seen (room, share, tab): {:?}", seen.borrow()))?;
        if room >= TAB_MIN * n as f32 {
            fitted = n;
            if let Some(cut) = ids.iter().find(|id| !visible(view, **id, cx)) {
                bail!("{n} tabs of {width:?} fit a room of {room:?}, yet tab {cut} is cut off");
            }
        } else if width != TAB_MIN {
            bail!("{n} tabs overflow a room of {room:?} at {width:?}, not the minimum {TAB_MIN:?}");
        }
    }
    let room = hit(view, STRIP, cx).map(|h| h.bounds.size.width).unwrap_or_default();
    if fitted == 0 || fitted >= COUNT {
        bail!("the strip ({room:?}) never overflowed or never fitted: {fitted} of {COUNT} tabs fitted at the minimum");
    }
    forget(view, &format!("{STRIP}-fade-left"), cx);
    if !painted(view, &format!("{STRIP}-fade-left"), cx).await || visible(view, ids[0], cx) {
        bail!("{COUNT} tabs, the last opened: no fade at the left, or the first tab still whole in view");
    }
    crate::selftest_manage::shot("tv-overflow-strip", cx).await;
    Ok((ids, format!("up to {fitted} tabs shared {room:?} whole, then {COUNT} at {TAB_MIN:?}, each opened tab scrolled into view")))
}

fn pinned(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    for key in [LIST_BUTTON, "new-shell", "new-shell-menu"] {
        if !hit(view, key, cx).is_some_and(|h| h.fully_visible()) {
            bail!("{key} is not whole in view beside {COUNT} tabs: {:?}", hit(view, key, cx));
        }
    }
    Ok("⌄ list, + and shell ⌄ in view".into())
}

fn scroll(at: Point<Pixels>, dy: f32, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    cx.update(|window, cx| {
        window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position: at, pressed_button: None, modifiers: Modifiers::default() }), cx);
        let event = ScrollEvent { position: at, delta: ScrollDelta::Pixels(point(px(0.), px(dy))), ..Default::default() };
        window.dispatch_event(PlatformInput::Scroll(event), cx);
    })?;
    Ok(())
}

/// The vertical wheel over the strip scrolls it sideways to the first tab and back to the last; nothing is activated.
async fn wheel(view: &Entity<FleetView>, ids: &[u64], cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let at = hit(view, STRIP, cx).ok_or_else(|| anyhow!("the strip is not painted"))?.bounds.center();
    let active = view.read_with(cx, |v, _| v.active);
    let mut notches = Vec::new();
    for (target, dy) in [(ids[0], NOTCH), (ids[COUNT - 1], -NOTCH)] {
        let mut n = 0;
        while !visible(view, target, cx) {
            if n == 60 {
                bail!("tab {target} not whole in view after {n} wheel notches: {:?}", hit(view, &format!("tab-{target}"), cx));
            }
            scroll(at, dy, cx)?;
            n += 1;
            sleep(cx, Duration::from_millis(40)).await;
        }
        notches.push(n);
    }
    if view.read_with(cx, |v, _| v.active) != active {
        bail!("the wheel changed the active tab from {active:?}");
    }
    Ok(format!("the wheel reached the first tab in {} notches and the last in {}", notches[0], notches[1]))
}

/// The ⌄ list from its button: every tab listed from the first, the active one marked, a query narrowing it, a click
/// and Enter each activating and revealing their tab.
async fn list(view: &Entity<FleetView>, ids: &[u64], cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let (first, last) = (ids[0], ids[COUNT - 1]);
    let row = |id: u64| format!("{LIST}-row-{id}");
    for id in ids {
        forget(view, &row(*id), cx);
    }
    forget(view, LIST, cx);
    click(view, LIST_BUTTON, cx).await?;
    until(view, cx, WITHIN, "the ⌄ list open", |v| v.hits.borrow().contains_key(LIST).then_some(())).await?;
    let listed = ids.iter().take_while(|id| hit(view, &row(**id), cx).is_some()).count();
    if listed < 5 {
        bail!("the ⌄ list paints {listed} rows from the first tab on");
    }
    type_text(cx, "30")?;
    let active = format!("{LIST}-active-{last}");
    until(view, cx, WITHIN, "the active tab marked in the list", |v| v.hits.borrow().contains_key(&active).then_some(())).await?;
    crate::selftest_manage::shot("tv-overflow-list", cx).await;

    press(cx, "backspace", Modifiers::default())?;
    press(cx, "backspace", Modifiers::default())?;
    forget(view, &row(first), cx);
    type_text(cx, "01")?;
    until(view, cx, WITHIN, "the first tab listed for \"01\"", |v| v.hits.borrow().contains_key(&row(first)).then_some(())).await?;
    click(view, &row(first), cx).await?;
    until(view, cx, WITHIN, "the first tab active and whole in view", |v| {
        let shown = v.hits.borrow().get(&format!("tab-{first}")).is_some_and(|h| h.fully_visible());
        (v.active == Some(first) && shown).then_some(())
    })
    .await?;
    forget(view, LIST, cx);
    if painted(view, LIST, cx).await {
        bail!("the ⌄ list stayed open after its row was clicked");
    }

    click(view, LIST_BUTTON, cx).await?;
    until(view, cx, WITHIN, "the ⌄ list open again", |v| v.hits.borrow().contains_key(LIST).then_some(())).await?;
    type_text(cx, "30")?;
    sleep(cx, Duration::from_millis(200)).await;
    press(cx, "enter", Modifiers::default())?;
    until(view, cx, WITHIN, "the last tab active and whole in view", |v| {
        let shown = v.hits.borrow().get(&format!("tab-{last}")).is_some_and(|h| h.fully_visible());
        (v.active == Some(last) && shown).then_some(())
    })
    .await?;
    Ok(format!("the ⌄ list painted {listed} rows from the first, marked the active tab; a click reached the first, Enter the last"))
}

/// The last tab dragged to the strip's left edge scrolls it to the first, and dropped there it takes the first place.
async fn drag(view: &Entity<FleetView>, ids: &[u64], cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let (first, last) = (ids[0], ids[COUNT - 1]);
    let from = hit(view, &format!("tab-{last}"), cx).filter(|h| h.fully_visible()).ok_or_else(|| anyhow!("tab {last} not in view to drag"))?.bounds.center();
    let strip = hit(view, STRIP, cx).ok_or_else(|| anyhow!("the strip is not painted"))?.bounds;
    let edge = point(strip.left() + px(4.), from.y);
    let none = Modifiers::default();
    let moved = |position: Point<Pixels>, cx: &mut AsyncWindowContext| {
        cx.update(|window, cx| {
            window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position, pressed_button: Some(MouseButton::Left), modifiers: none }), cx);
        })
    };
    cx.update(|window, cx| {
        window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position: from, pressed_button: None, modifiers: none }), cx);
        window.dispatch_event(PlatformInput::MouseDown(MouseDownEvent { button: MouseButton::Left, position: from, modifiers: none, click_count: 1, first_mouse: false }), cx);
    })?;
    for step in 1..=8 {
        let t = step as f32 / 8.;
        moved(point(from.x + (edge.x - from.x) * t, from.y), cx)?;
    }
    let mut moves = 0;
    while !visible(view, first, cx) {
        if moves == 400 {
            break;
        }
        moved(point(edge.x + px((moves % 2) as f32), edge.y), cx)?;
        moves += 1;
        sleep(cx, Duration::from_millis(16)).await;
    }
    let onto = hit(view, &format!("tab-{first}"), cx).map(|h| h.bounds.center());
    let reached = visible(view, first, cx);
    let to = onto.filter(|_| reached).unwrap_or(edge);
    moved(to, cx)?;
    sleep(cx, Duration::from_millis(100)).await;
    cx.update(|window, cx| {
        window.dispatch_event(PlatformInput::MouseUp(MouseUpEvent { button: MouseButton::Left, position: to, modifiers: none, click_count: 1 }), cx);
    })?;
    sleep(cx, Duration::from_millis(100)).await;
    if !reached {
        bail!("dragging tab {last} at the strip's left edge for {moves} moves never scrolled tab {first} into view");
    }
    let order: Vec<u64> = view.read_with(cx, |v, _| v.tabs.iter().map(|t| t.id).collect());
    if order.first() != Some(&last) || order.get(1) != Some(&first) {
        bail!("tab {last} dropped on tab {first} after the edge scroll: the order starts {:?}", &order[..3.min(order.len())]);
    }
    Ok(format!("a drag at the left edge scrolled to the first tab in {moves} moves and dropped the last tab first"))
}
