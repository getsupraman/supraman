use gpui::{Context, Pixels, Render, TestAppContext, VisualTestContext, Window, rems};

use super::*;

struct Row {
    hits: HitTargets,
}

impl Render for Row {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let label = div().relative().child("View all files").child(hit_target(&self.hits, "label".into()));
        let row = div().flex().items_center().gap_1p5().text_xs().child(checkbox(&self.hits, "tick", true, cx)).child(label);
        div().flex().flex_col().gap_2().p_2().child(row).child(div().flex().child(pill(&self.hits, "pill", "main", gpui::red(), cx)))
    }
}

fn centre(row: &gpui::Entity<Row>, key: &str, cx: &mut VisualTestContext) -> Pixels {
    row.read_with(cx, |r, _| r.hits.borrow().get(key).map(|h| h.bounds.center().y)).unwrap_or_else(|| panic!("{key} paints"))
}

#[gpui::test]
fn a_tick_box_sits_on_its_labels_letters_and_a_pills_letters_on_its_middle(cx: &mut TestAppContext) {
    cx.update(|cx| crate::init_globals(cx, "test"));
    let (row, cx) = cx.add_window_view(|_, _| Row { hits: HitTargets::default() });
    cx.run_until_parked();
    let below = cx.update(|window, _| {
        let mut style = window.text_style();
        style.font_size = rems(0.75).into();
        crate::ink::offset(&style, window)
    });
    assert!(below > px(0.), "the letters' middle is below the line's");
    assert!(crate::ink::level(centre(&row, "tick", cx), centre(&row, "label", cx) + below), "the square on the label's letters");
    assert!(crate::ink::level(centre(&row, "pill", cx), centre(&row, "pill-label", cx) + below), "the pill's letters on its middle");
}

const TONES: [Tone; 3] = [Tone::Neutral, Tone::Success, Tone::Danger];
const FILLS: [Fill; 2] = [Fill::Solid, Fill::Outline];

#[derive(Debug, Clone, Copy, PartialEq)]
struct Painted {
    fill: Option<Hsla>,
    border: Option<Hsla>,
    text: Option<Hsla>,
}

struct Actions {
    hits: HitTargets,
    painted: Vec<(Tone, Fill, Painted)>,
}

impl Render for Actions {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.painted.clear();
        let mut row = div().flex().gap_2().p_2();
        for tone in TONES {
            for fill in FILLS {
                let mut button = Button::new(&self.hits, format!("{tone:?}-{fill:?}")).label("Label").tone(tone).fill(fill).build(cx);
                let style = button.style();
                let painted = Painted { fill: style.background.as_ref().and_then(|f| f.color()).and_then(|b| b.as_solid()), border: style.border_color, text: style.text.color };
                self.painted.push((tone, fill, painted));
                row = row.child(button);
            }
        }
        row
    }
}

#[gpui::test]
fn each_tone_and_fill_paints_its_theme_colours(cx: &mut TestAppContext) {
    cx.update(|cx| crate::init_globals(cx, "test"));
    let (view, cx) = cx.add_window_view(|_, _| Actions { hits: HitTargets::default(), painted: Vec::new() });
    cx.run_until_parked();
    let (colors, status) = cx.update(|_, cx| (cx.theme().colors().clone(), cx.theme().status().clone()));
    let painted = view.read_with(cx, |v, _| v.painted.clone());
    assert_eq!(painted.len(), 6, "every tone in every fill");
    for (tone, fill, p) in painted {
        let key = format!("{tone:?}-{fill:?}");
        let hit = view.read_with(cx, |v, _| v.hits.borrow().get(&key).copied()).unwrap_or_else(|| panic!("{key} paints"));
        assert_eq!(hit.bounds.size.height, Size::Small.height() - px(2.), "{key}: one height, its hit inside the 1 px border");
        let tint = match tone {
            Tone::Neutral => colors.border,
            Tone::Success => status.success,
            Tone::Danger => status.error,
        };
        assert_eq!(p.border, Some(tint), "{key}: its tone's border");
        match fill {
            Fill::Outline => {
                assert_eq!(p.fill.map(|f| f.a), Some(0.), "{key}: a transparent fill");
                let text = if tone == Tone::Neutral { colors.text } else { tint };
                assert_eq!(p.text, Some(text), "{key}: its tone's text");
            }
            Fill::Solid => {
                let fill = if tone == Tone::Neutral { colors.element_background } else { tint };
                assert_eq!(p.fill, Some(fill), "{key}: its tone's fill");
                let text = p.text.expect("a text colour");
                assert!(crate::themes::contrast(text, fill) >= 3., "{key}: legible on its fill");
            }
        }
    }
}

