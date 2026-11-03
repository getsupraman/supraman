use gpui::Context;
use supraman_core::store::Save;

/// Writes `save` on the background executor, then hands its outcome to `done` on the owner.
pub fn save_off_thread<V: 'static>(save: Option<Save>, cx: &mut Context<V>, done: impl FnOnce(&mut V, Result<(), String>, &mut Context<V>) + 'static) {
    let Some(save) = save else { return };
    cx.spawn(async move |this, cx| {
        let saved = cx.background_executor().spawn(async move { save.write() }).await;
        this.update(cx, |this, cx| done(this, saved, cx)).ok();
    })
    .detach();
}
