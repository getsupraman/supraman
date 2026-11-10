//! The tab's file viewer in the graph's place, and the mode and layout the next one opens in.

use gpui::{App, Entity, Subscription};

use super::diff::{DiffView, Layout, Mode, Of};

#[derive(Default)]
pub(crate) struct Viewer {
    open: Option<(Entity<DiffView>, [Subscription; 3])>,
    pub(crate) mode: Mode,
    pub(crate) layout: Layout,
    /// The viewer opened since the last frame and takes the keyboard on the next.
    pub(crate) focus: bool,
}

impl Viewer {
    pub(crate) fn view(&self) -> Option<&Entity<DiffView>> {
        self.open.as_ref().map(|(d, _)| d)
    }

    pub(crate) fn show(&mut self, view: Entity<DiffView>, subscriptions: [Subscription; 3]) {
        self.open = Some((view, subscriptions));
        self.focus = true;
    }

    /// Closes the viewer, its mode (but a conflict's) and layout kept for the next; whether one was open.
    pub(crate) fn close(&mut self, cx: &App) -> bool {
        let Some((view, _)) = self.open.take() else { return false };
        let view = view.read(cx);
        if view.target.of != Of::Conflicted {
            self.mode = view.mode();
        }
        self.layout = view.layout();
        true
    }
}
