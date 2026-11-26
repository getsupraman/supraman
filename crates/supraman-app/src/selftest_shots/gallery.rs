//! `kit-<section>-<theme>.png`: the kit gallery (UI-KIT §10), each section at rest and in each of its probes' frames, in the dark default and the light selftest theme.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context as _, anyhow};
use gpui::{AppContext as _, AsyncWindowContext, Bounds, MouseButton, PlatformInput, Point, Pixels, TitlebarOptions, WindowBounds, WindowHandle, WindowOptions, point, size};
use supraman_ui::gallery::{Act, Gallery, Probe, Section, TITLE};

use super::{capture, home};

pub async fn shots(out: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let gallery = open(cx)?;
    let shown = shoot(gallery, out, cx).await;
    super::default_theme(&home(), cx);
    gallery.update(cx, |_, window, cx| crate::tray::close(window, cx)).ok();
    let closed = crate::selftest::until_with(cx, Duration::from_secs(10), "the kit gallery closed", |cx| gallery.read_with(cx, |_, _| ()).is_err().then_some(())).await;
    shown.and(closed)
}

fn open(cx: &mut AsyncWindowContext) -> anyhow::Result<WindowHandle<Gallery>> {
    cx.update(|_, cx| {
        let bounds = Bounds::centered(None, size(gpui::px(1240.), gpui::px(860.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions { title: Some(TITLE.into()), appears_transparent: false, traffic_light_position: None }),
            ..Default::default()
        };
        cx.open_window(options, |window, cx| cx.new(|cx| Gallery::new(window, cx)))
    })?
    .context("opening the kit gallery")
}

async fn shoot(gallery: WindowHandle<Gallery>, out: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    for theme in ["dark", "light"] {
        if theme == "light" && !super::light_theme(&home(), cx) {
            return Err(anyhow!("cannot plant the light selftest theme"));
        }
        for section in Section::ALL {
            gallery.update(cx, |g, window, cx| {
                g.show(section, cx);
                g.rest_focus(window, cx);
            })?;
            away(gallery, cx)?;
            if section == Section::Prompts {
                gallery.update(cx, |g, _, cx| g.show_toast(cx))?;
            }
            frame(gallery, out, &format!("kit-{}-{theme}.png", section.key()), cx).await?;
            for probe in section.probes() {
                probed(gallery, section, &probe, out, theme, cx).await?;
            }
        }
    }
    Ok(())
}

/// One probe's frame: the pointer over its sample (held down for a press, until its tooltip shows) or its focus sample holding the keys.
async fn probed(gallery: WindowHandle<Gallery>, section: Section, probe: &Probe, out: &Path, theme: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let name = format!("kit-{}-{}-{theme}.png", section.key(), probe.name);
    match &probe.act {
        Act::Hover(key) => {
            pointer(gallery, Pointer::Move(middle(gallery, key, cx)?), cx)?;
            frame(gallery, out, &name, cx).await?;
        }
        Act::Press(key) => {
            let at = middle(gallery, key, cx)?;
            pointer(gallery, Pointer::Move(at), cx)?;
            pointer(gallery, Pointer::Down(at), cx)?;
            frame(gallery, out, &name, cx).await?;
            away(gallery, cx)?;
            pointer(gallery, Pointer::Up(corner(gallery, cx)?), cx)?;
        }
        Act::Tooltip(key, text) => {
            supraman_ui::tooltip::LAST_SHOWN.with(|last| *last.borrow_mut() = None);
            pointer(gallery, Pointer::Move(middle(gallery, key, cx)?), cx)?;
            let shown = |_: &mut AsyncWindowContext| supraman_ui::tooltip::LAST_SHOWN.with(|last| (last.borrow().as_deref() == Some(text.as_str())).then_some(()));
            crate::selftest::until_with(cx, Duration::from_secs(10), &format!("{key}'s tooltip"), shown).await?;
            frame(gallery, out, &name, cx).await?;
        }
        Act::Focus(sample) => {
            if !gallery.update(cx, |g, window, cx| g.focus_sample(sample, window, cx))? {
                return Err(anyhow!("the gallery's {} has no focus sample {sample}", section.key()));
            }
            frame(gallery, out, &name, cx).await?;
        }
    }
    away(gallery, cx)?;
    gallery.update(cx, |g, window, cx| g.rest_focus(window, cx))?;
    Ok(())
}

/// The gallery drawn twice more (a hover's or focus's own frame, then what it laid out), then captured.
async fn frame(gallery: WindowHandle<Gallery>, out: &Path, name: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    for _ in 0..2 {
        crate::selftest::drawn_in(gallery.into(), cx).await.map_err(|e| anyhow!("kit gallery: {e}"))?;
    }
    let shot = capture(out.join(name), Some(TITLE), cx).await?;
    println!("shot: {}", shot.display());
    Ok(())
}

fn middle(gallery: WindowHandle<Gallery>, key: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<Point<Pixels>> {
    let hits = gallery.read_with(cx, |g, _| g.hits())?;
    let hit = hits.borrow().get(key).map(|h| h.bounds.center());
    hit.ok_or_else(|| anyhow!("the gallery painted no {key}"))
}

fn corner(gallery: WindowHandle<Gallery>, cx: &mut AsyncWindowContext) -> anyhow::Result<Point<Pixels>> {
    let size = gallery.update(cx, |_, window, _| window.viewport_size())?;
    Ok(point(size.width - gpui::px(2.), size.height - gpui::px(2.)))
}

/// The pointer to the window's bottom right corner, over no sample.
fn away(gallery: WindowHandle<Gallery>, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let at = corner(gallery, cx)?;
    pointer(gallery, Pointer::Move(at), cx)
}

enum Pointer {
    Move(Point<Pixels>),
    Down(Point<Pixels>),
    Up(Point<Pixels>),
}

fn pointer(gallery: WindowHandle<Gallery>, what: Pointer, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let modifiers = gpui::Modifiers::default();
    let input = match what {
        Pointer::Move(position) => PlatformInput::MouseMove(gpui::MouseMoveEvent { position, pressed_button: None, modifiers }),
        Pointer::Down(position) => PlatformInput::MouseDown(gpui::MouseDownEvent { button: MouseButton::Left, position, modifiers, click_count: 1, first_mouse: false }),
        Pointer::Up(position) => PlatformInput::MouseUp(gpui::MouseUpEvent { button: MouseButton::Left, position, modifiers, click_count: 1 }),
    };
    gallery.update(cx, |_, window, cx| {
        window.dispatch_event(input, cx);
    })?;
    Ok(())
}
