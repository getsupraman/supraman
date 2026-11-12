use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{Context, Entity, IntoElement, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Render, TestAppContext, VisualTestContext, Window, div, prelude::*, px};
use supraman_gitman::BackgroundSettings;
use supraman_ui::hit::HitTargets;

use crate::git_view::branch::OpenGit;
use crate::git_view::{GitView, Load, background, bind_keys};
use crate::occupancy;

/// The repo's background work on, with no fetch and no maintenance.
pub(crate) const QUIET: BackgroundSettings = BackgroundSettings { fetch_every_minutes: Some(0), maintenance: Some(false), worktree_wip: None };

/// How a test opens its Git tab: layout home, live sessions, background at [`QUIET`], editor and window size.
pub(crate) struct Opts {
    home: Option<PathBuf>,
    live: Option<occupancy::Live>,
    following: bool,
    background: bool,
    editor: bool,
    size: (f32, f32),
}

impl Default for Opts {
    fn default() -> Opts {
        Opts { home: None, live: None, following: false, background: false, editor: false, size: (1200., 800.) }
    }
}

impl Opts {
    pub(crate) fn home(self, home: &Path) -> Opts {
        Opts { home: Some(home.to_path_buf()), ..self }
    }

    pub(crate) fn live(self, live: occupancy::Live) -> Opts {
        Opts { live: Some(live), ..self }
    }

    pub(crate) fn following(self, live: occupancy::Live) -> Opts {
        Opts { live: Some(live), following: true, ..self }
    }

    pub(crate) fn background(self) -> Opts {
        Opts { background: true, ..self }
    }

    pub(crate) fn editor(self) -> Opts {
        Opts { editor: true, ..self }
    }

    pub(crate) fn size(self, width: f32, height: f32) -> Opts {
        Opts { size: (width, height), ..self }
    }
}

/// A window holding one view at a fixed size.
pub(crate) struct Holder<V: 'static> {
    pub view: Entity<V>,
    size: (f32, f32),
}

impl<V: Render> Render for Holder<V> {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(self.size.0)).h(px(self.size.1)).child(self.view.clone())
    }
}

/// A Git tab of `folder`, its first load done.
pub(crate) fn open<'a>(folder: &Path, opts: Opts, cx: &'a mut TestAppContext) -> (Entity<GitView>, &'a mut VisualTestContext) {
    let (view, cx) = open_unchecked(folder, opts, cx);
    assert_loaded(&view, cx);
    (view, cx)
}

/// A Git tab of `folder` as it opens, loaded or not.
pub(crate) fn open_unchecked<'a>(folder: &Path, opts: Opts, cx: &'a mut TestAppContext) -> (Entity<GitView>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        if !cx.has_global::<supraman_term::terminal_settings::TerminalConfig>() {
            supraman_ui::init_globals(cx, "test");
            supraman_ui::text_input::bind_keys(cx);
            bind_keys(cx);
        }
        if opts.editor {
            supraman_editor::init(supraman_ui::fonts::MONO, 13., cx);
        }
        if opts.background {
            background::start_for_test(QUIET, cx);
        }
    });
    let Opts { home, live, following, size, .. } = opts;
    let folder = folder.to_path_buf();
    let (holder, cx) = cx.add_window_view(|_, cx| {
        let hits = HitTargets::default();
        let view = match live {
            Some(live) if following => GitView::open_following(folder, home, hits, live, cx),
            Some(live) => GitView::open_with(folder, home, hits, Arc::new(live), cx),
            None => GitView::open(folder, home, hits, cx),
        };
        Holder { view, size }
    });
    let view = holder.read_with(cx, |h, _| h.view.clone());
    cx.run_until_parked();
    redraw(&view, cx);
    (view, cx)
}

/// Panics with the tab's problem unless its first load is done, so a failed load is named, not a missing row.
pub(crate) fn assert_loaded(view: &Entity<GitView>, cx: &mut VisualTestContext) {
    let load = view.read_with(cx, |v, _| match &v.load {
        Load::Ready => None,
        Load::Loading => Some("still loading".to_string()),
        Load::Problem(problem) => Some(format!("{problem:?}")),
    });
    if let Some(why) = load {
        panic!("the Git tab of {} did not load: {why}", view.read_with(cx, |v, _| v.folder.display().to_string()));
    }
}

