use gpui::{Bounds, Entity, Pixels, TestAppContext, VisualTestContext, px, size};
use supraman_core::text::byte_size;
use supraman_gitman::fixture::Fixture;

use crate::git_view::diff::*;
use crate::git_view::GitView;
use crate::git_view::test_support::{Opts, open as open_tab, redraw};

const KEYS: [&str; 4] = ["diff-image-old", "diff-image-new", "diff-binary", "diff-invisible"];

fn open<'a>(f: &Fixture, cx: &'a mut TestAppContext) -> (Entity<GitView>, &'a mut VisualTestContext) {
    open_tab(&f.dir, Opts::default().editor(), cx)
}

fn show(view: &Entity<GitView>, f: &Fixture, path: &str, cx: &mut VisualTestContext) -> Entity<DiffView> {
    let change = supraman_gitman::changed_files(&f.repo(), f.sha("media")).unwrap().into_iter().find(|c| c.path == path);
    assert!(change.is_some(), "{path} changes in media");
    let target = Target::commit(f.sha("media").into(), Some(f.sha("media_base").into()), path.into(), change);
    view.update(cx, |v, cx| v.open_diff(target, cx));
    cx.run_until_parked();
    redraw(view, cx);
    view.read_with(cx, |v, cx| v.diff(cx).cloned()).expect("the viewer is open")
}

/// The keys of [`KEYS`] painted in `mode` and `layout`, with their bounds.
fn painted(view: &Entity<GitView>, d: &Entity<DiffView>, mode: Mode, layout: Layout, cx: &mut VisualTestContext) -> Vec<(&'static str, Bounds<Pixels>)> {
    d.update(cx, |d, cx| {
        d.set_mode(mode, cx);
        d.set_layout(layout, cx);
    });
    view.read_with(cx, |v, _| v.hits.borrow_mut().clear());
    redraw(view, cx);
    view.read_with(cx, |v, _| KEYS.into_iter().filter_map(|k| v.hits.borrow().get(k).map(|h| (k, h.bounds))).collect())
}

