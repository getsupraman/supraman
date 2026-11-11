//! The toolbar's Pull and Push and the push questions through the real tab, on temp clones of the fixture's bare remote.

use std::path::{Path, PathBuf};

use gpui::{Entity, TestAppContext, VisualTestContext};
use supraman_gitman::fixture::Fixture;
use supraman_gitman::write::remote::PullChoice;

use super::bar::{PULL_KEY, PULL_MENU_KEY, PUSH_KEY};
use super::{DETACHED, NO_REMOTE};
use supraman_ui::confirm::Answer;
use crate::git_view::test_support::{self, Opts, click, redraw, toast};
use crate::git_view::*;

fn open<'a>(folder: &Path, home: &Path, cx: &'a mut TestAppContext) -> (Entity<GitView>, &'a mut VisualTestContext) {
    test_support::open(folder, Opts::default().home(home).size(1400., 900.), cx)
}

/// The band `key` across the tab under the toolbar, `inline` on one line with its text.
fn inline(view: &Entity<GitView>, key: &str, inline: &[&str], cx: &mut VisualTestContext) {
    let hits = view.read_with(cx, |v, _| v.hits.clone());
    hits.borrow_mut().clear();
    redraw(view, cx);
    let at = |k: &str| hits.borrow().get(k).map(|h| h.bounds).unwrap_or_else(|| panic!("{k} is painted"));
    let (area, band, text) = (at("git-below-toolbar"), at(&format!("{key}-band")), at(&format!("{key}-text")));
    assert_eq!((band.origin, band.size.width), (area.origin, area.size.width), "{key}: across the tab under the toolbar");
    assert_eq!(band.size.height, supraman_ui::confirm::ROW, "{key}: one line in {:?}", area.size.width);
    for k in inline {
        assert!((at(k).center().y - text.center().y).abs() < px(1.), "{k} on the text's line");
    }
}

fn clone(f: &Fixture, name: &str) -> PathBuf {
    let origin = f.temp.path().join("origin.git");
    f.at(f.temp.path(), &["clone", "-q", "-b", "main", &origin.to_string_lossy(), name]);
    let dir = f.temp.path().join(name);
    for (key, value) in [("user.name", "Other"), ("user.email", "other@example.test"), ("commit.gpgsign", "false"), ("core.autocrlf", "false")] {
        f.at(&dir, &["config", key, value]);
    }
    dir
}

fn commit(f: &Fixture, dir: &Path, file: &str, message: &str) -> String {
    std::fs::write(dir.join(file), format!("{message}\n")).unwrap();
    f.at(dir, &["add", file]);
    f.at(dir, &["commit", "-q", "-m", message]);
    f.at(dir, &["rev-parse", "HEAD"])
}

const BEHIND: &str = "'refs/heads/main' is behind 'refs/remotes/origin/main'. Update your branch by doing a Pull.";
const STALE: &str = "'refs/remotes/origin/main' has changed since your last fetch. Update it by doing a Fetch.";
const DESTRUCTIVE: &str = "Force push is destructive and rewrites history.";

fn asking(view: &Entity<GitView>, cx: &mut VisualTestContext) -> Option<String> {
    view.read_with(cx, |v, cx| v.remote.read(cx).asking().map(str::to_string))
}

fn why_not(view: &Entity<GitView>, cx: &mut VisualTestContext) -> [Option<String>; 2] {
    view.read_with(cx, |v, cx| v.remote_why_not(cx))
}

