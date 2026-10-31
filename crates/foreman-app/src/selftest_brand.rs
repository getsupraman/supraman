//! `--selftest` step 7z: the product as the user reads it. Windows OCR of the main window and of every Settings
//! page finds "Supraman" and never the word "Foreman" (a path such as `...\Foreman\` is a name on disk, ren/2).

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, bail};
use gpui::{AsyncWindowContext, Entity, px};

use crate::selftest::{cargo_root, click, sleep, until};
use crate::selftest_left::shown_and_painted;
use crate::selftest_manage::capture;
use crate::settings_window::page;
use crate::view::FleetView;

const WITHIN: Duration = Duration::from_secs(15);
const SETTINGS: &str = "Supraman Settings";
const PAGES: [&str; 5] = [page::SESSIONS, page::WORKSTATE, page::PLUGINS, page::APPEARANCE, page::ABOUT];

pub(crate) async fn run(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let result = check(view, cx).await;
    let _ = cx.update(|window, _| {
        window.set_rem_size(px(16.));
        window.refresh();
    });
    view.update(cx, |v, cx| v.close_settings(cx));
    result
}

async fn check(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<String> {
    let dir = cargo_root().join("target").join("selftest-brand");
    let _ = std::fs::remove_dir_all(&dir);
    let tip = view.read_with(cx, |v, _| v.tray_tooltip());
    if !tip.starts_with("Supraman") {
        bail!("the tray tooltip reads {tip:?}");
    }

    let main = scan(&dir, "main-100", None, cx).await?;
    need(&main, "Supraman", "the main window")?;
    cx.update(|window, _| {
        window.set_rem_size(px(24.));
        window.refresh();
    })?;
    sleep(cx, Duration::from_millis(400)).await;
    scan(&dir, "main-150", None, cx).await?;
    cx.update(|window, _| {
        window.set_rem_size(px(16.));
        window.refresh();
    })?;

    click(view, "bar-settings", cx).await?;
    until(view, cx, WITHIN, "Settings opens its window", |v| v.settings_window).await?;
    let mut read = Vec::new();
    for id in PAGES {
        view.update(cx, |v, cx| v.open_settings_at(Some(id), cx));
        let handle = view.read_with(cx, |v, _| v.settings_window).ok_or_else(|| anyhow!("the settings window closed"))?;
        let shown = handle.read_with(cx, |s, _| s.page()).map_err(|e| anyhow!("settings window: {e}"))?;
        if shown != id {
            bail!("open_settings_at({id}) left the settings window on {shown:?}");
        }
        shown_and_painted(handle, cx).await?;
        sleep(cx, Duration::from_millis(300)).await;
        let words = scan(&dir, &format!("settings-{id}"), Some(SETTINGS), cx).await?;
        match id {
            page::ABOUT => need(&words, "Supraman", "Settings > About")?,
            page::WORKSTATE => need(&words, "Workman", "Settings > Workman")?,
            _ => {}
        }
        read.push(format!("{id} {} words", words.len()));
    }
    view.update(cx, |v, cx| v.open_settings_at(Some(page::WORKSTATE), cx));
    let handle = view.read_with(cx, |v, _| v.settings_window).ok_or_else(|| anyhow!("the settings window closed"))?;
    let zoom = |rem: f32, cx: &mut AsyncWindowContext| {
        handle
            .update(cx, |_, window, _| {
                window.set_rem_size(px(rem));
                window.refresh();
            })
            .map_err(|e| anyhow!("settings window: {e}"))
    };
    zoom(24., cx)?;
    sleep(cx, Duration::from_millis(400)).await;
    let zoomed = scan(&dir, "settings-workstate-150", Some(SETTINGS), cx).await;
    zoom(16., cx)?;
    need(&zoomed?, "Workman", "Settings > Workman at rem 24 px")?;
    view.update(cx, |v, cx| v.close_settings(cx));
    tray_shot(&dir, cx).await;
    Ok(format!(
        "tray tooltip {tip:?}; OCR of the main window ({} words, again at rem 24 px) and of Settings ({}) reads Supraman, Workman and no Foreman",
        main.len(),
        read.join(", ")
    ))
}

/// Captures this process's window, keeps a copy under `FOREMAN_SHOTS` as `ren-1-<name>.png`, and OCRs it.
async fn scan(dir: &Path, name: &str, title: Option<&str>, cx: &mut AsyncWindowContext) -> anyhow::Result<Vec<String>> {
    let png = capture(dir.join(format!("{name}.png")), title, cx).await?;
    keep(&png, name);
    let words = {
        let png = png.clone();
        cx.background_executor().spawn(async move { ocr::words(&png) }).await?
    };
    if let Some(hit) = words.iter().find(|w| names_foreman(w)) {
        bail!("OCR of {} reads {hit:?}: {}", png.display(), words.join(" "));
    }
    Ok(words)
}

fn need(words: &[String], word: &str, what: &str) -> anyhow::Result<()> {
    if words.iter().any(|w| w.contains(word)) { Ok(()) } else { bail!("OCR of {what} does not read {word}: {}", words.join(" ")) }
}

fn keep(png: &Path, name: &str) {
    if let Some(shots) = std::env::var_os("FOREMAN_SHOTS").map(PathBuf::from) {
        let out = shots.join(format!("ren-1-{name}.png"));
        match std::fs::create_dir_all(&shots).and_then(|_| std::fs::copy(png, &out)) {
            Ok(_) => println!("shot: {}", out.display()),
            Err(e) => println!("shot: {} failed: {e}", out.display()),
        }
    }
}

/// The product name as a word: not a path, a file name or an env name (`FOREMAN_HOME`, its `_` lost to OCR).
fn names_foreman(word: &str) -> bool {
    if word.contains(['\\', '/']) {
        return false;
    }
    let bare = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '’');
    ["Foreman", "Foreman's", "Foreman’s"].contains(&bare)
}