struct Toned {
    hits: HitTargets,
    painted: Option<Painted>,
}

impl Render for Toned {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let error = cx.theme().status().error;
        let mut button = Button::new(&self.hits, "trash").icon(crate::icon::Icon::Trash.svg(14., error)).icon_only(px(24.)).tone(Tone::Danger).build(cx);
        let style = button.style();
        self.painted = Some(Painted { fill: style.background.as_ref().and_then(|f| f.color()).and_then(|b| b.as_solid()), border: style.border_color, text: style.text.color });
        div().flex().p_2().child(button)
    }
}

#[gpui::test]
fn a_danger_icon_rests_on_the_errors_tint_and_hovers_on_its_solid_fill(cx: &mut TestAppContext) {
    cx.update(|cx| crate::init_globals(cx, "test"));
    let (view, cx) = cx.add_window_view(|_, _| Toned { hits: HitTargets::default(), painted: None });
    cx.run_until_parked();
    let (colors, error) = cx.update(|_, cx| (cx.theme().colors().clone(), cx.theme().status().error));
    let painted = view.read_with(cx, |v, _| v.painted).expect("painted");
    let (outline, solid, toned) = cx.update(|_, cx| (look(Tone::Danger, Fill::Outline, cx), look(Tone::Danger, Fill::Solid, cx), icon_look(Tone::Danger, cx)));
    assert_eq!(painted.fill, Some(outline.hover), "at rest the error's tint");
    assert_eq!(painted.fill.map(|f| f.a), Some(error.a * 0.2), "a tint, not the solid fill");
    assert_eq!(painted.border, Some(error), "the error's border");
    assert_eq!(toned.hover, solid.fill, "hovered the solid error fill");
    assert_eq!(toned.hover, error);
    assert_ne!(toned.hover, colors.ghost_element_hover, "never the ghost's hover");
    assert_eq!(toned.hover_text, solid.text, "the icon in a solid danger's text colour while hovered");
    assert!(crate::themes::contrast(toned.hover_text, error) >= 3., "the icon legible on the solid fill");
    assert_eq!(toned.text, error, "the icon in the error colour at rest");
}

struct Keyed {
    hits: HitTargets,
}

impl Render for Keyed {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = cx.theme().colors().ghost_element_selected;
        let row = |n: &str, on: bool| {
            let keys = RowKeys { id: format!("session-{n}").into(), group: format!("session-group-{n}").into(), hit: format!("session-row-{n}").into() };
            row_in(&self.hits, keys, on.then_some(selected), cx).h(px(24.)).child(n.to_string())
        };
        div().w(px(200.)).flex().flex_col().child(row("a", true)).child(row("b", false))
    }
}

fn hovered_fills(view: &gpui::Entity<Keyed>, hit: &str, cx: &mut VisualTestContext) -> Vec<gpui::Background> {
    let bounds = view.read_with(cx, |v, _| v.hits.borrow().get(hit).map(|h| h.bounds)).unwrap_or_else(|| panic!("{hit} paints"));
    cx.simulate_mouse_move(bounds.center(), None, gpui::Modifiers::none());
    view.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    cx.update(|window, _| {
        let middle = bounds.center().scale(window.scale_factor());
        window.painted_quads().into_iter().filter(|q| q.bounds.contains(&middle) && q.content_mask.bounds.contains(&middle)).map(|q| q.background).collect()
    })
}

#[gpui::test]
fn a_row_under_its_own_hit_key_keeps_its_selected_fill_hovered_and_another_takes_the_hover(cx: &mut TestAppContext) {
    cx.update(|cx| crate::init_globals(cx, "test"));
    let (view, cx) = cx.add_window_view(|_, _| Keyed { hits: HitTargets::default() });
    cx.run_until_parked();
    let colors = cx.update(|_, cx| cx.theme().colors().clone());
    let (selected, hover) = (gpui::Background::from(colors.ghost_element_selected), gpui::Background::from(colors.ghost_element_hover));
    let shown = hovered_fills(&view, "session-row-a", cx);
    assert!(shown.contains(&selected) && !shown.contains(&hover), "the selected row under the pointer keeps its fill: {shown:?}");
    let other = hovered_fills(&view, "session-row-b", cx);
    assert!(other.contains(&hover) && !other.contains(&selected), "an unselected row under the pointer takes the hover: {other:?}");
    assert!(view.read_with(cx, |v, _| v.hits.borrow().get("session-a").is_none()), "the hit key is the row's own, not its id");
}

struct Sizes {
    hits: HitTargets,
}

