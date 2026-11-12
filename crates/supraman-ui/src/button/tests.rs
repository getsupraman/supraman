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
                let mut button = action(&self.hits, format!("{tone:?}-{fill:?}"), "Label", tone, fill, cx);
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
        assert_eq!(hit.bounds.size.height, HEIGHT - px(2.), "{key}: one height, its hit inside the 1 px border");
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
