//! The viewer's line menu writing a line, a selection or a hunk through the tab, on fixture copies.

use gpui::{Entity, Modifiers, MouseButton, Pixels, Point, TestAppContext, VisualTestContext, point, px};
use supraman_editor::{ZedEditor, rows};
use supraman_gitman::fixture::{Fixture, run_git};
use supraman_gitman::{ChangeKind, FileChange};

use crate::git_view::diff::*;
use crate::git_view::GitView;
use crate::git_view::test_support::{Opts, open as open_tab, redraw, repaint, settle};

const PATH: &str = "picked.txt";
const BASE: &str = "one\ntwo\nthree\nfour\nfive\nsix\nseven\n";
const WORK: &str = "one\nTWO\nthree\nfour\nfive\nsix\nSEVEN\neight\n";

fn fixture() -> Fixture {
    let f = Fixture::copy();
    f.write(PATH, BASE.as_bytes());
    f.git(&["add", PATH]);
    f.git(&["commit", "-q", "--no-verify", "-m", "Add picked.txt"]);
    f.write(PATH, WORK.as_bytes());
    f
}

fn index(f: &Fixture) -> String {
    String::from_utf8(run_git(&f.git, &f.dir, &["cat-file", "blob", &format!(":{PATH}")], &[], None).stdout).unwrap()
}

fn worktree(f: &Fixture) -> String {
    std::fs::read_to_string(f.dir.join(PATH)).unwrap()
}

fn open<'a>(f: &Fixture, cx: &'a mut TestAppContext) -> (Entity<GitView>, &'a mut VisualTestContext) {
    open_tab(&f.dir, Opts::default().editor(), cx)
}

/// The viewer of `PATH`'s `of` diff in `layout`, its editors painted and the picks placed on them.
fn show(view: &Entity<GitView>, of: Of, layout: Layout, cx: &mut VisualTestContext) -> Entity<DiffView> {
    let change = Some(FileChange { kind: ChangeKind::Modified, path: PATH.into(), old_path: None, similarity: None });
    view.update(cx, |v, cx| v.open_diff(Target { of, path: PATH.into(), change }, cx));
    let d = view.read_with(cx, |v, cx| v.diff(cx).cloned()).expect("the viewer is open");
    d.update(cx, |d, cx| d.set_layout(layout, cx));
    settle(view, cx);
    d
}

fn hit(view: &Entity<GitView>, key: &str, cx: &mut VisualTestContext) -> Option<Point<Pixels>> {
    let hits = view.read_with(cx, |v, _| v.hits.clone());
    hits.borrow_mut().clear();
    redraw(view, cx);
    hits.borrow().get(key).map(|h| h.bounds.center())
}

fn click(view: &Entity<GitView>, key: &str, cx: &mut VisualTestContext) {
    let at = hit(view, key, cx).unwrap_or_else(|| panic!("{key} is painted"));
    cx.simulate_click(at, Modifiers::none());
    settle(view, cx);
}

fn editor(d: &Entity<DiffView>, new: bool, cx: &mut VisualTestContext) -> Entity<ZedEditor> {
    d.update_in(cx, |d, window, cx| match d.layout() {
        Layout::Inline => d.inline_editor(window, cx),
        Layout::Split => {
            let split = d.split(window, cx);
            let s = split.read(cx);
            if new { s.new.clone() } else { s.old.clone() }
        }
    })
}

/// A point inside the row of `editor` that shows `line` (inline: the line of the old side when `old`).
fn at_line(d: &Entity<DiffView>, line: u32, new: bool, cx: &mut VisualTestContext) -> Point<Pixels> {
    let editor = editor(d, new, cx);
    let inline = d.read_with(cx, |d, _| d.layout() == Layout::Inline);
    cx.update(|window, cx| {
        let row = if inline { rows::row_lines(&editor, cx).iter().position(|l| *l == Some((line, !new))).expect("the line is shown") as u32 } else { line };
        let top = rows::row_top(&editor, row, window, cx).expect("the row is in view");
        point(rows::left(&editor, cx).unwrap() + px(120.), top + px(4.))
    })
}

