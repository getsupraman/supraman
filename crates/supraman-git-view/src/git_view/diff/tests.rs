mod cap;
mod media;

use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};
use supraman_gitman::fixture::Fixture;
use supraman_gitman::{ChangeKind, FileChange};

use super::*;
use crate::git_view::GitView;
use crate::git_view::details::FileView;
use crate::git_view::test_support::{Opts, open as open_tab, redraw};

const M_OLD: &str = "fn one() {}\nfn two() {}\nfn three() {}\nfn four() {}\nfn five() {}\nfn six() {}\nfn seven() {}\nfn eight() {}\n";
const M_NEW: &str = "fn one() {}\nfn TWO() {}\nfn three() {}\nfn four() {}\nfn five() {}\nfn six() {}\nfn seven() {}\nfn eight() {}\nfn nine() {}\n";
const MOVED: &str = "first\nsecond\nthird\n";

/// Commits on the fixture's `side` worktree: `base`, `change` (modify, add, delete, rename, binary) and a `merge`.
struct Repo {
    f: Fixture,
    base: String,
    change: String,
    merge: String,
}

fn write(f: &Fixture, path: &str, bytes: &[u8]) {
    std::fs::write(f.worktree.join(path), bytes).unwrap();
}

fn commit(f: &Fixture, message: &str) -> String {
    f.at(&f.worktree, &["add", "-A"]);
    f.at(&f.worktree, &["commit", "-q", "--no-verify", "-m", message]);
    f.at(&f.worktree, &["rev-parse", "HEAD"])
}

fn big(lines: usize, changed: usize) -> String {
    (0..lines).map(|i| if i % 1000 == 500 && changed > 0 { format!("line {i} changed\n") } else { format!("line {i} of a long file\n") }).collect()
}

fn repo() -> Repo {
    let f = Fixture::copy();
    write(&f, "m.rs", M_OLD.as_bytes());
    write(&f, "gone.txt", b"bye\n");
    write(&f, "mv.txt", MOVED.as_bytes());
    write(&f, "pic.bin", &[0, 1, 2, 3]);
    write(&f, "big.txt", big(5000, 0).as_bytes());
    write(&f, "f.txt", b"a\n");
    let base = commit(&f, "Base");
    write(&f, "m.rs", M_NEW.as_bytes());
    write(&f, "new.txt", b"hello\nworld\n");
    std::fs::remove_file(f.worktree.join("gone.txt")).unwrap();
    f.at(&f.worktree, &["mv", "mv.txt", "moved.txt"]);
    write(&f, "pic.bin", &[0, 9, 9, 9]);
    write(&f, "big.txt", big(5000, 1).as_bytes());
    let change = commit(&f, "Change");
    f.at(&f.worktree, &["checkout", "-q", "-b", "topic"]);
    write(&f, "f.txt", b"a\nb\n");
    commit(&f, "Topic");
    f.at(&f.worktree, &["checkout", "-q", "side"]);
    write(&f, "g.txt", b"g\n");
    commit(&f, "Side");
    f.at(&f.worktree, &["merge", "-q", "--no-ff", "--no-verify", "-m", "Merge topic", "topic"]);
    let merge = f.at(&f.worktree, &["rev-parse", "HEAD"]);
    Repo { f, base, change, merge }
}

fn open<'a>(r: &Repo, cx: &'a mut TestAppContext) -> (Entity<GitView>, &'a mut VisualTestContext) {
    open_tab(&r.f.dir, Opts::default().editor(), cx)
}

fn change(r: &Repo, path: &str) -> Option<FileChange> {
    supraman_gitman::changed_files(&r.f.repo(), &r.change).unwrap().into_iter().find(|c| c.path == path)
}

fn show(view: &Entity<GitView>, r: &Repo, path: &str, cx: &mut VisualTestContext) -> Entity<DiffView> {
    let target = Target::commit(r.change.clone(), Some(r.base.clone()), path.into(), change(r, path));
    view.update(cx, |v, cx| v.open_diff(target, cx));
    cx.run_until_parked();
    redraw(view, cx);
    view.read_with(cx, |v, cx| v.diff(cx).cloned()).expect("the viewer is open")
}

fn rows(d: &Entity<DiffView>, cx: &mut VisualTestContext) -> Vec<(std::ops::Range<u32>, std::ops::Range<u32>)> {
    d.read_with(cx, |d, _| match d.content() {
        Content::Text(t) => t.diff.hunks.iter().map(|h| (h.old_rows.clone(), h.new_rows.clone())).collect(),
        _ => panic!("not a text diff"),
    })
}

