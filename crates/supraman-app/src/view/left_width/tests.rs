use gpui::px;

use super::LeftWidth;
use crate::view::MIN_LEFT_WIDTH;

#[test]
fn the_saved_width_applies_until_the_user_drags_the_panel() {
    let mut width = LeftWidth::default();
    assert_eq!(width.loaded(Some(320)), Some(px(320.)));
    assert_eq!(width.loaded(Some(10)), Some(MIN_LEFT_WIDTH), "held at the panel's minimum");
    assert_eq!(width.loaded(None), None);
    width.dragged = true;
    assert_eq!(width.loaded(Some(320)), None, "a drag made before settings.toml was read is kept");
}
