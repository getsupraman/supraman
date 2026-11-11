use gpui::Context;
use supraman_core::store::Save;

use super::FleetView;

impl FleetView {
    pub fn save_off_thread(&self, save: Option<Save>, failed: &'static str, cx: &mut Context<Self>) {
        supraman_ui::store::save_off_thread(save, cx, move |this, saved, cx| {
            if let Err(e) = saved {
                this.action_error = Some(format!("{failed}: {e}"));
                cx.notify();
            }
        });
    }

    /// A save of entries pruned by Supraman itself; a failure leaves them in the file, harmless, so it is logged.
    pub(super) fn save_pruned(&self, save: Option<Save>, failed: &'static str, cx: &mut Context<Self>) {
        supraman_ui::store::save_off_thread(save, cx, move |_, saved, _| {
            if let Err(e) = saved {
                log::warn!("{failed}: {e}");
            }
        });
    }
}
