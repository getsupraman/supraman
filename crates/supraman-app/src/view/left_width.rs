//! The left panel's width: each drag's end saved to `settings.toml` off the UI thread, the newest width last.

use std::sync::Arc;

use gpui::{Context, Pixels, px};
use supraman_core::Settings;
use supraman_core::store::Newest;

use super::{FleetView, MIN_LEFT_WIDTH};

#[derive(Default)]
pub struct LeftWidth {
    saves: Arc<Newest<u32>>,
    dragged: bool,
}

impl LeftWidth {
    /// The width `settings.toml` keeps, unless the user dragged the panel before it was read.
    pub fn loaded(&self, saved: Option<u32>) -> Option<Pixels> {
        saved.filter(|_| !self.dragged).map(|width| px(width as f32).max(MIN_LEFT_WIDTH))
    }
}

impl FleetView {
    pub(super) fn drag_left(&mut self, width: Pixels, cx: &mut Context<Self>) {
        self.left_width = width;
        self.left_saves.update(cx, |saves, _| saves.dragged = true);
        cx.notify();
    }

    pub(super) fn save_left_width(&mut self, cx: &mut Context<Self>) {
        let Some(home) = self.home.clone() else { return };
        let width = f32::from(self.left_width).round() as u32;
        let saves = self.left_saves.read(cx).saves.clone();
        saves.put(width);
        cx.spawn(async move |this, cx| {
            let saved = cx.background_executor().spawn(async move { saves.write(|width| Settings::update(&home, |s| s.window.left_width = Some(width)).map(drop)) }).await;
            let Err(e) = saved else { return };
            this.update(cx, |this, cx| {
                this.action_error = Some(format!("Cannot save the panel width: {e}"));
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests;