/// The notification-area icon as the taskbar paints it, kept under `FOREMAN_SHOTS` only.
async fn tray_shot(dir: &Path, cx: &mut AsyncWindowContext) {
    if std::env::var_os("FOREMAN_SHOTS").is_none() {
        return;
    }
    let Some((rect, light)) = crate::tray_state() else {
        println!("shot: ren-1-tray failed: no tray icon rect");
        return;
    };
    let name = if light { "tray-on-light" } else { "tray-on-dark" };
    let pad = 24;
    let area = [rect.position.x as i32 - pad, rect.position.y as i32 - pad, rect.size.width as i32 + 2 * pad, rect.size.height as i32 + 2 * pad];
    match crate::selftest_manage::capture_screen(dir.join(format!("{name}.png")), area, cx).await {
        Ok(png) => keep(&png, name),
        Err(e) => println!("shot: ren-1-{name} failed: {e:#}"),
    }
}

#[cfg(windows)]
mod ocr {
    use std::path::Path;

    use anyhow::Context as _;
    use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
    use windows::Media::Ocr::OcrEngine;
    use windows::Storage::Streams::DataWriter;
    use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};

    /// Small UI text reads better twice as large; the engine refuses sides over `MaxImageDimension`.
    pub fn words(png: &Path) -> anyhow::Result<Vec<String>> {
        // SAFETY: joins this thread to the MTA; a thread already in it answers S_FALSE or RPC_E_CHANGED_MODE.
        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
        let image = image::open(png).with_context(|| png.display().to_string())?.into_rgba8();
        let max = OcrEngine::MaxImageDimension()?;
        let scale = (max as f32 / image.width().max(image.height()) as f32).min(2.);
        let (w, h) = ((image.width() as f32 * scale) as u32, (image.height() as f32 * scale) as u32);
        let image = image::imageops::resize(&image, w, h, image::imageops::FilterType::CatmullRom);
        let bgra: Vec<u8> = image.pixels().flat_map(|p| [p[2], p[1], p[0], 255]).collect();
        let writer = DataWriter::new()?;
        writer.WriteBytes(&bgra)?;
        let bitmap = SoftwareBitmap::CreateCopyFromBuffer(&writer.DetachBuffer()?, BitmapPixelFormat::Bgra8, w as i32, h as i32)?;
        let engine = OcrEngine::TryCreateFromUserProfileLanguages().context("no OCR engine for the user's languages")?;
        let result = engine.RecognizeAsync(&bitmap)?.join()?;
        let mut words = Vec::new();
        for line in result.Lines()? {
            for word in line.Words()? {
                words.push(word.Text()?.to_string());
            }
        }
        Ok(words)
    }
}

#[cfg(not(windows))]
mod ocr {
    pub fn words(_png: &std::path::Path) -> anyhow::Result<Vec<String>> {
        anyhow::bail!("OCR is Windows OCR")
    }
}

#[cfg(test)]
mod tests {
    use super::names_foreman;

    #[test]
    fn the_product_word_is_caught_and_a_path_is_not() {
        for word in ["Foreman", "Foreman's", "(Foreman)", "Foreman:", "Foreman,"] {
            assert!(names_foreman(word), "{word}");
        }
        for word in [r"C:\Users\x\AppData\Roaming\Foreman", r"\Foreman:", "FOREMAN", "foreman.exe", "Supraman", "fm-selftest"] {
            assert!(!names_foreman(word), "{word}");
        }
    }
}