#[gpui::test]
fn the_toolbars_pull_and_push_act_and_the_default_is_saved(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let home = tempfile::tempdir().unwrap();
    let (mine, other) = (clone(&f, "mine"), clone(&f, "other"));
    let theirs = commit(&f, &other, "t.txt", "Theirs");
    f.at(&other, &["push", "-q"]);
    let (view, cx) = open(&mine, home.path(), cx);
    assert_eq!(why_not(&view, cx), [None, None]);

    click(&view, PULL_KEY, cx);
    assert_eq!(f.at(&mine, &["rev-parse", "HEAD"]), theirs, "the default pull fast-forwards");
    assert_eq!(toast(&view, cx).as_deref(), Some("Pulled"));

    let pushed = commit(&f, &mine, "m.txt", "Mine");
    click(&view, PUSH_KEY, cx);
    assert_eq!(f.at(&f.temp.path().join("origin.git"), &["rev-parse", "main"]), pushed);
    assert_eq!(toast(&view, cx).as_deref(), Some("Pushed main to origin/main"));

    click(&view, PULL_MENU_KEY, cx);
    let labels = view.read_with(cx, |v, cx| v.remote.read(cx).menu_labels());
    assert_eq!(labels[..4].iter().map(|(l, _)| l.as_str()).collect::<Vec<_>>(), ["Fetch all", "Pull (fast-forward if possible)", "Pull (fast-forward only)", "Pull (rebase)"]);
    assert_eq!(labels[4..].iter().map(|(_, t)| *t).collect::<Vec<_>>(), [Some(false), Some(true), Some(false), Some(false)], "the default ticked");
    click(&view, "menu-pull-default-fetch", cx);
    assert_eq!(view.read_with(cx, |v, cx| v.remote.read(cx).pull_choice()), PullChoice::Fetch);
    let saved = std::fs::read_to_string(home.path().join("settings.toml")).unwrap();
    assert!(saved.contains("[gitman]") && saved.contains("pull = \"fetch\""), "{saved}");

    f.at(&other, &["pull", "-q"]);
    let fetched = commit(&f, &other, "t2.txt", "Theirs again");
    f.at(&other, &["push", "-q"]);
    click(&view, PULL_KEY, cx);
    assert_eq!(f.at(&mine, &["rev-parse", "origin/main"]), fetched, "the button fetches now");
    assert_eq!(f.at(&mine, &["rev-parse", "HEAD"]), pushed, "and pulls nothing");
}

#[gpui::test]
fn a_detached_head_dims_push_and_pull_but_not_a_fetch_and_no_remote_dims_both(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let home = tempfile::tempdir().unwrap();
    let mine = clone(&f, "mine");
    f.at(&mine, &["checkout", "-q", "--detach"]);
    let (view, cx) = open(&mine, home.path(), cx);
    assert_eq!(why_not(&view, cx), [Some(DETACHED.into()), Some(DETACHED.into())]);
    let before = f.at(&f.temp.path().join("origin.git"), &["rev-parse", "main"]);
    commit(&f, &mine, "d.txt", "Detached");
    click(&view, PUSH_KEY, cx);
    assert_eq!(f.at(&f.temp.path().join("origin.git"), &["rev-parse", "main"]), before, "a dimmed Push does nothing");
    view.update(cx, |v, cx| v.remote.update(cx, |r, cx| r.act(super::RemoteAct::SetDefault(PullChoice::Fetch), cx)));
    cx.run_until_parked();
    assert_eq!(why_not(&view, cx)[0], None, "a fetch needs no branch");

    let lone = clone(&f, "lone");
    f.at(&lone, &["remote", "remove", "origin"]);
    let (view, cx) = open(&lone, home.path(), cx);
    assert_eq!(why_not(&view, cx), [Some(NO_REMOTE.into()), Some(NO_REMOTE.into())]);
}

/// The palette's Remote commands as its top list shows them: key, words and why each is dimmed.
fn palette_remote(view: &Entity<GitView>, cx: &mut VisualTestContext) -> Vec<(String, String, Option<String>)> {
    use supraman_ui::palette::Entry as _;
    view.update_in(cx, |v, window, cx| v.open_find(None, window, cx));
    cx.simulate_input("remote");
    cx.run_until_parked();
    let palette = view.read_with(cx, |v, _| v.find().cloned()).expect("the palette is open");
    let rows = palette.read_with(cx, |p, _| {
        p.shown().iter().filter(|(e, _)| e.text().starts_with("Remote: ")).map(|(e, _)| (e.key().to_string(), e.text(), e.dimmed().map(|w| w.to_string()))).collect()
    });
    view.update_in(cx, |v, window, cx| v.close_find(window, cx));
    rows
}

fn pick(view: &Entity<GitView>, key: &str, cx: &mut VisualTestContext) {
    view.update_in(cx, |v, window, cx| v.open_find(None, window, cx));
    cx.simulate_input("remote");
    cx.run_until_parked();
    click(view, &format!("{}-row-{key}", crate::git_view::find::ID), cx);
}

const REMOTE_KEYS: [&str; 6] = ["cmd:push", "cmd:pull", "cmd:pull-ff", "cmd:pull-ff-only", "cmd:pull-rebase", "cmd:fetch-all"];

