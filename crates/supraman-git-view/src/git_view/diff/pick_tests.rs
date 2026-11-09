//! The viewer's hunk buttons, line + and − and line menu writing through the tab, on fixture copies (C196).

use gpui::{Entity, Modifiers, MouseButton, Pixels, Point, TestAppContext, VisualTestContext, point, px};
use supraman_editor::{ZedEditor, rows};
use supraman_gitman::fixture::{Fixture, run_git};
use supraman_gitman::{ChangeKind, FileChange};

use super::*;
use crate::git_view::GitView;
use crate::git_view::tests::{open as open_tab, redraw};

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
    cx.update(|cx| supraman_editor::init(supraman_ui::fonts::MONO, 13., cx));
    open_tab(&f.dir, cx)
}

/// The viewer of `PATH`'s `of` diff in `layout`, its editors painted and the picks placed on them.
fn show(view: &Entity<GitView>, of: Of, layout: Layout, cx: &mut VisualTestContext) -> Entity<DiffView> {
    let change = Some(FileChange { kind: ChangeKind::Modified, path: PATH.into(), old_path: None, similarity: None });
    view.update(cx, |v, cx| v.open_diff(Target { of, path: PATH.into(), change }, cx));
    let d = view.read_with(cx, |v, _| v.diff().cloned()).expect("the viewer is open");
    d.update(cx, |d, cx| d.set_layout(layout, cx));
    settle(view, cx);
    d
}

