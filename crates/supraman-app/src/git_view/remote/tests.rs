//! The toolbar's Pull and Push and the push questions through the real tab, on temp clones of the fixture's bare
//! remote (C196); `settings.toml` in a temp home.

use std::path::{Path, PathBuf};

use gpui::{Entity, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, TestAppContext, VisualTestContext};
use supraman_gitman::fixture::{Fixture, git_in};
use supraman_gitman::write::remote::PullChoice;
use supraman_ui::hit::HitTargets;

use super::bar::{PULL_KEY, PULL_MENU_KEY, PUSH_KEY};
use super::{DETACHED, NO_REMOTE};
use crate::git_view::tests::redraw;
use crate::git_view::*;

struct Holder(Entity<GitView>);

impl Render for Holder {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(1400.)).h(px(900.)).child(self.0.clone())
    }
}

fn open<'a>(folder: &Path, home: &Path, cx: &'a mut TestAppContext) -> (Entity<GitView>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        supraman_ui::init_globals(cx, "test");
        supraman_ui::text_input::bind_keys(cx);
        bind_keys(cx);
    });
    let (folder, home) = (folder.to_path_buf(), home.to_path_buf());
    let (holder, cx) = cx.add_window_view(|_, cx| Holder(GitView::open(folder, Some(home), HitTargets::default(), &GitSizes::default(), cx)));
    let view = holder.read_with(cx, |h, _| h.0.clone());
    cx.run_until_parked();
    redraw(&view, cx);
    (view, cx)
}

fn press(view: &Entity<GitView>, key: &str, cx: &mut VisualTestContext) {
    let hits = view.read_with(cx, |v, _| v.hits.clone());
    hits.borrow_mut().clear();
    redraw(view, cx);
    let position = hits.borrow().get(key).unwrap_or_else(|| panic!("{key} is painted")).bounds.center();
    cx.simulate_event(MouseDownEvent { position, button: MouseButton::Left, modifiers: Modifiers::none(), click_count: 1, first_mouse: false });
    cx.simulate_event(MouseUpEvent { position, button: MouseButton::Left, modifiers: Modifiers::none(), click_count: 1 });
    cx.run_until_parked();
    redraw(view, cx);
}

fn git(f: &Fixture, dir: &Path, args: &[&str]) -> String {
    git_in(&f.git, dir, args, &[], None)
}

fn clone(f: &Fixture, name: &str) -> PathBuf {
    let origin = f.temp.path().join("origin.git");
    git(f, f.temp.path(), &["clone", "-q", "-b", "main", &origin.to_string_lossy(), name]);
    let dir = f.temp.path().join(name);
    for (key, value) in [("user.name", "Other"), ("user.email", "other@example.test"), ("commit.gpgsign", "false"), ("core.autocrlf", "false")] {
        git(f, &dir, &["config", key, value]);
    }
    dir
}

fn commit(f: &Fixture, dir: &Path, file: &str, message: &str) -> String {
    std::fs::write(dir.join(file), format!("{message}\n")).unwrap();
    git(f, dir, &["add", file]);
    git(f, dir, &["commit", "-q", "-m", message]);
    git(f, dir, &["rev-parse", "HEAD"])
}

fn toast(view: &Entity<GitView>, cx: &mut VisualTestContext) -> Option<String> {
    view.read_with(cx, |v, _| v.toast.text().map(str::to_string))
}

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
    git(&f, &other, &["push", "-q"]);
    let (view, cx) = open(&mine, home.path(), cx);
    assert_eq!(why_not(&view, cx), [None, None]);

    press(&view, PULL_KEY, cx);
    assert_eq!(git(&f, &mine, &["rev-parse", "HEAD"]), theirs, "the default pull fast-forwards");
    assert_eq!(toast(&view, cx).as_deref(), Some("Pulled"));

    let pushed = commit(&f, &mine, "m.txt", "Mine");
    press(&view, PUSH_KEY, cx);
    assert_eq!(git(&f, &f.temp.path().join("origin.git"), &["rev-parse", "main"]), pushed);
    assert_eq!(toast(&view, cx).as_deref(), Some("Pushed main to origin/main"));

    press(&view, PULL_MENU_KEY, cx);
    let labels = view.read_with(cx, |v, cx| v.remote.read(cx).menu_labels());
    assert_eq!(labels[..4].iter().map(|(l, _)| l.as_str()).collect::<Vec<_>>(), ["Fetch all", "Pull (fast-forward if possible)", "Pull (fast-forward only)", "Pull (rebase)"]);
    assert_eq!(labels[4..].iter().map(|(_, t)| *t).collect::<Vec<_>>(), [Some(false), Some(true), Some(false), Some(false)], "the default ticked");
    press(&view, "menu-pull-default-fetch", cx);
    assert_eq!(view.read_with(cx, |v, cx| v.remote.read(cx).pull_choice()), PullChoice::Fetch);
    let saved = std::fs::read_to_string(home.path().join("settings.toml")).unwrap();
    assert!(saved.contains("[gitman]") && saved.contains("pull = \"fetch\""), "{saved}");

    git(&f, &other, &["pull", "-q"]);
    let fetched = commit(&f, &other, "t2.txt", "Theirs again");
    git(&f, &other, &["push", "-q"]);
    press(&view, PULL_KEY, cx);
    assert_eq!(git(&f, &mine, &["rev-parse", "origin/main"]), fetched, "the button fetches now");
    assert_eq!(git(&f, &mine, &["rev-parse", "HEAD"]), pushed, "and pulls nothing");
}

