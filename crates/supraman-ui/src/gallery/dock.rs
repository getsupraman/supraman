//! The dock section's samples: two panels, three, and a pair whose second panel a probe drags onto the first's bottom edge.

use gpui::{AnyElement, App, Context, SharedString, div, prelude::*, relative, rems};
use theme::ActiveTheme as _;

use super::Gallery;
use super::sections::group;
use crate::dnd::Payload;
use crate::dock::layout::Layout;
use crate::dock::view::{Dock, Panel};
use crate::hit::hit_target;
use crate::icon::Icon;
use crate::tokens;

/// The docks' ids, each its sample's index in [`Gallery::docks`].
const IDS: [&str; 3] = ["kit-dock-a", "kit-dock-b", "kit-dock-c"];
/// Where the drag probe lets go: inside the third dock's first slot, in its bottom band.
pub(super) const AIM: &str = "kit-dock-c-aim";

#[derive(Clone)]
struct Dragged(usize, u8);

impl Payload for Dragged {
    fn title(&self) -> SharedString {
        WORDS[usize::from(self.1)].into()
    }
}

const WORDS: [&str; 4] = ["Console", "History · sample", "Board · sample", "Facts"];
const ICONS: [Icon; 4] = [Icon::Terminal, Icon::Clock, Icon::ListFlat, Icon::File];

pub(super) fn layouts() -> [Layout<u8>; 3] {
    let two = Layout::Leaf(0).add(1).expect("two panels");
    [two.clone(), two.add(2).expect("three panels"), two]
}

fn panel(dock: usize, p: u8, cx: &App) -> (u8, Panel<Dragged>) {
    let colors = cx.theme().colors();
    let words = WORDS[usize::from(p)];
    let element = div().size_full().p(tokens::space::L).bg(colors.editor_background).text_color(colors.text_muted).text_size(tokens::text::SMALL).child(format!("{words}'s content")).into_any_element();
    (p, Panel { element, icon: ICONS[usize::from(p)], words: words.into(), payload: Dragged(dock, p) })
}

fn sample(g: &Gallery, ix: usize, cx: &mut Context<Gallery>) -> AnyElement {
    let panels = (0..4).map(|p| panel(ix, p, cx)).collect();
    let dock = Dock::new(&g.hits, IDS[ix], move |d: &Dragged, _| (d.0 == ix).then_some(d.1))
        .focused(Some(0))
        .on_layout(move |g: &mut Gallery, laid, cx| {
            g.docks[ix] = laid;
            cx.notify();
        })
        .render(&g.docks[ix], panels, cx);
    let aim = (ix == 2).then(|| div().absolute().left(relative(0.2)).top(relative(0.88)).w(relative(0.1)).h(relative(0.06)).child(hit_target(&g.hits, AIM.into())));
    let border = cx.theme().colors().border;
    div().relative().w(rems(40.)).h(rems(12.)).border(tokens::stroke::HAIRLINE).border_color(border).child(dock).children(aim).into_any_element()
}

pub(super) fn section(g: &Gallery, cx: &mut Context<Gallery>) -> AnyElement {
    let [two, three, dragged] = [0, 1, 2].map(|ix| sample(g, ix, cx));
    div()
        .flex()
        .flex_col()
        .gap(tokens::space::XXL)
        .child(group("dock::view::Dock: two panels, the first focused; the divider is the one line", cx).child(two))
        .child(group("Three panels: the second half split across", cx).child(three))
        .child(group("A title bar dragged onto the first panel's bottom edge paints the dropped layout and its ghost", cx).child(dragged))
        .into_any_element()
}
