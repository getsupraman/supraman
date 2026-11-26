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