impl Render for Sizes {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let mut column = div().flex().flex_col().gap_2().p_2();
        for size in Size::ALL {
            let plain = Button::new(&self.hits, format!("{size:?}")).label("Agj").size(size);
            let with = Button::new(&self.hits, format!("{size:?}-icon")).icon(crate::icon::Icon::Check).label("Open").tone(Tone::Success).size(size);
            column = column.child(Button::line().child(plain).child(with));
        }
        column
    }
}

fn bounds(hits: &HitTargets, key: &str) -> gpui::Bounds<Pixels> {
    hits.borrow().get(key).map(|h| h.bounds).unwrap_or_else(|| panic!("{key} paints"))
}

#[gpui::test]
fn every_size_stands_at_its_height_with_its_letters_on_its_middle(cx: &mut TestAppContext) {
    cx.update(|cx| crate::init_globals(cx, "test"));
    let (view, cx) = cx.add_window_view(|_, _| Sizes { hits: HitTargets::default() });
    cx.run_until_parked();
    let hits = view.read_with(cx, |v, _| v.hits.clone());
    for size in Size::ALL {
        let (height, _, text) = size.table();
        let below = cx.update(|window, _| {
            let mut style = window.text_style();
            style.font_size = text.into();
            crate::ink::offset(&style, window)
        });
        assert!(below > px(0.), "{size:?}: the letters' middle is below the line's");
        for key in [format!("{size:?}"), format!("{size:?}-icon")] {
            let (button, label) = (bounds(&hits, &key), bounds(&hits, &format!("{key}-label")));
            assert_eq!(button.size.height + px(2.), height, "{key}: its size's height");
            assert!(crate::ink::level(button.center().y, label.center().y + below), "{key}: the letters on the middle: {button:?} {label:?} by {below:?}");
        }
    }
    assert_eq!(Size::Medium.height(), crate::text_input::SINGLE_HEIGHT, "a medium button as high as a one-line field");
    let (plain, with) = (bounds(&hits, "Medium"), bounds(&hits, "Medium-icon"));
    assert_eq!(with.left() - plain.right(), GAP + px(2.), "a line's buttons one gap apart, border to border");
}

struct Three {
    hits: HitTargets,
    members: crate::focus_order::Members,
    pressed: Vec<usize>,
    skip: Option<usize>,
}

impl Render for Three {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.members.begin();
        let mut row = Button::line();
        for n in 0..3 {
            let key = format!("answer-{n}");
            let button = Button::new(&self.hits, key.clone()).label(format!("Answer {n}")).size(Size::Medium);
            row = row.child(if self.skip == Some(n) {
                button
            } else {
                let focus = self.members.handle(key, cx);
                button.acting(&focus, move |this: &mut Three, _, _| this.pressed.push(n), cx)
            });
        }
        self.members.order().on(row.id("three"))
    }
}

fn three(skip: Option<usize>, cx: &mut TestAppContext) -> (gpui::Entity<Three>, &mut VisualTestContext) {
    cx.update(|cx| {
        crate::init_globals(cx, "test");
        crate::focus_order::bind_keys(cx);
    });
    let (view, cx) = cx.add_window_view(|_, _| Three { hits: HitTargets::default(), members: Default::default(), pressed: Vec::new(), skip });
    cx.run_until_parked();
    (view, cx)
}

fn ringed(view: &gpui::Entity<Three>, cx: &mut VisualTestContext) -> Vec<usize> {
    cx.run_until_parked();
    cx.update(|window, cx| (0..3).filter(|n| view.read(cx).members.get(&format!("answer-{n}")).is_some_and(|h| h.is_focused(window))).collect())
}

#[gpui::test]
fn tab_walks_a_row_of_three_buttons_and_space_or_enter_presses_the_focused_one(cx: &mut TestAppContext) {
    let (view, cx) = three(None, cx);
    let first = view.read_with(cx, |v, _| v.members.get("answer-0")).expect("a handle per button");
    cx.update(|window, cx| first.focus(window, cx));
    let mut walked = vec![ringed(&view, cx)];
    for keys in ["tab", "tab", "tab", "shift-tab", "shift-tab"] {
        cx.simulate_keystrokes(keys);
        walked.push(ringed(&view, cx));
    }
    assert_eq!(walked, [vec![0], vec![1], vec![2], vec![0], vec![2], vec![1]], "Tab through each, wrapping; Shift+Tab back");
    cx.simulate_keystrokes("space");
    cx.simulate_keystrokes("tab enter");
    let pressed = view.read_with(cx, |v, _| v.pressed.clone());
    assert_eq!(pressed, [1, 2], "Space and Enter press the focused button");
    let at = bounds(&view.read_with(cx, |v, _| v.hits.clone()), "answer-0");
    cx.simulate_click(at.center(), gpui::Modifiers::none());
    assert_eq!(view.read_with(cx, |v, _| v.pressed.clone()), [1, 2, 0], "a click runs the same act");
}