fn right_click(view: &Entity<GitView>, at: Point<Pixels>, cx: &mut VisualTestContext) {
    cx.simulate_mouse_down(at, MouseButton::Right, Modifiers::none());
    cx.simulate_mouse_up(at, MouseButton::Right, Modifiers::none());
    settle(view, cx);
}

#[gpui::test]
fn diff_picks_the_menu_stages_discards_and_unstages_a_line_s_hunk(cx: &mut TestAppContext) {
    let f = fixture();
    let (view, cx) = open(&f, cx);
    let d = show(&view, Of::Unstaged, Layout::Split, cx);
    right_click(&view, at_line(&d, 1, true, cx), cx);
    assert_eq!(d.read_with(cx, |d, _| d.menu_labels()), ["Stage this line", "Discard this line", "Stage hunk", "Discard hunk"]);
    click(&view, "menu-hunk-stage", cx);
    assert_eq!(index(&f), "one
TWO
three
four
five
six
seven
", "Stage hunk stages the removed and the added line of the first hunk only");
    assert_eq!(d.read_with(cx, |d, _| d.hunk_count()), 1, "the viewer reads the file again");

    right_click(&view, at_line(&d, 7, true, cx), cx);
    click(&view, "menu-hunk-discard", cx);
    assert_eq!(worktree(&f), "one
TWO
three
four
five
six
seven
", "Discard hunk puts the index's lines back");
    assert_eq!(view.read_with(cx, |v, _| v.toast.text().map(str::to_string)).as_deref(), Some("Discarded a hunk; you can undo it"));
    view.update(cx, |v, cx| v.undo_bar().update(cx, |u, cx| u.undo(cx)));
    settle(&view, cx);
    assert_eq!(worktree(&f), WORK, "Undo brings the hunk back");

    let d = show(&view, Of::Staged, Layout::Inline, cx);
    right_click(&view, at_line(&d, 1, true, cx), cx);
    assert_eq!(d.read_with(cx, |d, _| d.menu_labels()), ["Unstage this line", "Unstage hunk"]);
    click(&view, "menu-hunk-unstage", cx);
    assert_eq!(index(&f), BASE, "Unstage hunk");
}

#[gpui::test]
fn diff_picks_paint_no_hunk_or_line_button_and_the_menu_stages_one_line(cx: &mut TestAppContext) {
    let f = fixture();
    let (view, cx) = open(&f, cx);
    let d = show(&view, Of::Unstaged, Layout::Inline, cx);
    let at = at_line(&d, 7, true, cx);
    cx.simulate_mouse_move(at, None, Modifiers::none());
    settle(&view, cx);
    let buttons: Vec<String> = repaint(&view, cx).borrow().keys().filter(|k| k.starts_with("diff-hunk-") || k.starts_with("diff-line-")).cloned().collect();
    assert!(buttons.is_empty(), "no hunk or line button over the text: {buttons:?}");
    right_click(&view, at, cx);
    click(&view, "menu-line-stage", cx);
    assert_eq!(index(&f), "one
two
three
four
five
six
seven
eight
", "Stage this line stages that line only");

    let d = show(&view, Of::Staged, Layout::Split, cx);
    right_click(&view, at_line(&d, 7, true, cx), cx);
    click(&view, "menu-line-unstage", cx);
    assert_eq!(index(&f), BASE, "Unstage this line");
}

#[gpui::test]
fn diff_picks_the_line_menu_acts_on_the_line_or_the_selection(cx: &mut TestAppContext) {
    let f = fixture();
    let (view, cx) = open(&f, cx);
    let d = show(&view, Of::Unstaged, Layout::Inline, cx);
    let at = at_line(&d, 6, true, cx);
    right_click(&view, at, cx);
    assert_eq!(d.read_with(cx, |d, _| d.menu_labels()), ["Stage this line", "Discard this line", "Stage hunk", "Discard hunk"], "one line: its items, then its hunk's");
    click(&view, "menu-line-discard", cx);
    assert_eq!(worktree(&f), "one\nTWO\nthree\nfour\nfive\nsix\neight\n", "Discard this line drops the added SEVEN only");

    let editor = editor(&d, true, cx);
    let (first, last) = cx.update(|_, cx| {
        let lines = rows::row_lines(&editor, cx);
        (lines.iter().position(|l| *l == Some((1, true))).unwrap() as u32, lines.iter().position(|l| *l == Some((1, false))).unwrap() as u32)
    });
    cx.update(|window, cx| rows::select_rows(&editor, first..last + 1, window, cx));
    let at = at_line(&d, 1, true, cx);
    right_click(&view, at, cx);
    assert_eq!(d.read_with(cx, |d, _| d.menu_labels()), ["Stage selected lines", "Discard selection"]);
    click(&view, "menu-line-stage", cx);
    assert_eq!(index(&f), "one\nTWO\nthree\nfour\nfive\nsix\nseven\n", "Stage selected lines stages the removed and the added line");

    show(&view, Of::Staged, Layout::Inline, cx);
    let d = view.read_with(cx, |v, cx| v.diff(cx).cloned()).unwrap();
    let at = at_line(&d, 1, true, cx);
    right_click(&view, at, cx);
    assert_eq!(d.read_with(cx, |d, _| d.menu_labels()), ["Unstage this line", "Unstage hunk"], "one line: its items, then its hunk's");
}

#[gpui::test]
fn diff_picks_wait_while_another_write_runs(cx: &mut TestAppContext) {
    let f = fixture();
    let (view, cx) = open(&f, cx);
    let d = show(&view, Of::Unstaged, Layout::Split, cx);
    let gate = view.read_with(cx, |v, _| v.gate().clone());
    let at = at_line(&d, 1, true, cx);
    let (blocked, dimmed) = d.update_in(cx, |d, window, cx| {
        gate.update(cx, |g, cx| g.request(supraman_gitman::write::LockSet::refresh(), |_| Ok(()), cx));
        d.open_line_menu(at, window, cx);
        let blocked = d.picks_blocked(cx);
        d.pick(Act::Hunk(supraman_gitman::write::lines::Pick::Stage, 0), cx);
        (blocked, d.menu_dimmed())
    });
    assert_eq!(blocked, Some(crate::git_view::write_gate::WORKING));
    assert_eq!(dimmed, vec![Some(crate::git_view::write_gate::WORKING.to_string()); 4], "every item dimmed with why");
    settle(&view, cx);
    assert_eq!(index(&f), BASE, "a pick while another write runs stages nothing");
    assert_eq!(view.read_with(cx, |v, _| v.toast.text().map(str::to_string)).as_deref(), Some(crate::git_view::write_gate::WORKING), "it says why");
    right_click(&view, at_line(&d, 1, false, cx), cx);
    assert_eq!(d.read_with(cx, |d, _| d.menu_dimmed()), vec![None; 4], "once the write ends");
    click(&view, "menu-hunk-stage", cx);
    assert_eq!(index(&f), "one
TWO
three
four
five
six
seven
");
}

#[gpui::test]
fn diff_picks_wait_while_a_merge_is_under_way(cx: &mut TestAppContext) {
    let f = fixture();
    let head = f.git(&["rev-parse", "HEAD"]);
    std::fs::write(f.dir.join(".git/MERGE_HEAD"), format!("{head}
")).unwrap();
    let (view, cx) = open(&f, cx);
    let d = show(&view, Of::Unstaged, Layout::Split, cx);
    let why = "Lines cannot be staged or discarded during a merge or rebase";
    assert_eq!(d.read_with(cx, |d, cx| d.picks_blocked(cx)), Some(why));
    right_click(&view, at_line(&d, 1, true, cx), cx);
    assert_eq!(d.read_with(cx, |d, _| d.menu_dimmed()), vec![Some(why.to_string()); 4]);
    click(&view, "menu-hunk-stage", cx);
    assert_eq!(index(&f), BASE, "a dimmed item stages nothing");
}
