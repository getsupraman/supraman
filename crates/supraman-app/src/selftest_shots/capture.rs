//! A shot's capture, checked: PrintWindow at times catches a frame with most of its text, avatars and icons not yet
//! drawn, which reads as a broken UI. A frame whose sampled text shows no ink is taken again after one more frame.

use std::path::PathBuf;

use anyhow::bail;
use gpui::AsyncWindowContext;

use crate::selftest::drawn;
use crate::view::FleetView;

/// Text hit targets sampled per main-window shot.
const PROBES: usize = 6;
/// Captures of one shot before it fails.
const TAKES: usize = 3;

/// A rectangle of the captured frame, in its device pixels.
#[derive(Debug, Clone, Copy)]
struct Rect {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

/// A PNG of this process's main window, or its window titled `title`, as the window paints itself; a main-window frame
/// whose sampled text did not paint is taken again after another drawn frame, and after the last take the step fails.
pub(crate) async fn capture(out: PathBuf, title: Option<&str>, cx: &mut AsyncWindowContext) -> anyhow::Result<PathBuf> {
    for take in 1..=TAKES {
        let probes = if title.is_none() { text_probes(cx) } else { Vec::new() };
        let owned = title.map(str::to_string);
        let frame = cx
            .background_executor()
            .spawn(async move {
                let hwnd = crate::selftest::win::window(owned.as_deref())?;
                crate::selftest::win::print_window(hwnd)
            })
            .await?;
        let inked = probes.iter().filter(|r| has_ink(&frame, **r)).count();
        if inked * 2 >= probes.len() {
            if let Some(dir) = out.parent() {
                std::fs::create_dir_all(dir)?;
            }
            frame.save(&out)?;
            return Ok(out);
        }
        println!("shot: {} caught {inked} of {} text samples painted (take {take})", out.display(), probes.len());
        if take < TAKES {
            drawn(cx).await?;
        }
    }
    bail!("{}: {TAKES} captures caught a frame whose text had not painted", out.display())
}

/// Up to [`PROBES`] fully visible letters' hit targets of the main window's last frame, in device pixels.
fn text_probes(cx: &mut AsyncWindowContext) -> Vec<Rect> {
    cx.update(|window, cx| {
        let scale = window.scale_factor();
        let Some(Some(view)) = window.root::<FleetView>() else { return Vec::new() };
        let hits = view.read(cx).hits.borrow();
        let mut keys: Vec<&String> = hits.keys().filter(|k| k.ends_with("-label") || k.ends_with("-name") || k.ends_with("-text")).collect();
        keys.sort();
        keys.into_iter()
            .filter_map(|k| hits.get(k))
            .filter(|h| h.fully_visible() && f32::from(h.bounds.size.width) >= 12. && f32::from(h.bounds.size.height) >= 8.)
            .take(PROBES)
            .map(|h| {
                let at = |v: gpui::Pixels| (f32::from(v) * scale).max(0.) as u32;
                Rect { x: at(h.bounds.left()), y: at(h.bounds.top()), w: at(h.bounds.size.width), h: at(h.bounds.size.height) }
            })
            .collect()
    })
    .unwrap_or_default()
}

/// Whether `rect` of `frame` holds pixels apart from its commonest colour, its background: letters drawn on it.
fn has_ink(frame: &image::RgbaImage, rect: Rect) -> bool {
    let (fw, fh) = frame.dimensions();
    let (x1, y1) = ((rect.x + rect.w).min(fw), (rect.y + rect.h).min(fh));
    let mut counts = std::collections::HashMap::<[u8; 3], u32>::new();
    for y in rect.y.min(y1)..y1 {
        for x in rect.x.min(x1)..x1 {
            let p = frame.get_pixel(x, y).0;
            *counts.entry([p[0], p[1], p[2]]).or_default() += 1;
        }
    }
    let total: u32 = counts.values().sum();
    let Some((&background, _)) = counts.iter().max_by_key(|(_, n)| **n) else { return false };
    let far = |c: &[u8; 3]| c.iter().zip(background).map(|(a, b)| a.abs_diff(b) as u32).sum::<u32>() > 48;
    let inked: u32 = counts.iter().filter(|(c, _)| far(c)).map(|(_, n)| *n).sum();
    inked >= 3.max(total / 100)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_on_a_fill_are_ink_and_a_bare_fill_is_not() {
        let mut frame = image::RgbaImage::from_pixel(40, 20, image::Rgba([30, 30, 30, 255]));
        let rect = Rect { x: 5, y: 5, w: 20, h: 10 };
        assert!(!has_ink(&frame, rect), "a bare fill");
        for x in 8..18 {
            frame.put_pixel(x, 9, image::Rgba([220, 220, 220, 255]));
        }
        assert!(has_ink(&frame, rect), "a stroke of letters");
        assert!(!has_ink(&frame, Rect { x: 30, y: 30, w: 10, h: 10 }), "outside the frame");
    }
}
