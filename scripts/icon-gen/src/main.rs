//! The brand kit's SVGs (`crates/foreman-app/assets/brand`, the kit's layout) → `<out>/supraman.ico`
//! (16, 20, 24 from the small set; 32, 40, 48, 64, 256 from the full icon), `<out>/tray-on-dark.ico` and
//! `<out>/tray-on-light.ico` (16, 20, 24, 32), and `<out>/supraman-preview.png`: each size on a dark and a
//! light taskbar colour, and each tray icon on its own.

use std::path::{Path, PathBuf};

use image::codecs::ico::{IcoEncoder, IcoFrame};
use image::{ExtendedColorType, Rgba, RgbaImage, imageops};
use resvg::{tiny_skia, usvg};

const SMALL: [u32; 3] = [16, 20, 24];
const LARGE: [u32; 5] = [32, 40, 48, 64, 256];
const TRAY: [u32; 4] = [16, 20, 24, 32];
const DARK: [u8; 4] = [0x20, 0x20, 0x20, 0xff];
const LIGHT: [u8; 4] = [0xf3, 0xf3, 0xf3, 0xff];

fn tree(path: &Path) -> usvg::Tree {
    let data = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    usvg::Tree::from_data(&data, &usvg::Options::default()).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn render(tree: &usvg::Tree, size: u32) -> RgbaImage {
    let mut pixmap = tiny_skia::Pixmap::new(size, size).expect("size > 0");
    let scale = size as f32 / tree.size().width();
    resvg::render(tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    let mut out = RgbaImage::new(size, size);
    for (dst, src) in out.pixels_mut().zip(pixmap.pixels()) {
        let c = src.demultiply();
        *dst = Rgba([c.red(), c.green(), c.blue(), c.alpha()]);
    }
    out
}

fn ico(frames: &[(&usvg::Tree, u32)], path: &Path) {
    let images: Vec<(RgbaImage, u32)> = frames.iter().map(|&(t, s)| (render(t, s), s)).collect();
    let frames: Vec<IcoFrame> = images
        .iter()
        .map(|(image, s)| IcoFrame::as_png(image.as_raw(), *s, *s, ExtendedColorType::Rgba8).expect("frame"))
        .collect();
    IcoEncoder::new(std::fs::File::create(path).expect("ico")).encode_images(&frames).expect("ico");
}

/// One row per (ground, icons): every icon at its size, left to right, 16 px apart.
fn preview(rows: &[([u8; 4], Vec<RgbaImage>)]) -> RgbaImage {
    let pad = 16u32;
    let width = rows.iter().map(|(_, icons)| icons.iter().map(|i| i.width() + pad).sum::<u32>() + pad).max().unwrap_or(0);
    let heights: Vec<u32> = rows.iter().map(|(_, icons)| icons.iter().map(RgbaImage::height).max().unwrap_or(0) + 2 * pad).collect();
    let mut sheet = RgbaImage::new(width, heights.iter().sum());
    let mut y0 = 0;
    for ((ground, icons), h) in rows.iter().zip(&heights) {
        for y in y0..y0 + h {
            for x in 0..width {
                sheet.put_pixel(x, y, Rgba(*ground));
            }
        }
        let mut x = pad;
        for icon in icons {
            imageops::overlay(&mut sheet, icon, x as i64, (y0 + (h - icon.height()) / 2) as i64);
            x += icon.width() + pad;
        }
        y0 += h;
    }
    sheet
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, kit, out] = args.as_slice() else {
        eprintln!("usage: icon-gen <brand dir> <out dir>");
        std::process::exit(2);
    };
    let (kit, out) = (Path::new(kit), PathBuf::from(out));
    let small = tree(&kit.join("icons/small/supraman-16.svg"));
    let large = tree(&kit.join("icons/supraman.svg"));
    let tray_dark = tree(&kit.join("icons/tray/supraman-tray-on-dark.svg"));
    let tray_light = tree(&kit.join("icons/tray/supraman-tray-on-light.svg"));
    std::fs::create_dir_all(&out).expect("out dir");

    let app: Vec<(&usvg::Tree, u32)> = SMALL.iter().map(|&s| (&small, s)).chain(LARGE.iter().map(|&s| (&large, s))).collect();
    ico(&app, &out.join("supraman.ico"));
    ico(&TRAY.map(|s| (&tray_dark, s)), &out.join("tray-on-dark.ico"));
    ico(&TRAY.map(|s| (&tray_light, s)), &out.join("tray-on-light.ico"));

    let shown: Vec<RgbaImage> = app.iter().filter(|(_, s)| *s <= 64).map(|&(t, s)| render(t, s)).collect();
    let rows = [
        (DARK, shown.clone()),
        (LIGHT, shown),
        (DARK, TRAY.iter().map(|&s| render(&tray_dark, s)).collect()),
        (LIGHT, TRAY.iter().map(|&s| render(&tray_light, s)).collect()),
    ];
    preview(&rows).save(out.join("supraman-preview.png")).expect("preview");
    println!("supraman.ico, tray-on-dark.ico, tray-on-light.ico, supraman-preview.png: {}", out.display());
}
