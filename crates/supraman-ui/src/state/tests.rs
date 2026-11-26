use gpui::{Hsla, hsla};

use super::*;

fn fills() -> (Fills, [Hsla; 5]) {
    let c = [0.1, 0.2, 0.3, 0.4, 0.5].map(|h| hsla(h, 0.5, 0.5, 1.));
    (Fills { rest: Some(c[0]), hover: Some(c[1]), pressed: Some(c[2]), selected: Some(c[3]), open: Some(c[4]) }, c)
}

#[test]
fn each_state_overrides_the_ones_before_it_and_open_wins_over_selected() {
    let (f, c) = fills();
    let at = |s: State| resolve(s, f);
    assert_eq!(at(State::default()).fill, Some(c[0]));
    assert_eq!(at(State::default()).hover, Some(c[1]), "the pointer's hover at rest");
    assert_eq!(at(State { hovered: true, ..State::default() }).fill, Some(c[1]));
    let selected = at(State { selected: true, hovered: true, ..State::default() });
    assert_eq!((selected.fill, selected.hover), (Some(c[3]), None), "a selected one keeps its fill under the pointer");
    let open = at(State { selected: true, open: true, ..State::default() });
    assert_eq!((open.fill, open.hover), (Some(c[4]), None), "open over selected, and no hover while the pointer is in its popup");
}

#[test]
fn a_disabled_one_is_dimmed_with_no_hover_or_press_and_the_ring_follows_keyboard_focus() {
    let (f, c) = fills();
    let off = resolve(State { disabled: true, ..State::default() }, f);
    assert_eq!((off.fill, off.hover, off.pressed, off.opacity), (Some(c[0]), None, None, tokens::DIMMED));
    assert!(resolve(State { focus_visible: true, ..State::default() }, f).ring);
    assert!(!resolve(State::default(), f).ring);
    let unset = resolve(State { open: true, ..State::default() }, Fills { rest: Some(c[0]), ..Fills::default() });
    assert_eq!(unset.fill, Some(c[0]), "a state with no fill of its own keeps the one below");
}
