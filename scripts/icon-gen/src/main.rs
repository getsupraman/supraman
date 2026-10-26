//! `<name>.svg` → `<out>/<name>-{256,48,32,16}.png`, `<out>/<name>-preview.png` (every size on a dark and a
//! light taskbar colour), `<out>/<name>.ico` (16, 24, 32, 48, 256) and `<out>/<name>-tray.ico` (16–48).

use std::path::{Path, PathBuf};

use image::codecs::ico::{IcoEncoder, IcoFrame};
use image::{ExtendedColorType, Rgba, RgbaImage, imageops};
use resvg::{tiny_skia, usvg};

const ICO_SIZES: [u32; 5] = [16, 24, 32, 48, 256];
const PREVIEW_SIZES: [u32; 5] = [128, 48, 32, 24, 16];

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

fn preview(tree: &usvg::Tree) -> RgbaImage {
    let (pad, half_w) = (16u32, PREVIEW_SIZES.iter().map(|s| s + 16).sum::<u32>() + 16);
    let mut sheet = RgbaImage::from_pixel(half_w * 2, 128 + 2 * pad, Rgba([0x20, 0x20, 0x20, 0xff]));
    for (i, bg) in [[0x20, 0x20, 0x20, 0xff], [0xf3, 0xf3, 0xf3, 0xff]].into_iter().enumerate() {
        let x0 = i as u32 * half_w;
        for y in 0..sheet.height() {
            for x in x0..x0 + half_w {
                sheet.put_pixel(x, y, Rgba(bg));
            }
        }
        let mut x = x0 + pad;
        for size in PREVIEW_SIZES {
            let icon = render(tree, size);
            imageops::overlay(&mut sheet, &icon, x as i64, (pad + (128 - size) / 2) as i64);
            x += size + 16;
        }
    }
    sheet
}

fn ico(tree: &usvg::Tree, sizes: &[u32], path: &Path) {
    let frames: Vec<IcoFrame> = sizes
        .iter()
        .map(|&s| IcoFrame::as_png(render(tree, s).as_raw(), s, s, ExtendedColorType::Rgba8).expect("frame"))
        .collect();
    IcoEncoder::new(std::fs::File::create(path).expect("ico")).encode_images(&frames).expect("ico");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, svg, out] = args.as_slice() else {
        eprintln!("usage: icon-gen <icon.svg> <out dir>");
        std::process::exit(2);
    };
    let (svg, out) = (Path::new(svg), PathBuf::from(out));
    let name = svg.file_stem().unwrap().to_string_lossy().into_owned();
    let tree = usvg::Tree::from_data(&std::fs::read(svg).expect("read svg"), &usvg::Options::default()).expect("parse svg");
    std::fs::create_dir_all(&out).expect("out dir");
    for size in [256, 48, 32, 16] {
        render(&tree, size).save(out.join(format!("{name}-{size}.png"))).expect("png");
    }
    preview(&tree).save(out.join(format!("{name}-preview.png"))).expect("preview");
    ico(&tree, &ICO_SIZES, &out.join(format!("{name}.ico")));
    // the tray's: `image` decodes an .ico's largest entry, and a 48 px one scales down to the tray better
    ico(&tree, &ICO_SIZES[..4], &out.join(format!("{name}-tray.ico")));
    println!("{name}: {}", out.display());
}
