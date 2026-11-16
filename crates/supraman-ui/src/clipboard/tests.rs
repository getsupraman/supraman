use std::path::{Path, PathBuf};

use gpui::{ClipboardItem, Image, ImageFormat, TestAppContext};

use super::{read, text, use_memory, write, write_text, writes};

#[gpui::test]
fn the_memory_clipboard_round_trips_text_and_an_image(cx: &mut TestAppContext) {
    cx.update(|cx| {
        use_memory(cx);
        assert_eq!(text(cx), None);
        write_text(cx, "copied");
        assert_eq!(text(cx).as_deref(), Some("copied"));
        let image = Image::from_bytes(ImageFormat::Png, vec![0x89, b'P', b'N', b'G']);
        write(cx, ClipboardItem::new_image(&image));
        assert_eq!(read(cx), Some(ClipboardItem::new_image(&image)));
        assert_eq!(text(cx), None, "an image is no text");
        assert_eq!(writes(cx), Some(2));
    });
}

#[gpui::test]
fn with_memory_nothing_reaches_the_platform_clipboard(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.write_to_clipboard(ClipboardItem::new_string("the user's".into()));
        assert_eq!(text(cx).as_deref(), Some("the user's"), "without memory the platform's is read");
        assert_eq!(writes(cx), None);
        use_memory(cx);
        write_text(cx, "a selftest's");
        let route = super::terminal();
        (route.write)(cx, ClipboardItem::new_string("a terminal's".into()));
        assert_eq!((route.read)(cx).and_then(|i| i.text()).as_deref(), Some("a terminal's"));
        assert_eq!(cx.read_from_clipboard().and_then(|i| i.text()).as_deref(), Some("the user's"));
    });
}

#[gpui::test]
fn a_terminal_copies_through_this_module(cx: &mut TestAppContext) {
    cx.update(|cx| {
        crate::init_globals(cx, "test");
        use_memory(cx);
        let route = supraman_term::terminal_settings::TerminalConfig::get(cx).clipboard;
        (route.write)(cx, ClipboardItem::new_string("selected".into()));
        assert_eq!(text(cx).as_deref(), Some("selected"));
    });
}

const CALLS: [&str; 4] = ["write_to_clipboard", "read_from_clipboard", "write_to_primary", "read_from_primary"];

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Every clipboard call outside this module, as `crate/path:line`.
fn direct_calls(crates: &Path) -> Vec<String> {
    let own = crates.join("supraman-ui").join("src").join("clipboard");
    let mut files = Vec::new();
    for member in std::fs::read_dir(crates).unwrap().flatten() {
        let src = member.path().join("src");
        if src.is_dir() {
            sources(&src, &mut files);
        }
    }
    let mut found = Vec::new();
    for file in files {
        if file.starts_with(&own) || file == own.with_extension("rs") {
            continue;
        }
        let body = std::fs::read_to_string(&file).unwrap();
        for (n, line) in body.lines().enumerate() {
            if CALLS.iter().any(|call| line.contains(call)) {
                found.push(format!("{}:{}", file.strip_prefix(crates).unwrap_or(&file).display(), n + 1));
            }
        }
    }
    found
}

#[test]
fn only_this_module_touches_the_clipboard() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let found = direct_calls(crates);
    assert!(found.is_empty(), "clipboard calls outside supraman_ui::clipboard: {found:?}");
}

#[test]
fn the_guard_finds_a_call_in_a_view() {
    let crates = std::env::temp_dir().join(format!("supraman-clipboard-guard-{}", std::process::id()));
    let view = crates.join("app").join("src");
    std::fs::create_dir_all(&view).unwrap();
    std::fs::write(view.join("view.rs"), "fn copy(cx: &App) {\n    cx.write_to_clipboard(item);\n}\n").unwrap();
    let found = direct_calls(&crates);
    std::fs::remove_dir_all(&crates).ok();
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].ends_with("view.rs:2"), "{found:?}");
}