fn keys(painted: &[(&'static str, Bounds<Pixels>)]) -> Vec<&'static str> {
    painted.iter().map(|(k, _)| *k).collect()
}

fn blob_size(f: &Fixture, sha: &str, path: &str) -> String {
    byte_size(supraman_gitman::blob(&f.repo(), sha, path).unwrap().bytes.len() as u64)
}

#[gpui::test]
fn a_changed_png_shows_both_sides_with_their_sizes_in_split_inline_and_file_view(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (view, cx) = open(&f, cx);
    let d = show(&view, &f, "changed.png", cx);
    assert_eq!(d.read_with(cx, |d, _| d.shown()), "image 4×3 → 8×6");
    assert_eq!(d.read_with(cx, |d, _| d.encoding_word()), "PNG image");

    let split = painted(&view, &d, Mode::Diff, Layout::Split, cx);
    assert_eq!(keys(&split), ["diff-image-old", "diff-image-new"]);
    let (old, new) = (split[0].1, split[1].1);
    assert!(old.right() <= new.left(), "Split: before on the left, after on the right");
    assert_eq!((old.size, new.size), (size(px(8.), px(6.)), size(px(16.), px(12.))), "a small image grows to 2×, no more");

    let inline = painted(&view, &d, Mode::Diff, Layout::Inline, cx);
    assert_eq!(keys(&inline), ["diff-image-old", "diff-image-new"]);
    assert!(inline[1].1.bottom() <= inline[0].1.top(), "Inline: after above before");

    assert_eq!(keys(&painted(&view, &d, Mode::File, Layout::Split, cx)), ["diff-image-new"], "File View: the image at the commit");

    let labels = d.read_with(cx, |d, _| match d.content() {
        Content::Image(i) => (image::label(Some("Before"), i.old.as_ref().unwrap()), image::label(Some("After"), i.new.as_ref().unwrap())),
        _ => panic!("not an image"),
    });
    let (base, media) = (f.sha("media_base"), f.sha("media"));
    assert_eq!(labels.0, format!("Before · 4 × 3 px · {}", blob_size(&f, base, "changed.png")));
    assert_eq!(labels.1, format!("After · 8 × 6 px · {}", blob_size(&f, media, "changed.png")));
}

#[gpui::test]
fn an_added_or_deleted_png_and_an_svg_show_as_images(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (view, cx) = open(&f, cx);
    let d = show(&view, &f, "added.png", cx);
    assert_eq!(d.read_with(cx, |d, _| d.shown()), "image none → 3×3");
    for (mode, layout) in [(Mode::Diff, Layout::Split), (Mode::Diff, Layout::Inline), (Mode::File, Layout::Split)] {
        assert_eq!(keys(&painted(&view, &d, mode, layout, cx)), ["diff-image-new"], "added, {mode:?} {layout:?}: one side");
    }
    let d = show(&view, &f, "deleted.png", cx);
    assert_eq!(d.read_with(cx, |d, _| d.shown()), "image 2×2 → none");
    for (mode, layout) in [(Mode::Diff, Layout::Split), (Mode::Diff, Layout::Inline), (Mode::File, Layout::Split)] {
        assert_eq!(keys(&painted(&view, &d, mode, layout, cx)), ["diff-image-old"], "deleted, {mode:?} {layout:?}: the image as it was");
    }
    let d = show(&view, &f, "logo.svg", cx);
    assert_eq!(d.read_with(cx, |d, _| d.shown()), "image 10×10 → 20×10");
}

#[gpui::test]
fn a_binary_or_an_undecodable_image_says_so_with_both_byte_sizes(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (view, cx) = open(&f, cx);
    let (base, media) = (f.sha("media_base"), f.sha("media"));
    for path in ["data.bin", "broken.png"] {
        let d = show(&view, &f, path, cx);
        assert_eq!(d.read_with(cx, |d, _| d.shown()), format!("binary {} → {}", blob_size(&f, base, path), blob_size(&f, media, path)), "{path}");
        assert_eq!(d.read_with(cx, |d, _| d.encoding_word()), "binary");
        for (mode, layout) in [(Mode::Diff, Layout::Split), (Mode::Diff, Layout::Inline), (Mode::File, Layout::Split)] {
            assert_eq!(keys(&painted(&view, &d, mode, layout, cx)), ["diff-binary"], "{path}, {mode:?} {layout:?}");
        }
    }
}

#[gpui::test]
fn a_change_only_bytes_show_is_named_and_a_text_change_keeps_its_hunks(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (view, cx) = open(&f, cx);
    let cases = [
        ("bom.txt", "same text: Byte-order mark added (UTF-8 → UTF-8 with BOM)", "UTF-8 → UTF-8 with BOM"),
        ("unbom.txt", "same text: Byte-order mark removed (UTF-8 with BOM → UTF-8)", "UTF-8 with BOM → UTF-8"),
        ("crlf.txt", "same text: Line endings changed from CRLF to LF on 2 lines", "UTF-8"),
        ("lf.txt", "same text: Line endings changed from LF to CRLF on 2 lines", "UTF-8"),
        ("invalid.txt", "same text: Bytes the text does not show changed (3 B → 3 B)", "Unknown (shown as UTF-8)"),
    ];
    for (path, shown, encoding) in cases {
        let d = show(&view, &f, path, cx);
        assert_eq!((d.read_with(cx, |d, _| d.shown()).as_str(), d.read_with(cx, |d, _| d.encoding_word()).as_ref()), (shown, encoding), "{path}");
        assert_eq!(d.read_with(cx, |d, _| d.hunk_count()), 0);
        for layout in [Layout::Split, Layout::Inline] {
            assert_eq!(keys(&painted(&view, &d, Mode::Diff, layout, cx)), ["diff-invisible"], "{path}, {layout:?}");
        }
        assert!(painted(&view, &d, Mode::File, Layout::Split, cx).is_empty(), "{path}: File View shows the file");
    }
    let d = show(&view, &f, "text.txt", cx);
    assert_eq!((d.read_with(cx, |d, _| d.shown()), d.read_with(cx, |d, _| d.hunk_count())), ("text".to_string(), 1));
    assert!(painted(&view, &d, Mode::Diff, Layout::Split, cx).is_empty(), "a real change shows its hunks");
}

#[gpui::test]
fn a_utf16_file_with_a_bom_shows_as_text_in_its_encoding(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (view, cx) = open(&f, cx);
    let d = show(&view, &f, "utf16.txt", cx);
    let said = (d.read_with(cx, |d, _| d.shown()), d.read_with(cx, |d, _| d.encoding_word().to_string()));
    assert_eq!(said, ("same text: Encoding changed: UTF-16 LE → UTF-8".to_string(), "UTF-16 LE → UTF-8".to_string()));
    for layout in [Layout::Split, Layout::Inline] {
        assert_eq!(keys(&painted(&view, &d, Mode::Diff, layout, cx)), ["diff-invisible"], "{layout:?}");
    }

    let d = show(&view, &f, "wide.txt", cx);
    let said = (d.read_with(cx, |d, _| d.shown()), d.read_with(cx, |d, _| d.encoding_word().to_string()), d.read_with(cx, |d, _| d.hunk_count()));
    assert_eq!(said, ("text".to_string(), "UTF-16 LE".to_string(), 1));
    assert!(painted(&view, &d, Mode::Diff, Layout::Split, cx).is_empty(), "UTF-16 text shows its hunks");
}
