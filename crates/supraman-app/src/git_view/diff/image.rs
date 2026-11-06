//! An image file's two sides: decoded off the UI thread with gpui's decoders, painted fitted to their pane
//! (down to fit, up to at most 2×) on a checkerboard of theme colours (C152).

use std::sync::Arc;

use gpui::{Bounds, Corners, Hsla, ImageFormat, Pixels, RenderImage, SMOOTH_SVG_SCALE_FACTOR, SvgRenderer, Window, canvas, fill, point, prelude::*, px, size};
use supraman_core::text::byte_size;
use supraman_ui::hit::{Hit, HitTargets};
use theme::ThemeColors;

const MAX_SCALE: f32 = 2.;
/// The most pixels a side may have to be decoded: 64 Mpx is 256 MB of RGBA.
pub const MAX_PIXELS: u64 = 64 * 1024 * 1024;
const CELL: f32 = 8.;

/// One side of an image: its pixels, its size in pixels and in bytes.
pub struct Picture {
    pub image: Arc<RenderImage>,
    pub width: u32,
    pub height: u32,
    pub bytes: usize,
}

pub struct Images {
    pub format: ImageFormat,
    pub old: Option<Picture>,
    pub new: Option<Picture>,
}

/// The format a path's extension names, for the formats shown as images.
pub fn format_of(path: &str) -> Option<ImageFormat> {
    let (_, ext) = path.rsplit_once('.')?;
    Some(match ext.to_ascii_lowercase().as_str() {
        "png" => ImageFormat::Png,
        "jpg" | "jpeg" => ImageFormat::Jpeg,
        "gif" => ImageFormat::Gif,
        "webp" => ImageFormat::Webp,
        "bmp" => ImageFormat::Bmp,
        "ico" => ImageFormat::Ico,
        "svg" => ImageFormat::Svg,
        _ => return None,
    })
}

/// A raster image's `width × height` from its header alone, without decoding its pixels; `None` for an SVG
/// or a header that does not read.
pub fn dimensions(format: ImageFormat, bytes: &[u8]) -> Option<(u32, u32)> {
    let format = match format {
        ImageFormat::Png => ::image::ImageFormat::Png,
        ImageFormat::Jpeg => ::image::ImageFormat::Jpeg,
        ImageFormat::Gif => ::image::ImageFormat::Gif,
        ImageFormat::Webp => ::image::ImageFormat::WebP,
        ImageFormat::Bmp => ::image::ImageFormat::Bmp,
        ImageFormat::Ico => ::image::ImageFormat::Ico,
        _ => return None,
    };
    ::image::ImageReader::with_format(std::io::Cursor::new(bytes), format).into_dimensions().ok()
}

/// `bytes` decoded, blocking; `None` when they do not decode or have no pixels.
pub fn decode(format: ImageFormat, bytes: &[u8], svg: &SvgRenderer) -> Option<Picture> {
    let image = gpui::Image::from_bytes(format, bytes.to_vec()).to_image_data(svg.clone()).ok()?;
    let pixels = image.size(0);
    let scale = if format == ImageFormat::Svg { SMOOTH_SVG_SCALE_FACTOR } else { 1. };
    let (width, height) = ((pixels.width.0 as f32 / scale).round() as u32, (pixels.height.0 as f32 / scale).round() as u32);
    (width > 0 && height > 0).then_some(Picture { image, width, height, bytes: bytes.len() })
}

/// A side's caption: `Before · 640 × 480 px · 12 KB`, without the word in File View.
pub fn label(word: Option<&str>, picture: &Picture) -> String {
    let facts = format!("{} × {} px · {}", picture.width, picture.height, byte_size(picture.bytes as u64));
    word.map_or(facts.clone(), |word| format!("{word} · {facts}"))
}

/// The checkerboard's two cells: the editor's background and the text faintly over it.
pub fn cells(colors: &ThemeColors) -> (Hsla, Hsla) {
    (colors.editor_background, colors.text.opacity(0.12))
}