#[gpui::test]
fn a_detached_head_dims_push_and_pull_but_not_a_fetch_and_no_remote_dims_both(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let home = tempfile::tempdir().unwrap();
    let mine = clone(&f, "mine");
    git(&f, &mine, &["checkout", "-q", "--detach"]);
    let (view, cx) = open(&mine, home.path(), cx);
    assert_eq!(why_not(&view, cx), [Some(DETACHED.into()), Some(DETACHED.into())]);
    let before = git(&f, &f.temp.path().join("origin.git"), &["rev-parse", "main"]);
    commit(&f, &mine, "d.txt", "Detached");
    press(&view, PUSH_KEY, cx);
    assert_eq!(git(&f, &f.temp.path().join("origin.git"), &["rev-parse", "main"]), before, "a dimmed Push does nothing");
    view.update(cx, |v, cx| v.remote.update(cx, |r, cx| r.act(super::RemoteAct::SetDefault(PullChoice::Fetch), cx)));
    cx.run_until_parked();
    assert_eq!(why_not(&view, cx)[0], None, "a fetch needs no branch");

    let lone = clone(&f, "lone");
    git(&f, &lone, &["remote", "remove", "origin"]);
    let (view, cx) = open(&lone, home.path(), cx);
    assert_eq!(why_not(&view, cx), [Some(NO_REMOTE.into()), Some(NO_REMOTE.into())]);
}

#[gpui::test]
fn a_push_behind_asks_pull_or_force_and_a_stale_lease_asks_fetch_or_force_anyway(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let home = tempfile::tempdir().unwrap();
    let (mine, other) = (clone(&f, "mine"), clone(&f, "other"));
    commit(&f, &other, "o.txt", "Other's");
    git(&f, &other, &["push", "-q"]);
    let (view, cx) = open(&mine, home.path(), cx);
    let rewritten = commit(&f, &mine, "m.txt", "Mine");
    press(&view, PUSH_KEY, cx);
    assert_eq!(asking(&view, cx).as_deref(), Some("origin/main has commits main lacks. Pull them first, or force the push with lease, dropping them from origin?"));
    press(&view, "git-remote-ask-alt", cx);
    assert_eq!(asking(&view, cx).as_deref(), Some("Force push main to origin/main? The commits only origin holds are lost there."), "a second question before forcing");
    press(&view, "git-remote-ask-yes", cx);
    assert_eq!(asking(&view, cx).as_deref(), Some("origin/main moved since this repository last fetched it. Fetch first, or force the push anyway?"), "never fetched: the lease is stale");
    press(&view, "git-remote-ask-alt", cx);
    press(&view, "git-remote-ask-yes", cx);
    assert_eq!(git(&f, &f.temp.path().join("origin.git"), &["rev-parse", "main"]), rewritten);
    assert_eq!(toast(&view, cx).as_deref(), Some("Pushed main to origin/main"));
}

#[gpui::test]
fn a_branch_without_upstream_is_pushed_after_the_dialog_chooses_one(cx: &mut TestAppContext) {
    let f = Fixture::copy();
    let home = tempfile::tempdir().unwrap();
    let mine = clone(&f, "mine");
    git(&f, &mine, &["checkout", "-q", "-b", "topic"]);
    let sha = commit(&f, &mine, "t.txt", "Topic");
    let (view, cx) = open(&mine, home.path(), cx);
    press(&view, PUSH_KEY, cx);
    let dialog = view.read_with(cx, |v, cx| v.remote.read(cx).upstream_dialog().map(|d| (d.local.clone(), d.remote.clone())));
    assert_eq!(dialog, Some(("topic".into(), "origin".into())));
    view.update(cx, |v, cx| v.remote.update(cx, |r, cx| r.upstream_dialog().unwrap().type_in("shared/topic", cx)));
    redraw(&view, cx);
    press(&view, "git-upstream-ok", cx);
    assert_eq!(git(&f, &f.temp.path().join("origin.git"), &["rev-parse", "shared/topic"]), sha);
    assert_eq!(git(&f, &mine, &["rev-parse", "--abbrev-ref", "topic@{upstream}"]), "origin/shared/topic");
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
        if remember {
            press(&view, "git-askpass-remember", cx);
        }
        press(&view, "git-askpass-ok", cx);
        assert_eq!(asking.join().unwrap().as_deref(), Some(typed));
        let kept = store.get(&secret_key("C:/keys/id_ed25519")).unwrap();
        assert_eq!(kept.as_deref(), remember.then_some(typed), "kept only when Remember is ticked");
    }
}
