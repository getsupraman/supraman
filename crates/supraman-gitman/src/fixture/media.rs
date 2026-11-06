//! Two commits no ref reaches, `media_base` and `media`, of files whose diff is not lines of text: images,
//! binaries, and text whose change only its bytes show. Dangling, so the graph, the refs and the work in
//! progress stay as the rest of the fixture has them.

use std::io::Cursor;

use super::{BINARY, Fixture, git_in};

/// The paths `media` changes from `media_base`: png changed (4×3 → 8×6), added (3×3) and deleted (2×2), a
/// png that does not decode, an svg (10×10 → 20×10), a binary, a BOM added and removed, CRLF → LF and back,
/// a text change, and invalid UTF-8 that decodes the same.
pub const MEDIA_FILES: [&str; 12] = [
    "changed.png",
    "added.png",
    "deleted.png",
    "broken.png",
    "logo.svg",
    "data.bin",
    "bom.txt",
    "unbom.txt",
    "crlf.txt",
    "lf.txt",
    "text.txt",
    "invalid.txt",
];

const SVG_SMALL: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10" fill="red"/></svg>"#;
const SVG_WIDE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="20" height="10" fill="blue"/></svg>"#;

/// A `width` × `height` PNG of one colour.
pub fn png(width: u32, height: u32, rgba: [u8; 4]) -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(width, height, image::Rgba(rgba));
    let mut out = Cursor::new(Vec::new());
    image.write_to(&mut out, image::ImageFormat::Png).expect("a png encodes");
    out.into_inner()
}

fn sides(path: &str) -> (Option<Vec<u8>>, Option<Vec<u8>>) {
    let both = |old: &[u8], new: &[u8]| (Some(old.to_vec()), Some(new.to_vec()));
    match path {
        "changed.png" => (Some(png(4, 3, [255, 0, 0, 255])), Some(png(8, 6, [0, 0, 255, 128]))),
        "added.png" => (None, Some(png(3, 3, [0, 255, 0, 255]))),
        "deleted.png" => (Some(png(2, 2, [0, 0, 0, 255])), None),
        "broken.png" => both(b"\x89PNG\0not an image", b"\x89PNG\0still not an image"),
        "logo.svg" => both(SVG_SMALL.as_bytes(), SVG_WIDE.as_bytes()),
        "data.bin" => both(BINARY, &[BINARY, BINARY].concat()),
        "bom.txt" => both(b"same\n", b"\xEF\xBB\xBFsame\n"),
        "unbom.txt" => both(b"\xEF\xBB\xBFsame\n", b"same\n"),
        "crlf.txt" => both(b"a\r\nb\r\nc\n", b"a\nb\nc\n"),
        "lf.txt" => both(b"a\nb\n", b"a\r\nb\r\n"),
        "text.txt" => both(b"one\n", b"one\ntwo\n"),
        "invalid.txt" => both(b"a\xFF\n", b"a\xFE\n"),
        other => unreachable!("no media file {other}"),
    }
}

impl Fixture {
    pub(super) fn media(&mut self) {
        let (mut old, mut new) = (String::new(), String::new());
        for path in MEDIA_FILES {
            let (before, after) = sides(path);
            for (tree, bytes) in [(&mut old, before), (&mut new, after)] {
                if let Some(bytes) = bytes {
                    let blob = git_in(&self.git, &self.dir, &["hash-object", "-w", "--stdin"], &[], Some(bytes));
                    tree.push_str(&format!("100644 blob {blob}\t{path}\n"));
                }
            }
        }
        let base = self.dangling(old, None, "Media base");
        let media = self.dangling(new, Some(&base), "Media");
        self.shas.insert("media_base", base);
        self.shas.insert("media", media);
    }

    fn dangling(&mut self, tree: String, parent: Option<&str>, message: &str) -> String {
        self.time += 60;
        let date = format!("@{} +0000", self.time);
        let env = [("GIT_AUTHOR_DATE", date.clone()), ("GIT_COMMITTER_DATE", date)];
        let tree = git_in(&self.git, &self.dir, &["mktree"], &[], Some(tree.into_bytes()));
        let mut args = vec!["commit-tree", tree.as_str(), "-m", message];
        if let Some(parent) = parent {
            args.extend(["-p", parent]);
        }
        git_in(&self.git, &self.dir, &args, &env, None)
    }
}