#[gpui::test]
fn the_palettes_push_pull_and_fetch_act_as_the_toolbar_does_and_are_dimmed_with_its_reasons(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let home = tempfile::tempdir().unwrap();
    let origin = f.temp.path().join("origin.git");
    let (mine, other) = (clone(&f, "mine"), clone(&f, "other"));
    let theirs = commit(&f, &other, "t.txt", "Theirs");
    f.at(&other, &["push", "-q"]);
    let (view, cx) = open(&mine, home.path(), cx);

    let rows = palette_remote(&view, cx);
    assert_eq!(rows.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(), REMOTE_KEYS);
    let words = ["Remote: Push", "Remote: Pull", "Remote: Pull (fast-forward if possible)", "Remote: Pull (fast-forward only)", "Remote: Pull (rebase)", "Remote: Fetch all"];
    assert_eq!(rows.iter().map(|r| r.1.as_str()).collect::<Vec<_>>(), words, "the default worded as the toolbar's button");
    assert!(rows.iter().all(|r| r.2.is_none()), "{rows:?}");

    pick(&view, "cmd:pull", cx);
    assert_eq!(f.at(&mine, &["rev-parse", "HEAD"]), theirs, "the default pull fast-forwards");
    assert_eq!(toast(&view, cx).as_deref(), Some("Pulled"));

    let pushed = commit(&f, &mine, "m.txt", "Mine");
    pick(&view, "cmd:push", cx);
    assert_eq!(f.at(&origin, &["rev-parse", "main"]), pushed);
    assert_eq!(toast(&view, cx).as_deref(), Some("Pushed main to origin/main"));

    f.at(&other, &["pull", "-q"]);
    let again = commit(&f, &other, "t2.txt", "Theirs again");
    f.at(&other, &["push", "-q"]);
    commit(&f, &mine, "m2.txt", "Mine again");
    pick(&view, "cmd:pull-rebase", cx);
    assert_eq!(toast(&view, cx).as_deref(), Some("Pulled and rebased"));
    assert_eq!(f.at(&mine, &["rev-parse", "HEAD^"]), again, "rebased onto theirs, no merge");

    let fetched = commit(&f, &other, "t3.txt", "Fetched");
    f.at(&other, &["push", "-q"]);
    let head = f.at(&mine, &["rev-parse", "HEAD"]);
    pick(&view, "cmd:fetch-all", cx);
    assert_eq!(toast(&view, cx).as_deref(), Some("Fetched every remote"));
    assert_eq!((f.at(&mine, &["rev-parse", "origin/main"]), f.at(&mine, &["rev-parse", "HEAD"])), (fetched, head));

    view.update(cx, |v, cx| v.remote.update(cx, |r, cx| r.act(super::RemoteAct::SetDefault(PullChoice::Fetch), cx)));
    cx.run_until_parked();
    let default = palette_remote(&view, cx).into_iter().find(|r| r.0 == "cmd:pull").map(|r| r.1);
    assert_eq!(default.as_deref(), Some("Remote: Fetch"), "the default's words follow the toolbar's");

    view.update(cx, |v, cx| v.remote.update(cx, |r, _| r.writes.set_doing(Some((0, super::Running { doing: super::Doing::FetchAll, progress: None, cancel: Default::default() })))));
    let rows = palette_remote(&view, cx);
    assert!(rows.iter().all(|r| r.2.as_deref() == Some("Fetching…")), "a transfer running dims every one: {rows:?}");
    assert_eq!(why_not(&view, cx), [Some("Fetching…".into()), Some("Fetching…".into())]);
    view.update(cx, |v, cx| v.remote.update(cx, |r, _| r.writes.set_doing(None)));
}

#[gpui::test]
fn the_palettes_remote_commands_are_dimmed_on_a_detached_head_and_without_a_remote(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let home = tempfile::tempdir().unwrap();
    let origin = f.temp.path().join("origin.git");
    let mine = clone(&f, "mine");
    f.at(&mine, &["checkout", "-q", "--detach"]);
    let (view, cx) = open(&mine, home.path(), cx);
    let rows = palette_remote(&view, cx);
    let [pull, push] = why_not(&view, cx);
    let detached = Some(DETACHED.to_string());
    assert_eq!(rows.iter().map(|r| r.2.clone()).collect::<Vec<_>>(), [push, pull, detached.clone(), detached.clone(), detached.clone(), None], "the toolbar's reasons; a fetch needs no branch");
    let before = f.at(&origin, &["rev-parse", "main"]);
    commit(&f, &mine, "d.txt", "Detached");
    pick(&view, "cmd:push", cx);
    assert_eq!(f.at(&origin, &["rev-parse", "main"]), before, "a dimmed Push does nothing");

    let lone = clone(&f, "lone");
    f.at(&lone, &["remote", "remove", "origin"]);
    let (view, cx) = open(&lone, home.path(), cx);
    let rows = palette_remote(&view, cx);
    assert!(rows.iter().all(|r| r.2.as_deref() == Some(NO_REMOTE)), "{rows:?}");
}

