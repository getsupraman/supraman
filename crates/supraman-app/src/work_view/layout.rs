//! The Work tab's column widths, `<home>/work-layout.json`: the one reader and writer of that file. Read off
//! the UI thread at open; each change written off it.

use std::path::{Path, PathBuf};

use gpui::{Context, Pixels, px};
use serde::{Deserialize, Serialize};
use supraman_core::store::JsonStore;
use supraman_ui::split::SplitPane;
use supraman_ui::store::save_off_thread;

use super::WorkView;

pub const NAV_MIN: f32 = 170.;
pub const NAV_DEFAULT: f32 = 250.;
pub const LIST_MIN: f32 = 300.;
pub const DETAIL_MIN: f32 = 320.;
pub const DETAIL_DEFAULT: f32 = 520.;
const LOST: &str = "Work tabs open at their default widths until the file is fixed or removed";

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkSizes {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nav: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<u32>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub nav_shut: bool,
}

pub fn path(home: &Path) -> PathBuf {
    home.join("work-layout.json")
}

pub fn nav_split() -> SplitPane {
    SplitPane { id: "work-nav-split".into(), min_first: px(NAV_MIN), min_second: px(LIST_MIN + DETAIL_MIN + 1.), default: px(NAV_DEFAULT) }
}

pub fn detail_split() -> SplitPane {
    SplitPane { id: "work-detail-split".into(), min_first: px(LIST_MIN), min_second: px(DETAIL_MIN), default: px(DETAIL_DEFAULT) }
}

#[derive(Default)]
pub struct Layout {
    store: Option<JsonStore<WorkSizes>>,
    sizes: WorkSizes,
}

impl Layout {
    pub fn nav(&self) -> Pixels {
        px(self.sizes.nav.map_or(NAV_DEFAULT, |w| w as f32))
    }

    pub fn detail(&self) -> Pixels {
        px(self.sizes.detail.map_or(DETAIL_DEFAULT, |w| w as f32))
    }

    pub fn nav_shut(&self) -> bool {
        self.sizes.nav_shut
    }

    pub fn set_nav(&mut self, width: Pixels) {
        self.sizes.nav = Some(f32::from(width).round() as u32);
    }

    pub fn set_detail(&mut self, width: Pixels) {
        self.sizes.detail = Some(f32::from(width).round() as u32);
    }

    pub fn load(&mut self, home: Option<PathBuf>, cx: &mut Context<WorkView>) {
        let Some(home) = home else { return };
        cx.spawn(async move |this, cx| {
            let (store, notice) = cx.background_executor().spawn(async move { JsonStore::<WorkSizes>::load(Some(path(&home)), LOST) }).await;
            this.update(cx, |this, cx| {
                this.layout.sizes = store.get().clone();
                this.layout.store = Some(store);
                if let Some(notice) = notice {
                    this.toast.show_why(notice, |this: &mut WorkView| &mut this.toast, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn keep(&mut self, cx: &mut Context<WorkView>) {
        let sizes = self.sizes.clone();
        let save = self.store.as_mut().and_then(|s| s.edit(|kept| std::mem::replace(kept, sizes.clone()) != sizes));
        save_off_thread(save, cx, |this: &mut WorkView, saved, cx| {
            if let Err(why) = saved {
                this.toast.show_why(format!("The Work tab's widths were not kept: {why}"), |this: &mut WorkView| &mut this.toast, cx);
            }
        });
    }
}

impl WorkView {
    pub fn toggle_nav(&mut self, cx: &mut Context<Self>) {
        self.layout.sizes.nav_shut = !self.layout.sizes.nav_shut;
        self.layout.keep(cx);
        cx.notify();
    }
}
