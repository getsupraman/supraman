use std::path::Path;

use gpui::{Context, Entity, IntoElement, Render, Subscription, TestAppContext, VisualTestContext, Window, div, prelude::*, px};
use supraman_ui::hit::HitTargets;

use super::{Opts, assert_loaded, init, redraw};
use crate::git_view::GitView;

/// Each view's width in [`open_pair`]'s window, room for its ref panel, graph and commit panel.
pub(crate) const PAIR_WIDTH: f32 = 1200.;

/// Two Git tabs side by side in one window, as two panels of a dock: one hit map, each view's keys under its scope.
pub(crate) struct Pair {
    pub views: [Entity<GitView>; 2],
    _restore: Subscription,
}

impl Render for Pair {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let half = |view: &Entity<GitView>| div().flex_none().w(px(PAIR_WIDTH)).h_full().child(view.clone());
        supraman_ui::focus::root(div().flex().w(px(PAIR_WIDTH * 2.)).h(px(800.))).child(half(&self.views[0])).child(half(&self.views[1]))
    }
}

/// Git tabs of `folders`, the first's keys under `scopes[0]`, the second's under `scopes[1]`, both loaded.
pub(crate) fn open_pair<'a>(folders: [&Path; 2], scopes: [&str; 2], cx: &'a mut TestAppContext) -> ([Entity<GitView>; 2], &'a mut VisualTestContext) {
    init(&Opts::default(), cx);
    let folders = folders.map(Path::to_path_buf);
    let (holder, cx) = cx.add_window_view(|window, cx| {
        let hits = HitTargets::default();
        let [a, b] = folders;
        let views = [GitView::open(a, None, hits.scoped(scopes[0]), cx), GitView::open(b, None, hits.scoped(scopes[1]), cx)];
        Pair { views, _restore: supraman_ui::focus::restore_on_loss(window, cx) }
    });
    cx.update(|window, _| window.activate_window());
    let views = holder.read_with(cx, |p, _| p.views.clone());
    cx.run_until_parked();
    for view in &views {
        redraw(view, cx);
        assert_loaded(view, cx);
    }
    (views, cx)
}