#[gpui::test]
fn a_push_behind_asks_pull_or_force_push_and_a_stale_lease_asks_fetch_or_force_push(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let home = tempfile::tempdir().unwrap();
    let (mine, other) = (clone(&f, "mine"), clone(&f, "other"));
    commit(&f, &other, "o.txt", "Other's");
    f.at(&other, &["push", "-q"]);
    let (view, cx) = open(&mine, home.path(), cx);
    let rewritten = commit(&f, &mine, "m.txt", "Mine");
    click(&view, PUSH_KEY, cx);
    assert_eq!(asking(&view, cx).as_deref(), Some(BEHIND));
    click(&view, "git-remote-ask-alt", cx);
    assert_eq!(asking(&view, cx).as_deref(), Some(DESTRUCTIVE), "a second question before forcing");
    click(&view, "git-remote-ask-yes", cx);
    assert_eq!(asking(&view, cx).as_deref(), Some(STALE), "never fetched: the lease is stale");
    click(&view, "git-remote-ask-alt", cx);
    click(&view, "git-remote-ask-yes", cx);
    assert_eq!(f.at(&f.temp.path().join("origin.git"), &["rev-parse", "main"]), rewritten);
    assert_eq!(toast(&view, cx).as_deref(), Some("Pushed main to origin/main"));
}

#[gpui::test]
fn a_branch_without_upstream_is_pushed_after_the_dialog_chooses_one(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let home = tempfile::tempdir().unwrap();
    let mine = clone(&f, "mine");
    f.at(&mine, &["checkout", "-q", "-b", "topic"]);
    let sha = commit(&f, &mine, "t.txt", "Topic");
    let (view, cx) = open(&mine, home.path(), cx);
    click(&view, PUSH_KEY, cx);
    let dialog = view.read_with(cx, |v, cx| v.remote.read(cx).upstream_dialog().map(|d| (d.local.clone(), d.remote.clone())));
    assert_eq!(dialog, Some(("topic".into(), "origin".into())));
    view.update(cx, |v, cx| v.remote.update(cx, |r, cx| r.upstream_dialog().unwrap().type_in("shared/topic", cx)));
    redraw(&view, cx);
    inline(&view, "git-upstream", &["git-upstream-origin", "git-upstream-field", "git-upstream-ok", "git-upstream-cancel"], cx);
    click(&view, "git-upstream-ok", cx);
    assert_eq!(f.at(&f.temp.path().join("origin.git"), &["rev-parse", "shared/topic"]), sha);
    assert_eq!(f.at(&mine, &["rev-parse", "--abbrev-ref", "topic@{upstream}"]), "origin/shared/topic");
}

#[gpui::test]
fn sshs_passphrase_prompt_answers_through_the_dialog_and_is_kept_only_when_remember_is_ticked(cx: &mut TestAppContext) {
    use std::sync::{Arc, mpsc};
    use supraman_core::askpass::{Bridge, ask_at, secret_key};
    use supraman_core::secret::{MemoryStore, SecretStore};

    let f = Fixture::copy();
    let home = tempfile::tempdir().unwrap();
    let (view, cx) = open(&f.dir, home.path(), cx);
    let store: Arc<dyn SecretStore> = Arc::new(MemoryStore::default());
    let (to, asked) = mpsc::channel();
    let to = std::sync::Mutex::new(to);
    let bridge = Bridge::start(Some(store.clone()), move |a| drop(to.lock().unwrap().send(a))).unwrap();
    let prompt = "Enter passphrase for key 'C:/keys/id_ed25519': ";

    for (remember, typed) in [(false, "first"), (true, "second")] {
        let spec = bridge.spec();
        let asking = std::thread::spawn(move || ask_at(&spec, prompt));
        let a = asked.recv_timeout(std::time::Duration::from_secs(30)).expect("the bridge hears the prompt");
        view.update(cx, |v, cx| v.remote.update(cx, |r, cx| r.heard(0, super::Heard::Asked(a), cx)));
        redraw(&view, cx);
        view.update(cx, |v, cx| v.remote.update(cx, |r, cx| r.prompting().unwrap().type_in(typed, cx)));
        inline(&view, "git-askpass", &["git-askpass-field", "git-askpass-ok", "git-askpass-cancel"], cx);
        if remember {
            click(&view, "git-askpass-remember", cx);
        }
        click(&view, "git-askpass-ok", cx);
        assert_eq!(asking.join().unwrap().as_deref(), Some(typed));
        let kept = store.get(&secret_key("C:/keys/id_ed25519")).unwrap();
        assert_eq!(kept.as_deref(), remember.then_some(typed), "kept only when Remember is ticked");
    }
}

