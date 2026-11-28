use supraman_ui::dnd::Zone;

use super::*;

const TARGET: u64 = 1;
const KINDS: [Kind; 4] = [Kind::Terminal, Kind::Git, Kind::Work, Kind::Details];
const EDGES: [Zone; 4] = [Zone::Left, Zone::Right, Zone::Top, Zone::Bottom];

/// Every payload the dock can be offered, by name: a panel of the target's own dock, another dock's, a tab of one, a tab
/// of two panels and a session row.
fn payloads() -> [(&'static str, Dragged<'static>); 5] {
    [
        ("own dock's panel", Dragged::Panel { id: 2, dock: TARGET }),
        ("other dock's panel", Dragged::Panel { id: 3, dock: 9 }),
        ("tab of one", Dragged::Tab { id: 4, panels: 1 }),
        ("tab of two", Dragged::Tab { id: 5, panels: 2 }),
        ("session row", Dragged::Session("s-5")),
    ]
}

fn expected(kind: Kind, zone: Zone, name: &str) -> Rule {
    let terminal = kind == Kind::Terminal;
    match (name, zone) {
        ("session row", _) | (_, Zone::Centre) => if terminal { Rule::Paste } else { Rule::Nothing },
        ("tab of two", _) => Rule::Nothing,
        ("own dock's panel", _) => Rule::Place(2),
        ("other dock's panel", _) => Rule::Place(3),
        ("tab of one", _) => Rule::Place(4),
        _ => unreachable!(),
    }
}

#[test]
fn panel_drop_each_zone_payload_and_target_kind() {
    for kind in KINDS {
        for zone in Zone::ALL {
            for (name, dragged) in payloads() {
                assert_eq!(panel_drop(TARGET, kind, zone, dragged), expected(kind, zone, name), "{name} on a {kind:?} panel's {zone:?}");
            }
        }
    }
}

#[test]
fn panel_drop_a_terminals_centre_still_pastes_and_only_a_terminals() {
    for (name, dragged) in payloads() {
        assert_eq!(panel_drop(TARGET, Kind::Terminal, Zone::Centre, dragged), Rule::Paste, "{name}");
        for kind in [Kind::Git, Kind::Work, Kind::Details, Kind::Other] {
            assert_eq!(panel_drop(TARGET, kind, Zone::Centre, dragged), Rule::Nothing, "{name} on {kind:?}'s centre");
        }
    }
}

#[test]
fn panel_drop_onto_its_own_slot_does_nothing() {
    for kind in KINDS {
        for zone in Zone::ALL {
            assert_eq!(panel_drop(TARGET, kind, zone, Dragged::Panel { id: TARGET, dock: TARGET }), Rule::Nothing, "a panel on itself, {kind:?} {zone:?}");
            assert_eq!(panel_drop(TARGET, kind, zone, Dragged::Tab { id: TARGET, panels: 1 }), Rule::Nothing, "a tab on its own content, {kind:?} {zone:?}");
        }
    }
}

#[test]
fn panel_drop_a_tab_of_two_or_more_is_never_placed() {
    for panels in 2..=4 {
        assert_eq!(placed(Dragged::Tab { id: 5, panels }), None);
        for zone in EDGES {
            assert_eq!(panel_drop(TARGET, Kind::Git, zone, Dragged::Tab { id: 5, panels }), Rule::Nothing);
        }
    }
    assert_eq!(placed(Dragged::Tab { id: 4, panels: 1 }), Some(4));
    assert_eq!(placed(Dragged::Session("s")), None);
}

#[test]
fn panel_drop_reads_a_dragged_tab_by_its_dock() {
    let panel = DraggedTab { id: 3, title: "Git".into(), dock: Some(7) };
    let tab = DraggedTab { id: 7, title: "shell".into(), dock: None };
    assert_eq!(dragged(&panel, 1), Dragged::Panel { id: 3, dock: 7 });
    assert_eq!(dragged(&tab, 2), Dragged::Tab { id: 7, panels: 2 }, "a dock's own tab dragged from the strip is the whole tab");
    let own = DraggedTab { id: 7, title: "shell".into(), dock: Some(7) };
    assert_eq!(placed(dragged(&own, 2)), Some(7), "its first panel out of its title bar is one panel");
}

mod drags {
    use gpui::{Bounds, Modifiers, MouseButton, Pixels, Point, TestAppContext, VisualTestContext, point, px};
    use supraman_gitman::fixture::Fixture;
    use supraman_ui::dock::layout::{Axis, Layout};

    use crate::view::FleetView;
    use crate::view::dock::tests::{draw, git_panel, shell, window};

    fn hit(view: &gpui::Entity<FleetView>, key: &str, cx: &mut VisualTestContext) -> Option<Bounds<Pixels>> {
        view.read_with(cx, |v, _| v.hits.borrow().get(key).map(|h| h.bounds))
    }

