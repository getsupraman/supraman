use gpui::{Context, Entity, Render, TestAppContext, VisualTestContext, Window, div, prelude::*, px};
use supraman_ui::controls::button::RowKeys;
use supraman_ui::hit::HitTargets;
use supraman_ui::list::{self, Chrome};
use supraman_ui::popup::Popup;
use supraman_ui::tokens::Layer;
use theme::ActiveTheme as _;

use super::{schedule_chrome, sidebar_chrome};

/// The sidebar's and a schedule's rows as the sidebar paints them: `r-0` a selected row while the sidebar holds the keys,
/// `r-1` one without them, `r-2` a selected schedule row, `r-3` a schedule row at rest, `r-4` a row under an open menu.
struct Rows {
    hits: HitTargets,
    popup: Popup,
}

impl Render for Rows {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.popup.shown(window, cx);
        let chromes = [sidebar_chrome(Chrome::new(), true, true), sidebar_chrome(Chrome::new(), true, false), sidebar_chrome(schedule_chrome(false, cx), true, true), schedule_chrome(false, cx), sidebar_chrome(Chrome::new(), false, true)];
        div().w(px(200.)).flex().flex_col().children(chromes.into_iter().enumerate().map(|(i, c)| list::row(&self.hits, RowKeys::one(format!("r-{i}").into()), c, cx).h(px(20.)).child("row")))
    }
}

fn fills_at(view: &Entity<Rows>, hit: &str, hover: bool, cx: &mut VisualTestContext) -> Vec<gpui::Background> {
    let bounds = view.read_with(cx, |v, _| v.hits.borrow().get(hit).map(|h| h.bounds)).unwrap_or_else(|| panic!("{hit} paints"));
    let at = if hover { bounds.center() } else { gpui::point(px(500.), px(500.)) };
    cx.simulate_mouse_move(at, None, gpui::Modifiers::none());
    view.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    cx.update(|window, _| {
        let middle = bounds.center().scale(window.scale_factor());
        window.painted_quads().into_iter().filter(|q| q.bounds.contains(&middle) && q.content_mask.bounds.contains(&middle)).map(|q| q.background).collect()
    })
}

#[gpui::test]
fn sidebar_and_schedule_rows_take_their_hover_selection_and_open_fills_from_the_roles(cx: &mut TestAppContext) {
    cx.update(|cx| supraman_ui::init_globals(cx, "test"));
    let (view, cx) = cx.add_window_view(|_, _| Rows { hits: HitTargets::default(), popup: Popup::open(Some("r-4".into()), Layer::Menu) });
    cx.run_until_parked();
    let (roles, hover) = cx.update(|_, cx| (supraman_ui::tokens::roles(cx), cx.theme().colors().ghost_element_hover));
    let [active, inactive, open, hover] = [roles.selection, roles.selection_inactive, roles.open, hover].map(gpui::Background::from);
    assert!(fills_at(&view, "r-0", false, cx).contains(&active), "a selected sidebar row while the sidebar has the keys: the selection role");
    assert!(fills_at(&view, "r-1", false, cx).contains(&inactive), "without the keys: the inactive selection role");
    assert!(fills_at(&view, "r-2", false, cx).contains(&active), "a selected schedule row: the same role");
    assert!(fills_at(&view, "r-3", true, cx).contains(&hover), "a schedule row under the pointer: the ghost hover");
    assert!(!fills_at(&view, "r-3", false, cx).contains(&hover), "and none once the pointer leaves");
    assert!(fills_at(&view, "r-4", false, cx).contains(&open), "a row under its open menu: the open role");
}
