//! An image file's two sides: decoded off the UI thread with gpui's decoders, painted fitted to their pane
//! (down to fit, up to at most 2×) on a checkerboard of theme colours (C152).

use std::cell::RefCell;
use std::sync::Arc;

use gpui::{Bounds, Corners, Hsla, ImageFormat, Pixels, RenderImage, Rgba, SMOOTH_SVG_SCALE_FACTOR, SvgRenderer, Window, canvas, point, prelude::*, px, size};
use supraman_core::text::byte_size;
use supraman_ui::hit::{Hit, HitTargets};
use supraman_ui::themes::lanes::blend;
use theme::ThemeColors;

const MAX_SCALE: f32 = 2.;
/// The most pixels a side may have to be decoded: 64 Mpx is 256 MB of RGBA.
pub(crate) const MAX_PIXELS: u64 = 64 * 1024 * 1024;
const CELL: f32 = 8.;
/// A tile's side in cells, even so tiles continue the pattern.
const TILE: u32 = 32;
/// The tiles kept: a scale or theme switched back to finds its own.
const TILES_KEPT: usize = 4;

/// One side of an image: its pixels, its size in pixels and in bytes.
pub(crate) struct Picture {
    pub(crate) image: Arc<RenderImage>,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) bytes: usize,
}

pub(crate) struct Images {
    pub(crate) format: ImageFormat,
    pub(crate) old: Option<Picture>,
    pub(crate) new: Option<Picture>,
}

/// The format a path's extension names, for the formats shown as images.
pub(crate) fn format_of(path: &str) -> Option<ImageFormat> {
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
pub(crate) fn dimensions(format: ImageFormat, bytes: &[u8]) -> Option<(u32, u32)> {
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
pub(crate) fn decode(format: ImageFormat, bytes: &[u8], svg: &SvgRenderer) -> Option<Picture> {
    let image = gpui::Image::from_bytes(format, bytes.to_vec()).to_image_data(svg.clone()).ok()?;
    let pixels = image.size(0);
    let scale = if format == ImageFormat::Svg { SMOOTH_SVG_SCALE_FACTOR } else { 1. };
    let (width, height) = ((pixels.width.0 as f32 / scale).round() as u32, (pixels.height.0 as f32 / scale).round() as u32);
    (width > 0 && height > 0).then_some(Picture { image, width, height, bytes: bytes.len() })
}

/// A side's caption: `Before · 640 × 480 px · 12 KB`, without the word in File View.
pub(crate) fn label(word: Option<&str>, picture: &Picture) -> String {
    let facts = format!("{} × {} px · {}", picture.width, picture.height, byte_size(picture.bytes as u64));
    word.map_or(facts.clone(), |word| format!("{word} · {facts}"))
}

/// The checkerboard's two cells: the editor's background and the text faintly over it.
pub(crate) fn cells(colors: &ThemeColors) -> (Hsla, Hsla) {
    (colors.editor_background, colors.text.opacity(0.12))
}

/// Where a `width` × `height` image paints in `bounds`: scaled to fit, at most [`MAX_SCALE`], centred.
pub(crate) fn fitted(bounds: Bounds<Pixels>, width: u32, height: u32) -> Bounds<Pixels> {
    let (w, h) = (width as f32, height as f32);
    let scale = (f32::from(bounds.size.width) / w).min(f32::from(bounds.size.height) / h).min(MAX_SCALE);
    let shown = size(px(w * scale), px(h * scale));
    let origin = point(bounds.origin.x + (bounds.size.width - shown.width) / 2., bounds.origin.y + (bounds.size.height - shown.height) / 2.);
    Bounds::new(origin, shown)
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct TileKey {
    cell: u32,
    light: [u8; 4],
    dark: [u8; 4],
}

thread_local! {
    static TILES: RefCell<Vec<(TileKey, Arc<RenderImage>)>> = const { RefCell::new(Vec::new()) };
}

fn bgra(colour: Hsla) -> [u8; 4] {
    let Rgba { r, g, b, .. } = colour.into();
    let byte = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
    [byte(b), byte(g), byte(r), 255]
}

/// [`TILE`] × [`TILE`] cells of `key.cell` device pixels, `dark` where the row and column add up even.
fn tile_pixels(key: TileKey) -> ::image::RgbaImage {
    let side = key.cell * TILE;
    ::image::RgbaImage::from_fn(side, side, |x, y| ::image::Rgba(if (x / key.cell + y / key.cell) % 2 == 0 { key.dark } else { key.light }))
}

/// The tile of the window's scale and these colours, made once.
fn tile(light: Hsla, dark: Hsla, window: &mut Window) -> Arc<RenderImage> {
    let key = TileKey { cell: (CELL * window.scale_factor()).round().max(1.) as u32, light: bgra(light), dark: bgra(blend(dark, 1., light)) };
    TILES.with_borrow_mut(|kept| {
        if let Some((_, image)) = kept.iter().find(|(k, _)| *k == key) {
            return image.clone();
        }
        let image = Arc::new(RenderImage::new(vec![::image::Frame::new(tile_pixels(key))]));
        kept.push((key, image.clone()));
        if kept.len() > TILES_KEPT {
            let (_, old) = kept.remove(0);
            window.drop_image(old).ok();
        }
        image
    })
}

/// The checkerboard under `area`: one cached tile repeated, the last ones cut at its edges.
fn checkerboard(area: Bounds<Pixels>, light: Hsla, dark: Hsla, window: &mut Window) {
    let tile = tile(light, dark, window);
    let side = CELL * TILE as f32;
    let (columns, rows) = ((f32::from(area.size.width) / side).ceil() as usize, (f32::from(area.size.height) / side).ceil() as usize);
    for row in 0..rows {
        for column in 0..columns {
            let at = Bounds::new(point(area.origin.x + px(column as f32 * side), area.origin.y + px(row as f32 * side)), size(px(side), px(side)));
            if let Err(e) = window.paint_image(area, at, Corners::default(), tile.clone(), 0, false) {
                log::warn!("diff view: painting the checkerboard: {e:#}");
                return;
            }
        }
    }
}

/// The picture fitted to the element's bounds on its checkerboard, its painted bounds kept under `key`.
pub(crate) fn painted(picture: &Picture, key: String, hits: &HitTargets, cells: (Hsla, Hsla)) -> impl IntoElement + use<> {
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
    fn the_tile_alternates_its_cells_from_a_dark_one() {
        let (light, dark) = ([1, 1, 1, 255], [2, 2, 2, 255]);
        let pixels = tile_pixels(TileKey { cell: 2, light, dark });
        assert_eq!(pixels.dimensions(), (2 * TILE, 2 * TILE));
        let row = |y: u32| (0..5).map(|x| pixels.get_pixel(x, y).0).collect::<Vec<_>>();
        assert_eq!(row(0), [dark, dark, light, light, dark]);
        assert_eq!(row(2), [light, light, dark, dark, light]);
    }

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