struct Stepped {
    hits: HitTargets,
    members: crate::focus_order::Members,
    n: i32,
}

impl Render for Stepped {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.members.begin();
        let control = stepper(&self.hits, "max", self.n.to_string(), &self.members, |this: &mut Stepped, by, _, cx| {
            this.n += by;
            cx.notify();
        }, cx);
        let row = crate::settings_page::row("Busy sessions at most", "How many may work at once.", control.into_any_element(), None, cx);
        self.members.order().on(div().id("page").w(px(600.)).child(row))
    }
}

#[gpui::test]
fn a_settings_rows_stepper_stands_one_gap_apart_with_the_values_letters_on_the_buttons_and_steps_by_key(cx: &mut TestAppContext) {
    cx.update(|cx| {
        crate::init_globals(cx, "test");
        crate::focus_order::bind_keys(cx);
    });
    let (view, cx) = cx.add_window_view(|_, _| Stepped { hits: HitTargets::default(), members: Default::default(), n: 20 });
    cx.run_until_parked();
    let hits = view.read_with(cx, |v, _| v.hits.clone());
    let [dec, value, inc, dec_label] = ["max-dec", "max-value", "max-inc", "max-dec-label"].map(|k| bounds(&hits, k));
    assert!(crate::ink::level(value.center().y, dec_label.center().y), "the value's letters level with the buttons': {value:?} {dec_label:?}");
    assert!(crate::ink::level(dec.center().y, inc.center().y), "both buttons on one middle");
    assert!(value.left() - dec.right() >= GAP + px(1.) && inc.left() - value.right() >= GAP + px(1.), "one gap each side of the value");
    let first = view.read_with(cx, |v, _| v.members.get("max-dec")).expect("a handle");
    cx.update(|window, cx| first.focus(window, cx));
    cx.simulate_keystrokes("space tab enter enter");
    assert_eq!(view.read_with(cx, |v, _| v.n), 21, "− once, then + twice by key");
}

struct Listed {
    hits: HitTargets,
    members: crate::focus_order::Members,
    chosen: usize,
}

impl Render for Listed {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.members.begin();
        let before = Button::new(&self.hits, "before").label("Before").size(Size::Medium);
        let before = before.acting(&self.members.handle("before", cx), |_: &mut Listed, _, _| {}, cx);
        let mut list = div().flex().gap_1();
        for n in 0..3 {
            list = list.child(choice(("pick", n), format!("Pick {n}"), n == self.chosen, cx));
        }
        let focus = self.members.handle("list", cx);
        let list = choices(list, &focus, 3, Some(self.chosen), |this: &mut Listed, ix, _, cx| {
            this.chosen = ix;
            cx.notify();
        }, cx);
        self.members.order().on(div().id("form").child(before).child(list))
    }
}

#[gpui::test]
fn tab_reaches_a_choice_list_as_one_member_and_the_arrows_pick_its_neighbours_up_to_its_ends(cx: &mut TestAppContext) {
    cx.update(|cx| {
        crate::init_globals(cx, "test");
        crate::focus_order::bind_keys(cx);
    });
    let (view, cx) = cx.add_window_view(|_, _| Listed { hits: HitTargets::default(), members: Default::default(), chosen: 1 });
    cx.run_until_parked();
    let (before, list) = view.read_with(cx, |v, _| (v.members.get("before").unwrap(), v.members.get("list").unwrap()));
    cx.update(|window, cx| before.focus(window, cx));
    cx.simulate_keystrokes("tab");
    assert!(cx.update(|window, _| list.is_focused(window)), "Tab from the button to the list");
    let mut picked = Vec::new();
    for key in ["right", "right", "left", "left", "left", "down", "up"] {
        cx.simulate_keystrokes(key);
        cx.run_until_parked();
        picked.push(view.read_with(cx, |v, _| v.chosen));
    }
    assert_eq!(picked, [2, 2, 1, 0, 0, 1, 0], "each arrow its neighbour, nothing past an end");
    cx.simulate_keystrokes("tab");
    assert!(cx.update(|window, _| before.is_focused(window)), "the list is one member: Tab leaves it");
    assert_eq!((neighbour(None, 3, false), neighbour(None, 3, true), neighbour(Some(0), 0, false)), (Some(0), Some(2), None));
}
