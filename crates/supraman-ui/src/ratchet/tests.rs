use std::path::{Path, PathBuf};

use super::{CEILINGS, Found, Kind, ceilings, code_of, problems, scan, table, without_tests};

fn crates() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
}

fn holds(kind: Kind) {
    let found = scan(crates());
    let problems = problems(&found, &ceilings(CEILINGS), kind);
    assert!(problems.is_empty(), "crates/supraman-ui/src/ratchet/ceilings.toml [{}]:\n{}", kind.key(), problems.join("\n"));
}

#[test]
fn overflow_scroll_holds_its_ceilings() {
    holds(Kind::OverflowScroll);
}

#[test]
fn render_focus_holds_its_ceilings() {
    holds(Kind::RenderFocus);
}

#[test]
fn hover_holds_its_ceilings() {
    holds(Kind::Hover);
}

#[test]
fn px_holds_its_ceilings() {
    holds(Kind::Px);
}

#[test]
fn kit_px_holds_its_ceilings() {
    holds(Kind::KitPx);
}

/// `cargo test -p supraman-ui print_ceilings -- --ignored --nocapture`: today's counts as `ceilings.toml`.
#[test]
#[ignore]
fn print_ceilings() {
    println!("{}", table(&scan(crates())));
}

struct Planted(PathBuf);

impl Planted {
    fn new(files: &[(&str, &str)]) -> Self {
        let root = std::env::temp_dir().join(format!("supraman-ratchet-guard-{}-{}", std::process::id(), supraman_core::time::hex6()));
        for (path, text) in files {
            let file = root.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, text).unwrap();
        }
        Self(root)
    }
}

impl Drop for Planted {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

fn lines(found: &Found, kind: Kind, file: &str) -> Vec<usize> {
    found.get(&kind).and_then(|f| f.get(file)).cloned().unwrap_or_default()
}

const VIEW: &str = r#"impl Render for View {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.focus(&self.field, cx);
        self.panel.update(cx, |p, cx| p.ensure_field(window, cx));
        let later = div().on_click(cx.listener(|this, _, window, cx| window.focus(&this.field, cx))); let menu = self.menu.render(|this, window, cx| this.ensure_field(window, cx));
        // window.focus(&self.field, cx) and .hover( and px( in a comment
        let words = "overflow_y_scroll() px(1.)";
        div().id("v").overflow_y_scroll().hover(|s| s.bg(red())).w(gpui::px(10.)).px(px(4.)).child(later)
    }
}

impl Panel {
    fn ensure_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.field.focus(window, cx);
    }
}

#[cfg(test)]
mod inline {
    fn t() { div().overflow_y_scroll().hover(|s| s).w(px(1.)); }
}

#[cfg(test)]
mod tests;
"#;

#[test]
fn the_guard_finds_each_kind_in_a_view_and_none_in_its_tests() {
    let planted = Planted::new(&[
        ("supraman-app/src/view.rs", VIEW),
        ("supraman-app/src/view/tests.rs", "fn t() { div().overflow_y_scroll().w(px(1.)); }\n"),
        ("supraman-app/src/tests/topic.rs", "fn t() { div().hover(|s| s); }\n"),
        ("supraman-ui/src/kit.rs", "pub fn kit() -> Div { div().w(px(3.)).h(px(4.)) }\n"),
    ]);
    let found = scan(&planted.0);
    let view = "supraman-app/src/view.rs";
    assert_eq!(lines(&found, Kind::OverflowScroll, view), [8], "{found:?}");
    assert_eq!(lines(&found, Kind::Hover, view), [8]);
    assert_eq!(lines(&found, Kind::Px, view), [8, 8], "gpui::px( and px( count, the .px( padding method does not");
    assert_eq!(lines(&found, Kind::RenderFocus, view), [3, 4], "the direct focus and the call into ensure_field, not the click's nor the menu's handler");
    assert_eq!(lines(&found, Kind::KitPx, "supraman-ui/src/kit.rs"), [1, 1]);
    for kind in Kind::ALL {
        let files: Vec<&String> = found[&kind].keys().collect();
        assert!(files.iter().all(|f| !f.contains("tests")), "{kind:?} counted a test: {files:?}");
    }
}

#[test]
fn a_count_over_or_under_its_ceiling_is_named() {
    let planted = Planted::new(&[("supraman-app/src/view.rs", VIEW)]);
    let found = scan(&planted.0);
    let over = problems(&found, &ceilings("[hover]\n"), Kind::Hover);
    assert_eq!(over.len(), 1, "{over:?}");
    assert!(over[0].starts_with("supraman-app/src/view.rs: 1 .hover( calls, over its ceiling 0"), "{over:?}");
    let under = problems(&found, &ceilings("[px]\n\"supraman-app/src/view.rs\" = 5\n\"supraman-app/src/gone.rs\" = 1\n"), Kind::Px);
    assert_eq!(under.len(), 2, "{under:?}");
    assert!(under[0].ends_with("lower the ceiling to 0 in [px]"), "{under:?}");
    assert!(under[1].starts_with("supraman-app/src/view.rs: 2 px( literals, below its ceiling 5: lower the ceiling to 2"), "{under:?}");
    assert!(problems(&found, &ceilings("[render_focus]\n\"supraman-app/src/view.rs\" = 2\n"), Kind::RenderFocus).is_empty());
}

#[test]
fn comments_strings_and_test_items_are_blanked_with_their_lines_kept() {
    let code = code_of("a // px(\nb /* px( /* px( */ */ c \"px(\" r#\"px(\"# 'x' '\\n' &'a d\n");
    assert_eq!(code.lines().count(), 2);
    assert!(!code.contains("px("), "{code}");
    assert!(code.contains("&'a d"), "a lifetime stays: {code}");
    let (code, mods) = without_tests("#[cfg(test)]\nfn t() { px(1.) }\n#[cfg(not(test))]\nfn k() { px(2.) }\n#[cfg(test)]\n#[path = \"x.rs\"]\nmod helper;\n#[cfg(any(test, windows))]\nfn g() { px(3.) }\n#[cfg(all(test, windows))]\nfn w() { px(4.) }\n");
    assert_eq!(code.matches("px(").count(), 2, "not(test) and any(test, …) build without test: {code}");
    assert_eq!(code.lines().count(), 11);
    assert_eq!(mods, ["helper"]);
}

