use gpui::TestAppContext;

use super::{Act, Gallery, Section};

#[gpui::test]
fn every_section_paints_what_its_probes_point_at(cx: &mut TestAppContext) {
    cx.update(|cx| crate::init_globals(cx, "test"));
    let (gallery, cx) = cx.add_window_view(|window, cx| Gallery::new(window, cx));
    for section in Section::ALL {
        gallery.update(cx, |g, cx| g.show(section, cx));
        cx.run_until_parked();
        cx.update(|_, _| ());
        let hits = gallery.read_with(cx, |g, _| g.hits());
        assert!(!hits.borrow().is_empty() || section == Section::Marks, "{section:?} paints its samples");
        for probe in section.probes() {
            match &probe.act {
                Act::Hover(key) | Act::Press(key) | Act::Tooltip(key, _) => {
                    assert!(hits.borrow().contains_key(key), "{section:?}'s {} points at {key}, which did not paint", probe.name);
                }
                Act::Drag(from, to) => {
                    assert!(hits.borrow().contains_key(from) && hits.borrow().contains_key(to), "{section:?}'s {} drags {from} over {to}, which did not both paint", probe.name);
                }
                Act::Focus(name) => {
                    let focused = cx.update(|window, cx| gallery.update(cx, |g, cx| g.focus_sample(name, window, cx)));
                    assert!(focused, "{section:?} has no focus sample {name}");
                }
            }
        }
    }
}

#[gpui::test]
fn the_dock_probe_drags_onto_a_zone_that_paints_a_ghost(cx: &mut TestAppContext) {
    use gpui::{Modifiers, MouseButton};
    cx.update(|cx| crate::init_globals(cx, "test"));
    let (gallery, cx) = cx.add_window_view(|window, cx| Gallery::new(window, cx));
    gallery.update(cx, |g, cx| g.show(Section::Dock, cx));
    cx.run_until_parked();
    let Some(Act::Drag(from, to)) = Section::Dock.probes().into_iter().map(|p| p.act).find(|a| matches!(a, Act::Drag(..))) else { panic!("the dock's drag probe") };
    let hits = gallery.read_with(cx, |g, _| g.hits());
    let at = |key: &str| hits.borrow()[key].bounds.center();
    let (from, to) = (at(&from), at(&to));
    cx.simulate_mouse_move(from, None, Modifiers::none());
    cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
    for step in 1..=8 {
        let t = step as f32 / 8.;
        cx.simulate_mouse_move(from + (to - from) * t, Some(MouseButton::Left), Modifiers::none());
    }
    cx.run_until_parked();
    hits.borrow_mut().clear();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    assert!(hits.borrow().contains_key("kit-dock-c-ghost"), "the probe's frame paints the ghost");
}
