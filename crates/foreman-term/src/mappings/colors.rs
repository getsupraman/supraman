// Vendored from Zed crates/terminal/src/mappings/colors.rs @ 03efd49220ba06703d04f9c4732fdf814f9c523c
// (GPL-3.0-or-later).

use gpui::Rgba;
use vte::ansi::Rgb as VteRgb;

pub(crate) fn to_vte_rgb(color: impl Into<Rgba>) -> VteRgb {
    let color = color.into();
    let r = ((color.r * color.a) * 255.) as u8;
    let g = ((color.g * color.a) * 255.) as u8;
    let b = ((color.b * color.a) * 255.) as u8;
    VteRgb { r, g, b }
}
