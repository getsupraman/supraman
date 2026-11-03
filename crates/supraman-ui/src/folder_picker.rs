use std::path::PathBuf;

use gpui::{Context, PathPromptOptions, SharedString, Window};

/// The system folder picker; `picked` runs on the owner with the chosen folder once the window is active again,
/// so keys reach it whatever Windows gave the activation to. Cancel runs nothing.
pub fn pick_folder<V: 'static>(
    prompt: impl Into<SharedString>,
    window: &mut Window,
    cx: &mut Context<V>,
    picked: impl FnOnce(&mut V, PathBuf, &mut Window, &mut Context<V>) + 'static,
) {
    let paths = cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some(prompt.into()) });
    cx.spawn_in(window, async move |this, cx| {
        let Ok(Ok(Some(paths))) = paths.await else {
            return;
        };
        if let Some(path) = paths.into_iter().next() {
            this.update_in(cx, |this, window, cx| {
                window.activate_window();
                picked(this, path, window, cx);
            })
            .ok();
        }
    })
    .detach();
}
