use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};
use supraman_gitman::fixture::{Fixture, git_in};
use supraman_gitman::{ChangeKind, FileChange};

use super::*;
use crate::git_view::GitView;
use crate::git_view::details::FileView;
use crate::git_view::tests::{open as open_tab, redraw};

const M_OLD: &str = "fn one() {}\nfn two() {}\nfn three() {}\nfn four() {}\nfn five() {}\nfn six() {}\nfn seven() {}\nfn eight() {}\n";
const M_NEW: &str = "fn one() {}\nfn TWO() {}\nfn three() {}\nfn four() {}\nfn five() {}\nfn six() {}\nfn seven() {}\nfn eight() {}\nfn nine() {}\n";
const MOVED: &str = "first\nsecond\nthird\n";

/// Commits on the fixture's `side` worktree: `base` with the files, `change` modifying, adding, deleting,
/// renaming and rewriting a binary, and `merge` of a topic whose second parent already had its line.
struct Repo {
    f: Fixture,
    base: String,
    change: String,
    merge: String,
}

fn git(f: &Fixture, args: &[&str]) -> String {
    git_in(&f.git, &f.worktree, args, &[], None)
}

fn write(f: &Fixture, path: &str, bytes: &[u8]) {
    std::fs::write(f.worktree.join(path), bytes).unwrap();
}

fn commit(f: &Fixture, message: &str) -> String {
    git(f, &["add", "-A"]);
    git(f, &["commit", "-q", "--no-verify", "-m", message]);
    git(f, &["rev-parse", "HEAD"])
}

fn big(lines: usize, changed: usize) -> String {
    (0..lines).map(|i| if i % 1000 == 500 && changed > 0 { format!("line {i} changed\n") } else { format!("line {i} of a long file\n") }).collect()
}

fn repo() -> Repo {
    let f = Fixture::new();
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
    git(&f, &["mv", "mv.txt", "moved.txt"]);
    write(&f, "pic.bin", &[0, 9, 9, 9]);
    write(&f, "big.txt", big(5000, 1).as_bytes());
    let change = commit(&f, "Change");
    git(&f, &["checkout", "-q", "-b", "topic"]);
    write(&f, "f.txt", b"a\nb\n");
    commit(&f, "Topic");
    git(&f, &["checkout", "-q", "side"]);
    write(&f, "g.txt", b"g\n");
    commit(&f, "Side");
    git(&f, &["merge", "-q", "--no-ff", "--no-verify", "-m", "Merge topic", "topic"]);
    let merge = git(&f, &["rev-parse", "HEAD"]);
    Repo { f, base, change, merge }
}

fn open<'a>(r: &Repo, cx: &'a mut TestAppContext) -> (Entity<GitView>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        supraman_editor::init(supraman_ui::fonts::MONO, 13., cx);
        bind_keys(cx);
    });
    open_tab(&r.f.dir, cx)
}

fn change(r: &Repo, path: &str) -> Option<FileChange> {
    supraman_gitman::changed_files(&r.f.repo(), &r.change).unwrap().into_iter().find(|c| c.path == path)
}

fn show(view: &Entity<GitView>, r: &Repo, path: &str, cx: &mut VisualTestContext) -> Entity<DiffView> {
    let target = Target::commit(r.change.clone(), Some(r.base.clone()), path.into(), change(r, path));
    view.update(cx, |v, cx| v.open_diff(target, cx));
    cx.run_until_parked();
    redraw(view, cx);
    view.read_with(cx, |v, _| v.diff().cloned()).expect("the viewer is open")
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
    assert!(d.read_with(cx, |d, _| matches!(d.content(), Content::Binary)), "binary: a placeholder");
    for layout in [Layout::Split, Layout::Inline] {
        d.update(cx, |d, cx| d.set_layout(layout, cx));
        redraw(&view, cx);
        assert!(d.read_with(cx, |d, _| matches!(d.content(), Content::Binary) && d.text().is_none()), "no editor for a binary file");
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
    let gone = show(&view, &r, "gone.txt", cx);
    gone.update(cx, |d, cx| d.set_mode(Mode::File, cx));
    redraw(&view, cx);
    let file = gone.update_in(cx, |d, window, cx| d.file_editor(window, cx)).unwrap();
    assert_eq!(cx.update(|_, cx| supraman_editor::text(&file, cx)), "bye\n", "a deleted file as it was");
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
    let d = view.read_with(cx, |v, _| v.diff().cloned()).expect("a file's click opens the viewer");
    assert_eq!(texts(&d, cx), ("a\n".into(), "a\nb\n".into()), "against the first parent, which lacked the line");
    assert_eq!(rows(&d, cx), [(1..1, 1..2)]);
}

#[gpui::test]
fn diff_view_opens_a_5000_line_file_without_a_stall(cx: &mut TestAppContext) {
    let r = repo();
    let (view, cx) = open(&r, cx);
    for layout in [Layout::Split, Layout::Inline] {
        view.update(cx, |v, _| v.diff_layout = layout);
        supraman_ui::stall::take();
        let started = std::time::Instant::now();
        let d = show(&view, &r, "big.txt", cx);
        redraw(&view, cx);
        let took = started.elapsed();
        assert_eq!(texts(&d, cx).1.lines().count(), 5000);
        let stalls = supraman_ui::stall::take();
        assert!(stalls.is_empty(), "{layout:?} opened with stalls: {stalls:?}");
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
        view.read_with(cx, |v, _| {
            let ix = v.table.selected().expect("a row is selected");
            (v.selected(), v.hits.borrow().get(&format!("git-row-{ix}")).map(|h| h.bounds.origin))
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
    assert!(view.read_with(cx, |v, _| v.diff().is_none()));
    assert_eq!(at(&view, cx), before, "selection and scroll kept");
    let d = show(&view, &r, "m.rs", cx);
    assert_eq!(d.read_with(cx, |d, _| d.layout()), Layout::Inline, "the next file opens in the last layout");
    cx.simulate_keystrokes("escape");
    redraw(&view, cx);
    assert!(view.read_with(cx, |v, _| v.diff().is_none()), "Esc closes too");
}