fn texts(d: &Entity<DiffView>, cx: &mut VisualTestContext) -> (String, String) {
    d.read_with(cx, |d, cx| match d.content() {
        Content::Text(t) => (t.diff.old.read(cx).text(), t.diff.new.read(cx).text()),
        _ => panic!("not a text diff"),
    })
}

/// Split's two sides as displayed, and the inline editor's text, after showing both layouts.
fn both_layouts(view: &Entity<GitView>, d: &Entity<DiffView>, cx: &mut VisualTestContext) -> ((u32, u32), String) {
    d.update(cx, |d, cx| d.set_layout(Layout::Split, cx));
    redraw(view, cx);
    let split = d.update_in(cx, |d, window, cx| d.split(window, cx));
    let rows = split.update(cx, |s, cx| s.display_rows(cx));
    d.update(cx, |d, cx| d.set_layout(Layout::Inline, cx));
    redraw(view, cx);
    let inline = d.update_in(cx, |d, window, cx| d.inline_editor(window, cx));
    let text = cx.update(|_, cx| supraman_editor::text(&inline, cx));
    (rows, text)
}

#[gpui::test]
fn diff_view_shows_each_kind_of_change_split_and_inline(cx: &mut TestAppContext) {
    let r = repo();
    let (view, cx) = open(&r, cx);

    let d = show(&view, &r, "m.rs", cx);
    assert_eq!(texts(&d, cx), (M_OLD.into(), M_NEW.into()));
    assert_eq!(rows(&d, cx), [(1..2, 1..2), (8..8, 8..9)], "a changed line and an added one");
    let ((old, new), inline) = both_layouts(&view, &d, cx);
    assert_eq!(old, new, "the sides line up row for row");
    let removed_above = M_NEW.replacen("fn TWO() {}
", "fn two() {}
fn TWO() {}
", 1);
    assert_eq!(inline, removed_above, "inline: one column, the removed row above the one that replaced it");
    d.read_with(cx, |d, _| match d.content() {
        Content::Text(t) => assert!(!t.diff.hunks[0].new_words.is_empty() && !t.diff.hunks[0].old_words.is_empty(), "the changed word is marked"),
        _ => unreachable!(),
    });

    let d = show(&view, &r, "new.txt", cx);
    assert_eq!(texts(&d, cx), (String::new(), "hello\nworld\n".into()), "added: nothing before");
    assert_eq!(rows(&d, cx), [(0..0, 0..2)]);
    let ((old, new), inline) = both_layouts(&view, &d, cx);
    assert_eq!((old, new, inline.as_str()), (new, new, "hello\nworld\n"), "the empty side keeps its rows blank");

    let d = show(&view, &r, "gone.txt", cx);
    assert_eq!(texts(&d, cx), ("bye\n".into(), String::new()), "deleted: nothing after");
    assert_eq!(rows(&d, cx), [(0..1, 0..0)]);
    let ((old, new), inline) = both_layouts(&view, &d, cx);
    assert_eq!((old, inline.as_str()), (new, "bye
"), "inline shows the removed rows");

    let d = show(&view, &r, "moved.txt", cx);
    assert_eq!(d.read_with(cx, |d, _| (d.target.kind(), d.target.old_path().to_string())), (Some(ChangeKind::Renamed), "mv.txt".into()));
    assert_eq!(texts(&d, cx), (MOVED.into(), MOVED.into()), "the old side read at the old path");
    assert_eq!(rows(&d, cx), [], "a pure rename changes no line");

    let d = show(&view, &r, "pic.bin", cx);
    assert!(d.read_with(cx, |d, _| matches!(d.content(), Content::Binary(_))), "binary: a placeholder");
    for layout in [Layout::Split, Layout::Inline] {
        d.update(cx, |d, cx| d.set_layout(layout, cx));
        redraw(&view, cx);
        assert!(d.read_with(cx, |d, _| matches!(d.content(), Content::Binary(_)) && d.text().is_none()), "no editor for a binary file");
    }
}

#[gpui::test]
fn diff_view_moves_between_changes_and_shows_the_whole_file(cx: &mut TestAppContext) {
    let r = repo();
    let (view, cx) = open(&r, cx);
    let d = show(&view, &r, "big.txt", cx);
    let hunks = rows(&d, cx);
    assert_eq!(hunks.len(), 5, "{hunks:?}");
    d.update(cx, |d, cx| d.set_layout(Layout::Split, cx));
    redraw(&view, cx);
    let new = d.update_in(cx, |d, window, cx| d.split(window, cx)).read_with(cx, |s, _| s.new.clone());
    let mut seen = Vec::new();
    for _ in 0..hunks.len() {
        cx.simulate_click(view.read_with(cx, |v, _| v.hits.borrow()["diff-next"].bounds.center()), Modifiers::none());
        redraw(&view, cx);
        seen.push((d.read_with(cx, |d, _| d.current()), cx.update(|_, cx| supraman_editor::cursor_row(&new, cx))));
    }
    let want: Vec<_> = hunks.iter().enumerate().map(|(i, (_, n))| (Some(i), n.start)).collect();
    assert_eq!(seen, want, "next visits every change in order");
    cx.simulate_keystrokes("shift-f7");
    redraw(&view, cx);
    assert_eq!(d.read_with(cx, |d, _| d.current()), Some(hunks.len() - 2), "Shift+F7 goes back one");
    cx.simulate_click(view.read_with(cx, |v, _| v.hits.borrow()["diff-previous"].bounds.center()), Modifiers::none());
    redraw(&view, cx);
    assert_eq!(cx.update(|_, cx| supraman_editor::cursor_row(&new, cx)), hunks[hunks.len() - 3].1.start);

    d.update(cx, |d, cx| d.set_layout(Layout::Inline, cx));
    redraw(&view, cx);
    cx.simulate_keystrokes("f7");
    redraw(&view, cx);
    let inline = d.update_in(cx, |d, window, cx| d.inline_editor(window, cx));
    assert_eq!((d.read_with(cx, |d, _| d.current()), cx.update(|_, cx| supraman_editor::cursor_row(&inline, cx))), (Some(0), hunks[0].1.start));

    cx.simulate_click(view.read_with(cx, |v, _| v.hits.borrow()["diff-mode-file"].bounds.center()), Modifiers::none());
    redraw(&view, cx);
    let file = d.update_in(cx, |d, window, cx| d.file_editor(window, cx)).expect("a text file");
    assert_eq!(cx.update(|_, cx| supraman_editor::text(&file, cx)), big(5000, 1), "File View: the whole file at the commit");
    cx.simulate_click(view.read_with(cx, |v, _| v.hits.borrow()["diff-mode-diff"].bounds.center()), Modifiers::none());
    redraw(&view, cx);
    assert_eq!(d.read_with(cx, |d, _| d.mode()), Mode::Diff, "Diff View again");
    for (key, layout) in [("diff-layout-split", Layout::Split), ("diff-layout-inline", Layout::Inline)] {
        cx.simulate_click(view.read_with(cx, |v, _| v.hits.borrow()[key].bounds.center()), Modifiers::none());
        redraw(&view, cx);
        assert_eq!(d.read_with(cx, |d, _| d.layout()), layout, "{key}");
    }
    let gone = show(&view, &r, "gone.txt", cx);
    gone.update(cx, |d, cx| d.set_mode(Mode::File, cx));
    redraw(&view, cx);
    let file = gone.update_in(cx, |d, window, cx| d.file_editor(window, cx)).unwrap();
    assert_eq!(cx.update(|_, cx| supraman_editor::text(&file, cx)), "bye\n", "a deleted file as it was");
}

#[gpui::test]
fn diff_view_opens_at_its_first_change_split_and_inline(cx: &mut TestAppContext) {
    let r = repo();
    let (view, cx) = open(&r, cx);
    for layout in [Layout::Split, Layout::Inline] {
        view.update(cx, |v, cx| {
            v.close_diff(cx);
            v.viewer.update(cx, |w, _| w.layout = layout);
        });
        let d = show(&view, &r, "big.txt", cx);
        assert_eq!(d.read_with(cx, |d, _| d.layout()), layout);
        let first = rows(&d, cx)[0].1.start;
        assert!(first > 100, "the first change is far down: row {first}");
        assert_eq!(d.read_with(cx, |d, _| d.current()), Some(0), "{layout:?}: on the first change");
        let editor = match layout {
            Layout::Split => d.update_in(cx, |d, window, cx| d.split(window, cx)).read_with(cx, |s, _| s.new.clone()),
            Layout::Inline => d.update_in(cx, |d, window, cx| d.inline_editor(window, cx)),
        };
        let (cursor, top) = cx.update(|_, cx| (supraman_editor::cursor_row(&editor, cx), supraman_editor::scroll_row(&editor, cx)));
        assert_eq!(cursor, first, "{layout:?}");
        assert!(top > (first - 10) as f64 && top <= first as f64, "{layout:?}: the view starts just above the change: top {top}, change {first}");
    }
}

#[gpui::test]
fn diff_view_reloaded_after_a_change_below_keeps_its_top_row_and_change(cx: &mut TestAppContext) {
    let r = repo();
    let (view, cx) = open(&r, cx);
    for layout in [Layout::Split, Layout::Inline] {
        view.update(cx, |v, cx| {
            v.close_diff(cx);
            v.viewer.update(cx, |w, _| w.layout = layout);
        });
        let d = show(&view, &r, "big.txt", cx);
        d.update_in(cx, |d, window, cx| d.next_change(window, cx));
        redraw(&view, cx);
        let editor = |d: &Entity<DiffView>, cx: &mut VisualTestContext| match layout {
            Layout::Split => d.update_in(cx, |d, window, cx| d.split(window, cx)).read_with(cx, |s, _| s.new.clone()),
            Layout::Inline => d.update_in(cx, |d, window, cx| d.inline_editor(window, cx)),
        };
        let top = |d: &Entity<DiffView>, cx: &mut VisualTestContext| {
            let editor = editor(d, cx);
            cx.update(|_, cx| supraman_editor::scroll_row(&editor, cx))
        };
        let before = top(&d, cx);
        assert!(before > 1000., "{layout:?}: on the second change: top {before}");
        let (old, new) = texts(&d, cx);
        let below = new.replace("line 4990 of a long file", "line 4990 edited by an agent");
        let task = cx.update(|_, cx| Diff::compute(old, below, None, cx));
        cx.run_until_parked();
        let diff = cx.foreground_executor().block_test(task);
        d.update(cx, |d, cx| d.show(diff, "UTF-8".into(), Vec::new(), cx));
        redraw(&view, cx);
        assert_eq!(rows(&d, cx).len(), 6, "{layout:?}: the reload has one more change");
        assert_eq!(top(&d, cx), before, "{layout:?}: the same row on top");
        assert_eq!(d.read_with(cx, |d, _| d.current()), Some(1), "{layout:?}: the same change current");
    }
}

#[gpui::test]
fn diff_view_compares_a_merge_with_its_first_parent(cx: &mut TestAppContext) {
    let r = repo();
    let (view, cx) = open(&r, cx);
    view.update(cx, |v, cx| v.select_sha(&r.merge, cx));
    cx.run_until_parked();
    let panel = view.read_with(cx, |v, _| v.panel().clone());
    panel.update(cx, |p, cx| p.set_view(FileView::Path, cx));
    redraw(&view, cx);
    let row = view.read_with(cx, |v, _| v.hits.borrow()["commit-files-row-f.txt"].bounds.center());
    cx.simulate_click(row, Modifiers::none());
    cx.run_until_parked();
    redraw(&view, cx);
    let d = view.read_with(cx, |v, cx| v.diff(cx).cloned()).expect("a file's click opens the viewer");
    assert_eq!(texts(&d, cx), ("a\n".into(), "a\nb\n".into()), "against the first parent, which lacked the line");
    assert_eq!(rows(&d, cx), [(1..1, 1..2)]);
}

#[gpui::test]
fn diff_view_opens_a_5000_line_file_without_a_stall(cx: &mut TestAppContext) {
    let r = repo();
    let (view, cx) = open(&r, cx);
    for layout in [Layout::Split, Layout::Inline] {
        view.update(cx, |v, cx| v.viewer.update(cx, |w, _| w.layout = layout));
        let started = std::time::Instant::now();
        let target = Target::commit(r.change.clone(), Some(r.base.clone()), "big.txt".into(), change(&r, "big.txt"));
        let d = view.update(cx, |v, cx| {
            v.open_diff(target, cx);
            v.diff(cx).cloned().expect("the viewer is open")
        });
        assert!(d.read_with(cx, |d, _| matches!(d.content(), Content::Loading)), "{layout:?}: read and diffed off the UI thread");
        cx.run_until_parked();
        redraw(&view, cx);
        redraw(&view, cx);
        let took = started.elapsed();
        assert_eq!(texts(&d, cx).1.lines().count(), 5000);
        log::info!("{layout:?}: 5,000 lines opened in {} ms", took.as_millis());
    }
}

#[gpui::test]
fn diff_view_closes_back_to_the_graph_as_it_was(cx: &mut TestAppContext) {
    let r = repo();
    let (view, cx) = open(&r, cx);
    view.update(cx, |v, cx| v.select_sha(&r.change, cx));
    redraw(&view, cx);
    let at = |view: &Entity<GitView>, cx: &mut VisualTestContext| {
        view.read_with(cx, |v, cx| {
            let ix = v.table.selected().expect("a row is selected");
            (v.selected(cx), v.hits.borrow().get(&format!("git-row-{ix}")).map(|h| h.bounds.origin))
        })
    };
    let before = at(&view, cx);
    assert!(before.1.is_some(), "the selected row is painted");
    let d = show(&view, &r, "m.rs", cx);
    d.update(cx, |d, cx| d.set_layout(Layout::Inline, cx));
    view.read_with(cx, |v, _| v.hits.borrow_mut().clear());
    redraw(&view, cx);
    assert!(view.read_with(cx, |v, _| !v.hits.borrow().contains_key("git-col-sha")), "the viewer takes the graph's place");
    assert!(view.read_with(cx, |v, _| v.hits.borrow().contains_key("commit-sha")), "the panel stays");
    cx.simulate_click(view.read_with(cx, |v, _| v.hits.borrow()["diff-close"].bounds.center()), Modifiers::none());
    view.read_with(cx, |v, _| v.hits.borrow_mut().clear());
    redraw(&view, cx);
    assert!(view.read_with(cx, |v, cx| v.diff(cx).is_none()));
    assert_eq!(at(&view, cx), before, "selection and scroll kept");
    let d = show(&view, &r, "m.rs", cx);
    assert_eq!(d.read_with(cx, |d, _| d.layout()), Layout::Inline, "the next file opens in the last layout");
    cx.simulate_keystrokes("escape");
    redraw(&view, cx);
    assert!(view.read_with(cx, |v, cx| v.diff(cx).is_none()), "Esc closes too");
}

#[test]
fn diff_tints_keep_the_text_readable_and_the_stripes_seen_in_a_dark_and_a_light_theme() {
    use supraman_ui::themes::{SECONDARY_FLOOR, contrast, dark_modern, from_zed, one_dark, parse_family};
    let light = from_zed(&one_dark(), &parse_family(supraman_ui::themes::selftest_light::LIGHT).unwrap().themes[0]).theme;
    for theme in [dark_modern().theme, light] {
        let c = supraman_editor::diff_colors(&theme);
        let (bg, text) = (theme.colors().editor_background, theme.colors().editor_foreground);
        for tint in [c.removed, c.added] {
            let line = crate::git_view::colours::blend(tint.line, bg);
            let word = crate::git_view::colours::blend(tint.word, line);
            assert!(contrast(text, word) >= SECONDARY_FLOOR, "{}: text on a changed word {:.2}:1", theme.name, contrast(text, word));
            assert!(contrast(word, line) > 1.15, "{}: a word stands out of its line ({:.2}:1)", theme.name, contrast(word, line));
            assert!(contrast(line, bg) > 1.15, "{}: a changed line stands out ({:.2}:1)", theme.name, contrast(line, bg));
        }
        assert!(contrast(c.stripes, bg) > 1.15, "{}: the filler's stripes show", theme.name);
        let (light, dark) = super::image::cells(theme.colors());
        assert!(contrast(crate::git_view::colours::blend(dark, light), light) > 1.15, "{}: the checkerboard's cells differ", theme.name);
    }
}

#[gpui::test]
fn a_stash_file_opens_its_change_against_the_base(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    cx.update(|cx| supraman_editor::init(supraman_ui::fonts::MONO, 13., cx));
    let (view, cx) = open_tab(&f.dir, Opts::default(), cx);
    view.update(cx, |v, cx| v.select_sha(f.sha("stash_message_index"), cx));
    cx.run_until_parked();
    redraw(&view, cx);
    let files = view.read_with(cx, |v, cx| v.panel.read(cx).file_order(f.sha("stash_message")));
    let of: Vec<(&Of, &str)> = files.iter().map(|t| (&t.of, t.path.as_str())).collect();
    assert_eq!(of, [(&Of::Commit { sha: f.sha("stash_message").into(), parent: Some(f.sha("merge").into()) }, "a.txt")], "the stash commit against its base");
    view.update(cx, |v, cx| v.open_diff(files[0].clone(), cx));
    cx.run_until_parked();
    let d = view.read_with(cx, |v, cx| v.diff(cx).cloned()).expect("the viewer is open");
    assert_eq!(texts(&d, cx), ("one\ntwo\n".to_string(), "one\ntwo\nstashed\n".to_string()));
}
