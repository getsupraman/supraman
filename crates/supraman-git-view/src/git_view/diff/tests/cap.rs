use gpui::{Entity, TestAppContext, VisualTestContext};
use supraman_gitman::fixture::Fixture;

use crate::git_view::diff::*;
use crate::git_view::GitView;
use crate::git_view::test_support::{Opts, open as open_tab, redraw};

fn commit(f: &Fixture, message: &str) -> String {
    f.at(&f.worktree, &["add", "-A"]);
    f.at(&f.worktree, &["commit", "-q", "--no-verify", "-m", message]);
    f.at(&f.worktree, &["rev-parse", "HEAD"])
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for b in bytes {
        crc ^= u32::from(*b);
        for _ in 0..8 {
            crc = if crc & 1 == 1 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn chunk(png: &mut Vec<u8>, kind: &[u8], data: &[u8]) {
    let mut body = kind.to_vec();
    body.extend(data);
    png.extend(u32::try_from(data.len()).unwrap().to_be_bytes());
    png.extend(&body);
    png.extend(crc32(&body).to_be_bytes());
}

/// A PNG's signature, its IHDR for `width × height` RGBA and an empty IDAT: what a header read needs, no pixels.
pub(crate) fn png_header(width: u32, height: u32) -> Vec<u8> {
    let mut ihdr = width.to_be_bytes().to_vec();
    ihdr.extend(height.to_be_bytes());
    ihdr.extend([8, 6, 0, 0, 0]);
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(&mut png, b"IHDR", &ihdr);
    chunk(&mut png, b"IDAT", &[]);
    chunk(&mut png, b"IEND", &[]);
    png
}

fn over_cap(fill: u8) -> Vec<u8> {
    let mut text = vec![fill; MAX_SHOWN_BYTES as usize + 1024];
    text.iter_mut().step_by(80).for_each(|b| *b = b'\n');
    text
}

fn show(view: &Entity<GitView>, target: Target, cx: &mut VisualTestContext) -> Entity<DiffView> {
    view.update(cx, |v, cx| v.open_diff(target, cx));
    cx.run_until_parked();
    redraw(view, cx);
    view.read_with(cx, |v, _| v.diff().cloned()).expect("the viewer is open")
}

#[gpui::test]
fn a_file_over_the_cap_shows_its_sizes_and_an_image_over_the_pixel_cap_its_dimensions(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    std::fs::write(f.worktree.join("huge.txt"), over_cap(b'a')).unwrap();
    std::fs::write(f.worktree.join("vast.png"), png_header(4, 4)).unwrap();
    let base = commit(&f, "Base");
    std::fs::write(f.worktree.join("huge.txt"), over_cap(b'b')).unwrap();
    std::fs::write(f.worktree.join("vast.png"), png_header(20_000, 20_000)).unwrap();
    let change = commit(&f, "Change");
    cx.update(|cx| supraman_editor::init(supraman_ui::fonts::MONO, 13., cx));
    let (view, cx) = open_tab(&f.dir, Opts::default(), cx);
    let changed = |path: &str| supraman_gitman::changed_files(&f.repo(), &change).unwrap().into_iter().find(|c| c.path == path);

    let d = show(&view, Target::commit(change.clone(), Some(base.clone()), "huge.txt".into(), changed("huge.txt")), cx);
    let size = supraman_core::text::byte_size(MAX_SHOWN_BYTES + 1024);
    assert_eq!(d.read_with(cx, |d, _| d.shown()), format!("too large {size} → {size}"));
    assert!(view.read_with(cx, |v, _| v.hits.borrow().contains_key("diff-too-large")), "the notice is painted");

    let d = show(&view, Target::commit(change.clone(), Some(base), "vast.png".into(), changed("vast.png")), cx);
    assert_eq!(d.read_with(cx, |d, _| d.shown()), "too large 4×4 → 20000×20000", "refused from the header, before any decode");

    std::fs::write(f.dir.join("loose.txt"), over_cap(b'c')).unwrap();
    let d = show(&view, Target { of: Of::Untracked, path: "loose.txt".into(), change: None }, cx);
    assert_eq!(d.read_with(cx, |d, _| d.shown()), format!("too large none → {size}"), "a worktree file is capped by its metadata");
}