/// The band's answers, each with whether it destroys something, and what Enter answers.
fn answers(view: &Entity<GitView>, cx: &mut VisualTestContext) -> Option<(Vec<(String, bool)>, Answer)> {
    view.read_with(cx, |v, cx| v.remote.read(cx).answers())
}

fn owned(answers: &[(&str, bool)]) -> Vec<(String, bool)> {
    answers.iter().map(|(w, d)| (w.to_string(), *d)).collect()
}

fn key(view: &Entity<GitView>, keys: &str, cx: &mut VisualTestContext) {
    assert!(cx.update(|window, cx| view.read(cx).band_focused(window)), "the band takes the keyboard before {keys}");
    cx.simulate_keystrokes(keys);
    cx.run_until_parked();
    redraw(view, cx);
}

fn ask_stale(view: &Entity<GitView>, cx: &mut VisualTestContext) {
    view.update(cx, |v, cx| v.remote.update(cx, |r, cx| r.ask(super::Ask::Stale { local: "main".into(), remote: "origin".into(), branch: "main".into() }, cx)));
    redraw(view, cx);
}

#[gpui::test]
fn force_push_asks_a_second_band_and_only_its_force_push_pushes_with_lease(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let home = tempfile::tempdir().unwrap();
    let origin = f.temp.path().join("origin.git");
    let (mine, other) = (clone(&f, "mine"), clone(&f, "other"));
    commit(&f, &other, "o.txt", "Other's");
    f.at(&other, &["push", "-q"]);
    f.at(&mine, &["fetch", "-q"]);
    let (view, cx) = open(&mine, home.path(), cx);
    let rewritten = commit(&f, &mine, "m.txt", "Mine");
    let theirs = f.at(&origin, &["rev-parse", "main"]);
    let first = (owned(&[("Pull (fast-forward if possible)", false), ("Force Push", true), ("Cancel", false)]), Answer::Yes);
    let second = (owned(&[("Force Push", true), ("Cancel", false)]), Answer::No);

    click(&view, PUSH_KEY, cx);
    assert_eq!((asking(&view, cx).as_deref(), answers(&view, cx)), (Some(BEHIND), Some(first)), "Pull the default, Force Push destructive");
    key(&view, "escape", cx);
    assert_eq!((asking(&view, cx), f.at(&origin, &["rev-parse", "main"])), (None, theirs.clone()), "Esc at the first band pushes nothing");

    click(&view, PUSH_KEY, cx);
    click(&view, "git-remote-ask-cancel", cx);
    assert_eq!((asking(&view, cx), f.at(&origin, &["rev-parse", "main"])), (None, theirs.clone()), "Cancel at the first band pushes nothing");

    click(&view, PUSH_KEY, cx);
    click(&view, "git-remote-ask-alt", cx);
    assert_eq!((asking(&view, cx).as_deref(), answers(&view, cx)), (Some(DESTRUCTIVE), Some(second)), "the second band: Enter cancels");
    assert_eq!(f.at(&origin, &["rev-parse", "main"]), theirs, "the first Force Push pushes nothing");
    key(&view, "enter", cx);
    assert_eq!((asking(&view, cx), f.at(&origin, &["rev-parse", "main"])), (None, theirs.clone()), "Enter at the second band pushes nothing");

    click(&view, PUSH_KEY, cx);
    click(&view, "git-remote-ask-alt", cx);
    click(&view, "git-remote-ask-cancel", cx);
    assert_eq!((asking(&view, cx), f.at(&origin, &["rev-parse", "main"])), (None, theirs.clone()), "Cancel at the second band pushes nothing");

    click(&view, PUSH_KEY, cx);
    click(&view, "git-remote-ask-alt", cx);
    f.at(&other, &["commit", "-q", "--allow-empty", "-m", "Moved"]);
    f.at(&other, &["push", "-q"]);
    let moved = f.at(&origin, &["rev-parse", "main"]);
    click(&view, "git-remote-ask-yes", cx);
    assert_eq!((asking(&view, cx).as_deref(), f.at(&origin, &["rev-parse", "main"])), (Some(STALE), moved), "with lease: a remote moved since the fetch is kept");

    click(&view, "git-remote-ask-cancel", cx);
    f.at(&mine, &["fetch", "-q"]);
    click(&view, PUSH_KEY, cx);
    click(&view, "git-remote-ask-alt", cx);
    click(&view, "git-remote-ask-yes", cx);
    assert_eq!(f.at(&origin, &["rev-parse", "main"]), rewritten, "the second Force Push pushes");
    assert_eq!(toast(&view, cx).as_deref(), Some("Pushed main to origin/main"));
}