/// Where a `width` × `height` image paints in `bounds`: scaled to fit, at most [`MAX_SCALE`], centred.
pub fn fitted(bounds: Bounds<Pixels>, width: u32, height: u32) -> Bounds<Pixels> {
    let (w, h) = (width as f32, height as f32);
    let scale = (f32::from(bounds.size.width) / w).min(f32::from(bounds.size.height) / h).min(MAX_SCALE);
    let shown = size(px(w * scale), px(h * scale));
    let origin = point(bounds.origin.x + (bounds.size.width - shown.width) / 2., bounds.origin.y + (bounds.size.height - shown.height) / 2.);
    Bounds::new(origin, shown)
}

fn checkerboard(area: Bounds<Pixels>, light: Hsla, dark: Hsla, window: &mut Window) {
    window.paint_quad(fill(area, light));
    let (w, h) = (f32::from(area.size.width), f32::from(area.size.height));
    let (columns, rows) = ((w / CELL).ceil() as usize, (h / CELL).ceil() as usize);
    for row in 0..rows {
        for column in (row % 2..columns).step_by(2) {
            let (x, y) = (column as f32 * CELL, row as f32 * CELL);
            let cell = Bounds::new(point(area.origin.x + px(x), area.origin.y + px(y)), size(px(CELL.min(w - x)), px(CELL.min(h - y))));
            window.paint_quad(fill(cell, dark));
        }
    }
}

/// The picture fitted to the element's bounds on its checkerboard, its painted bounds kept under `key`.
pub fn painted(picture: &Picture, key: String, hits: &HitTargets, cells: (Hsla, Hsla)) -> impl IntoElement + use<> {
    let (image, width, height, hits) = (picture.image.clone(), picture.width, picture.height, hits.clone());
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let area = fitted(bounds, width, height);
            hits.borrow_mut().insert(key, Hit { bounds: area, visible: window.content_mask().bounds });
            checkerboard(area, cells.0, cells.1, window);
            if let Err(e) = window.paint_image(area, area, Corners::default(), image, 0, false) {
                log::warn!("diff view: painting an image: {e:#}");
            }
        },
    )
    .size_full()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_image_fits_its_pane_and_grows_at_most_twice() {
        let pane = Bounds::new(point(px(0.), px(0.)), size(px(400.), px(300.)));
        assert_eq!(fitted(pane, 800, 300).size, size(px(400.), px(150.)), "down to the width");
        assert_eq!(fitted(pane, 10, 5).size, size(px(20.), px(10.)), "up to 2× only");
        assert_eq!(fitted(pane, 10, 5).origin, point(px(190.), px(145.)), "centred");
    }

    #[gpui::test]
    fn each_image_format_decodes_with_its_size(cx: &mut gpui::TestAppContext) {
        let svg = cx.update(|cx| cx.svg_renderer());
        let pixels = image::RgbaImage::from_pixel(3, 2, image::Rgba([10, 20, 30, 255]));
        for (format, ours) in [
            (image::ImageFormat::Png, ImageFormat::Png),
            (image::ImageFormat::Jpeg, ImageFormat::Jpeg),
            (image::ImageFormat::Gif, ImageFormat::Gif),
            (image::ImageFormat::WebP, ImageFormat::Webp),
            (image::ImageFormat::Bmp, ImageFormat::Bmp),
            (image::ImageFormat::Ico, ImageFormat::Ico),
        ] {
            let mut bytes = std::io::Cursor::new(Vec::new());
            let encoded = if format == image::ImageFormat::Jpeg { image::DynamicImage::ImageRgba8(pixels.clone()).to_rgb8().write_to(&mut bytes, format) } else { pixels.write_to(&mut bytes, format) };
            encoded.unwrap();
            let picture = decode(ours, bytes.get_ref(), &svg).unwrap_or_else(|| panic!("{format:?} decodes"));
            assert_eq!((picture.width, picture.height, picture.bytes), (3, 2, bytes.get_ref().len()), "{format:?}");
        }
        let logo = br#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="20" height="10"/></svg>"#;
        let picture = decode(ImageFormat::Svg, logo, &svg).expect("an svg renders");
        assert_eq!((picture.width, picture.height), (20, 10));
        assert!(decode(ImageFormat::Png, b"\x89PNG\0nope", &svg).is_none());
        assert_eq!(format_of("a/b.JPEG"), Some(ImageFormat::Jpeg));
        assert_eq!(format_of("doc.docx"), None);
    }
}