    fn painted(view: &gpui::Entity<FleetView>, key: &str, cx: &mut VisualTestContext) -> Bounds<Pixels> {
        view.read_with(cx, |v, _| v.hits.borrow_mut().clear());
        draw(view, cx);
        hit(view, key, cx).unwrap_or_else(|| panic!("{key} paints"))
    }

    fn press(cx: &mut VisualTestContext, from: Point<Pixels>) {
        cx.simulate_mouse_move(from, None, Modifiers::none());
        cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(from + point(px(10.), px(4.)), Some(MouseButton::Left), Modifiers::none());
        cx.run_until_parked();
    }

    fn over(view: &gpui::Entity<FleetView>, at: Point<Pixels>, cx: &mut VisualTestContext) {
        cx.simulate_mouse_move(at, Some(MouseButton::Left), Modifiers::none());
        view.read_with(cx, |v, _| v.hits.borrow_mut().clear());
        draw(view, cx);
    }

    fn release(view: &gpui::Entity<FleetView>, at: Point<Pixels>, cx: &mut VisualTestContext) {
        cx.simulate_mouse_up(at, MouseButton::Left, Modifiers::none());
        draw(view, cx);
    }

    #[gpui::test]
    fn panel_drop_a_git_panel_to_the_strip_and_its_tab_back_onto_the_shells_right_edge(cx: &mut TestAppContext) {
        let f = Fixture::shared();
        let (view, cx) = window(cx);
        let (tab, _) = shell(&view, cx);
        let (panel, _) = git_panel(&view, &f, cx);
        let (other, _) = shell(&view, cx);
        assert!(view.update(cx, |v, cx| v.add_panel(tab, panel, cx)));
        view.update(cx, |v, cx| v.activate_tab(tab, cx));
        assert_eq!(view.read_with(cx, |v, _| v.header_ids()), [tab, other]);

        let title = painted(&view, &format!("dock-{tab}-title-1"), cx).center();
        press(cx, title);
        let onto = painted(&view, &format!("tab-{other}"), cx);
        let left_half = point(onto.left() + onto.size.width * 0.25, onto.center().y);
        over(&view, left_half, cx);
        release(&view, left_half, cx);
        let state = view.read_with(cx, |v, _| (v.header_ids(), v.docks.of(tab).is_some(), v.active));
        assert_eq!(state, (vec![tab, panel, other], false, Some(tab)), "the panel is a tab before the one it was let go on, its dock closed up, the shell still shown");

        let header = painted(&view, &format!("tab-{panel}"), cx).center();
        press(cx, header);
        let aim = painted(&view, &format!("dock-{tab}-aim-0"), cx);
        let right = point(aim.right() - aim.size.width * 0.1, aim.center().y);
        over(&view, right, cx);
        let ghost = hit(&view, &format!("dock-{tab}-ghost"), cx).expect("the ghost in the right half");
        assert!(ghost.left() > aim.center().x - px(2.), "the ghost takes the right half: {ghost:?} of {aim:?}");
        release(&view, right, cx);
        let laid = view.read_with(cx, |v, _| v.docks.of(tab).map(|d| d.layout.clone()));
        let want = Layout::Split { axis: Axis::Columns, ratio: 0.5, first: Box::new(Layout::Leaf(tab)), second: Box::new(Layout::Leaf(panel)) };
        assert_eq!(laid, Some(want), "the Git tab lands on the shell's right");
        assert_eq!(view.read_with(cx, |v, _| (v.header_ids(), v.active)), (vec![tab, other], Some(tab)));
        assert!(hit(&view, &format!("dock-{tab}-title-1"), cx).is_some(), "the placed panel paints its title bar");
        assert_eq!(hit(&view, &format!("dock-{tab}-ghost"), cx), None);
    }

    #[gpui::test]
    fn panel_drop_a_tab_of_two_panels_shows_no_ghost_on_another_tab(cx: &mut TestAppContext) {
        let f = Fixture::shared();
        let (view, cx) = window(cx);
        let (tab, _) = shell(&view, cx);
        let (panel, _) = git_panel(&view, &f, cx);
        let (other, _) = shell(&view, cx);
        assert!(view.update(cx, |v, cx| v.add_panel(tab, panel, cx)));
        view.update(cx, |v, cx| v.activate_tab(other, cx));

        let header = painted(&view, &format!("tab-{tab}"), cx).center();
        press(cx, header);
        let aim = painted(&view, &format!("dock-{other}-aim-0"), cx);
        let right = point(aim.right() - aim.size.width * 0.1, aim.center().y);
        over(&view, right, cx);
        assert_eq!(hit(&view, &format!("dock-{other}-ghost"), cx), None, "a tab of two is refused");
        release(&view, right, cx);
        assert_eq!(view.read_with(cx, |v, _| (v.header_ids(), v.docks.of(other).is_some(), v.docks.panels(tab))), (vec![tab, other], false, vec![tab, panel]));
    }
}
