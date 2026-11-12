//! A write running past `CANCEL_AFTER` shows its words, seconds and Cancel; Cancel ends it and the toast says what git left.

use supraman_gitman::fixture::Fixture;

use crate::git_view::test_support::{Opts, click_held, frame, open, painted, settle, toast};
use crate::git_view::write_gate::{CANCEL_AFTER, WORKING};
use crate::git_view::write_strip::CANCEL_KEY;

#[gpui::test]
fn a_long_write_shows_its_strip_and_cancel_ends_it_saying_what_git_left(cx: &mut gpui::TestAppContext) {
    let f = Fixture::copy();
    let (view, cx) = open(&f.dir, Opts::default(), cx);
    view.update(cx, |v, cx| v.wip_panel.update(cx, |p, cx| p.stage_all(cx)));
    assert!(view.read_with(cx, |v, cx| v.gate.read(cx).busy().is_some()), "the WIP panel's Stage all starts");
    assert!(!frame(&view, cx).borrow().contains_key(CANCEL_KEY), "no strip before CANCEL_AFTER");
    assert_eq!(view.read_with(cx, |v, cx| v.stash_why_not(cx)[0].clone()).as_deref(), Some(WORKING), "the write buttons are dimmed");

    view.update(cx, |v, cx| v.gate.update(cx, |g, _| g.backdate(CANCEL_AFTER)));
    assert!(frame(&view, cx).borrow().contains_key(CANCEL_KEY), "the strip's Cancel shows once the write has run CANCEL_AFTER");
    assert_eq!(view.read_with(cx, |v, cx| v.gate.read(cx).long().map(|(w, s)| (w.to_string(), s))), Some(("Staging".into(), CANCEL_AFTER.as_secs())));

    std::fs::write(f.dir.join(".git/index.lock"), b"").unwrap();
    click_held(&view, CANCEL_KEY, cx);
    settle(&view, cx);
    std::fs::remove_file(f.dir.join(".git/index.lock")).unwrap();
    assert_eq!(toast(&view, cx).as_deref(), Some("Staging was cancelled; git left index.lock: remove it once no git runs"));
    assert!(!painted(&view, CANCEL_KEY, cx), "the strip goes with the write");
    assert_eq!(view.read_with(cx, |v, cx| v.stash_why_not(cx)[0].clone()), None, "the write buttons undim");
    let staged = f.at(&f.dir, &["diff", "--cached", "--name-only"]);
    assert!(!staged.lines().any(|l| l == "u.txt"), "the cancelled write staged nothing: {staged}");
}