pub(crate) fn redraw<V: 'static>(view: &Entity<V>, cx: &mut VisualTestContext) {
    view.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    cx.update(|_, _| ());
}

/// The hit keys of a frame drawn now only.
pub(crate) fn repaint(view: &Entity<GitView>, cx: &mut VisualTestContext) -> HitTargets {
    let hits = view.read_with(cx, |v, _| v.hits.clone());
    hits.borrow_mut().clear();
    redraw(view, cx);
    hits
}

/// The hit keys of a frame drawn now with no parked work run: a write just requested is still under way.
pub(crate) fn frame(view: &Entity<GitView>, cx: &mut VisualTestContext) -> HitTargets {
    let hits = view.read_with(cx, |v, _| v.hits.clone());
    hits.borrow_mut().clear();
    view.update(cx, |_, cx| cx.notify());
    cx.update(|_, _| ());
    hits
}

/// Whether `key` is painted in a frame drawn now.
pub(crate) fn painted(view: &Entity<GitView>, key: &str, cx: &mut VisualTestContext) -> bool {
    repaint(view, cx).borrow().contains_key(key)
}

/// Three rounds of parked work and a frame, for a background read that lands after the first.
pub(crate) fn settle(view: &Entity<GitView>, cx: &mut VisualTestContext) {
    for _ in 0..3 {
        cx.run_until_parked();
        redraw(view, cx);
    }
}

/// `button` pressed `click_count` times at the middle of `key` as painted now; the hit keys after are the next frame's.
pub(crate) fn press(view: &Entity<GitView>, key: &str, button: MouseButton, click_count: usize, cx: &mut VisualTestContext) {
    let position = repaint(view, cx).borrow().get(key).unwrap_or_else(|| panic!("{key} is painted")).bounds.center();
    let modifiers = Modifiers::none();
    cx.simulate_event(MouseDownEvent { position, button, modifiers, click_count, first_mouse: false });
    cx.simulate_event(MouseUpEvent { position, button, modifiers, click_count });
    cx.run_until_parked();
    repaint(view, cx);
}

pub(crate) fn click(view: &Entity<GitView>, key: &str, cx: &mut VisualTestContext) {
    press(view, key, MouseButton::Left, 1, cx);
}

pub(crate) fn right_click(view: &Entity<GitView>, key: &str, cx: &mut VisualTestContext) {
    press(view, key, MouseButton::Right, 1, cx);
}

/// `keys` typed into the window, then a frame drawn.
pub(crate) fn keys(view: &Entity<GitView>, keys: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(keys);
    cx.run_until_parked();
    redraw(view, cx);
}

pub(crate) fn toast(view: &Entity<GitView>, cx: &mut VisualTestContext) -> Option<String> {
    view.read_with(cx, |v, _| v.toast.text().map(str::to_string))
}

/// The graph's row of the commit `sha`.
pub(crate) fn row_of(view: &Entity<GitView>, sha: &str, cx: &mut VisualTestContext) -> usize {
    view.read_with(cx, |v, cx| v.pages.read(cx).rows.iter().position(|r| r.sha.as_ref() == sha)).unwrap_or_else(|| panic!("no row of {sha}"))
}

pub(crate) fn select_wip(view: &Entity<GitView>, cx: &mut VisualTestContext) {
    view.update(cx, |v, cx| v.select_wip(cx));
    cx.run_until_parked();
    redraw(view, cx);
}

/// The folders the tab's `OpenGit` asks for, by path key, as it emits them.
pub(crate) fn opened(view: &Entity<GitView>, cx: &mut VisualTestContext) -> Rc<RefCell<Vec<String>>> {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let heard = seen.clone();
    cx.update(|_, cx| cx.subscribe(view, move |_, e: &OpenGit, _| heard.borrow_mut().push(supraman_core::paths::path_key(&e.0))).detach());
    seen
}

/// The labels of the tab's one open menu.
pub(crate) fn menu(view: &Entity<GitView>, cx: &mut VisualTestContext) -> Vec<String> {
    view.read_with(cx, |v, cx| v.branch.read(cx).menu_labels())
}
