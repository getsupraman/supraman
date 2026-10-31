//! The Supraman kit's marks (`assets/brand`, see its NOTICE.md): Supraman's in their own colours, Workman's in one.

use std::sync::{Arc, LazyLock};

use gpui::{App, Hsla, Image, ImageFormat, Img, Length, Styled as _, Svg, img, px, svg};
use theme::{ActiveTheme as _, Appearance};

const SUPRAMAN: &[u8] = include_bytes!("../assets/brand/icons/supraman.svg");
const SUPRAMAN_16: &[u8] = include_bytes!("../assets/brand/icons/small/supraman-16.svg");
const WORKMAN_16_MONO: &[u8] = include_bytes!("../assets/brand/icons/small/workman-16-mono.svg");
const WORDMARK_ON_DARK: &[u8] = include_bytes!("../assets/brand/wordmarks/supraman-wordmark-on-dark.svg");
const WORDMARK_ON_LIGHT: &[u8] = include_bytes!("../assets/brand/wordmarks/supraman-wordmark-on-light.svg");

/// The kit's small set is for 16 to 24 px (its README).
const SMALL_UP_TO: f32 = 24.;
const WORDMARK_ASPECT: f32 = 6157. / 892.;

/// `svg` with its root sized `width` x `height`, so gpui rasterises it for that size, not its viewBox's.
fn sized(svg: &[u8], width: f32, height: f32) -> Arc<Image> {
    let text = std::str::from_utf8(svg).expect("the kit's SVGs are UTF-8");
    let text = text.replacen("<svg ", &format!(r#"<svg width="{width}" height="{height}" "#), 1);
    Arc::new(Image::from_bytes(ImageFormat::Svg, text.into_bytes()))
}

fn icon(image: &Arc<Image>, size: f32) -> Img {
    img(image.clone()).size(px(size)).flex_none()
}

static SUPRAMAN_SMALL: LazyLock<Arc<Image>> = LazyLock::new(|| sized(SUPRAMAN_16, 16., 16.));
static SUPRAMAN_LARGE: LazyLock<Arc<Image>> = LazyLock::new(|| sized(SUPRAMAN, 64., 64.));
static WORDMARK_DARK: LazyLock<Arc<Image>> = LazyLock::new(|| sized(WORDMARK_ON_DARK, 24. * WORDMARK_ASPECT, 24.));
static WORDMARK_LIGHT: LazyLock<Arc<Image>> = LazyLock::new(|| sized(WORDMARK_ON_LIGHT, 24. * WORDMARK_ASPECT, 24.));

pub fn supraman(size: f32) -> Img {
    icon(if size <= SMALL_UP_TO { &SUPRAMAN_SMALL } else { &SUPRAMAN_LARGE }, size)
}

/// The kit's small one-colour Workman mark (16 to 24 px) in `color`, the colour of the text it stands beside.
pub fn workman(size: impl Into<Length>, color: Hsla) -> Svg {
    svg().data(WORKMAN_16_MONO).size(size.into()).flex_none().text_color(color)
}

/// The text mark for the window's theme, `height` px tall.
pub fn wordmark(height: f32, cx: &App) -> Img {
    let image: &Arc<Image> = if on_dark(cx.theme().appearance) { &WORDMARK_DARK } else { &WORDMARK_LIGHT };
    img(image.clone()).h(px(height)).w(px(height * WORDMARK_ASPECT)).flex_none()
}

pub fn on_dark(appearance: Appearance) -> bool {
    appearance == Appearance::Dark
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use sha2::{Digest as _, Sha256};

    use super::*;

    const KIT: &str = r"P:\Projects\LarchSys\getsupraman\brand";

    fn brand() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("assets").join("brand")
    }

    fn copies(dir: &Path, base: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                copies(&path, base, out);
            } else if path.file_name().is_some_and(|n| n != "NOTICE.md" && n != ".gitattributes") {
                out.push(path.strip_prefix(base).unwrap().to_path_buf());
            }
        }
    }

    fn sha256(path: &Path) -> String {
        Sha256::digest(std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))).iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn brand_kit_copies_match_the_kit() {
        let kit = std::env::var_os("SUPRAMAN_BRAND_KIT").map_or_else(|| PathBuf::from(KIT), PathBuf::from);
        if !kit.is_dir() {
            eprintln!("skipped: no brand kit at {}", kit.display());
            return;
        }
        let mut files = Vec::new();
        copies(&brand(), &brand(), &mut files);
        assert!(files.len() >= 9, "{files:?}");
        for file in files {
            assert_eq!(sha256(&brand().join(&file)), sha256(&kit.join(&file)), "assets/brand/{} differs from the kit's", file.display());
        }
    }

    #[test]
    fn marks_are_sized_for_their_slot() {
        let text = |image: &Image| String::from_utf8(image.bytes().to_vec()).unwrap();
        assert!(text(&SUPRAMAN_SMALL).starts_with(r#"<svg width="16" height="16" xmlns="#), "{}", text(&SUPRAMAN_SMALL));
        assert!(text(&WORDMARK_DARK).contains(r#"height="24""#) && text(&WORDMARK_DARK).contains("#f5a524"));
        assert!(!text(&WORDMARK_LIGHT).to_lowercase().contains("#f5a524"), "amber never on a light ground (the kit's README)");
        assert!(text(&WORDMARK_LIGHT).contains("#b06c00"));
        let mono = std::str::from_utf8(WORKMAN_16_MONO).unwrap();
        assert!(mono.contains(r#"viewBox="0 0 16 16""#) && mono.contains("currentColor") && !mono.contains("#f5a524"), "{mono}");
    }

    /// The ICO directory's entries as (width, height); 0 stands for 256.
    fn ico_sizes(path: &Path) -> Vec<(u32, u32)> {
        let bytes = std::fs::read(path).unwrap();
        assert_eq!(&bytes[..4], &[0, 0, 1, 0], "{} is not an icon", path.display());
        let count = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
        let side = |b: u8| if b == 0 { 256 } else { b as u32 };
        (0..count).map(|i| (side(bytes[6 + 16 * i]), side(bytes[7 + 16 * i]))).collect()
    }

    #[test]
    fn the_exe_and_tray_icons_carry_every_size() {
        let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
        let square = |sizes: &[u32]| sizes.iter().map(|&s| (s, s)).collect::<Vec<_>>();
        assert_eq!(ico_sizes(&assets.join("supraman.ico")), square(&[16, 20, 24, 32, 40, 48, 64, 256]));
        for tray in ["tray-on-dark.ico", "tray-on-light.ico"] {
            assert_eq!(ico_sizes(&assets.join(tray)), square(&[16, 20, 24, 32]), "{tray}");
        }
        let rc = std::fs::read_to_string(assets.join("foreman.rc")).unwrap();
        for line in [r#"1 ICON "supraman.ico""#, r#"2 ICON "tray-on-dark.ico""#, r#"3 ICON "tray-on-light.ico""#] {
            assert!(rc.contains(line), "foreman.rc lacks {line}");
        }
    }
}