fn settle(view: &Entity<GitView>, cx: &mut VisualTestContext) {
    for _ in 0..3 {
        cx.run_until_parked();
        redraw(view, cx);
    }
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

#[gpui::test]
fn diff_picks_hunk_buttons_stage_and_discard_and_unstage(cx: &mut TestAppContext) {
    let f = fixture();
    let (view, cx) = open(&f, cx);
    let d = show(&view, Of::Unstaged, Layout::Split, cx);
    assert!(hit(&view, &hunk_key(0, "discard"), cx).is_some() && hit(&view, &hunk_key(1, "stage"), cx).is_some(), "each hunk has its buttons");
    assert!(hit(&view, &hunk_key(0, "unstage"), cx).is_none(), "no Unstage on the Unstaged diff");

    click(&view, &hunk_key(0, "stage"), cx);
    assert_eq!(index(&f), "one\nTWO\nthree\nfour\nfive\nsix\nseven\n", "Stage hunk stages the first hunk only");
    assert_eq!(d.read_with(cx, |d, _| d.hunk_count()), 1, "the viewer reads the file again");

    click(&view, &hunk_key(0, "discard"), cx);
    assert_eq!(worktree(&f), "one\nTWO\nthree\nfour\nfive\nsix\nseven\n", "Discard hunk puts the index's lines back");
    assert_eq!(view.read_with(cx, |v, _| v.toast.text().map(str::to_string)).as_deref(), Some("Discarded a hunk; you can undo it"));
    view.update(cx, |v, cx| v.undo_bar().update(cx, |u, cx| u.undo(cx)));
    settle(&view, cx);
    assert_eq!(worktree(&f), "one\nTWO\nthree\nfour\nfive\nsix\nSEVEN\neight\n", "Undo brings the hunk back");

    show(&view, Of::Staged, Layout::Inline, cx);
    assert!(hit(&view, &hunk_key(0, "stage"), cx).is_none(), "no Stage on the Staged diff");
    click(&view, &hunk_key(0, "unstage"), cx);
    assert_eq!(index(&f), BASE, "Unstage hunk");
}

#[gpui::test]
fn diff_picks_a_line_stages_and_unstages_from_its_gutter_button(cx: &mut TestAppContext) {
    let f = fixture();
    let (view, cx) = open(&f, cx);
    let d = show(&view, Of::Unstaged, Layout::Inline, cx);
    let at = at_line(&d, 7, true, cx);
    cx.simulate_mouse_move(at, None, Modifiers::none());
    settle(&view, cx);
    assert_eq!(d.read_with(cx, |d, _| d.hovered()), Some(Line { new: true, row: 7 }), "the pointer is over the added line");
    click(&view, "diff-line-stage", cx);
    assert_eq!(index(&f), "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\n", "Stage this line stages that line only");

    let d = show(&view, Of::Staged, Layout::Split, cx);
    let at = at_line(&d, 7, true, cx);
    cx.simulate_mouse_move(at, None, Modifiers::none());
    settle(&view, cx);
    assert!(hit(&view, "diff-line-stage", cx).is_none());
    click(&view, "diff-line-unstage", cx);
    assert_eq!(index(&f), BASE, "Unstage this line");
}

fn right_click(view: &Entity<GitView>, at: Point<Pixels>, cx: &mut VisualTestContext) {
    cx.simulate_mouse_down(at, MouseButton::Right, Modifiers::none());
    cx.simulate_mouse_up(at, MouseButton::Right, Modifiers::none());
    settle(view, cx);
}

#[gpui::test]
fn diff_picks_the_line_menu_acts_on_the_line_or_the_selection(cx: &mut TestAppContext) {
    let f = fixture();
    let (view, cx) = open(&f, cx);
    let d = show(&view, Of::Unstaged, Layout::Inline, cx);
    let at = at_line(&d, 6, true, cx);
    right_click(&view, at, cx);
    assert_eq!(d.read_with(cx, |d, _| d.menu_labels()), ["Stage this line", "Discard this line"]);
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
    let d = view.read_with(cx, |v, _| v.diff().cloned()).unwrap();
    let at = at_line(&d, 1, true, cx);
    right_click(&view, at, cx);
    assert_eq!(d.read_with(cx, |d, _| d.menu_labels()), ["Unstage this line"]);
}

#[gpui::test]
fn diff_picks_wait_while_another_write_runs(cx: &mut TestAppContext) {
    let f = fixture();
    let (view, cx) = open(&f, cx);
    let d = show(&view, Of::Unstaged, Layout::Split, cx);
    let gate = view.read_with(cx, |v, _| v.gate().clone());
    let writer = supraman_gitman::write::Writer::open(&f.repo()).unwrap();
    let blocked = view.update(cx, |_, cx| {
        gate.update(cx, |g, cx| g.request(writer, supraman_gitman::write::LockSet::refresh(), |_| Ok(()), cx));
        d.update(cx, |d, cx| {
            let blocked = d.picks_blocked(cx);
            d.pick(Act::Hunk(supraman_gitman::write::lines::Pick::Stage, 0), cx);
            blocked
        })
    });
    assert_eq!(blocked, Some(crate::git_view::write_gate::WORKING));
    settle(&view, cx);
    assert_eq!(index(&f), BASE, "a pick while another write runs stages nothing");
    assert_eq!(view.read_with(cx, |v, _| v.toast.text().map(str::to_string)).as_deref(), Some(crate::git_view::write_gate::WORKING), "it says why");
    click(&view, &hunk_key(0, "stage"), cx);
    assert_eq!(index(&f), "one\nTWO\nthree\nfour\nfive\nsix\nseven\n");
}

#[gpui::test]
fn diff_picks_wait_while_a_merge_is_under_way(cx: &mut TestAppContext) {
    let f = fixture();
    let head = f.git(&["rev-parse", "HEAD"]);
    std::fs::write(f.dir.join(".git/MERGE_HEAD"), format!("{head}\n")).unwrap();
    let (view, cx) = open(&f, cx);
    let d = show(&view, Of::Unstaged, Layout::Split, cx);
    assert_eq!(d.read_with(cx, |d, cx| d.picks_blocked(cx)), Some("Lines cannot be staged or discarded during a merge or rebase"));
    click(&view, &hunk_key(0, "stage"), cx);
    assert_eq!(index(&f), BASE, "a dimmed hunk button stages nothing");
}
