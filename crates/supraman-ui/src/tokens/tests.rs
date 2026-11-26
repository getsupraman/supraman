use gpui::TestAppContext;

use super::*;
use crate::themes::{self, INACTIVE_SELECTION_SHOWS, INFO_SATURATION, SECONDARY_FLOOR, contrast};

fn bundled() -> Vec<theme::Theme> {
    let light = themes::parse_family(themes::selftest_light::LIGHT).unwrap().themes[0].clone();
    vec![themes::dark_modern().theme, themes::supraman_dark().theme, themes::from_zed(&themes::one_dark(), &light).theme]
}

#[test]
fn the_selection_reads_stronger_than_the_inactive_one_which_still_shows_on_the_panel() {
    for theme in bundled() {
        let (r, panel) = (Roles::of(&theme), theme.colors().panel_background);
        let (active, inactive) = (contrast(r.selection, panel), contrast(r.selection_inactive, panel));
        assert!(active > inactive, "{}: active {active:.3} over inactive {inactive:.3}", theme.name);
        assert!(inactive >= INACTIVE_SELECTION_SHOWS, "{}: inactive {inactive:.3} shows", theme.name);
    }
}

#[test]
fn info_is_a_colour_apart_from_muted_text_and_holds_its_floor() {
    for theme in bundled() {
        let r = Roles::of(&theme);
        let muted = theme.colors().text_muted;
        assert!(r.info.s >= INFO_SATURATION, "{}: info {:?}", theme.name, r.info);
        assert!(r.info.s - muted.s >= INFO_SATURATION / 2., "{}: info {:?} beside muted {:?}", theme.name, r.info, muted);
        assert_eq!(r.info, theme.status().info, "{}: one info", theme.name);
        for surface in themes::surfaces(&theme) {
            assert!(contrast(r.info, surface) >= SECONDARY_FLOOR, "{}: info on {surface:?}", theme.name);
        }
    }
}

#[gpui::test]
fn the_roles_follow_the_theme_applied(cx: &mut TestAppContext) {
    cx.update(|cx| crate::init_globals(cx, "test"));
    let light = themes::from_zed(&themes::one_dark(), &themes::parse_family(themes::selftest_light::LIGHT).unwrap().themes[0]);
    let shown = cx.update(|cx| {
        let before = roles(cx);
        let applied = themes::apply_resolved(cx, themes::Resolved { theme: light.theme.clone(), adjusted: light.adjusted.clone(), problem: None });
        (before, roles(cx), Roles::of(&applied.theme))
    });
    assert_ne!(shown.0, shown.1, "a new theme, new roles");
    assert_eq!(shown.1, shown.2, "derived from the theme applied");
}

#[test]
fn the_layers_stack_in_their_order() {
    let order = [Layer::Float, Layer::Form, Layer::Card, Layer::OverForm, Layer::Menu, Layer::Dialog];
    assert!(order.windows(2).all(|w| w[0].priority() < w[1].priority()));
    assert_eq!(Layer::Float.priority(), 1);
}

#[test]
fn the_open_fill_reads_in_every_theme_apart_from_the_hover_and_both_selections() {
    for theme in bundled() {
        let (r, c) = (Roles::of(&theme), theme.colors());
        let panel = c.panel_background;
        assert!(contrast(r.open, panel) >= INACTIVE_SELECTION_SHOWS, "{}: open {:?} shows on the panel", theme.name, r.open);
        assert!(contrast(c.text, r.open) >= themes::NAME_FLOOR, "{}: its text holds the name floor", theme.name);
        for (name, other) in [("hover", c.ghost_element_hover), ("hover", c.element_hover), ("selection", r.selection), ("inactive selection", r.selection_inactive)] {
            let d = themes::apart(r.open, other, panel);
            assert!(d >= themes::OPEN_APART, "{}: open {:?} and {name} {other:?} only {d:.3} apart", theme.name, r.open);
        }
    }
}