#[gpui::test]
fn a_stale_lease_asks_fetch_or_force_push_and_its_force_push_asks_the_same_second_band(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let home = tempfile::tempdir().unwrap();
    let origin = f.temp.path().join("origin.git");
    let (mine, other) = (clone(&f, "mine"), clone(&f, "other"));
    commit(&f, &other, "o.txt", "Other's");
    f.at(&other, &["push", "-q"]);
    let (view, cx) = open(&mine, home.path(), cx);
    let rewritten = commit(&f, &mine, "m.txt", "Mine");
    let theirs = f.at(&origin, &["rev-parse", "main"]);

    ask_stale(&view, cx);
    assert_eq!((asking(&view, cx).as_deref(), answers(&view, cx)), (Some(STALE), Some((owned(&[("Fetch", false), ("Force Push", true), ("Cancel", false)]), Answer::Yes))));
    click(&view, "git-remote-ask-cancel", cx);
    assert_eq!((asking(&view, cx), f.at(&origin, &["rev-parse", "main"])), (None, theirs.clone()), "Cancel pushes nothing");

    ask_stale(&view, cx);
    click(&view, "git-remote-ask-alt", cx);
    assert_eq!((asking(&view, cx).as_deref(), answers(&view, cx)), (Some(DESTRUCTIVE), Some((owned(&[("Force Push", true), ("Cancel", false)]), Answer::No))));
    assert_eq!(f.at(&origin, &["rev-parse", "main"]), theirs, "the first Force Push pushes nothing");
    click(&view, "git-remote-ask-yes", cx);
    assert_eq!(f.at(&origin, &["rev-parse", "main"]), rewritten, "forced anyway over a remote never fetched");

    ask_stale(&view, cx);
    commit(&f, &other, "o2.txt", "Other's again");
    f.at(&other, &["push", "-q", "--force"]);
    let again = f.at(&origin, &["rev-parse", "main"]);
    key(&view, "enter", cx);
    assert_eq!(f.at(&mine, &["rev-parse", "origin/main"]), again, "Enter fetches");
}

#[gpui::test]
fn the_pull_answer_fast_forwards_a_branch_only_behind_and_merges_a_diverged_one(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let home = tempfile::tempdir().unwrap();
    let (mine, other) = (clone(&f, "mine"), clone(&f, "other"));
    let theirs = commit(&f, &other, "o.txt", "Other's");
    f.at(&other, &["push", "-q"]);
    let (view, cx) = open(&mine, home.path(), cx);

    click(&view, PUSH_KEY, cx);
    assert_eq!(asking(&view, cx).as_deref(), Some(BEHIND), "a branch only behind is refused too");
    key(&view, "enter", cx);
    assert_eq!(f.at(&mine, &["rev-parse", "HEAD"]), theirs, "Enter pulls: a fast-forward");
    assert_eq!(toast(&view, cx).as_deref(), Some("Pulled"));

    let again = commit(&f, &other, "o2.txt", "Other's again");
    f.at(&other, &["push", "-q"]);
    let ours = commit(&f, &mine, "m.txt", "Mine");
    click(&view, PUSH_KEY, cx);
    assert_eq!(asking(&view, cx).as_deref(), Some(BEHIND));
    click(&view, "git-remote-ask-yes", cx);
    assert_eq!((f.at(&mine, &["rev-parse", "HEAD^1"]), f.at(&mine, &["rev-parse", "HEAD^2"])), (ours, again), "a diverged branch is merged");
}
